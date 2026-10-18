use super::databases::p;
use super::*;

#[test]
fn demo_document_has_geometry_and_bounds() {
    let controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let drawing = controller.drawing().unwrap();
    assert!(drawing.entity_count() >= 6);
    let (min, max) = drawing.bounds().unwrap();
    assert!(max.x - min.x > 0.0);
}

#[test]
fn line_capture_commits_once_and_invalid_confirm_keeps_capture() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let before = controller.drawing().unwrap();
    let count = before.entity_count();
    let revision = before.revision();

    controller.begin_draw_tool(DrawToolKind::Line).unwrap();
    controller.capture_tool_point(p(10.0, 20.0)).unwrap();
    assert!(controller.confirm_draw().is_err());
    let preview = controller.session.draw_preview().unwrap();
    assert_eq!(preview.points, vec![p(10.0, 20.0)]);

    controller.capture_tool_point(p(40.0, 20.0)).unwrap();
    let outcome = controller.confirm_draw().unwrap();
    assert_eq!(outcome.objects.len(), 1);
    let after = controller.drawing().unwrap();
    assert_eq!(after.entity_count(), count + 1);
    assert_ne!(after.revision(), revision);
    assert!(controller.application.can_undo(&controller.document_id));
    assert!(matches!(controller.session.tool, crate::ToolState::Idle));
}

#[test]
fn failed_geometry_confirmation_preserves_capture_and_cancel_writes_nothing() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let revision = controller.drawing().unwrap().revision();
    controller.begin_draw_tool(DrawToolKind::Circle).unwrap();
    controller.capture_tool_point(p(5.0, 5.0)).unwrap();
    controller.capture_tool_point(p(5.0, 5.0)).unwrap();
    assert!(controller.confirm_draw().is_err());
    assert!(controller.session.draw_preview().unwrap().ready);
    assert_eq!(controller.drawing().unwrap().revision(), revision);

    let generation = controller.session.tool_generation;
    controller.cancel_tool().unwrap();
    assert_eq!(controller.drawing().unwrap().revision(), revision);
    assert!(matches!(controller.session.tool, crate::ToolState::Idle));
    assert_eq!(controller.session.tool_generation, generation + 1);
}

#[test]
fn draw_capture_uses_start_selection_snapshot_and_cursor_keeps_generation() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let entity = controller.drawing().unwrap().model_space()[0].id;
    let reference = SelectionRef {
        document: controller.document_id,
        entity,
        instance: InstancePath(Vec::new()),
        sub_element: None,
    };
    controller.session.selection.replace([reference.clone()]);
    controller.begin_draw_tool(DrawToolKind::Move).unwrap();
    let generation = controller.session.tool_generation;
    controller.session.selection.clear();
    assert_eq!(
        controller.session.draw_selection_snapshot(),
        Some(&[reference][..])
    );
    assert_eq!(
        controller.session.draw_preview().unwrap().selection_count,
        1
    );
    controller.move_tool_cursor(p(1.0, 2.0)).unwrap();
    controller.move_tool_cursor(p(3.0, 4.0)).unwrap();
    assert_eq!(controller.session.tool_generation, generation);
}

#[test]
fn select_and_select_all_invalidate_and_discard_draw_capture_without_writes() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let revision = controller.drawing().unwrap().revision();
    controller.begin_draw_tool(DrawToolKind::Line).unwrap();
    controller.capture_tool_point(p(1.0, 2.0)).unwrap();
    let before_select = controller.session.tool_generation;
    controller
        .application
        .execute(
            &mut controller.session,
            Command {
                schema_version: 1,
                id: crate::CommandId::Select,
                document: controller.document_id,
                viewport: controller.viewport_id,
                payload: crate::CommandPayload::Selection(Vec::new()),
            },
        )
        .unwrap();
    assert_eq!(controller.session.tool_generation, before_select + 1);
    assert!(controller.session.draw_preview().is_none());
    assert!(matches!(
        controller.session.tool,
        crate::ToolState::Selecting
    ));
    assert!(controller.session.selection.is_empty());

    controller.begin_draw_tool(DrawToolKind::Line).unwrap();
    controller.capture_tool_point(p(5.0, 6.0)).unwrap();
    let before_select_all = controller.session.tool_generation;
    controller
        .application
        .execute(
            &mut controller.session,
            Command {
                schema_version: 1,
                id: crate::CommandId::SelectAll,
                document: controller.document_id,
                viewport: controller.viewport_id,
                payload: crate::CommandPayload::None,
            },
        )
        .unwrap();
    assert_eq!(controller.session.tool_generation, before_select_all + 1);
    assert!(controller.session.draw_preview().is_none());
    assert!(matches!(
        controller.session.tool,
        crate::ToolState::Selecting
    ));
    assert!(!controller.session.selection.is_empty());
    assert_eq!(controller.drawing().unwrap().revision(), revision);
}

