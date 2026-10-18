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

#[test]
fn a_click_on_a_ready_trim_is_refused_as_waiting_for_confirmation() {
    // Both TRIM steps resolved -> the tool is ready and waiting for Enter. A
    // further click falls past the entity-pick routing (no step is pending) and
    // must say so honestly instead of the stale "captures entity picks" text,
    // and must never be silently ignored.
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    controller.begin_draw_tool(DrawToolKind::Trim).unwrap();
    let (document, first, second) = {
        let drawing = controller.drawing().unwrap();
        let entities: Vec<_> = drawing
            .model_space()
            .iter()
            .map(|entity| entity.id)
            .collect();
        (controller.document_id, entities[0], entities[1])
    };
    let reference = |entity| SelectionRef {
        document,
        entity,
        instance: InstancePath(Vec::new()),
        sub_element: None,
    };
    // Resolve both picks directly; pick resolution itself has its own tests.
    controller
        .session
        .capture_draw_pick(p(0.0, 0.0), reference(first))
        .unwrap();
    controller
        .session
        .capture_draw_pick(p(1.0, 1.0), reference(second))
        .unwrap();
    assert!(controller.session.draw_preview().unwrap().ready);

    let error = controller.capture_tool_point(p(2.0, 2.0)).unwrap_err();
    let message = match error {
        CadError::InvalidInput(message) => message,
        other => panic!("expected an InvalidInput refusal, got {other:?}"),
    };
    assert!(message.contains("ready"), "{message}");
    assert!(message.contains("Enter"), "{message}");

    // The refused click did not capture anything or consume a step.
    let preview = controller.session.draw_preview().unwrap();
    assert!(preview.ready);
    assert_eq!(preview.picks.len(), 2);
    assert_eq!(preview.points.len(), 2);
}

#[test]
fn polyline_capture_confirms_one_open_polyline_and_maps_the_exact_points() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    let count = controller.drawing().unwrap().entity_count();
    let revision = controller.drawing().unwrap().revision();

    controller.begin_draw_tool(DrawToolKind::Polyline).unwrap();
    let captured = [p(10.0, 10.0), p(30.0, 10.0), p(30.0, 25.0)];
    for point in captured {
        controller.capture_tool_point(point).unwrap();
    }
    // The open-ended capture is confirmable at/above its two-point minimum, and
    // its preview reports no fixed remainder (never a fabricated 0).
    let preview = controller.session.draw_preview().unwrap();
    assert!(preview.can_confirm());
    assert!(preview.is_open_ended());
    assert_eq!(preview.remaining_points(), None);

    // The DrawIntent maps to exactly the captured points on the command payload.
    let intent = controller.session.draw_intent().unwrap();
    assert_eq!(
        intent,
        crate::DrawIntent::Polyline {
            points: captured.to_vec()
        }
    );
    let selection = controller
        .session
        .draw_selection_snapshot()
        .unwrap()
        .to_vec();
    let command = controller
        .command_for_draw_intent(intent, &selection)
        .unwrap();
    assert_eq!(command.id, crate::CommandId::CreatePolyline);
    match command.payload {
        crate::CommandPayload::Points(points) => {
            assert_eq!(points, captured.to_vec(), "payload matches capture exactly")
        }
        _ => panic!("expected a Points payload"),
    }

    let outcome = controller.confirm_draw().unwrap();
    assert_eq!(outcome.objects.len(), 1);
    let after = controller.drawing().unwrap();
    assert_eq!(after.entity_count(), count + 1);
    assert_ne!(after.revision(), revision);
    // The polyline is the newest model-space entity (highest draw order).
    let model = after.model_space();
    let entity = model.last().unwrap();
    match &entity.geometry {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => {
            assert_eq!(points, &captured.to_vec());
            assert_eq!(bulges, &vec![0.0, 0.0, 0.0]);
            assert!(!*closed);
        }
        other => panic!("expected a polyline, got {other:?}"),
    }
    assert!(matches!(controller.session.tool, crate::ToolState::Idle));
}

#[test]
fn polyline_single_point_confirm_is_refused_without_a_write() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    let revision = controller.drawing().unwrap().revision();
    controller.begin_draw_tool(DrawToolKind::Polyline).unwrap();
    controller.capture_tool_point(p(5.0, 5.0)).unwrap();
    // One point is below the open-ended minimum: confirm is refused and the
    // capture survives with nothing written.
    assert!(controller.confirm_draw().is_err());
    assert!(controller.session.draw_preview().is_some());
    assert_eq!(controller.drawing().unwrap().revision(), revision);
    assert!(!controller.application.can_undo(&controller.document_id));
}

