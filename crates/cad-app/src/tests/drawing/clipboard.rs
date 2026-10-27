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
        CommandPayload::PasteClip {
            insertion: point(0.0, 0.0),
        },
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
        CommandPayload::PasteClip {
            insertion: point(0.0, 0.0),
        },
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
        CommandPayload::PasteClip {
            insertion: point(0.0, 0.0),
        },
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

#[test]
fn copybase_captures_entries_and_the_base_point_without_writing() {
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let depth = app
        .history
        .get(&DocumentId(1))
        .map(|h| h.undo_depth())
        .unwrap_or(0);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CopyToClipboardBase,
        CommandPayload::CopyBase {
            refs: vec![ref_of(1)],
            base: p3(2.0, 3.0, 0.0),
        },
    )
    .unwrap();
    assert_eq!(session.clipboard_len(), 1, "COPYBASE captures the refs");
    assert_eq!(
        session.clipboard_base,
        Some(p3(2.0, 3.0, 0.0)),
        "COPYBASE records the picked base point"
    );
    assert!(
        outcome.changes.is_none(),
        "COPYBASE must not write the database"
    );
    assert!(outcome
        .diagnostics
        .iter()
        .any(|d| d.code == "drawing.clipboard.copybase"));
    assert_eq!(revision(&app), rev);
    assert_eq!(
        app.history
            .get(&DocumentId(1))
            .map(|h| h.undo_depth())
            .unwrap_or(0),
        depth,
        "COPYBASE is read-only: no undo step"
    );
}

#[test]
fn copybase_is_allowed_in_viewer() {
    let (mut app, _) = drawing_app();
    let mut viewer = SessionState::new(DocumentId(1), AppMode::Viewer);
    execute(
        &mut app,
        &mut viewer,
        CommandId::CopyToClipboardBase,
        CommandPayload::CopyBase {
            refs: vec![ref_of(2)],
            base: p3(0.0, 0.0, 0.0),
        },
    )
    .unwrap();
    assert_eq!(viewer.clipboard_len(), 1, "COPYBASE is a read-only capture");
    assert_eq!(viewer.clipboard_base, Some(p3(0.0, 0.0, 0.0)));
}

#[test]
fn copybase_refuses_an_empty_selection_a_wrong_payload_and_a_non_finite_base() {
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    // No refs: an explicit refusal, never an empty clipboard success.
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CopyToClipboardBase,
        CommandPayload::CopyBase {
            refs: Vec::new(),
            base: p3(0.0, 0.0, 0.0),
        },
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    // The bare selector's payload (`None`) is not this command's input.
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CopyToClipboardBase,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    // A non-finite base is refused before anything is captured.
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CopyToClipboardBase,
        CommandPayload::CopyBase {
            refs: vec![ref_of(1)],
            base: Point3 {
                x: f64::NAN,
                y: 0.0,
                z: 0.0,
            },
        },
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    assert_eq!(session.clipboard_len(), 0);
    assert_eq!(session.clipboard_base, None);
    assert_eq!(revision(&app), rev);
}

#[test]
fn cutbase_deletes_the_payload_refs_in_one_undo_and_commits_entries_and_base() {
    let attributes = non_default_attributes();
    let mut app = single_line_app(Some(attributes.clone()));
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CutToClipboardBase,
        CommandPayload::CutBase {
            refs: vec![ref_of(1)],
            base: p3(1.0, 2.0, 0.0),
        },
    )
    .unwrap();
    assert_eq!(outcome.diagnostics[0].code, "drawing.clipboard.cutbase");
    assert_eq!(session.clipboard_len(), 1);
    assert_eq!(
        session.clipboard_base,
        Some(p3(1.0, 2.0, 0.0)),
        "CUTBASE commits the base with the entries"
    );
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(
        drawing.entity(EntityId(1)).is_none(),
        "CUTBASE deletes the payload refs"
    );
    assert_eq!(
        drawing.entity_render_attributes(EntityId(1)),
        cad_db::EntityRenderAttributes::default(),
        "the delete pruned the display state"
    );
    assert_eq!(
        app.history
            .get(&DocumentId(1))
            .map(|h| h.undo_depth())
            .unwrap_or(0),
        1,
        "the capture + delete is exactly one undo step"
    );
    // One undo restores the entity, its geometry and its display state.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    let restored = drawing.entity(EntityId(1)).expect("restored");
    match &restored.geometry {
        SemanticGeometry::Line { start, end } => {
            assert!(start.x.abs() < 1e-9 && start.y.abs() < 1e-9, "{start:?}");
            assert!((end.x - 10.0).abs() < 1e-9 && end.y.abs() < 1e-9, "{end:?}");
        }
        other => panic!("expected a line, got {other:?}"),
    }
    assert_eq!(drawing.entity_render_attributes(EntityId(1)), attributes);
}

