use super::*;

#[test]
fn copy_populates_the_clipboard_without_changing_the_drawing() {
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let depth = app
        .history
        .get(&DocumentId(1))
        .map(|h| h.undo_depth())
        .unwrap_or(0);
    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(vec![ref_of(1)]),
    )
    .unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CopyToClipboard,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(session.clipboard_len(), 1);
    assert!(
        outcome.changes.is_none(),
        "copy must not write the database"
    );
    assert!(outcome
        .diagnostics
        .iter()
        .any(|d| d.code == "drawing.clipboard.copy"));
    assert_eq!(revision(&app), rev);
    assert_eq!(
        app.history
            .get(&DocumentId(1))
            .map(|h| h.undo_depth())
            .unwrap_or(0),
        depth,
        "copy must not add an undo step"
    );
}

#[test]
fn copy_with_an_empty_selection_is_refused() {
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CopyToClipboard,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(session.clipboard_len(), 0);
    assert_eq!(revision(&app), rev);
}

#[test]
fn copy_to_clipboard_is_allowed_in_viewer() {
    let (mut app, _) = drawing_app();
    let mut viewer = SessionState::new(DocumentId(1), AppMode::Viewer);
    execute(
        &mut app,
        &mut viewer,
        CommandId::Select,
        CommandPayload::Selection(vec![ref_of(1)]),
    )
    .unwrap();
    execute(
        &mut app,
        &mut viewer,
        CommandId::CopyToClipboard,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(viewer.clipboard_len(), 1, "COPYCLIP is read-only");
}

#[test]
fn cut_removes_in_one_undo_and_populates_the_clipboard() {
    let attributes = non_default_attributes();
    let mut app = single_line_app(Some(attributes.clone()));
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(vec![ref_of(1)]),
    )
    .unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CutToClipboard,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(session.clipboard_len(), 1);
    // The cut reports the cut (deletion into the clipboard), not the capture.
    assert_eq!(outcome.diagnostics[0].code, "drawing.clipboard.cut");
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(drawing.entity(EntityId(1)).is_none(), "cut deletes");
    assert_eq!(
        app.history
            .get(&DocumentId(1))
            .map(|h| h.undo_depth())
            .unwrap_or(0),
        1,
        "cut is exactly one undo step"
    );
    // The delete pruned the display state; undo restores it.
    assert_eq!(
        drawing.entity_render_attributes(EntityId(1)),
        cad_db::EntityRenderAttributes::default()
    );
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    let restored = drawing.entity(EntityId(1)).expect("restored");
    assert_eq!(restored.layer, LayerId(0));
    match &restored.geometry {
        SemanticGeometry::Line { start, end } => {
            assert!(start.x.abs() < 1e-9 && start.y.abs() < 1e-9, "{start:?}");
            assert!((end.x - 10.0).abs() < 1e-9 && end.y.abs() < 1e-9, "{end:?}");
        }
        other => panic!("expected a line, got {other:?}"),
    }
    assert_eq!(
        drawing.entity_render_attributes(EntityId(1)),
        attributes,
        "undo restores the render attributes after a cut"
    );
}

#[test]
fn cut_with_an_empty_selection_is_refused() {
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CutToClipboard,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    // Nothing captured and nothing deleted.
    assert_eq!(session.clipboard_len(), 0);
    assert_eq!(revision(&app), rev);
}

#[test]
fn cut_and_paste_are_refused_in_viewer() {
    let (mut app, _) = drawing_app();
    let mut viewer = SessionState::new(DocumentId(1), AppMode::Viewer);
    execute(
        &mut app,
        &mut viewer,
        CommandId::Select,
        CommandPayload::Selection(vec![ref_of(1)]),
    )
    .unwrap();
    for id in [CommandId::CutToClipboard, CommandId::PasteFromClipboard] {
        let err = execute(&mut app, &mut viewer, id, CommandPayload::None).unwrap_err();
        assert_eq!(err, CadError::PermissionDenied, "{id:?}");
    }
}

#[test]
fn paste_inserts_fresh_id_clones_at_original_coordinates_in_one_undo() {
    let (mut app, mut session) = drawing_app();
    let before = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count();
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
        CommandId::CopyToClipboard,
        CommandPayload::None,
    )
    .unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::PasteFromClipboard,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 1);
    let pasted_id = EntityId(outcome.objects[0].0);
    assert_ne!(pasted_id, EntityId(1), "paste must use a fresh id");
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        before + 1
    );
    let pasted = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(pasted_id)
        .unwrap();
    assert_eq!(pasted.layer, LayerId(0), "source layer preserved");
    match &pasted.geometry {
        SemanticGeometry::Line { start, end } => {
            assert!(start.x.abs() < 1e-9 && start.y.abs() < 1e-9, "{start:?}");
            assert!((end.x - 10.0).abs() < 1e-9 && end.y.abs() < 1e-9, "{end:?}");
        }
        other => panic!("expected a line, got {other:?}"),
    }
    // The clipboard survives a paste; one undo removes every pasted entity.
    assert_eq!(session.clipboard_len(), 1);
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(pasted_id)
        .is_none());
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        before
    );
}

#[test]
fn paste_of_two_clipboard_entities_is_one_undo_step() {
    let (mut app, mut session) = drawing_app();
    let before = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count();
    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(vec![ref_of(1), ref_of(3)]),
    )
    .unwrap();
    execute(
        &mut app,
        &mut session,
        CommandId::CopyToClipboard,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(session.clipboard_len(), 2);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::PasteFromClipboard,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 2);
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        before + 2
    );
    assert_eq!(
        app.history
            .get(&DocumentId(1))
            .map(|h| h.undo_depth())
            .unwrap_or(0),
        1,
        "the whole paste is one undo step"
    );
    // One undo removes both pasted entities.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    for object in &outcome.objects {
        assert!(app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(object.0))
            .is_none());
    }
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        before
    );
}

#[test]
fn paste_carries_render_attributes_through_undo_and_redo() {
    let attributes = non_default_attributes();
    let mut app = single_line_app(Some(attributes.clone()));
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
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
        CommandId::CopyToClipboard,
        CommandPayload::None,
    )
    .unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::PasteFromClipboard,
        CommandPayload::None,
    )
    .unwrap();
    let pasted_id = EntityId(outcome.objects[0].0);
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_render_attributes(pasted_id),
        attributes,
        "the pasted copy carries the source render attributes"
    );
    // Undo removes it; redo restores it with the attributes intact.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(pasted_id)
        .is_none());
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
            .entity_render_attributes(pasted_id),
        attributes,
        "redo re-applies the captured display state"
    );
}

#[test]
fn paste_with_an_empty_clipboard_is_refused() {
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::PasteFromClipboard,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
}