#[test]
fn rectang_capture_confirms_one_closed_rectangle_and_maps_the_exact_corners() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    let count = controller.drawing().unwrap().entity_count();

    controller.begin_draw_tool(DrawToolKind::Rectang).unwrap();
    // RECTANG is fixed arity 2: one corner is not confirmable.
    let first = p(10.0, 10.0);
    let second = p(40.0, 35.0);
    controller.capture_tool_point(first).unwrap();
    let preview = controller.session.draw_preview().unwrap();
    assert!(!preview.can_confirm());
    assert_eq!(preview.remaining_points(), Some(1));
    assert!(!preview.is_open_ended());
    assert!(
        controller.confirm_draw().is_err(),
        "one corner is not a rectangle"
    );

    controller.capture_tool_point(second).unwrap();
    let preview = controller.session.draw_preview().unwrap();
    assert!(preview.can_confirm());
    assert_eq!(preview.points, vec![first, second]);
    assert_eq!(preview.remaining_points(), Some(0));

    // The DrawIntent carries the two captured corners in click order, and maps to
    // exactly those points on the command payload.
    let intent = controller.session.draw_intent().unwrap();
    assert_eq!(
        intent,
        crate::DrawIntent::Rect {
            corner1: first,
            corner2: second
        }
    );
    let selection = controller
        .session
        .draw_selection_snapshot()
        .unwrap()
        .to_vec();
    let command = controller
        .command_for_draw_intent(intent, &selection)
        .unwrap();
    assert_eq!(command.id, crate::CommandId::CreateRectangle);
    match command.payload {
        crate::CommandPayload::Points(points) => {
            assert_eq!(
                points,
                vec![first, second],
                "payload matches capture exactly"
            )
        }
        _ => panic!("expected a Points payload"),
    }

    let outcome = controller.confirm_draw().unwrap();
    assert_eq!(outcome.objects.len(), 1);
    let after = controller.drawing().unwrap();
    assert_eq!(after.entity_count(), count + 1);
    // The rectangle is the newest model-space entity (highest draw order) and is a
    // **closed** four-vertex straight polyline.
    let model = after.model_space();
    let entity = model.last().unwrap();
    match &entity.geometry {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => {
            assert_eq!(
                points,
                &vec![p(10.0, 10.0), p(40.0, 10.0), p(40.0, 35.0), p(10.0, 35.0)]
            );
            assert_eq!(bulges, &vec![0.0; 4]);
            assert!(*closed);
        }
        other => panic!("expected a polyline, got {other:?}"),
    }
    assert!(matches!(controller.session.tool, crate::ToolState::Idle));
}

/// Split a DXF text document into `(group code, value)` pairs.
fn dxf_pairs(text: &str) -> Vec<(i32, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut pairs = Vec::new();
    let mut index = 0;
    while index + 1 < lines.len() {
        let code: i32 = lines[index]
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("group code expected at line {index}: {:?}", lines[index]));
        pairs.push((code, lines[index + 1].trim().to_string()));
        index += 2;
    }
    pairs
}

/// The `(code, value)` pairs of every `name` entity record (each DXF record starts
/// at a `0` group whose value is the entity type).
fn dxf_entities(pairs: &[(i32, String)], name: &str) -> Vec<Vec<(i32, String)>> {
    let mut records = Vec::new();
    let mut current: Option<Vec<(i32, String)>> = None;
    for (code, value) in pairs {
        if *code == 0 {
            if let Some(record) = current.take() {
                records.push(record);
            }
            current = (value == name).then(Vec::new);
            continue;
        }
        if let Some(record) = current.as_mut() {
            record.push((*code, value.clone()));
        }
    }
    if let Some(record) = current.take() {
        records.push(record);
    }
    records
}

