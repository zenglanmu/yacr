//! LINE/RECTANG typed dimensions: the session composes the Measurements
//! parameters over the pure capture and commits the typed geometry.
use super::*;

#[test]
fn typed_line_length_and_angle_override_the_captured_endpoint() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    controller.begin_draw_tool(DrawToolKind::Line).unwrap();
    controller.capture_tool_point(p(1.0, 2.0)).unwrap();
    controller
        .session
        .set_tool_parameter(
            crate::ToolParameter::LineLength,
            crate::ToolValue::Number(10.0),
        )
        .unwrap();
    controller
        .session
        .set_tool_parameter(
            crate::ToolParameter::LineAngle,
            crate::ToolValue::Number(90.0),
        )
        .unwrap();
    // The pending preview is the typed endpoint, not the raw cursor.
    let cursor = controller
        .session
        .draw_preview()
        .unwrap()
        .cursor
        .expect("a typed cursor");
    assert!((cursor.x - 1.0).abs() < 1e-9, "{cursor:?}");
    assert!((cursor.y - 12.0).abs() < 1e-9, "{cursor:?}");
    // The captured second point is deliberately different; the typed length wins.
    controller.capture_tool_point(p(100.0, 100.0)).unwrap();
    let outcome = controller.confirm_draw().unwrap();
    assert_eq!(outcome.objects.len(), 1);
    let drawing = controller.drawing().unwrap();
    let entity = drawing
        .entity(crate::EntityId(outcome.objects[0].0))
        .unwrap();
    match &entity.geometry {
        cad_domain::SemanticGeometry::Line { start, end } => {
            assert_eq!(*start, p(1.0, 2.0));
            assert!(
                (end.x - 1.0).abs() < 1e-9,
                "typed angle/length endpoint: {end:?}"
            );
            assert!(
                (end.y - 12.0).abs() < 1e-9,
                "typed angle/length endpoint: {end:?}"
            );
        }
        other => panic!("expected a line, got {other:?}"),
    }
}

#[test]
fn a_typed_line_length_without_a_direction_is_refused_by_name() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    controller.begin_draw_tool(DrawToolKind::Line).unwrap();
    controller.capture_tool_point(p(1.0, 2.0)).unwrap();
    controller
        .session
        .set_tool_parameter(
            crate::ToolParameter::LineLength,
            crate::ToolValue::Number(10.0),
        )
        .unwrap();
    // The same point leaves no direction: the confirm refuses instead of
    // committing a zero-length line.
    controller.capture_tool_point(p(1.0, 2.0)).unwrap();
    let error = controller.confirm_draw().unwrap_err();
    assert!(format!("{error:?}").contains("line.direction"), "{error:?}");
}

#[test]
fn typed_rectangle_extents_own_their_axes_and_keep_the_cursor_sign() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    controller.begin_draw_tool(DrawToolKind::Rectang).unwrap();
    controller.capture_tool_point(p(10.0, 10.0)).unwrap();
    controller
        .session
        .set_tool_parameter(
            crate::ToolParameter::RectWidth,
            crate::ToolValue::Number(20.0),
        )
        .unwrap();
    controller
        .session
        .set_tool_parameter(
            crate::ToolParameter::RectHeight,
            crate::ToolValue::Number(5.0),
        )
        .unwrap();
    // The typed extents are the preview corner, not the raw cursor.
    controller.move_tool_cursor(p(100.0, 100.0)).unwrap();
    let cursor = controller
        .session
        .draw_preview()
        .unwrap()
        .cursor
        .expect("a typed cursor");
    assert!((cursor.x - 30.0).abs() < 1e-9, "{cursor:?}");
    assert!((cursor.y - 15.0).abs() < 1e-9, "{cursor:?}");

    controller.capture_tool_point(p(100.0, 100.0)).unwrap();
    let outcome = controller.confirm_draw().unwrap();
    assert_eq!(outcome.objects.len(), 1);
    let drawing = controller.drawing().unwrap();
    let entity = drawing
        .entity(crate::EntityId(outcome.objects[0].0))
        .unwrap();
    let cad_domain::SemanticGeometry::Polyline { points, closed, .. } = &entity.geometry else {
        panic!("expected a closed polyline, got {:?}", entity.geometry);
    };
    assert!(*closed);
    let xs: Vec<f64> = points.iter().map(|point| point.x).collect();
    let ys: Vec<f64> = points.iter().map(|point| point.y).collect();
    let width =
        xs.iter().cloned().fold(f64::MIN, f64::max) - xs.iter().cloned().fold(f64::MAX, f64::min);
    let height =
        ys.iter().cloned().fold(f64::MIN, f64::max) - ys.iter().cloned().fold(f64::MAX, f64::min);
    assert!((width - 20.0).abs() < 1e-9, "typed width: {width}");
    assert!((height - 5.0).abs() < 1e-9, "typed height: {height}");

    // The cursor's sign is kept: dragging to the upper-left still builds the
    // rectangle in that quadrant with the same typed extents.
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    controller.begin_draw_tool(DrawToolKind::Rectang).unwrap();
    controller.capture_tool_point(p(10.0, 10.0)).unwrap();
    controller
        .session
        .set_tool_parameter(
            crate::ToolParameter::RectWidth,
            crate::ToolValue::Number(20.0),
        )
        .unwrap();
    controller
        .session
        .set_tool_parameter(
            crate::ToolParameter::RectHeight,
            crate::ToolValue::Number(5.0),
        )
        .unwrap();
    controller.capture_tool_point(p(-100.0, -100.0)).unwrap();
    let outcome = controller.confirm_draw().unwrap();
    let drawing = controller.drawing().unwrap();
    let entity = drawing
        .entity(crate::EntityId(outcome.objects[0].0))
        .unwrap();
    let cad_domain::SemanticGeometry::Polyline { points, .. } = &entity.geometry else {
        panic!("expected a closed polyline");
    };
    assert!(
        points.iter().any(|point| point.x < 10.0 && point.y < 10.0),
        "{points:?}"
    );
}