#[test]
fn tool_switches_are_mutually_exclusive_and_repeated_cancel_is_idempotent() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.begin_draw_tool(DrawToolKind::Line).unwrap();
    controller.capture_tool_point(p(1.0, 2.0)).unwrap();
    let draw_generation = controller.session.tool_generation;
    controller
        .session
        .begin_measurement(crate::MeasurementToolKind::Distance)
        .unwrap();
    assert!(controller.session.tool_generation > draw_generation);
    assert!(controller.session.draw_preview().is_none());
    assert!(controller.session.measurement_preview().is_some());

    let measurement_generation = controller.session.tool_generation;
    controller.begin_draw_tool(DrawToolKind::Line).unwrap();
    assert!(controller.session.tool_generation > measurement_generation);
    assert!(controller.session.measurement_preview().is_none());
    assert!(controller.session.draw_preview().is_some());

    let generation = controller.session.tool_generation;
    controller.cancel_tool().unwrap();
    assert_eq!(controller.session.tool_generation, generation + 1);
    controller.cancel_tool().unwrap();
    assert_eq!(controller.session.tool_generation, generation + 1);
}

#[test]
fn mode_and_document_replacement_invalidate_active_tool_generation() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.begin_draw_tool(DrawToolKind::Line).unwrap();
    let generation = controller.session.tool_generation;
    controller.session.switch_mode(AppMode::Viewer).unwrap();
    assert!(controller.session.tool_generation > generation);
    assert!(matches!(controller.session.tool, crate::ToolState::Idle));

    controller.begin_draw_tool(DrawToolKind::Line).unwrap_err();
    controller.session.switch_mode(AppMode::Work).unwrap();
    controller.begin_draw_tool(DrawToolKind::Line).unwrap();
    let generation = controller.session.tool_generation;
    controller.reset_for_new_content();
    assert!(controller.session.tool_generation > generation);
    assert!(matches!(controller.session.tool, crate::ToolState::Idle));
}

#[test]
fn undo_keeps_capture_but_invalidated_selection_fails_confirmation_explicitly() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller
        .create_line(p(100.0, 100.0), p(200.0, 100.0))
        .unwrap();
    let entity = controller
        .drawing()
        .unwrap()
        .model_space()
        .last()
        .unwrap()
        .id;
    let reference = SelectionRef {
        document: controller.document_id,
        entity,
        instance: InstancePath(Vec::new()),
        sub_element: None,
    };
    controller.session.selection.replace([reference.clone()]);
    controller.begin_draw_tool(DrawToolKind::Move).unwrap();
    controller.capture_tool_point(p(0.0, 0.0)).unwrap();
    controller.capture_tool_point(p(10.0, 0.0)).unwrap();

    controller
        .application
        .execute(
            &mut controller.session,
            Command {
                schema_version: 1,
                id: crate::CommandId::Undo,
                document: controller.document_id,
                viewport: controller.viewport_id,
                payload: crate::CommandPayload::None,
            },
        )
        .unwrap();
    assert!(controller.session.draw_preview().is_some());
    assert_eq!(
        controller.session.draw_selection_snapshot(),
        Some(&[reference][..])
    );
    assert!(controller.confirm_draw().is_err());
    assert!(controller.session.draw_preview().is_some());
}

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

