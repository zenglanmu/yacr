use super::*;

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
            .and_then(|r| r.value.clone())
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
fn a_box_selection_count_reaches_only_the_selection_not_a_ui_channel() {
    // ADR 0014 B.8 ("result feedback") is an OPEN display item: `select.box.count`
    // is produced as a machine diagnostic but no host/UI channel surfaces its
    // `count`/`precision`. This pins the finding — the count travels on the raw
    // command diagnostic (which no production reader consumes) and on the real
    // selection set; there is no user-facing count display today, exactly like
    // `SELECTALL`, whose count is only visible through `selection.count`.
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    // A WINDOW rectangle larger than the whole demo extent contains every
    // entity, so the box selection is non-empty.
    let outcome = controller
        .execute(Command {
            schema_version: 1,
            id: crate::CommandId::SelectInBox,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: crate::CommandPayload::BoxSelect {
                rect_logical: [[-100_000.0, -100_000.0], [100_000.0, 100_000.0]],
                crossing: false,
                mode: crate::interaction::SelectionMode::Replace,
            },
        })
        .unwrap();
    let diagnostic = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "select.box.count")
        .expect("the box selection carries the stable select.box.count diagnostic");
    let expected = controller.selection().len();
    assert!(expected > 0, "the window selects the demo geometry");
    assert!(
        diagnostic.message.contains(&format!("count={expected}")),
        "the diagnostic count matches the real selection: {} vs {expected}",
        diagnostic.message
    );
    // The only channel a user sees is the real selection set. The count has no
    // localized/UI status channel: the controller's status string stays the raw
    // machine message, and nothing else is fabricated for display.
    assert_eq!(controller.status(), diagnostic.message);
}
