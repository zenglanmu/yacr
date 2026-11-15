//! `LENGTHEN` on an open SPLINE: the declared tangent-tail extension policy, the
//! exact sub-curve shrink, the dynamic-mode rule and the SPLINE refusals.
//!
//! Split out of `lengthen.rs` (code-size limit) as a private child module, so the
//! fixtures and assertions are the shared ones: `lengthen()` drives the real
//! command, `entity()` reads the committed row back and every case checks that a
//! refusal writes nothing.

use super::*;

/// A degree-3 fit spline (one control point per fit point, clamped chord knots):
/// the ordinary curved SPLINE the other SPLINE contracts use.
fn fit_spline() -> SemanticGeometry {
    let fit = vec![
        point(0.0, 0.0),
        point(1.0, 2.0),
        point(2.5, 2.5),
        point(4.0, 1.0),
        point(5.5, -1.0),
    ];
    let (knots, control_points) =
        crate::draw_tool::spline_control_points(&fit).expect("a valid fit spline");
    SemanticGeometry::Spline {
        degree: cad_geometry::FIT_SPLINE_DEGREE,
        knots,
        control_points,
        weights: vec![1.0; fit.len()],
    }
}

/// A **degree-1** spline `(0,0) → (3,0) → (3,4)`: every span is straight, so its
/// arc length is exactly `7` and the LENGTHEN length estimate is exact too (chords
/// never cut a corner). Percent/Total arithmetic can therefore be pinned without a
/// sampling allowance; its end tangent is `+Y` at `(3,4)`.
fn linear_spline() -> SemanticGeometry {
    SemanticGeometry::Spline {
        degree: 1,
        knots: vec![0.0, 0.0, 1.0, 2.0, 2.0],
        control_points: vec![point(0.0, 0.0), point(3.0, 0.0), point(3.0, 4.0)],
        weights: vec![1.0; 3],
    }
}

fn spline_target(geometry: &SemanticGeometry) -> cad_geometry::NurbsCurve {
    let (degree, knots, control_points, weights) = spline_parts(geometry);
    cad_geometry::NurbsCurve::new(degree as usize, control_points, knots, weights)
        .expect("a valid spline definition")
}

fn spline_parts(geometry: &SemanticGeometry) -> (u32, Vec<f64>, Vec<Point3>, Vec<f64>) {
    match geometry {
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => (
            *degree,
            knots.clone(),
            control_points.clone(),
            weights.clone(),
        ),
        other => panic!("expected a SPLINE, got {other:?}"),
    }
}

/// The curve's own sampled length: the end of its knot domain is the last control
/// point of a clamped definition.
fn spline_endpoints(geometry: &SemanticGeometry) -> (Point3, Point3) {
    let curve = spline_target(geometry);
    let (t_min, t_max) = curve.domain();
    (curve.evaluate(t_min), curve.evaluate(t_max))
}

/// The unit end tangent of a stored spline, in the direction the curve grows.
fn spline_end_tangent(geometry: &SemanticGeometry) -> Point3 {
    let curve = spline_target(geometry);
    let (_, t_max) = curve.domain();
    let raw = curve.tangent(t_max);
    let len = cad_geometry::length(raw);
    assert!(len > 0.0, "the fixture must have an end tangent");
    Point3 {
        x: raw.x / len,
        y: raw.y / len,
        z: raw.z / len,
    }
}

fn close3(a: Point3, b: Point3, tol: f64) -> bool {
    (a.x - b.x).abs() <= tol && (a.y - b.y).abs() <= tol && (a.z - b.z).abs() <= tol
}

fn offset(from: Point3, unit: Point3, distance: f64) -> Point3 {
    Point3 {
        x: from.x + unit.x * distance,
        y: from.y + unit.y * distance,
        z: from.z + unit.z * distance,
    }
}

