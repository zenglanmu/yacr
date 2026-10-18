use super::*;

#[test]
fn open_bytes_failure_keeps_the_current_document() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let before = controller.drawing().unwrap().id();
    let garbage: Arc<[u8]> = Arc::from(vec![0u8, 1, 2, 3].into_boxed_slice());
    assert!(controller.open_bytes(garbage, "bad.dwg").is_err());
    assert_eq!(controller.drawing().unwrap().id(), before);
}

#[test]
fn opening_new_content_resets_history_and_session_state() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.create_line(p(0.0, 0.0), p(100.0, 0.0)).unwrap();
    assert!(controller.application.can_undo(&controller.document_id));

    controller.reset_for_new_content();
    assert!(!controller.application.can_undo(&controller.document_id));
    assert!(matches!(controller.session.tool, crate::ToolState::Idle));
    assert!(controller.session.selection.is_empty());
    assert!(controller.session.layer_overrides.is_empty());
}

#[test]
fn replacing_the_document_clears_the_in_process_clipboard() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let drawing = controller.drawing().unwrap();
    let entity = drawing.model_space()[0].to_owned();
    let display = drawing.entity_display_state(entity.id);
    controller.session.clipboard = vec![crate::ClipboardEntry { entity, display }];
    assert_eq!(controller.session.clipboard_len(), 1);

    // New content rebuilds the session; the same reset path runs on a
    // successful open, so a paste can never carry entities across documents.
    controller.new_blank_document().unwrap();
    assert_eq!(controller.session.clipboard_len(), 0);
}

#[test]
fn new_blank_document_has_layer_zero_and_no_extent() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    // Make the demo dirty so the content reset is observable.
    controller.create_line(p(0.0, 0.0), p(100.0, 0.0)).unwrap();
    assert!(controller.application.can_undo(&controller.document_id));

    let report = controller.new_blank_document().unwrap();
    assert_eq!(report.entities, 0);
    assert!(!report.has_extent);

    let drawing = controller.drawing().unwrap();
    assert_eq!(drawing.entity_count(), 0);
    assert!(drawing.bounds().is_none());

    let rows = controller.layer_rows().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "0");
    assert!(rows[0].database_visible);

    // Derived state is rebuilt for the new content.
    assert!(!controller.application.can_undo(&controller.document_id));
    assert!(controller.session.selection.is_empty());
    assert!(controller.session.layer_overrides.is_empty());
    assert!(matches!(controller.session.tool, crate::ToolState::Idle));
    assert_eq!(controller.document_name_hint, "untitled");
    assert!(controller.last_import_report.is_none());
}

#[test]
fn new_blank_document_identities_are_fresh() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let identity = |controller: &HostController| {
        controller
            .application
            .workspace
            .documents
            .get(&controller.document_id)
            .unwrap()
            .identity
            .clone()
    };
    controller.new_blank_document().unwrap();
    let first = identity(&controller);
    controller.new_blank_document().unwrap();
    let second = identity(&controller);
    assert_ne!(first, second);
    assert!(matches!(second, DocumentIdentity::Temporary(id) if id >= 2));
}

#[test]
fn fit_on_a_blank_drawing_is_a_documented_no_op() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.new_blank_document().unwrap();
    let generation = controller.session.generation;
    controller.fit().unwrap();
    assert_eq!(
        controller.session.generation, generation,
        "an empty drawing has nothing to frame; fit must not pretend to"
    );
}

#[test]
fn fit_command_reports_the_empty_diagnostic_only_when_nothing_is_framed() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let fit = |controller: &mut HostController| {
        controller
            .execute(Command {
                schema_version: 1,
                id: crate::CommandId::FitDrawing,
                document: controller.document_id,
                viewport: controller.viewport_id,
                payload: crate::CommandPayload::None,
            })
            .unwrap()
    };
    // Populated demo: a real fit, no empty diagnostic.
    assert!(fit(&mut controller).diagnostics.is_empty());

    controller.new_blank_document().unwrap();
    let outcome = fit(&mut controller);
    assert_eq!(outcome.diagnostics.len(), 1);
    assert_eq!(outcome.diagnostics[0].code, "fit.empty");
    // The host status mirrors the code for the presentation layer.
    assert_eq!(controller.status(), "fit.empty");
}

#[test]
fn new_blank_document_resets_the_viewport_camera_to_the_default() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    // Frame the demo, then move the camera so the reset is observable.
    controller.fit().unwrap();
    controller
        .application
        .workspace
        .viewports
        .get_mut(&controller.viewport_id)
        .unwrap()
        .camera
        .projection = crate::Projection::Orthographic { scale: 250.0 };

    controller.new_blank_document().unwrap();

    let camera = controller
        .application
        .workspace
        .viewports
        .get(&controller.viewport_id)
        .unwrap()
        .camera;
    assert_eq!(
        camera.target,
        p(0.0, 0.0),
        "New anchors the camera at origin"
    );
    assert_eq!(
        camera.projection,
        crate::Projection::Orthographic {
            scale: crate::camera::MIN_ORTHO_SCALE
        },
        "New uses the fresh-document default scale, not the old framing"
    );
}

#[test]
fn new_blank_document_supersedes_a_pending_open() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let garbage: Arc<[u8]> = Arc::from(vec![0u8, 1, 2, 3].into_boxed_slice());
    controller.begin_async_open(garbage, "pending.dwg");
    assert!(controller.pending_open_label.is_some());
    assert!(controller.async_open.is_some());
    assert!(controller.import_manager.is_running());

    controller.new_blank_document().unwrap();

    assert!(controller.pending_open_label.is_none());
    assert!(controller.pending_open_guard.is_none());
    assert!(controller.async_open.is_none());
    assert!(
        !controller.import_manager.is_running(),
        "New must release the superseded open job"
    );
}

#[test]
fn application_refuses_new_drawing_as_host_owned() {
    let mut app = Application::new();
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let result = app.execute(
        &mut session,
        Command {
            schema_version: 1,
            id: crate::CommandId::NewDrawing,
            document: DocumentId(1),
            viewport: ViewportId(1),
            payload: crate::CommandPayload::None,
        },
    );
    assert!(matches!(result, Err(CadError::Unsupported(_))));
}