#[test]
fn rectangle_round_trips_through_the_dxf_export_as_a_closed_lwpolyline() {
    // The export code lives in the frozen `cad-import-acadrust` crate, but the
    // host's `export_dxf` is the real production Save path, so the rectangle can be
    // round-tripped here without touching that crate: the emitted DXF text must
    // contain exactly one closed (`70` bit 1) four-vertex LWPOLYLINE at the two
    // corners.
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    // A blank document so the only LWPOLYLINE is the rectangle under test.
    controller.new_blank_document().unwrap();
    controller
        .create_rectangle(p(0.0, 0.0), p(4.0, 3.0))
        .unwrap();

    let (bytes, report) = controller.export_dxf().unwrap();
    assert_eq!(report.counts.dropped, 0, "nothing may be dropped");
    let text = String::from_utf8(bytes).expect("DXF text is UTF-8");
    let pairs = dxf_pairs(&text);
    let polylines = dxf_entities(&pairs, "LWPOLYLINE");
    assert_eq!(
        polylines.len(),
        1,
        "exactly one LWPOLYLINE for one rectangle"
    );
    let record = &polylines[0];
    let flag = record
        .iter()
        .find(|(code, _)| *code == 70)
        .map(|(_, value)| value.parse::<i32>().expect("a numeric 70 flag"))
        .expect("an LWPOLYLINE writes a 70 flag");
    assert_eq!(flag, 1, "RECTANG exports a closed LWPOLYLINE (70 bit 1)");
    let vertices: Vec<(f64, f64)> = record
        .iter()
        .filter(|(code, _)| *code == 10)
        .map(|(_, x)| {
            x.parse::<f64>()
                .unwrap_or_else(|_| panic!("numeric vertex x: {x:?}"))
        })
        .zip(record.iter().filter(|(code, _)| *code == 20).map(|(_, y)| {
            y.parse::<f64>()
                .unwrap_or_else(|_| panic!("numeric vertex y: {y:?}"))
        }))
        .collect();
    assert_eq!(
        vertices,
        vec![(0.0, 0.0), (4.0, 0.0), (4.0, 3.0), (0.0, 3.0)],
        "the exported vertices are the canonical closed rectangle"
    );
}

#[test]
fn ellipse_capture_confirms_one_entity_and_maps_the_exact_picks() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    let count = controller.drawing().unwrap().entity_count();

    controller.begin_draw_tool(DrawToolKind::Ellipse).unwrap();
    // ELLIPSE is fixed arity 3 (axis-endpoint method): two picks are not enough.
    let first = p(0.0, 0.0);
    let second = p(4.0, 0.0);
    let third = p(2.0, 3.0);
    controller.capture_tool_point(first).unwrap();
    controller.capture_tool_point(second).unwrap();
    let preview = controller.session.draw_preview().unwrap();
    assert!(!preview.can_confirm(), "two picks are not an ellipse");
    assert_eq!(preview.remaining_points(), Some(1));
    assert!(!preview.is_open_ended());
    assert!(controller.confirm_draw().is_err());

    controller.capture_tool_point(third).unwrap();
    let preview = controller.session.draw_preview().unwrap();
    assert!(preview.can_confirm());
    assert_eq!(preview.points, vec![first, second, third]);
    assert_eq!(preview.remaining_points(), Some(0));

    // The DrawIntent carries the three captured picks in click order, and maps to
    // exactly those points on the command payload.
    let intent = controller.session.draw_intent().unwrap();
    assert_eq!(
        intent,
        crate::DrawIntent::Ellipse {
            p1: first,
            p2: second,
            p3: third,
        }
    );
    let selection = controller
        .session
        .draw_selection_snapshot()
        .unwrap()
        .to_vec();
    let command = controller
        .command_for_draw_intent(intent, &selection)
        .unwrap();
    assert_eq!(command.id, crate::CommandId::CreateEllipse);
    match command.payload {
        crate::CommandPayload::Points(points) => assert_eq!(
            points,
            vec![first, second, third],
            "payload matches capture exactly"
        ),
        _ => panic!("expected a Points payload"),
    }

    let outcome = controller.confirm_draw().unwrap();
    assert_eq!(outcome.objects.len(), 1);
    let after = controller.drawing().unwrap();
    assert_eq!(after.entity_count(), count + 1);
    // The ellipse is the newest model-space entity (highest draw order): centre is
    // the first-axis midpoint; the half-axis {a=2, b=3} keeps b as the longer
    // (major) one, so ratio = 2/3.
    let model = after.model_space();
    let entity = model.last().unwrap();
    match &entity.geometry {
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => {
            assert_eq!(*center, p(2.0, 0.0));
            assert_eq!(
                *normal,
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0
                }
            );
            assert!(
                (major_axis.y - 3.0).abs() < 1e-9 && major_axis.x.abs() < 1e-9,
                "the longer (3) half-axis is stored as major: {major_axis:?}"
            );
            assert!(*ratio <= 1.0, "ratio must stay <= 1: {ratio}");
            assert!((*ratio - 2.0 / 3.0).abs() < 1e-9, "{ratio}");
            assert_eq!(*start, 0.0);
            assert!((*sweep - std::f64::consts::TAU).abs() < 1e-9);
        }
        other => panic!("expected an ellipse, got {other:?}"),
    }
    assert!(matches!(controller.session.tool, crate::ToolState::Idle));
}

