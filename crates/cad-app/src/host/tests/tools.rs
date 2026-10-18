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
    // This contract is about raw capture/confirm; snapping has its own tests.
    controller.set_snap_enabled(false);
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