#[test]
fn spline_delta_extension_appends_a_straight_tangent_tail() {
    let before = fit_spline();
    let original = spline_target(&before);
    let (t_min, t_max) = original.domain();
    let (_, old_end) = spline_endpoints(&before);
    let tangent = spline_end_tangent(&before);
    let growth = 1.5;

    let (mut app, mut session) = app_with(vec![curve_entity(
        1,
        LayerId(0),
        before.clone(),
        "AcDbSpline",
        0,
    )]);
    set_mode_value(&mut session, DELTA, growth);
    lengthen(&mut app, &mut session, vec![ref_of(1)]).expect("the open spline grows");

    let after = entity(&app, 1).geometry;
    let grown = spline_target(&after);
    let (_, new_t_max) = grown.domain();
    let new_end = grown.evaluate(new_t_max);

    // The declared policy: the new end is exactly `E + d·T`.
    let expected_end = offset(old_end, tangent, growth);
    assert!(
        close3(new_end, expected_end, 1e-12),
        "new end {new_end:?} is not E + d·T = {expected_end:?}"
    );
    // The tail is exactly the requested length (it is the straight segment).
    assert!(
        near(cad_geometry::distance(old_end, new_end), growth),
        "the tangent tail is not {growth} long"
    );

    // The stored definition grew by exactly `degree + 1` control points and
    // `degree + 1` knots, and the source's own entries are unchanged.
    let (degree, knots, control_points, weights) = spline_parts(&after);
    let (old_degree, old_knots, old_points, old_weights) = spline_parts(&before);
    assert_eq!(degree, old_degree);
    assert_eq!(
        control_points.len(),
        old_points.len() + degree as usize + 1,
        "one appended clamped span"
    );
    assert_eq!(knots.len(), control_points.len() + degree as usize + 1);
    assert_eq!(weights.len(), control_points.len());
    assert_eq!(
        &knots[..old_knots.len()],
        &old_knots[..],
        "interior knots moved"
    );
    assert_eq!(
        &control_points[..old_points.len()],
        &old_points[..],
        "source control points moved"
    );
    assert_eq!(
        &weights[..old_weights.len()],
        &old_weights[..],
        "source weights moved"
    );

    // The pre-existing span is reproduced point for point: nothing was re-fitted.
    for index in 0..=8 {
        let t = t_min + (t_max - t_min) * index as f64 / 8.0;
        let point = grown.evaluate(t);
        let expected = original.evaluate(t);
        assert!(
            close3(point, expected, 1e-12),
            "the old span changed at t = {t}: {point:?} != {expected:?}"
        );
    }

    // One transaction / one undo step, and undo restores the stored definition.
    assert_eq!(undo_depth(&app), 1);
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("the spline extension is undoable");
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(entity(&app, 1).geometry, before);
}

#[test]
fn spline_total_and_percent_growth_use_the_requested_length() {
    // The degree-1 fixture is exactly 7 long with its end tangent +Y at (3, 4), so
    // the policy's end point is fully determined by the requested length.
    let cases: [(f64, f64, Point3, bool); 3] = [
        (TOTAL, 10.0, point(3.0, 7.0), true),
        (PERCENT, 150.0, point(3.0, 7.5), true),
        // A Total equal to the current length is the named no-op.
        (TOTAL, 7.0, point(3.0, 4.0), false),
    ];
    for (mode, value, expected_end, changed) in cases {
        let label = format!("mode {mode} value {value}");
        let (mut app, mut session) = app_with(vec![curve_entity(
            1,
            LayerId(0),
            linear_spline(),
            "AcDbSpline",
            0,
        )]);
        set_mode_value(&mut session, mode, value);
        let outcome = lengthen(&mut app, &mut session, vec![ref_of(1)])
            .unwrap_or_else(|error| panic!("{label} refused: {error:?}"));
        if !changed {
            assert_eq!(outcome.diagnostics[0].code, "lengthen.unchanged", "{label}");
            assert_eq!(undo_depth(&app), 0, "{label}: no transaction");
            assert_eq!(entity(&app, 1).geometry, linear_spline(), "{label}");
            continue;
        }
        let after = entity(&app, 1).geometry;
        let (start, end) = spline_endpoints(&after);
        assert!(
            close3(start, point(0.0, 0.0), 1e-12),
            "{label}: start moved"
        );
        assert!(
            close3(end, expected_end, 1e-12),
            "{label}: end {end:?} != {expected_end:?}"
        );
        // The old span is intact: the degree-1 corner is still at (3, 0).
        assert!(
            close3(spline_target(&after).evaluate(1.0), point(3.0, 0.0), 1e-12),
            "{label}: the pre-existing span changed"
        );
        assert_eq!(undo_depth(&app), 1, "{label}: one undo step");
    }
}

