//! Selection reconciliation after an edit that removes entities.
//!
//! Real-host evidence: after `ERASE` of the selected entity the session
//! selection still held a ref to the deleted entity, so the status projection
//! reported a `status: unresolved` row and the render overlay raised a
//! `highlight.unresolved` "… is not present in the document" error. These tests
//! pin that an edit drops exactly the removed refs, keeps the rest, and leaves a
//! selection the status/overlay projections can resolve.

use super::*;

/// Assert the session selection is fully resolved: every ref names a real
/// entity, the property projection has no `status = unresolved` row, and the
/// render overlay produces no `highlight.unresolved` diagnostic (the real host's
/// "… is not present in the document" error).
fn assert_selection_resolves(app: &Application, session: &SessionState) {
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    for reference in session.selection.refs() {
        assert!(
            drawing.entity(reference.entity).is_some(),
            "selected entity {} must still exist in the document",
            reference.entity.0
        );
    }
    let properties = SelectionProperties::extract(drawing, &session.selection);
    assert!(
        !properties
            .rows
            .iter()
            .any(|row| row.key == "status" && row.value == "unresolved"),
        "selection must not report an `unresolved` row: {:?}",
        properties.rows
    );
    let inputs = crate::render_scene::OverlayInputs {
        selection: session.selection.clone(),
        ..Default::default()
    };
    let flags = crate::render_scene::selection_entity_flags(drawing, &inputs, DocumentId(1));
    assert!(
        flags.diagnostics.is_empty(),
        "selection overlay must have no diagnostic: {:?}",
        flags.diagnostics
    );
    assert!(
        !flags.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("is not present in the document")),
        "the real-host `not present in the document` error must not occur"
    );
}

#[test]
fn erase_of_one_selected_entity_of_two_leaves_the_other_selected() {
    // Two refs are selected; the edit removes exactly one of them, so the
    // surviving ref must stay selected and stay resolvable. A plain `ERASE`
    // removes the whole selection, so this uses a TRIM that deletes only its
    // target while the boundary ref (also selected) survives.
    let (mut app, mut session) = drawing_app();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    // Target line id 1, fully inside the closed square boundary id 60.
    b.insert_entity(ent(
        1,
        SemanticGeometry::Line {
            start: point(2.0, 2.0),
            end: point(4.0, 2.0),
        },
        0,
    ))
    .unwrap();
    b.insert_entity(ent(
        60,
        SemanticGeometry::Polyline {
            points: vec![
                point(0.0, 0.0),
                point(10.0, 0.0),
                point(10.0, 10.0),
                point(0.0, 10.0),
            ],
            bulges: vec![0.0, 0.0, 0.0, 0.0],
            closed: true,
        },
        1,
    ))
    .unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(b.finish().unwrap());

    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(vec![ref_of(1), ref_of(60)]),
    )
    .unwrap();
    assert_eq!(session.selection.len(), 2);

    execute(
        &mut app,
        &mut session,
        CommandId::TrimEntity,
        CommandPayload::Trim {
            target: ref_of(1),
            boundary: vec![ref_of(60)],
            pick_point: point(3.0, 2.0),
        },
    )
    .unwrap();

    assert!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(1))
            .is_none(),
        "the trim target is deleted"
    );
    assert_eq!(
        session.selection.len(),
        1,
        "only the removed ref is dropped"
    );
    assert!(
        session.selection.contains(&ref_of(60)),
        "the surviving ref stays selected"
    );
    assert_selection_resolves(&app, &session);
}

#[test]
fn erase_of_the_whole_selection_leaves_it_empty() {
    let (mut app, mut session) = drawing_app();
    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(vec![ref_of(1), ref_of(2)]),
    )
    .unwrap();
    execute(
        &mut app,
        &mut session,
        CommandId::EraseEntities,
        CommandPayload::None,
    )
    .unwrap();

    assert!(
        session.selection.is_empty(),
        "the erased refs must not linger in the selection"
    );
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(drawing.entity(EntityId(1)).is_none());
    assert!(drawing.entity(EntityId(2)).is_none());
    assert!(
        drawing.entity(EntityId(3)).is_some(),
        "an unselected entity must survive"
    );
    assert_selection_resolves(&app, &session);
}

#[test]
fn erase_of_one_selected_entity_of_a_three_entity_drawing_clears_the_selection() {
    // The document has several entities; only entity 1 is selected and erased,
    // so the selection must be empty while the unselected entities remain and
    // the status/overlay projections stay resolvable.
    let (mut app, mut session) = drawing_app();
    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(vec![ref_of(1)]),
    )
    .unwrap();
    execute(
        &mut app,
        &mut session,
        CommandId::EraseEntities,
        CommandPayload::None,
    )
    .unwrap();

    assert!(session.selection.is_empty());
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(drawing.entity(EntityId(1)).is_none());
    assert!(drawing.entity(EntityId(2)).is_some());
    assert!(drawing.entity(EntityId(3)).is_some());
    assert_selection_resolves(&app, &session);
}

#[test]
fn undo_of_a_create_drops_the_removed_ref_and_keeps_the_rest() {
    // Select an existing entity and a just-created one; undoing the create
    // removes only the created entity, so its ref is dropped and the other stays.
    let (mut app, mut session) = drawing_app();
    execute(
        &mut app,
        &mut session,
        CommandId::CreateLine,
        CommandPayload::Points(vec![point(0.0, 0.0), point(1.0, 1.0)]),
    )
    .unwrap();
    let created = app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .iter()
        .map(|entity| entity.id)
        .max()
        .unwrap();
    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(vec![ref_of(1), ref_of(created.0)]),
    )
    .unwrap();
    assert_eq!(session.selection.len(), 2);

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();

    assert!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(created)
            .is_none(),
        "undo removed the created entity"
    );
    assert_eq!(session.selection.len(), 1);
    assert!(
        session.selection.contains(&ref_of(1)),
        "the still-existing ref stays selected"
    );
    assert_selection_resolves(&app, &session);
}

#[test]
fn a_transform_keeps_a_still_resolving_selection() {
    // MOVE transforms in place (same ids) and must not disturb a selection whose
    // refs still resolve — reconciliation drops only removed refs.
    let (mut app, mut session) = drawing_app();
    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(vec![ref_of(1), ref_of(2)]),
    )
    .unwrap();
    execute(
        &mut app,
        &mut session,
        CommandId::MoveEntities,
        CommandPayload::Move {
            refs: vec![ref_of(1), ref_of(2)],
            delta: p3(5.0, 0.0, 0.0),
        },
    )
    .unwrap();
    assert_eq!(
        session.selection.len(),
        2,
        "a transform keeps the selection"
    );
    assert_selection_resolves(&app, &session);
}

#[test]
fn a_copy_keeps_the_source_selection_and_does_not_select_the_copies() {
    // COPY inserts fresh ids and keeps the sources, so the selection keeps the
    // original refs and is never retargeted onto the copies.
    let (mut app, mut session) = drawing_app();
    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(vec![ref_of(1)]),
    )
    .unwrap();
    execute(
        &mut app,
        &mut session,
        CommandId::CopyEntities,
        CommandPayload::Copy {
            refs: vec![ref_of(1)],
            delta: p3(1.0, 1.0, 0.0),
        },
    )
    .unwrap();
    assert_eq!(session.selection.len(), 1);
    assert!(session.selection.contains(&ref_of(1)));
    assert_selection_resolves(&app, &session);
}