#[test]
fn a_typed_circle_radius_owns_the_distance_not_the_direction() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    controller.begin_draw_tool(DrawToolKind::Circle).unwrap();
    controller.capture_tool_point(p(10.0, 10.0)).unwrap();
    controller
        .session
        .set_tool_parameter(
            crate::ToolParameter::CircleRadius,
            crate::ToolValue::Number(5.0),
        )
        .unwrap();
    // The cursor points along +X at distance 100; the typed radius keeps that
    // direction and places the edge at 5.
    controller.move_tool_cursor(p(110.0, 10.0)).unwrap();
    let cursor = controller
        .session
        .draw_preview()
        .unwrap()
        .cursor
        .expect("a typed edge");
    assert!((cursor.x - 15.0).abs() < 1e-9, "{cursor:?}");
    assert!((cursor.y - 10.0).abs() < 1e-9, "{cursor:?}");

    controller.capture_tool_point(p(110.0, 10.0)).unwrap();
    let outcome = controller.confirm_draw().unwrap();
    assert_eq!(outcome.objects.len(), 1);
    let drawing = controller.drawing().unwrap();
    let entity = drawing
        .entity(crate::EntityId(outcome.objects[0].0))
        .unwrap();
    let cad_domain::SemanticGeometry::Circle { radius, .. } = &entity.geometry else {
        panic!("expected a circle, got {:?}", entity.geometry);
    };
    assert!((radius - 5.0).abs() < 1e-9, "typed radius: {radius}");
}

#[test]
fn a_typed_rotate_angle_keeps_the_cursor_radius_and_replaces_the_direction() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    let entity = controller.drawing().unwrap().model_space()[0].id;
    controller
        .plugin_select_ids(vec![cad_domain::ObjectId(entity.0)])
        .expect("the demo entity is selectable");
    controller.begin_draw_tool(DrawToolKind::Rotate).unwrap();
    controller.capture_tool_point(p(0.0, 0.0)).unwrap();
    controller
        .session
        .set_tool_parameter(
            crate::ToolParameter::RotateAngle,
            crate::ToolValue::Number(90.0),
        )
        .unwrap();
    controller.move_tool_cursor(p(10.0, 0.0)).unwrap();
    let cursor = controller
        .session
        .draw_preview()
        .unwrap()
        .cursor
        .expect("a typed cursor");
    assert!(cursor.x.abs() < 1e-9, "{cursor:?}");
    assert!((cursor.y - 10.0).abs() < 1e-9, "{cursor:?}");
    controller.capture_tool_point(p(10.0, 0.0)).unwrap();
    match controller.session.draw_intent().expect("a rotated intent") {
        crate::DrawIntent::Rotate { base, angle } => {
            assert_eq!(base, p(0.0, 0.0));
            assert!(
                (angle - std::f64::consts::FRAC_PI_2).abs() < 1e-9,
                "{angle}"
            );
        }
        other => panic!("expected Rotate, got {other:?}"),
    }
}

#[test]
fn a_typed_move_distance_keeps_the_cursor_direction() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    let entity = controller.drawing().unwrap().model_space()[0].id;
    controller
        .plugin_select_ids(vec![cad_domain::ObjectId(entity.0)])
        .expect("the demo entity is selectable");
    controller.begin_draw_tool(DrawToolKind::Move).unwrap();
    controller.capture_tool_point(p(0.0, 0.0)).unwrap();
    controller
        .session
        .set_tool_parameter(
            crate::ToolParameter::MoveDistance,
            crate::ToolValue::Number(10.0),
        )
        .unwrap();
    controller.move_tool_cursor(p(100.0, 0.0)).unwrap();
    let cursor = controller
        .session
        .draw_preview()
        .unwrap()
        .cursor
        .expect("a typed cursor");
    assert!((cursor.x - 10.0).abs() < 1e-9, "{cursor:?}");
    assert!(cursor.y.abs() < 1e-9, "{cursor:?}");
    controller.capture_tool_point(p(100.0, 0.0)).unwrap();
    match controller.session.draw_intent().expect("a moved intent") {
        crate::DrawIntent::Move { delta } => {
            assert!((delta.x - 10.0).abs() < 1e-9, "{delta:?}");
            assert!(delta.y.abs() < 1e-9, "{delta:?}");
            assert!(delta.z.abs() < 1e-9, "{delta:?}");
        }
        other => panic!("expected Move, got {other:?}"),
    }
}

