//! Undo/redo history and ChangeSet assertions for drawing edits.

use super::*;

#[test]
fn move_lines_updates_geometry_and_is_undoable_in_one_step() {
    let (mut app, mut session) = drawing_app();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MoveEntities,
        CommandPayload::Move {
            refs: vec![ref_of(1)],
            delta: p3(2.0, 3.0, 0.0),
        },
    )
    .unwrap();
    let cs = outcome.changes.expect("change set");
    assert_eq!(cs.changes.len(), 1, "one transaction for the selection");
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .unwrap();
    match &entity.geometry {
        SemanticGeometry::Line { start, end } => {
            assert_eq!(*start, point(2.0, 3.0));
            assert_eq!(*end, point(12.0, 3.0));
        }
        other => panic!("expected a line, got {other:?}"),
    }
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .unwrap()
        .geometry
    {
        SemanticGeometry::Line { start, end } => {
            assert_eq!(*start, point(0.0, 0.0));
            assert_eq!(*end, point(10.0, 0.0));
        }
        other => panic!("expected a line, got {other:?}"),
    }
}

#[test]
fn move_multiple_entities_is_one_undo_step() {
    let (mut app, mut session) = drawing_app();
    execute(
        &mut app,
        &mut session,
        CommandId::MoveEntities,
        CommandPayload::Move {
            refs: vec![ref_of(1), ref_of(2)],
            delta: p3(1.0, 0.0, 0.0),
        },
    )
    .unwrap();
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .is_some());
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(2))
        .is_some());
}

#[test]
fn drawing_undo_redo_round_trips() {
    let (mut app, mut session) = drawing_app();
    let before = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count();
    // One drawing create, then a second, then undo both in reverse order.
    execute(
        &mut app,
        &mut session,
        CommandId::CreateLine,
        CommandPayload::Points(vec![point(0.0, 0.0), point(1.0, 1.0)]),
    )
    .unwrap();
    let oid = app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .iter()
        .map(|e| e.id)
        .max()
        .unwrap();
    execute(
        &mut app,
        &mut session,
        CommandId::CreateCircle,
        CommandPayload::Points(vec![point(5.0, 5.0), point(6.0, 5.0)]),
    )
    .unwrap();
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        before + 2
    );

    // Undo the circle first, then the line create.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        before + 1
    );
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(oid)
        .is_none());
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        before
    );

    // Redo restores them in order.
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(oid)
        .is_some());
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        before + 2
    );
}

#[test]
fn two_creates_are_two_transactions_not_one_or_zero() {
    let (mut app, mut session) = drawing_app();
    for _ in 0..2 {
        execute(
            &mut app,
            &mut session,
            CommandId::CreateLine,
            CommandPayload::Points(vec![point(0.0, 0.0), point(1.0, 0.0)]),
        )
        .unwrap();
    }
    // Each command is exactly one transaction and one undo step: never a
    // merged pair (one step) and never a silent empty success (zero steps).
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 2);
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .model_space()
            .len(),
        6,
        "the five seed model entities plus two created lines"
    );
}

#[test]
fn erase_selection_deletes_exactly_those_entities_in_one_undo_step() {
    let (mut app, mut session) = drawing_app();
    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(vec![ref_of(1), ref_of(2)]),
    )
    .unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::EraseEntities,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(outcome.changes.as_ref().unwrap().changes.len(), 2);
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(drawing.entity(EntityId(1)).is_none());
    assert!(drawing.entity(EntityId(2)).is_none());
    assert!(
        drawing.entity(EntityId(3)).is_some(),
        "an unselected entity must stay"
    );
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    // One undo restores both erased entities.
    let undo = execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(drawing.entity(EntityId(1)).is_some());
    assert!(drawing.entity(EntityId(2)).is_some());

    // The undo restores each entity's captured display state (render attributes
    // + visibility membership) **inside** the same commit: exactly one revision
    // bump, and the ChangeSet names every restored entity with STYLE so its
    // representation is rebuilt.
    let undo_changes = undo.changes.as_ref().expect("undo publishes a change set");
    assert_eq!(
        undo_changes.after.0,
        undo_changes.before.0 + 1,
        "one undo is exactly one revision bump"
    );
    assert_eq!(undo_changes.after, revision(&app));
    for id in [1u128, 2] {
        let mask = undo_changes
            .changes
            .iter()
            .find_map(|change| match change {
                cad_db::ObjectChange::Update(object, mask) if object.0 == id => Some(*mask),
                _ => None,
            })
            .unwrap_or_else(|| panic!("undo must report entity {id}: {:?}", undo_changes.changes));
        assert!(
            mask.contains(cad_db::ChangeMask::STYLE),
            "entity {id} display restore must carry STYLE, got {mask:?}"
        );
    }
}

#[test]
fn copy_selection_duplicates_with_translation_in_one_undo_step() {
    let (mut app, mut session) = drawing_app();
    let before_count = app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .len();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CopyEntities,
        CommandPayload::Copy {
            refs: vec![ref_of(1), ref_of(2)],
            delta: p3(5.0, 1.0, 0.0),
        },
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 2, "one copy per selected entity");

    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(drawing.model_space().len(), before_count + 2);
    // The originals are untouched.
    match &drawing.entity(EntityId(1)).unwrap().geometry {
        SemanticGeometry::Line { start, end } => {
            assert_eq!(*start, point(0.0, 0.0));
            assert_eq!(*end, point(10.0, 0.0));
        }
        other => panic!("expected a line, got {other:?}"),
    }
    // The copies are new ids with translated geometry, same layer/space.
    let copy_line = drawing
        .entity(EntityId(outcome.objects[0].0))
        .expect("copy 1");
    match &copy_line.geometry {
        SemanticGeometry::Line { start, end } => {
            assert_eq!(*start, point(5.0, 1.0));
            assert_eq!(*end, point(15.0, 1.0));
        }
        other => panic!("expected a line, got {other:?}"),
    }
    assert_eq!(copy_line.layer, LayerId(0));
    assert_eq!(copy_line.space, SpaceId::Model);
    let copy_arc = drawing
        .entity(EntityId(outcome.objects[1].0))
        .expect("copy 2");
    match &copy_arc.geometry {
        SemanticGeometry::Arc { center, .. } => assert_eq!(*center, point(5.0, 1.0)),
        other => panic!("expected an arc, got {other:?}"),
    }
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    // One undo removes every copy and keeps the originals.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(drawing.model_space().len(), before_count);
    assert!(drawing.entity(EntityId(outcome.objects[0].0)).is_none());
    assert!(drawing.entity(EntityId(outcome.objects[1].0)).is_none());
    assert!(drawing.entity(EntityId(1)).is_some());
}

#[test]
fn an_invalid_create_writes_no_revision_and_no_undo_entry() {
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreateLine,
        CommandPayload::Points(vec![point(0.0, 0.0), point(f64::NAN, 1.0)]),
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("non-finite"),
        "the database refuses the invalid row by name: {err}"
    );
    assert_eq!(revision(&app), rev, "revision unchanged");
    // The history map is populated lazily on the first committed edit, so a
    // refused create leaves no entry at all (and certainly no undo depth).
    assert_eq!(
        app.history
            .get(&DocumentId(1))
            .map_or(0, |history| history.undo_depth()),
        0,
        "a refused create records no undo step"
    );
    assert!(!app.can_undo(&DocumentId(1)));
}
