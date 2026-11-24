//! `TCIRCLE` contract tests: the three boundary shapes, the distance offset
//! factor and the variable/constant size modes.

use super::*;

// --- TCIRCLE -----------------------------------------------------------------

#[test]
fn tcircle_draws_a_circle_on_the_text_layer_and_keeps_the_text() {
    let (mut app, mut session) = app_with(vec![simple_text(1, "abcd", p3(0.0, 0.0, 0.0), 5)]);
    select(&mut session, &[1]);
    let before_count = entity_count(&app);

    execute(
        &mut app,
        &mut session,
        CommandId::TextCircle,
        CommandPayload::TextCircle(TCircleCommand {
            shape: TextBoundaryShape::Circle,
            offset_factor: 0.0,
            size_mode: TCircleSizeMode::Variable,
        }),
    )
    .expect("TCIRCLE CIRCLE");

    assert_eq!(entity_count(&app), before_count + 1);
    let circle = find_by_type(&app, "AcDbCircle").expect("a circle boundary");
    assert_eq!(circle.layer, LayerId(7), "the text's layer");
    let SemanticGeometry::Circle {
        center,
        radius,
        normal,
    } = &circle.geometry
    else {
        panic!("expected a Circle");
    };
    // Estimated bounds (0,0)-(8,2): centre (4,1), radius half the diagonal.
    assert!(near(center.x, 4.0) && near(center.y, 1.0));
    assert!(near(*radius, 0.5 * 8.0_f64.hypot(2.0)));
    assert!(near(normal.z, 1.0));
    assert!(entity(&app, 1).is_some(), "the text is retained");
    assert_eq!(undo_depth(&app), 1);

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo");
    assert!(find_by_type(&app, "AcDbCircle").is_none());
    assert_eq!(entity_count(&app), before_count);
}

#[test]
fn tcircle_rectangle_is_a_closed_four_vertex_polyline() {
    let (mut app, mut session) = app_with(vec![simple_text(1, "abcd", p3(0.0, 0.0, 0.0), 0)]);
    select(&mut session, &[1]);
    execute(
        &mut app,
        &mut session,
        CommandId::TextCircle,
        CommandPayload::TextCircle(TCircleCommand {
            shape: TextBoundaryShape::Rectangle,
            offset_factor: 0.0,
            size_mode: TCircleSizeMode::Variable,
        }),
    )
    .expect("TCIRCLE RECTANGLE");
    let boundary = find_by_type(&app, "AcDbPolyline").expect("a polyline boundary");
    let SemanticGeometry::Polyline {
        points,
        bulges,
        closed,
        ..
    } = &boundary.geometry
    else {
        panic!("expected a Polyline");
    };
    assert_eq!(points.len(), 4);
    assert_eq!(bulges.len(), 4);
    assert!(*closed);
    assert!(near(points[2].x, 8.0) && near(points[2].y, 2.0));
}

#[test]
fn tcircle_slot_and_offset_expand_the_estimated_box() {
    let (mut app, mut session) = app_with(vec![simple_text(1, "abcd", p3(0.0, 0.0, 0.0), 0)]);
    select(&mut session, &[1]);
    execute(
        &mut app,
        &mut session,
        CommandId::TextCircle,
        CommandPayload::TextCircle(TCircleCommand {
            shape: TextBoundaryShape::Slot,
            offset_factor: 0.0,
            size_mode: TCircleSizeMode::Variable,
        }),
    )
    .expect("TCIRCLE SLOT");
    let boundary = find_by_type(&app, "AcDbPolyline").expect("a slot polyline");
    let SemanticGeometry::Polyline {
        points,
        bulges,
        closed,
        ..
    } = &boundary.geometry
    else {
        panic!("expected a Polyline");
    };
    assert_eq!(points.len(), 4);
    assert_eq!(bulges, &vec![0.0, 1.0, 0.0, 1.0], "semicircular ends");
    assert!(*closed);
    assert!(near(points[2].x, 8.0) && near(points[2].y, 2.0));

    // A 0.5 offset factor on a height-2 text pads every side by 1 unit.
    let (mut app, mut session) = app_with(vec![simple_text(1, "abcd", p3(0.0, 0.0, 0.0), 0)]);
    select(&mut session, &[1]);
    execute(
        &mut app,
        &mut session,
        CommandId::TextCircle,
        CommandPayload::TextCircle(TCircleCommand {
            shape: TextBoundaryShape::Rectangle,
            offset_factor: 0.5,
            size_mode: TCircleSizeMode::Variable,
        }),
    )
    .expect("TCIRCLE RECTANGLE OFFSET");
    let boundary = find_by_type(&app, "AcDbPolyline").expect("an offset rectangle");
    let SemanticGeometry::Polyline { points, .. } = &boundary.geometry else {
        panic!("expected a Polyline");
    };
    assert_eq!(points[0], p3(-1.0, -1.0, 0.0));
    assert_eq!(points[2], p3(9.0, 3.0, 0.0));
}

#[test]
fn tcircle_constant_mode_uses_the_largest_text_size() {
    let (mut app, mut session) = app_with(vec![
        simple_text(1, "ab", p3(0.0, 0.0, 0.0), 0),
        simple_text(2, "abcd", p3(20.0, 0.0, 0.0), 1),
    ]);
    select(&mut session, &[1, 2]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::TextCircle,
        CommandPayload::TextCircle(TCircleCommand {
            shape: TextBoundaryShape::Rectangle,
            offset_factor: 0.0,
            size_mode: TCircleSizeMode::ConstantBoth,
        }),
    )
    .expect("TCIRCLE CONSTANT");
    let widths: Vec<f64> = outcome
        .objects
        .iter()
        .map(|object| match &entity(&app, object.0).unwrap().geometry {
            SemanticGeometry::Polyline { points, .. } => {
                points.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max)
                    - points.iter().map(|p| p.x).fold(f64::INFINITY, f64::min)
            }
            other => panic!("expected a Polyline, got {other:?}"),
        })
        .collect();
    assert_eq!(widths, vec![8.0, 8.0], "both take the widest text's width");
    assert_eq!(undo_depth(&app), 1, "one transaction for both boundaries");
}

#[test]
fn tcircle_invalid_offset_writes_nothing() {
    let (mut app, mut session) = app_with(vec![simple_text(1, "abcd", p3(0.0, 0.0, 0.0), 0)]);
    select(&mut session, &[1]);
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::TextCircle,
        CommandPayload::TextCircle(TCircleCommand {
            shape: TextBoundaryShape::Circle,
            offset_factor: -0.1,
            size_mode: TCircleSizeMode::Variable,
        }),
    )
    .expect_err("a negative offset factor is refused");
    assert_invalid(error, "tcircle.invalid_offset");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
    assert!(find_by_type(&app, "AcDbCircle").is_none());
}