#[test]
fn spline_percent_under_100_still_shrinks_exactly() {
    // 50% of the exact 7 is 3.5: three units along the first span, then half of the
    // second one. The kept piece is the source's own sub-curve, so the end lands on
    // (3, 0.5) exactly.
    let (mut app, mut session) = app_with(vec![curve_entity(
        1,
        LayerId(0),
        linear_spline(),
        "AcDbSpline",
        0,
    )]);
    set_mode_value(&mut session, PERCENT, 50.0);
    lengthen(&mut app, &mut session, vec![ref_of(1)]).expect("50% of 7 is 3.5");

    let after = entity(&app, 1).geometry;
    let (start, end) = spline_endpoints(&after);
    assert!(close3(start, point(0.0, 0.0), 1e-12), "the kept end moved");
    assert!(
        close3(end, point(3.0, 0.5), 1e-12),
        "the exact sub-curve should end at (3, 0.5), got {end:?}"
    );
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn spline_dynamic_mode_extends_on_the_tangent_and_shrinks_to_the_curve() {
    // Beyond the end: the picked point is projected onto the end-tangent ray, so it
    // extends by the projected distance (5 here).
    let (mut app, mut session) = app_with(vec![curve_entity(
        1,
        LayerId(0),
        linear_spline(),
        "AcDbSpline",
        0,
    )]);
    let outcome = lengthen_dynamic(&mut app, &mut session, 1, point(3.0, 4.0), point(3.0, 9.0))
        .expect("the dynamic extension is a tangent continuation");
    assert_eq!(outcome.diagnostics[0].code, "drawing.lengthen");
    let (start, end) = spline_endpoints(&entity(&app, 1).geometry);
    assert!(close3(start, point(0.0, 0.0), 1e-12));
    assert!(close3(end, point(3.0, 9.0), 1e-12), "got {end:?}");
    assert_eq!(undo_depth(&app), 1);

    // In front of the end: the picked point is projected onto the curve itself, so
    // it is an exact shrink to (3, 2).
    let (mut app, mut session) = app_with(vec![curve_entity(
        1,
        LayerId(0),
        linear_spline(),
        "AcDbSpline",
        0,
    )]);
    lengthen_dynamic(&mut app, &mut session, 1, point(3.0, 4.0), point(3.0, 2.0))
        .expect("the dynamic shrink keeps the exact sub-curve");
    let (start, end) = spline_endpoints(&entity(&app, 1).geometry);
    assert!(close3(start, point(0.0, 0.0), 1e-12));
    assert!(close3(end, point(3.0, 2.0), 1e-12), "got {end:?}");
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn a_closed_spline_frame_is_refused_by_name() {
    // First control point == last: a closed frame has no end to move.
    let closed = SemanticGeometry::Spline {
        degree: 3,
        knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 2.0, 2.0, 2.0],
        control_points: vec![
            point(0.0, 0.0),
            point(2.0, 0.0),
            point(2.0, 2.0),
            point(0.0, 2.0),
            point(0.0, 0.0),
        ],
        weights: vec![1.0; 5],
    };
    for (mode, value) in [(DELTA, 1.0), (DELTA, -1.0), (TOTAL, 20.0)] {
        let (mut app, mut session) = app_with(vec![curve_entity(
            1,
            LayerId(0),
            closed.clone(),
            "AcDbSpline",
            0,
        )]);
        set_mode_value(&mut session, mode, value);
        let error = lengthen(&mut app, &mut session, vec![ref_of(1)])
            .expect_err("a closed SPLINE has no end");
        assert_invalid(error, "lengthen.closed_spline");
        assert_eq!(undo_depth(&app), 0, "mode {mode} {value}: no transaction");
        assert_eq!(entity(&app, 1).geometry, closed, "mode {mode} {value}");
    }
}

#[test]
fn spline_degenerate_and_collapsing_requests_are_refused_by_name() {
    // A spline with no measurable length.
    let collapsed = SemanticGeometry::Spline {
        degree: 2,
        knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        control_points: vec![point(2.0, 2.0); 3],
        weights: vec![1.0; 3],
    };
    let (mut app, mut session) = app_with(vec![curve_entity(
        1,
        LayerId(0),
        collapsed.clone(),
        "AcDbSpline",
        0,
    )]);
    set_mode_value(&mut session, DELTA, 1.0);
    let error = lengthen(&mut app, &mut session, vec![ref_of(1)])
        .expect_err("a zero-length spline is refused");
    assert_invalid(error, "lengthen.degenerate");
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(entity(&app, 1).geometry, collapsed);

    // A delta that would collapse the curve: 7 - 7 is not a length.
    let (mut app, mut session) = app_with(vec![curve_entity(
        1,
        LayerId(0),
        linear_spline(),
        "AcDbSpline",
        0,
    )]);
    set_mode_value(&mut session, DELTA, -7.0);
    let error = lengthen(&mut app, &mut session, vec![ref_of(1)])
        .expect_err("a non-positive new length is refused");
    assert_invalid(error, "lengthen.non_positive");
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(entity(&app, 1).geometry, linear_spline());

    // An end whose first derivative vanishes (the last two control points
    // coincide): there is no direction to continue along.
    let flat_end = SemanticGeometry::Spline {
        degree: 2,
        knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        control_points: vec![point(0.0, 0.0), point(1.0, 1.0), point(1.0, 1.0)],
        weights: vec![1.0; 3],
    };
    let (mut app, mut session) = app_with(vec![curve_entity(
        1,
        LayerId(0),
        flat_end.clone(),
        "AcDbSpline",
        0,
    )]);
    set_mode_value(&mut session, DELTA, 1.0);
    let error = lengthen(&mut app, &mut session, vec![ref_of(1)])
        .expect_err("a degenerate end tangent is refused");
    assert_invalid(error, "lengthen.spline_degenerate_tangent");
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(entity(&app, 1).geometry, flat_end);

    // An unclamped moving end: appending a span would change the existing curve
    // instead of adding a tail, so the definition is refused, never re-clamped.
    let unclamped = SemanticGeometry::Spline {
        degree: 2,
        knots: vec![0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 3.0],
        control_points: vec![
            point(0.0, 0.0),
            point(1.0, 1.0),
            point(2.0, 0.0),
            point(3.0, -1.0),
        ],
        weights: vec![1.0; 4],
    };
    let (mut app, mut session) = app_with(vec![curve_entity(
        1,
        LayerId(0),
        unclamped.clone(),
        "AcDbSpline",
        0,
    )]);
    set_mode_value(&mut session, DELTA, 1.0);
    let error =
        lengthen(&mut app, &mut session, vec![ref_of(1)]).expect_err("an unclamped end is refused");
    assert_invalid(error, "lengthen.spline_unclamped");
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(entity(&app, 1).geometry, unclamped);
}

#[test]
fn spline_growth_moves_the_end_nearest_the_pick() {
    // The start-tangent mirror: picking near the start grows that end along
    // `−C'(t_min)` = (−1, 0), and the source control points reappear verbatim as
    // the tail of the new definition.
    let before = linear_spline();
    let (mut app, mut session) = app_with(vec![curve_entity(
        1,
        LayerId(0),
        before.clone(),
        "AcDbSpline",
        0,
    )]);
    set_mode_value(&mut session, DELTA, 2.0);
    lengthen_static_pick(&mut app, &mut session, 1, point(0.0, 0.0))
        .expect("the picked (near) end grows");

    let after = entity(&app, 1).geometry;
    let (start, end) = spline_endpoints(&after);
    assert!(
        close3(start, point(-2.0, 0.0), 1e-12),
        "the start should grow to (-2, 0), got {start:?}"
    );
    assert!(
        close3(end, point(3.0, 4.0), 1e-12),
        "the far end must not move, got {end:?}"
    );
    let (_, old_knots, old_points, _) = spline_parts(&before);
    let (degree, knots, control_points, _) = spline_parts(&after);
    assert_eq!(
        &control_points[control_points.len() - old_points.len()..],
        &old_points[..],
        "the source control points must reappear verbatim"
    );
    assert_eq!(control_points.len(), old_points.len() + degree as usize + 1);
    assert_eq!(knots.len(), control_points.len() + degree as usize + 1);
    // Mirroring back re-parameterises the whole curve, so the source knots sit at
    // an offset: the *shape* is what must be reproduced (old start, old corner,
    // old end), not the raw knot values.
    let curve = spline_target(&after);
    assert_eq!(
        old_knots.len(),
        5,
        "the fixture is a 3-point degree-1 spline"
    );
    assert!(close3(curve.evaluate(1.0), point(0.0, 0.0), 1e-12));
    assert!(close3(curve.evaluate(2.0), point(3.0, 0.0), 1e-12));
    assert!(close3(curve.evaluate(3.0), point(3.0, 4.0), 1e-12));
    assert_eq!(undo_depth(&app), 1);
}