/// The `(code, value)` pairs of the ELLIPSE record's ELLIPSE entity.
fn ellipse_record(pairs: &[(i32, String)]) -> Vec<(i32, String)> {
    let mut records = dxf_entities(pairs, "ELLIPSE");
    assert_eq!(records.len(), 1, "exactly one ELLIPSE for one ellipse");
    records.remove(0)
}

/// The single numeric value of `code` in an entity record.
fn group(record: &[(i32, String)], code: i32) -> f64 {
    record
        .iter()
        .find(|(candidate, _)| *candidate == code)
        .map(|(_, value)| {
            value
                .parse::<f64>()
                .unwrap_or_else(|_| panic!("numeric group {code}: {value:?}"))
        })
        .unwrap_or_else(|| panic!("no group code {code} in {record:?}"))
}

#[test]
fn exported_ellipse_round_trips_idempotently() {
    // AUTH is the production save path (`HostController::export_dxf`), so the
    // ellipse is round-tripped without touching the frozen `cad-import-acadrust`
    // crate. DXF ELLIPSE groups: 10/20/30 centre, 11/21/31 endpoint of the major
    // axis **relative to the centre**, 40 ratio, 41/42 start/end parameter,
    // 210/220/230 extrusion normal.
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    controller.new_blank_document().unwrap();
    // First axis (-1,0)->(1,0) (semi-axis a = 1), third pick (0,3) (b = 3): the
    // longer y half-axis becomes the stored major axis with ratio = 1/3.
    controller
        .create_ellipse(p(-1.0, 0.0), p(1.0, 0.0), p(0.0, 3.0))
        .unwrap();

    let (bytes, report) = controller.export_dxf().unwrap();
    assert_eq!(report.counts.dropped, 0, "nothing may be dropped");
    let text = String::from_utf8(bytes.clone()).expect("DXF text is UTF-8");
    let pairs = dxf_pairs(&text);
    let record = ellipse_record(&pairs);
    // Centre = midpoint of the first axis.
    assert!((group(&record, 10) - 0.0).abs() < 1e-9);
    assert!((group(&record, 20) - 0.0).abs() < 1e-9);
    assert!((group(&record, 30) - 0.0).abs() < 1e-9);
    // Major-axis vector relative to the centre: the longer (3) perpendicular axis.
    assert!(group(&record, 11).abs() < 1e-9);
    assert!((group(&record, 21).abs() - 3.0).abs() < 1e-9);
    assert!((group(&record, 31)).abs() < 1e-9);
    // Ratio <= 1 and equal to a/b.
    let ratio = group(&record, 40);
    assert!(ratio <= 1.0, "exported group 40 must stay <= 1: {ratio}");
    assert!((ratio - 1.0 / 3.0).abs() < 1e-9, "{ratio}");
    // A full ellipse: start 0, end 2π.
    assert!((group(&record, 41) - 0.0).abs() < 1e-9);
    assert!((group(&record, 42) - std::f64::consts::TAU).abs() < 1e-9);
    // The +Z work-plane normal is the DXF default, so the writer omits the
    // extrusion groups (210/220/230); their absence is the +Z construction.
    assert!(
        !record
            .iter()
            .any(|(code, _)| matches!(*code, 210 | 220 | 230)),
        "a +Z ellipse omits the extrusion groups (DXF default +Z): {record:?}"
    );

    // Re-export is stable: the same entity maps to the same DXF text.
    let (again, again_report) = controller.export_dxf().unwrap();
    assert_eq!(bytes, again, "re-export is byte-identical");
    assert_eq!(report.counts.dropped, again_report.counts.dropped);
}