#[test]
fn measurement_preview_getter_is_pure_and_reports_the_active_tool() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    // No tool: no preview and no command dispatched.
    assert!(controller.measurement_preview().is_none());

    controller
        .execute(Command {
            schema_version: 1,
            id: crate::CommandId::Measure,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: crate::CommandPayload::None,
        })
        .unwrap();
    let preview = controller.measurement_preview().expect("active preview");
    assert_eq!(preview.kind, crate::MeasurementToolKind::Distance);
    assert!(preview.points.is_empty());

    // Reading the preview twice is stable: no points are captured.
    let again = controller.measurement_preview().unwrap();
    assert_eq!(again.points.len(), preview.points.len());
    assert_eq!(controller.session.measurement_preview().unwrap(), again);
}

#[test]
fn unit_label_reports_the_document_unit_context() {
    let controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    // The demo document has unknown units: never silently "mm".
    assert_eq!(controller.unit_label(), "drawing units");
}

#[test]
fn layer_rows_come_from_the_demo_database_and_track_overrides() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let rows = controller.layer_rows().unwrap();
    assert_eq!(rows.len(), 2);
    // Both demo layers are stored visible and start un-overridden.
    assert!(rows.iter().all(|r| r.database_visible));
    assert!(rows.iter().all(|r| !r.is_overridden()));

    controller.set_layer_visible(LayerId(1), false).unwrap();
    let rows = controller.layer_rows().unwrap();
    let walls = rows.iter().find(|r| r.id == LayerId(1)).unwrap();
    assert!(!walls.effective_visible);
    assert!(walls.database_visible, "stored flag is untouched");
    assert!(walls.is_overridden());

    controller.restore_layers().unwrap();
    let rows = controller.layer_rows().unwrap();
    assert!(rows.iter().all(|r| !r.is_overridden()));
    assert!(rows.iter().all(|r| r.effective_visible));

    // layer_ids matches layer_rows order.
    let ids = controller.layer_ids().unwrap();
    assert_eq!(ids, rows.iter().map(|r| r.id).collect::<Vec<_>>());
}

#[test]
fn layer_search_returns_real_matches_only() {
    let controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    assert_eq!(controller.search_layers("wall").unwrap().len(), 1);
    assert!(controller.search_layers("roof").unwrap().is_empty());
}

#[test]
fn selection_properties_are_read_only_and_report_demo_geometry() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    // Empty selection is explicit.
    assert!(controller.selection_properties().unwrap().empty);

    let revision_before = controller.drawing().unwrap().revision();
    controller
        .set_selection(vec![SelectionRef {
            document: controller.document_id,
            entity: EntityId(1),
            instance: InstancePath::default(),
            sub_element: None,
        }])
        .unwrap();
    let props = controller.selection_properties().unwrap();
    assert_eq!(props.count, 1);
    let row = |key: &str| {
        props
            .rows
            .iter()
            .find(|r| r.key == key)
            .map(|r| r.value.clone())
    };
    assert_eq!(row("id").as_deref(), Some("1"));
    assert_eq!(row("type").as_deref(), Some("AcDbLine"));
    assert_eq!(row("layer").as_deref(), Some("WALLS (1)"));
    assert_eq!(row("length").as_deref(), Some("4000.000000"));
    // Selection did not mutate the drawing or create history.
    assert_eq!(controller.drawing().unwrap().revision(), revision_before);
    assert!(!controller.application.can_undo(&controller.document_id));
}

#[test]
fn host_drawing_commands_round_trip_with_undo() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let before = controller.drawing().unwrap().entity_count();

    // Create a line on layer 0 (the demo's default active layer).
    let outcome = controller.create_line(p(0.0, 0.0), p(100.0, 0.0)).unwrap();
    assert_eq!(outcome.objects.len(), 1);
    assert_eq!(controller.drawing().unwrap().entity_count(), before + 1);
    assert!(controller.application.can_undo(&controller.document_id));
    assert!(controller.history_availability().can_undo);

    // Undo removes it.
    controller
        .execute(Command {
            schema_version: 1,
            id: crate::CommandId::Undo,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: crate::CommandPayload::None,
        })
        .unwrap();
    assert_eq!(controller.drawing().unwrap().entity_count(), before);
}

