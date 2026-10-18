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

#[test]
fn a_foreign_document_ref_is_not_retained_by_a_coinciding_entity_id() {
    // The open drawing has an entity 2, so an entity-only prune would keep a
    // ref whose *document* is a different one. Reconciliation keys on
    // `(document, entity)`, so the foreign ref is dropped while the local ref
    // survives and no ref is silently attributed to a document it is not from.
    let (mut app, mut session) = drawing_app();
    let mut foreign = ref_of(2);
    foreign.document = DocumentId(2);
    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(vec![ref_of(1), foreign]),
    )
    .unwrap();
    assert_eq!(session.selection.len(), 2);

    // A transform of a still-resolving local ref runs reconciliation.
    execute(
        &mut app,
        &mut session,
        CommandId::MoveEntities,
        CommandPayload::Move {
            refs: vec![ref_of(1)],
            delta: p3(1.0, 0.0, 0.0),
        },
    )
    .unwrap();

    assert!(
        !session
            .selection
            .refs()
            .iter()
            .any(|reference| reference.document != DocumentId(1)),
        "a foreign-document ref must not survive reconciliation: {:?}",
        session.selection.refs()
    );
    assert!(
        session.selection.contains(&ref_of(1)),
        "the local ref survives"
    );
    assert_eq!(session.selection.len(), 1);
    assert_selection_resolves(&app, &session);
}

#[test]
fn cutting_the_selection_drops_every_removed_ref() {
    // CUT deletes the selected entities and moves them to the clipboard: every
    // removed ref must be dropped, leaving no `unresolved` row or dangling ref.
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
        CommandId::CutToClipboard,
        CommandPayload::None,
    )
    .unwrap();

    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(
        drawing.entity(EntityId(1)).is_none(),
        "cut deletes entity 1"
    );
    assert!(
        drawing.entity(EntityId(2)).is_none(),
        "cut deletes entity 2"
    );
    assert_eq!(
        session.clipboard_len(),
        2,
        "both cut entities are on the clipboard"
    );
    assert!(
        session.selection.is_empty(),
        "every cut ref is dropped from the selection"
    );
    assert_selection_resolves(&app, &session);
}

#[test]
fn pasting_after_a_cut_does_not_retarget_the_emptied_selection() {
    // PASTE inserts fresh ids for the clipboard entities (never reusing the cut
    // ids), so after a cut the emptied selection must stay empty: it must not
    // silently retarget onto the new ids.
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
        CommandId::CutToClipboard,
        CommandPayload::None,
    )
    .unwrap();
    assert!(session.selection.is_empty());
    let before = app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .len();

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::PasteFromClipboard,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 1, "one pasted copy");

    let pasted = EntityId(outcome.objects[0].0);
    assert_ne!(pasted, EntityId(1), "the paste is a fresh id, not entity 1");
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(drawing.entity(pasted).is_some(), "the pasted entity exists");
    assert_eq!(
        drawing.model_space().len(),
        before + 1,
        "the paste adds exactly one entity"
    );
    assert!(
        session.selection.is_empty(),
        "paste must not silently select the new ids"
    );
    assert_selection_resolves(&app, &session);
}

#[test]
fn redo_of_a_delete_drops_the_reselected_refs() {
    // REDO of an ERASE removes entities again (application.rs reconciles REDO
    // like the original edit), so a selection made after the UNDO that pointed
    // at the now-restored entities must be dropped on the redo.
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
    assert!(session.selection.is_empty(), "the erase dropped both refs");

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
            .entity(EntityId(1))
            .is_some(),
        "undo restored the erased entity"
    );

    // Re-select the (restored) entities, then redo the delete.
    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(vec![ref_of(1), ref_of(2)]),
    )
    .unwrap();
    assert_eq!(session.selection.len(), 2);
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();

    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(
        drawing.entity(EntityId(1)).is_none(),
        "redo deletes entity 1"
    );
    assert!(
        drawing.entity(EntityId(2)).is_none(),
        "redo deletes entity 2"
    );
    assert!(
        session.selection.is_empty(),
        "a redone delete drops the reselected refs"
    );
    assert_selection_resolves(&app, &session);
}