#[test]
fn cutbase_deletes_the_payload_refs_not_the_session_selection() {
    // The payload is the authority: a session selection that no longer matches the
    // snapshot the capture started with can neither add nor retarget a delete.
    let (mut app, mut session) = drawing_app();
    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(vec![ref_of(2)]),
    )
    .unwrap();
    execute(
        &mut app,
        &mut session,
        CommandId::CutToClipboardBase,
        CommandPayload::CutBase {
            refs: vec![ref_of(1)],
            base: p3(0.0, 0.0, 0.0),
        },
    )
    .unwrap();
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(drawing.entity(EntityId(1)).is_none(), "payload ref deleted");
    assert!(
        drawing.entity(EntityId(2)).is_some(),
        "the unrelated selection survives"
    );
    assert_eq!(session.clipboard_len(), 1);
    assert!(session.selection.contains(&ref_of(2)));
}

#[test]
fn cutbase_is_refused_in_viewer_and_with_an_empty_selection() {
    let (mut app, _) = drawing_app();
    let mut viewer = SessionState::new(DocumentId(1), AppMode::Viewer);
    let err = execute(
        &mut app,
        &mut viewer,
        CommandId::CutToClipboardBase,
        CommandPayload::CutBase {
            refs: vec![ref_of(1)],
            base: p3(0.0, 0.0, 0.0),
        },
    )
    .unwrap_err();
    assert_eq!(err, CadError::PermissionDenied);
    assert_eq!(viewer.clipboard_len(), 0);

    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CutToClipboardBase,
        CommandPayload::CutBase {
            refs: Vec::new(),
            base: p3(0.0, 0.0, 0.0),
        },
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    assert_eq!(session.clipboard_len(), 0);
    assert_eq!(session.clipboard_base, None);
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn cutbase_keeps_the_previous_clipboard_when_a_ref_is_unaddressable() {
    // "Commit only after the delete succeeded": a batch containing a ref that does
    // not resolve refuses the whole cut before any write, and the previous
    // clipboard (entries **and** base) survives untouched.
    let (mut app, mut session) = drawing_app();
    execute(
        &mut app,
        &mut session,
        CommandId::CopyToClipboardBase,
        CommandPayload::CopyBase {
            refs: vec![ref_of(2)],
            base: p3(9.0, 9.0, 0.0),
        },
    )
    .unwrap();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CutToClipboardBase,
        CommandPayload::CutBase {
            refs: vec![ref_of(1), ref_of(999)],
            base: p3(0.0, 0.0, 0.0),
        },
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(
        drawing.entity(EntityId(1)).is_some(),
        "nothing is deleted when the batch refuses"
    );
    assert_eq!(revision(&app), rev);
    assert_eq!(session.clipboard_len(), 1);
    assert_eq!(
        session.clipboard_base,
        Some(p3(9.0, 9.0, 0.0)),
        "a refused cut cannot replace the clipboard"
    );
    assert_eq!(
        app.history
            .get(&DocumentId(1))
            .map(|h| h.undo_depth())
            .unwrap_or(0),
        0,
        "no undo entry for a refused cut"
    );
}

#[test]
fn pasteclip_after_copybase_translates_by_insertion_minus_base() {
    // The one translation rule of PASTECLIP: `insertion − clipboard base`.
    let attributes = non_default_attributes();
    let mut app = single_line_app(Some(attributes.clone()));
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    execute(
        &mut app,
        &mut session,
        CommandId::CopyToClipboardBase,
        CommandPayload::CopyBase {
            refs: vec![ref_of(1)],
            base: p3(2.0, 3.0, 0.0),
        },
    )
    .unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::PasteFromClipboard,
        CommandPayload::PasteClip {
            insertion: p3(7.0, 11.0, 0.0),
        },
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 1);
    let pasted_id = EntityId(outcome.objects[0].0);
    assert_ne!(pasted_id, EntityId(1), "paste must use a fresh id");
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    let pasted = drawing.entity(pasted_id).expect("pasted entity");
    // delta = (7 − 2, 11 − 3) = (5, 8); the source line is (0,0)→(10,0).
    match &pasted.geometry {
        SemanticGeometry::Line { start, end } => {
            assert!((start.x - 5.0).abs() < 1e-9, "{start:?}");
            assert!((start.y - 8.0).abs() < 1e-9, "{start:?}");
            assert!((end.x - 15.0).abs() < 1e-9, "{end:?}");
            assert!((end.y - 8.0).abs() < 1e-9, "{end:?}");
        }
        other => panic!("expected a line, got {other:?}"),
    }
    assert_eq!(pasted.layer, LayerId(0), "source layer preserved");
    assert_eq!(pasted.space, SpaceId::Model, "source space preserved");
    assert_eq!(
        drawing.entity_render_attributes(pasted_id),
        attributes,
        "the captured display state travels with the translated copy"
    );
    assert_eq!(
        app.history
            .get(&DocumentId(1))
            .map(|h| h.undo_depth())
            .unwrap_or(0),
        1,
        "the whole paste is one undo step"
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
        .entity(pasted_id)
        .is_none());
    assert_eq!(session.clipboard_len(), 1, "the clipboard survives a paste");
    assert_eq!(session.clipboard_base, Some(p3(2.0, 3.0, 0.0)));
}

