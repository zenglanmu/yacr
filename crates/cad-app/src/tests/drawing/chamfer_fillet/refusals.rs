//! The named CHAMFER/FILLET refusals: bad geometry, a repeated pick, a foreign
//! document, a missing layer, an over-long distance and negative parameters.
//! Every refusal must leave the document untouched.
//!
//! Split out of `tests/drawing/chamfer_fillet.rs` to keep that file under the
//! code-size limit.

use super::*;

#[test]
fn chamfer_refuses_parallel_lines_and_fillet_uses_the_auto_radius() {
    let parallel = || {
        two_line_app(
            (point(0.0, 0.0), point(10.0, 0.0)),
            (point(0.0, 5.0), point(10.0, 5.0)),
        )
    };
    let (mut app, mut session) = parallel();
    set_number(&mut session, ToolParameter::ChamferDistanceA, 1.0);
    set_number(&mut session, ToolParameter::ChamferDistanceB, 1.0);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::ChamferLines,
        chamfer(1, 2, point(8.0, 0.0), point(8.0, 5.0)),
        false,
    );
    // FILLET ignores the stored radius for parallel lines and uses half the
    // perpendicular gap (5 / 2 = 2.5).
    let (mut app, mut session) = parallel();
    set_number(&mut session, ToolParameter::FilletRadius, 20.0);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::FilletLines,
        fillet(1, 2, point(8.0, 0.0), point(10.0, 5.0)),
    )
    .expect("parallel lines take the auto-radius tangent arc");
    assert_eq!(outcome.changes.as_ref().unwrap().changes.len(), 3);
    let arc = db_entity(&app, outcome.objects[2].0);
    let SemanticGeometry::Arc {
        center,
        radius,
        sweep,
        ..
    } = arc.geometry
    else {
        panic!("expected the tangent ARC, got {:?}", arc.geometry);
    };
    assert!((radius - 2.5).abs() < 1e-9, "effective radius {radius}");
    assert!((sweep.abs() - std::f64::consts::PI).abs() < 1e-9);
    // The centre sits on the midline; the pick projections average to x = 9.
    assert!((center.x - 9.0).abs() < 1e-9 && (center.y - 2.5).abs() < 1e-9);
    // Both lines are trimmed at their perpendicular feet on the pick side.
    assert!(line_is(&app, 1, point(0.0, 0.0), point(9.0, 0.0)));
    assert!(line_is(&app, 2, point(9.0, 5.0), point(10.0, 5.0)));
}

#[test]
fn chamfer_and_fillet_refuse_the_same_entity_picked_twice() {
    let (mut app, mut session) = l_corner();
    set_number(&mut session, ToolParameter::ChamferDistanceA, 1.0);
    set_number(&mut session, ToolParameter::ChamferDistanceB, 1.0);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::ChamferLines,
        chamfer(1, 1, point(8.0, 0.0), point(2.0, 0.0)),
        false,
    );
}

#[test]
fn chamfer_and_fillet_refuse_a_cross_document_reference() {
    let (mut app, mut session) = l_corner();
    set_number(&mut session, ToolParameter::ChamferDistanceA, 1.0);
    set_number(&mut session, ToolParameter::ChamferDistanceB, 1.0);
    let foreign = SelectionRef {
        document: DocumentId(2),
        entity: EntityId(2),
        instance: InstancePath::default(),
        sub_element: None,
    };
    assert_refused(
        &mut app,
        &mut session,
        CommandId::ChamferLines,
        CommandPayload::Chamfer(Box::new(TwoLineCommand {
            first: ref_of(1),
            second: foreign,
            pick_first: point(8.0, 0.0),
            pick_second: point(0.0, 8.0),
        })),
        false,
    );
}

#[test]
fn chamfer_and_fillet_refuse_a_pick_that_is_not_a_straight_line() {
    let (mut app, mut session) = l_corner();
    set_number(&mut session, ToolParameter::ChamferDistanceA, 1.0);
    set_number(&mut session, ToolParameter::ChamferDistanceB, 1.0);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::ChamferLines,
        chamfer(3, 2, point(5.0, 0.0), point(0.0, 8.0)),
        true,
    );
}

#[test]
fn chamfer_and_fillet_refuse_non_coplanar_lines() {
    let (mut app, mut session) = two_line_app(
        (point(0.0, 0.0), point(10.0, 0.0)),
        (p3(0.0, 0.0, 0.0), p3(0.0, 10.0, 5.0)),
    );
    set_number(&mut session, ToolParameter::ChamferDistanceA, 1.0);
    set_number(&mut session, ToolParameter::ChamferDistanceB, 1.0);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::ChamferLines,
        chamfer(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
        false,
    );
}

#[test]
fn chamfer_and_fillet_refuse_a_pick_on_the_corner() {
    let (mut app, mut session) = l_corner();
    set_number(&mut session, ToolParameter::ChamferDistanceA, 1.0);
    set_number(&mut session, ToolParameter::ChamferDistanceB, 1.0);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::ChamferLines,
        chamfer(1, 2, point(0.0, 0.0), point(0.0, 8.0)),
        false,
    );
}

#[test]
fn chamfer_and_fillet_refuse_a_distance_too_long_for_the_line() {
    let (mut app, mut session) = l_corner();
    // The first line's pick side reaches x = 10, so A = 10 collapses the kept
    // segment; it must be refused rather than written.
    set_number(&mut session, ToolParameter::ChamferDistanceA, 10.0);
    set_number(&mut session, ToolParameter::ChamferDistanceB, 1.0);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::ChamferLines,
        chamfer(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
        false,
    );

    let (mut app, mut session) = l_corner();
    set_number(&mut session, ToolParameter::FilletRadius, 20.0);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::FilletLines,
        fillet(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
        false,
    );
}

#[test]
fn chamfer_and_fillet_refuse_a_missing_active_layer() {
    let (mut app, mut session) = l_corner();
    execute(
        &mut app,
        &mut session,
        CommandId::SetActiveLayer,
        CommandPayload::ActiveLayer(LayerId(99)),
    )
    .unwrap();
    set_number(&mut session, ToolParameter::ChamferDistanceA, 1.0);
    set_number(&mut session, ToolParameter::ChamferDistanceB, 1.0);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::ChamferLines,
        chamfer(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
        false,
    );
    // Even the corner-only FILLET (no new entity) refuses a missing layer.
    set_number(&mut session, ToolParameter::FilletRadius, 0.0);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::FilletLines,
        fillet(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
        false,
    );
}

#[test]
fn negative_radius_and_distance_are_named_refusals() {
    let (mut app, mut session) = l_corner();
    session.force_tool_parameter(ToolParameter::FilletRadius, ToolValue::Number(-1.0));
    let error = execute(
        &mut app,
        &mut session,
        CommandId::FilletLines,
        fillet(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
    )
    .unwrap_err();
    assert!(error.to_string().contains("radius_negative"), "{error}");

    let (mut app, mut session) = l_corner();
    session.force_tool_parameter(ToolParameter::ChamferDistanceA, ToolValue::Number(-1.0));
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ChamferLines,
        chamfer(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
    )
    .unwrap_err();
    assert!(error.to_string().contains("non-negative"), "{error}");
}