#[test]
fn a_typed_scale_factor_scales_the_reference_vector() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    let entity = controller.drawing().unwrap().model_space()[0].id;
    controller
        .plugin_select_ids(vec![cad_domain::ObjectId(entity.0)])
        .expect("the demo entity is selectable");
    controller.begin_draw_tool(DrawToolKind::Scale).unwrap();
    controller.capture_tool_point(p(0.0, 0.0)).unwrap();
    controller.capture_tool_point(p(10.0, 0.0)).unwrap();
    controller
        .session
        .set_tool_parameter(
            crate::ToolParameter::ScaleFactor,
            crate::ToolValue::Number(2.0),
        )
        .unwrap();
    let cursor = controller
        .session
        .draw_preview()
        .unwrap()
        .cursor
        .expect("a typed cursor");
    assert!((cursor.x - 20.0).abs() < 1e-9, "{cursor:?}");
    assert!(cursor.y.abs() < 1e-9, "{cursor:?}");
    controller.capture_tool_point(p(30.0, 0.0)).unwrap();
    match controller.session.draw_intent().expect("a scaled intent") {
        crate::DrawIntent::Scale { base, factor } => {
            assert_eq!(base, p(0.0, 0.0));
            assert!((factor - 2.0).abs() < 1e-9, "{factor}");
        }
        other => panic!("expected Scale, got {other:?}"),
    }
}

#[test]
fn a_typed_offset_distance_selects_the_side_and_sets_the_distance() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    let entity = controller.drawing().unwrap().model_space()[0].id;
    controller
        .plugin_select_ids(vec![cad_domain::ObjectId(entity.0)])
        .expect("the demo entity is selectable");
    controller.begin_draw_tool(DrawToolKind::Offset).unwrap();
    controller.capture_tool_point(p(10.0, 4.0)).unwrap();
    let intent = controller
        .session
        .draw_intent()
        .expect("a side-point intent");
    let selection = controller
        .session
        .draw_selection_snapshot()
        .unwrap()
        .to_vec();
    let mapped = |controller: &HostController| {
        controller
            .command_for_draw_intent(intent.clone(), &selection)
            .expect("OFFSET maps")
            .payload
    };

    // A positive typed distance travels on the atomic option payload; the side
    // point is only the side selector.
    controller
        .session
        .set_tool_parameter(
            crate::ToolParameter::OffsetDistance,
            crate::ToolValue::Number(5.0),
        )
        .unwrap();
    match mapped(&controller) {
        crate::CommandPayload::OffsetOptions(options) => {
            assert_eq!(options.mode, crate::OffsetMode::Distance(5.0));
            assert_eq!(options.side_points, vec![p(10.0, 4.0)]);
            assert_eq!(options.layer, crate::OffsetLayer::Source);
            assert_eq!(options.gap, crate::OffsetGapType::Sharp);
        }
        _ => panic!("expected OffsetOptions"),
    }

    // The typed value wins over the stored `OFFSET <distance>` default, and `0`
    // (the unset sentinel) falls back to that stored default.
    controller
        .session
        .modify_options
        .set_offset_distance(Some(3.0));
    controller
        .session
        .set_tool_parameter(
            crate::ToolParameter::OffsetDistance,
            crate::ToolValue::Number(5.0),
        )
        .unwrap();
    match mapped(&controller) {
        crate::CommandPayload::OffsetOptions(options) => {
            assert_eq!(options.mode, crate::OffsetMode::Distance(5.0))
        }
        _ => panic!("expected OffsetOptions"),
    }
    controller
        .session
        .set_tool_parameter(
            crate::ToolParameter::OffsetDistance,
            crate::ToolValue::Number(0.0),
        )
        .unwrap();
    match mapped(&controller) {
        crate::CommandPayload::OffsetOptions(options) => {
            assert_eq!(options.mode, crate::OffsetMode::Distance(3.0))
        }
        _ => panic!("expected OffsetOptions"),
    }

    // With neither typed nor stored distance the legacy `Through` payload stays.
    controller.session.modify_options.set_offset_distance(None);
    match mapped(&controller) {
        crate::CommandPayload::Offset { side_point, refs } => {
            assert_eq!(side_point, p(10.0, 4.0));
            assert_eq!(refs, selection);
        }
        _ => panic!("expected the legacy Offset payload"),
    }
}