#[test]
fn pasteclip_after_copyclip_translates_by_the_insertion_point_itself() {
    // COPYCLIP records no base, so the base is the world origin.
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
        CommandId::CopyToClipboard,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        session.clipboard_base, None,
        "COPYCLIP must not fabricate a base"
    );
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::PasteFromClipboard,
        CommandPayload::PasteClip {
            insertion: p3(3.0, 4.0, 0.0),
        },
    )
    .unwrap();
    let pasted_id = EntityId(outcome.objects[0].0);
    let pasted = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(pasted_id)
        .expect("pasted entity");
    match &pasted.geometry {
        SemanticGeometry::Line { start, end } => {
            assert!((start.x - 3.0).abs() < 1e-9, "{start:?}");
            assert!((start.y - 4.0).abs() < 1e-9, "{start:?}");
            assert!((end.x - 13.0).abs() < 1e-9, "{end:?}");
            assert!((end.y - 4.0).abs() < 1e-9, "{end:?}");
        }
        other => panic!("expected a line, got {other:?}"),
    }
}

#[test]
fn pasteorig_ignores_the_base_and_keeps_the_original_coordinates() {
    // PASTEORIG is the pre-existing PASTECLIP behaviour under its own name: the
    // base is retained on the clipboard but deliberately not applied.
    let mut app = single_line_app(None);
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    execute(
        &mut app,
        &mut session,
        CommandId::CopyToClipboardBase,
        CommandPayload::CopyBase {
            refs: vec![ref_of(1)],
            base: p3(2.0, 3.0, 0.0),
        },
    )
    .unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::PasteOriginalCoordinates,
        CommandPayload::None,
    )
    .unwrap();
    let pasted_id = EntityId(outcome.objects[0].0);
    assert_ne!(pasted_id, EntityId(1));
    let pasted = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(pasted_id)
        .expect("pasted entity");
    match &pasted.geometry {
        SemanticGeometry::Line { start, end } => {
            assert!(start.x.abs() < 1e-9 && start.y.abs() < 1e-9, "{start:?}");
            assert!((end.x - 10.0).abs() < 1e-9 && end.y.abs() < 1e-9, "{end:?}");
        }
        other => panic!("expected a line, got {other:?}"),
    }
    assert_eq!(
        session.clipboard_base,
        Some(p3(2.0, 3.0, 0.0)),
        "PASTEORIG ignores the base without clearing it"
    );
    assert_eq!(
        app.history
            .get(&DocumentId(1))
            .map(|h| h.undo_depth())
            .unwrap_or(0),
        1
    );
}

#[test]
fn the_paste_pair_refuses_an_empty_clipboard_and_a_wrong_payload() {
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    // Empty clipboard: both pastes refuse explicitly (never an empty success).
    let err = execute(
        &mut app,
        &mut session,
        CommandId::PasteFromClipboard,
        CommandPayload::PasteClip {
            insertion: p3(1.0, 1.0, 0.0),
        },
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    let err = execute(
        &mut app,
        &mut session,
        CommandId::PasteOriginalCoordinates,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    assert_eq!(revision(&app), rev);

    // Wrong payloads, also refused before any document read.
    let err = execute(
        &mut app,
        &mut session,
        CommandId::PasteFromClipboard,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    let err = execute(
        &mut app,
        &mut session,
        CommandId::PasteOriginalCoordinates,
        CommandPayload::PasteClip {
            insertion: p3(0.0, 0.0, 0.0),
        },
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
}

#[test]
fn pasteclip_refuses_a_non_finite_insertion_point() {
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
        CommandId::CopyToClipboard,
        CommandPayload::None,
    )
    .unwrap();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::PasteFromClipboard,
        CommandPayload::PasteClip {
            insertion: Point3 {
                x: 0.0,
                y: f64::INFINITY,
                z: 0.0,
            },
        },
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    assert_eq!(revision(&app), rev);
    assert_eq!(session.clipboard_len(), 1);
}