#[test]
fn host_drawing_commands_are_refused_in_viewer_mode() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_mode(AppMode::Viewer).unwrap();
    let before = controller.drawing().unwrap().entity_count();
    let result = controller.create_circle(p(0.0, 0.0), p(10.0, 0.0));
    assert!(matches!(result, Err(CadError::PermissionDenied)));
    assert_eq!(controller.drawing().unwrap().entity_count(), before);
}

#[test]
fn snap_candidates_near_finds_endpoint_and_midpoint_of_the_demo_line() {
    let drawing = demo_database();
    let document = DocumentId(1);

    // The demo's first entity is a line (0,0)-(4000,0); a cursor 5 world
    // units from its end resolves an endpoint hint at (0,0).
    let near_end =
        snap_candidates_near(&drawing, document, p(0.0, 5.0), 1.0, SpaceId::Model).unwrap();
    let endpoint = near_end
        .iter()
        .find(|c| c.kind == cad_measure::SnapKind::Endpoint && c.point == p(0.0, 0.0))
        .expect("the line end is an endpoint hint");
    assert!((endpoint.logical_pixel_distance - 5.0).abs() < 1e-9);
    assert_eq!(endpoint.space, SpaceId::Model);

    // Near the segment middle the midpoint wins.
    let near_mid =
        snap_candidates_near(&drawing, document, p(2000.0, 5.0), 1.0, SpaceId::Model).unwrap();
    assert!(
        near_mid
            .iter()
            .any(|c| { c.kind == cad_measure::SnapKind::Midpoint && c.point == p(2000.0, 0.0) }),
        "{near_mid:?}"
    );
}

#[test]
fn snap_candidates_near_returns_nothing_far_from_geometry() {
    let drawing = demo_database();
    let far = snap_candidates_near(
        &drawing,
        DocumentId(1),
        p(1_000_000.0, 1_000_000.0),
        1.0,
        SpaceId::Model,
    )
    .unwrap();
    assert!(far.is_empty(), "{far:?}");
}

#[test]
fn snap_candidates_near_rejects_a_bad_cursor_or_tolerance() {
    let drawing = demo_database();
    let nan_cursor = Point3 {
        x: f64::NAN,
        y: 0.0,
        z: 0.0,
    };
    assert!(matches!(
        snap_candidates_near(&drawing, DocumentId(1), nan_cursor, 1.0, SpaceId::Model),
        Err(CadError::InvalidInput(_))
    ));
    assert!(matches!(
        snap_candidates_near(&drawing, DocumentId(1), p(0.0, 0.0), 0.0, SpaceId::Model),
        Err(CadError::InvalidInput(_))
    ));
}

#[test]
fn snap_candidates_near_only_offers_model_space_targets() {
    // Only model geometry is expanded, so a paper-space request honestly
    // yields no hints rather than snapping model geometry into a layout.
    let drawing = demo_database();
    let paper = snap_candidates_near(
        &drawing,
        DocumentId(1),
        p(0.0, 5.0),
        1.0,
        SpaceId::Paper(cad_domain::LayoutId(0)),
    )
    .unwrap();
    assert!(paper.is_empty(), "{paper:?}");
}

#[test]
fn controller_snap_hints_are_empty_without_a_live_cursor() {
    let controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    // No tool, no cursor: the explicit empty overlay, never a stale hint.
    assert!(controller.snap_hints_near_cursor().unwrap().is_empty());
}

#[test]
fn controller_snap_hints_follow_the_measurement_cursor() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller
        .execute(Command {
            schema_version: 1,
            id: crate::CommandId::Measure,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: crate::CommandPayload::None,
        })
        .unwrap();
    // A running tool with no cursor yet still yields nothing.
    assert!(controller.snap_hints_near_cursor().unwrap().is_empty());

    // The demo line starts at (0,0); a cursor just off that end resolves a
    // real endpoint hint through the viewport's world_per_px.
    controller
        .session
        .set_measurement_cursor(Some(p(0.0, 5.0)))
        .unwrap();
    let hints = controller.snap_hints_near_cursor().unwrap();
    assert!(
        hints
            .iter()
            .any(|c| { c.kind == cad_measure::SnapKind::Endpoint && c.point == p(0.0, 0.0) }),
        "{hints:?}"
    );
}
