//! Extended `JOIN` modes: gaps, `Close`, ELLIPSE/POLYLINE/SPLINE sources and
//! MULTIPLE (including the mixed-type exact NURBS weld).

use super::*;
use cad_geometry::NurbsCurve;

// --- source-object modes: GAPS, Close, ELLIPSE, POLYLINE, SPLINE -------------

#[test]
fn two_arcs_with_a_gap_join_into_the_covering_arc() {
    let (mut app, mut session) = app_with(vec![
        arc_entity(
            1,
            LayerId(0),
            arc_geometry(point(0.0, 0.0), 5.0, 0.0, 0.5),
            0,
        ),
        arc_entity(
            2,
            LayerId(0),
            arc_geometry(point(0.0, 0.0), 5.0, 1.2, 0.3),
            1,
        ),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    join(&mut app, &mut session).expect("the ARC source-object mode allows a gap");
    match one_geometry(&app) {
        SemanticGeometry::Arc {
            center,
            radius,
            start,
            sweep,
            ..
        } => {
            assert!(near(center, point(0.0, 0.0)));
            assert!((radius - 5.0).abs() < 1e-12);
            assert!((start - 0.0).abs() < 1e-12, "start = {start}");
            assert!((sweep - 1.5).abs() < 1e-12, "sweep = {sweep}");
        }
        other => panic!("expected one ARC, got {other:?}"),
    }
    assert_eq!(entity_count(&app), 1);
}

#[test]
fn arc_close_on_a_full_turn_makes_a_circle() {
    let (mut app, mut session) = app_with(vec![
        arc_entity(
            1,
            LayerId(0),
            arc_geometry(point(0.0, 0.0), 5.0, 0.0, std::f64::consts::PI),
            0,
        ),
        arc_entity(
            2,
            LayerId(0),
            arc_geometry(
                point(0.0, 0.0),
                5.0,
                std::f64::consts::PI,
                std::f64::consts::PI,
            ),
            1,
        ),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    join_with(&mut app, &mut session, true, JoinMode::SourceObject)
        .expect("a full turn closes into a CIRCLE");
    match one_geometry(&app) {
        SemanticGeometry::Circle {
            center,
            radius,
            normal,
        } => {
            assert!(near(center, point(0.0, 0.0)));
            assert!((radius - 5.0).abs() < 1e-12);
            assert!(near(normal, p3(0.0, 0.0, 1.0)));
        }
        other => panic!("expected one CIRCLE, got {other:?}"),
    }
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn arc_close_short_of_a_full_turn_is_refused() {
    let (mut app, mut session) = app_with(vec![
        arc_entity(
            1,
            LayerId(0),
            arc_geometry(point(0.0, 0.0), 5.0, 0.0, 0.5),
            0,
        ),
        arc_entity(
            2,
            LayerId(0),
            arc_geometry(point(0.0, 0.0), 5.0, 1.2, 0.3),
            1,
        ),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    let before = revision(&app);
    let error = join_with(&mut app, &mut session, true, JoinMode::SourceObject).unwrap_err();
    assert!(
        format!("{error:?}").contains("join.not_closed"),
        "{error:?}"
    );
    assert_eq!(revision(&app), before);
    assert_eq!(entity_count(&app), 2);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn elliptical_arcs_with_a_gap_join_into_one_elliptical_arc() {
    let (mut app, mut session) = app_with(vec![
        part(
            1,
            ellipse_geometry(point(0.0, 0.0), 0.0, 1.0),
            LayerId(0),
            SpaceId::Model,
            "AcDbEllipse",
            0,
        ),
        part(
            2,
            ellipse_geometry(point(0.0, 0.0), 2.0, 1.0),
            LayerId(0),
            SpaceId::Model,
            "AcDbEllipse",
            1,
        ),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    join(&mut app, &mut session).expect("same-frame ELLIPSE arcs with a gap merge");
    match one_geometry(&app) {
        SemanticGeometry::Ellipse {
            center,
            major_axis,
            ratio,
            start,
            sweep,
            ..
        } => {
            assert!(near(center, point(0.0, 0.0)));
            assert!(near(major_axis, p3(5.0, 0.0, 0.0)));
            assert!((ratio - 0.5).abs() < 1e-12);
            assert!((start - 0.0).abs() < 1e-12, "start = {start}");
            assert!((sweep - 3.0).abs() < 1e-12, "sweep = {sweep}");
        }
        other => panic!("expected one ELLIPSE, got {other:?}"),
    }
}

#[test]
fn ellipse_close_on_a_full_turn_keeps_a_full_ellipse() {
    let (mut app, mut session) = app_with(vec![
        part(
            1,
            ellipse_geometry(point(0.0, 0.0), 0.0, std::f64::consts::PI),
            LayerId(0),
            SpaceId::Model,
            "AcDbEllipse",
            0,
        ),
        part(
            2,
            ellipse_geometry(point(0.0, 0.0), std::f64::consts::PI, std::f64::consts::PI),
            LayerId(0),
            SpaceId::Model,
            "AcDbEllipse",
            1,
        ),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    join_with(&mut app, &mut session, true, JoinMode::SourceObject)
        .expect("a full ELLIPSE turn closes");
    match one_geometry(&app) {
        SemanticGeometry::Ellipse { sweep, .. } => {
            assert!(
                (sweep.abs() - std::f64::consts::TAU).abs() < 1e-9,
                "sweep = {sweep}"
            );
        }
        other => panic!("expected one ELLIPSE, got {other:?}"),
    }
}

#[test]
fn polyline_source_welds_a_touching_line_and_arc() {
    let (mut app, mut session) = app_with(vec![
        part(
            1,
            polyline_geometry(vec![point(0.0, 0.0), point(4.0, 0.0)], vec![0.0], false),
            LayerId(0),
            SpaceId::Model,
            "AcDbPolyline",
            0,
        ),
        arc_entity(
            2,
            LayerId(0),
            SemanticGeometry::Arc {
                center: point(4.0, 4.0),
                normal: p3(0.0, 0.0, 1.0),
                radius: 4.0,
                start: -std::f64::consts::FRAC_PI_2,
                sweep: std::f64::consts::FRAC_PI_2,
            },
            1,
        ),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    join(&mut app, &mut session).expect("a POLYLINE source accepts an ARC");
    match one_geometry(&app) {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
            ..
        } => {
            assert!(!closed);
            assert_eq!(points.len(), 3);
            assert!(near(points[0], point(0.0, 0.0)));
            assert!(near(points[1], point(4.0, 0.0)));
            assert!(near(points[2], point(8.0, 4.0)));
            assert_eq!(bulges.len(), 2);
            assert!((bulges[0] - 0.0).abs() < 1e-12);
            assert!(
                (bulges[1] - (std::f64::consts::FRAC_PI_8).tan()).abs() < 1e-9,
                "the ARC is one bulge: {}",
                bulges[1]
            );
        }
        other => panic!("expected one POLYLINE, got {other:?}"),
    }
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn spline_source_concatenates_contiguous_pieces_exactly() {
    let first = vec![
        point(0.0, 0.0),
        point(1.0, 1.0),
        point(2.0, 1.0),
        point(3.0, 0.0),
    ];
    let second = vec![
        point(3.0, 0.0),
        point(4.0, -1.0),
        point(5.0, -1.0),
        point(6.0, 0.0),
    ];
    let (mut app, mut session) = app_with(vec![
        part(
            1,
            spline_geometry(first.clone(), 3),
            LayerId(0),
            SpaceId::Model,
            "AcDbSpline",
            0,
        ),
        part(
            2,
            spline_geometry(second, 3),
            LayerId(0),
            SpaceId::Model,
            "AcDbSpline",
            1,
        ),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    join(&mut app, &mut session).expect("contiguous SPLINEs concatenate");
    match one_geometry(&app) {
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => {
            assert_eq!(degree, 3);
            assert_eq!(control_points.len(), 8, "both control frames are kept");
            assert_eq!(knots.len(), 12, "n + degree + 1");
            assert_eq!(weights, vec![1.0; 8]);
            assert!(near(control_points[0], point(0.0, 0.0)));
            assert!(near(
                *control_points.last().expect("eight points"),
                point(6.0, 0.0)
            ));
        }
        other => panic!("expected one SPLINE, got {other:?}"),
    }
    assert_eq!(entity_count(&app), 1);
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn a_spline_degree_mismatch_is_unsupported() {
    let first = vec![
        point(0.0, 0.0),
        point(1.0, 1.0),
        point(2.0, 1.0),
        point(3.0, 0.0),
    ];
    let second = vec![point(3.0, 0.0), point(4.0, -1.0), point(5.0, 0.0)];
    let (mut app, mut session) = app_with(vec![
        part(
            1,
            spline_geometry(first, 3),
            LayerId(0),
            SpaceId::Model,
            "AcDbSpline",
            0,
        ),
        part(
            2,
            spline_geometry(second, 2),
            LayerId(0),
            SpaceId::Model,
            "AcDbSpline",
            1,
        ),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    let before = revision(&app);
    let error = join(&mut app, &mut session).unwrap_err();
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert!(
        format!("{error:?}").contains("join.spline_degree"),
        "{error:?}"
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

// --- MULTIPLE mode -----------------------------------------------------------

#[test]
fn multiple_mode_welds_line_plus_arc_into_a_polyline() {
    let (mut app, mut session) = app_with(vec![
        line_entity(1, LayerId(0), line_geometry(0.0, 0.0, 4.0, 0.0), 0),
        arc_entity(
            2,
            LayerId(0),
            SemanticGeometry::Arc {
                center: point(4.0, 4.0),
                normal: p3(0.0, 0.0, 1.0),
                radius: 4.0,
                start: -std::f64::consts::FRAC_PI_2,
                sweep: std::f64::consts::FRAC_PI_2,
            },
            1,
        ),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    join_with(&mut app, &mut session, false, JoinMode::Multiple)
        .expect("LINE + ARC is valid in MULTIPLE mode");
    match one_geometry(&app) {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
            ..
        } => {
            assert!(!closed);
            assert_eq!(points.len(), 3);
            assert!(near(points[2], point(8.0, 4.0)));
            assert!((bulges[1] - (std::f64::consts::FRAC_PI_8).tan()).abs() < 1e-9);
        }
        other => panic!("expected one POLYLINE, got {other:?}"),
    }
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn multiple_mode_collinear_lines_make_one_line_across_the_gap() {
    let (mut app, mut session) = app_with(vec![
        line_entity(1, LayerId(0), line_geometry(0.0, 0.0, 4.0, 0.0), 0),
        line_entity(2, LayerId(0), line_geometry(5.0, 0.0, 9.0, 0.0), 1),
        line_entity(3, LayerId(0), line_geometry(2.0, 0.0, 6.0, 0.0), 2),
    ]);
    select(&mut app, &mut session, &[1, 2, 3]);
    join_with(&mut app, &mut session, false, JoinMode::Multiple)
        .expect("collinear LINEs collapse to one LINE in MULTIPLE mode");
    match one_geometry(&app) {
        SemanticGeometry::Line { start, end } => {
            assert!(near(start, point(0.0, 0.0)));
            assert!(near(end, point(9.0, 0.0)));
        }
        other => panic!("expected one LINE, got {other:?}"),
    }
}

// --- MULTIPLE mode: mixed-type NURBS weld ------------------------------------

/// Evaluate the welded spline of the drawing at `t` (its own knot domain).
fn evaluate_weld(app: &Application, t: f64) -> Point3 {
    match one_geometry(app) {
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => NurbsCurve::new(degree as usize, control_points, knots, weights)
            .expect("valid weld")
            .evaluate(t),
        other => panic!("expected one SPLINE, got {other:?}"),
    }
}

fn spline_cps(app: &Application) -> (u32, Vec<f64>, Vec<Point3>, Vec<f64>) {
    match one_geometry(app) {
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => (degree, knots, control_points, weights),
        other => panic!("expected one SPLINE, got {other:?}"),
    }
}

/// The exact eccentric angle at local Bezier parameter `s` of one rational
/// quadratic span starting at `start` with angular `step`.
///
/// The rational quadratic map is `tan(θ/2) = s·sin(h) / (1 − s + s·cos(h))`
/// with `h = step/2`, so the parameter is not the angle and a sample has to
/// invert that map instead of assuming `θ = start + step·s`.
fn rational_ellipse_angle(start: f64, step: f64, s: f64) -> f64 {
    let half = step * 0.5;
    start + 2.0 * (s * half.sin()).atan2(1.0 - s + s * half.cos())
}

#[test]
fn multiple_mode_welds_a_line_and_a_spline_exactly() {
    // LINE (0,0)→(4,0) touches a cubic single-span spline starting at (4,0).
    let source = spline_geometry(
        vec![
            point(4.0, 0.0),
            point(5.0, 2.0),
            point(7.0, 2.0),
            point(9.0, 0.0),
        ],
        3,
    );
    let SemanticGeometry::Spline {
        degree,
        knots,
        control_points,
        weights,
    } = source
    else {
        unreachable!()
    };
    let original = NurbsCurve::new(degree as usize, control_points, knots, weights).unwrap();
    let (mut app, mut session) = app_with(vec![
        line_entity(1, LayerId(0), line_geometry(0.0, 0.0, 4.0, 0.0), 0),
        part(
            2,
            spline_geometry(
                vec![
                    point(4.0, 0.0),
                    point(5.0, 2.0),
                    point(7.0, 2.0),
                    point(9.0, 0.0),
                ],
                3,
            ),
            LayerId(0),
            SpaceId::Model,
            "AcDbSpline",
            1,
        ),
    ]);
    let before_revision = revision(&app);
    select(&mut app, &mut session, &[1, 2]);
    join_with(&mut app, &mut session, false, JoinMode::Multiple)
        .expect("LINE + SPLINE welds into one exact SPLINE");
    let (degree_out, knots_out, control_points, weights) = spline_cps(&app);
    assert_eq!(degree_out, 3);
    assert_eq!(
        control_points.len(),
        8,
        "the elevated LINE span plus the spline span"
    );
    assert_eq!(knots_out.len(), 8 + 3 + 1);
    assert_eq!(weights, vec![1.0; 8], "the LINE elevation is polynomial");
    assert!(near(control_points[0], point(0.0, 0.0)));
    assert!(near(
        *control_points.last().expect("eight points"),
        point(9.0, 0.0)
    ));
    // First span = the straight LINE, exact: t = 0.5 is its midpoint.
    assert!(near(evaluate_weld(&app, 0.5), point(2.0, 0.0)));
    assert!(near(evaluate_weld(&app, 1.0), point(4.0, 0.0)));
    // Second span = the source spline, unchanged: t = 1 + u.
    for step in 0..=10 {
        let u = step as f64 / 10.0;
        let welded = evaluate_weld(&app, 1.0 + u);
        let expected = original.evaluate(u);
        assert!(
            near(welded, expected),
            "spline span at u={u}: {welded:?} vs {expected:?}"
        );
    }
    assert_eq!(entity_count(&app), 1);
    assert_eq!(revision(&app).0, before_revision.0 + 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "one undo step");
}

#[test]
fn multiple_mode_welds_a_polyline_and_a_reversed_spline_exactly() {
    // POLYLINE (0,0)→(4,0)→(8,0), then a spline GIVEN right-to-left: its tail
    // touches the polyline, so the weld must reverse it.
    let reversed_source = vec![
        point(10.0, 0.0),
        point(9.0, 1.5),
        point(8.5, 1.5),
        point(8.0, 0.0),
    ];
    let original = NurbsCurve::new(
        3,
        reversed_source.clone(),
        clamped_uniform_knots(4, 3),
        vec![1.0; 4],
    )
    .unwrap();
    let (mut app, mut session) = app_with(vec![
        part(
            1,
            polyline_geometry(
                vec![point(0.0, 0.0), point(4.0, 0.0), point(8.0, 0.0)],
                vec![0.0, 0.0],
                false,
            ),
            LayerId(0),
            SpaceId::Model,
            "AcDbPolyline",
            0,
        ),
        part(
            2,
            spline_geometry(reversed_source, 3),
            LayerId(0),
            SpaceId::Model,
            "AcDbSpline",
            1,
        ),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    join_with(&mut app, &mut session, false, JoinMode::Multiple)
        .expect("POLYLINE + SPLINE welds into one exact SPLINE");
    let (degree, knots, control_points, weights) = spline_cps(&app);
    assert_eq!(degree, 3);
    assert_eq!(
        control_points.len(),
        12,
        "two polyline spans + one spline span"
    );
    assert_eq!(knots.len(), 16);
    assert_eq!(weights, vec![1.0; 12]);
    assert!(near(control_points[0], point(0.0, 0.0)));
    assert!(near(
        *control_points.last().expect("twelve"),
        point(10.0, 0.0)
    ));
    assert!(near(evaluate_weld(&app, 1.0), point(4.0, 0.0)));
    assert!(near(evaluate_weld(&app, 2.0), point(8.0, 0.0)));
    // The reversed span is the source curve traversed backwards.
    for step in 0..=10 {
        let u = step as f64 / 10.0;
        let welded = evaluate_weld(&app, 2.0 + u);
        let expected = original.evaluate(1.0 - u);
        assert!(
            near(welded, expected),
            "reversed spline span at u={u}: {welded:?} vs {expected:?}"
        );
    }
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn multiple_mode_welds_an_arc_and_a_spline_into_a_rational_spline() {
    // LINE (0,0)→(4,0), the 90° ARC (4,0)→(8,4), then a cubic spline from (8,4).
    let (mut app, mut session) = app_with(vec![
        line_entity(1, LayerId(0), line_geometry(0.0, 0.0, 4.0, 0.0), 0),
        arc_entity(
            2,
            LayerId(0),
            arc_geometry(
                point(4.0, 4.0),
                4.0,
                -std::f64::consts::FRAC_PI_2,
                std::f64::consts::FRAC_PI_2,
            ),
            1,
        ),
        part(
            3,
            spline_geometry(
                vec![
                    point(8.0, 4.0),
                    point(9.0, 6.0),
                    point(11.0, 6.0),
                    point(12.0, 4.0),
                ],
                3,
            ),
            LayerId(0),
            SpaceId::Model,
            "AcDbSpline",
            2,
        ),
    ]);
    select(&mut app, &mut session, &[1, 2, 3]);
    join_with(&mut app, &mut session, false, JoinMode::Multiple)
        .expect("LINE + ARC + SPLINE welds into one rational SPLINE");
    let (degree, _, _, weights) = spline_cps(&app);
    assert_eq!(degree, 3, "the common degree is the spline's");
    assert!(
        weights.iter().any(|weight| (weight - 1.0).abs() > 1e-6),
        "the arc keeps its rational weights through the elevation: {weights:?}"
    );
    // Span 0 is the line, span 1 the arc (t ∈ [1, 2]), span 2 the spline.
    assert!(near(evaluate_weld(&app, 1.0), point(4.0, 0.0)));
    assert!(near(evaluate_weld(&app, 2.0), point(8.0, 4.0)));
    let center = point(4.0, 4.0);
    for step in 1..=9 {
        let u = step as f64 / 10.0;
        let point_on_arc = evaluate_weld(&app, 1.0 + u);
        let radius =
            ((point_on_arc.x - center.x).powi(2) + (point_on_arc.y - center.y).powi(2)).sqrt();
        assert!(
            (radius - 4.0).abs() <= 1e-9,
            "the welded arc stays on the circle at u={u}: radius {radius}"
        );
    }
    // The exact mid-arc point is at -45°.
    let mid = evaluate_weld(&app, 1.5);
    let expected_mid = point(
        4.0 + 4.0 * (std::f64::consts::FRAC_PI_4).cos(),
        4.0 - 4.0 * (std::f64::consts::FRAC_PI_4).cos(),
    );
    assert!(near(mid, expected_mid), "{mid:?} vs {expected_mid:?}");
    assert_eq!(entity_count(&app), 1);
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn multiple_mode_welds_an_ellipse_arc_and_a_line_exactly() {
    // LINE (0,0)→(5,0), then the quarter elliptical arc from that point
    // (P(0) = (5,0)) to P(π/2) = (0, 2.5) on the ellipse with major axis
    // (5,0,0) and ratio 0.5.
    let (mut app, mut session) = app_with(vec![
        line_entity(1, LayerId(0), line_geometry(0.0, 0.0, 5.0, 0.0), 0),
        part(
            2,
            ellipse_geometry(point(0.0, 0.0), 0.0, std::f64::consts::FRAC_PI_2),
            LayerId(0),
            SpaceId::Model,
            "AcDbEllipse",
            1,
        ),
    ]);
    let before_revision = revision(&app);
    select(&mut app, &mut session, &[1, 2]);
    join_with(&mut app, &mut session, false, JoinMode::Multiple)
        .expect("ELLIPSE arc + LINE welds into one exact SPLINE");
    let (degree, knots, control_points, weights) = spline_cps(&app);
    assert_eq!(degree, 2, "one line span elevated to the ellipse's degree");
    assert_eq!(
        control_points.len(),
        6,
        "the elevated LINE span plus the ellipse span"
    );
    assert_eq!(knots.len(), 6 + 2 + 1);
    assert!(
        weights.iter().any(|weight| (weight - 1.0).abs() > 1e-6),
        "the ellipse keeps its rational weights: {weights:?}"
    );
    // Endpoints are the two sources': LINE start and the ellipse arc's end.
    assert!(near(evaluate_weld(&app, 0.0), point(0.0, 0.0)));
    assert!(near(evaluate_weld(&app, 1.0), point(5.0, 0.0)));
    assert!(near(evaluate_weld(&app, 2.0), point(0.0, 2.5)));
    // Span 1 is the ellipse, exactly `c + M·cos a + N·sin a`.
    for step in 0..=10 {
        let u = step as f64 / 10.0;
        let welded = evaluate_weld(&app, 1.0 + u);
        let angle = rational_ellipse_angle(0.0, std::f64::consts::FRAC_PI_2, u);
        let expected = point(5.0 * angle.cos(), 2.5 * angle.sin());
        assert!(
            near(welded, expected),
            "ellipse span at u={u}: {welded:?} vs {expected:?}"
        );
    }
    assert_eq!(entity_count(&app), 1);
    assert_eq!(revision(&app).0, before_revision.0 + 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "one undo step");
}

#[test]
fn multiple_mode_welds_an_ellipse_first_selection_in_chain_order() {
    // The same two curves picked ELLIPSE first: the LINE lies ahead of the
    // ellipse's start, so the chain has to be prepended with the LINE reversed.
    // A wrong prepend still returns a SPLINE, but it jumps at the junction.
    let (mut app, mut session) = app_with(vec![
        part(
            1,
            ellipse_geometry(point(0.0, 0.0), 0.0, std::f64::consts::FRAC_PI_2),
            LayerId(0),
            SpaceId::Model,
            "AcDbEllipse",
            0,
        ),
        line_entity(2, LayerId(0), line_geometry(5.0, 0.0, 9.0, 0.0), 1),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    join_with(&mut app, &mut session, false, JoinMode::Multiple)
        .expect("ELLIPSE arc + LINE welds regardless of pick order");
    let (degree, _, control_points, _) = spline_cps(&app);
    assert_eq!(degree, 2);
    assert_eq!(control_points.len(), 6);
    assert!(near(evaluate_weld(&app, 0.0), point(9.0, 0.0)));
    assert!(near(evaluate_weld(&app, 0.5), point(7.0, 0.0)));
    // The junction is the shared ellipse start, not the LINE's far end.
    assert!(near(evaluate_weld(&app, 1.0), point(5.0, 0.0)));
    assert!(near(evaluate_weld(&app, 2.0), point(0.0, 2.5)));
}

#[test]
fn multiple_mode_welds_an_ellipse_arc_and_an_arc_exactly() {
    // The 90° ARC (1.5,0)→(0,1.5), then the elliptical arc whose P(π/2) is that
    // same point and whose P(π) is (-3,0).
    let (mut app, mut session) = app_with(vec![
        arc_entity(
            1,
            LayerId(0),
            arc_geometry(point(0.0, 0.0), 1.5, 0.0, std::f64::consts::FRAC_PI_2),
            0,
        ),
        part(
            2,
            SemanticGeometry::Ellipse {
                center: point(0.0, 0.0),
                normal: p3(0.0, 0.0, 1.0),
                major_axis: p3(3.0, 0.0, 0.0),
                ratio: 0.5,
                start: std::f64::consts::FRAC_PI_2,
                sweep: std::f64::consts::FRAC_PI_2,
            },
            LayerId(0),
            SpaceId::Model,
            "AcDbEllipse",
            1,
        ),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    join_with(&mut app, &mut session, false, JoinMode::Multiple)
        .expect("ELLIPSE arc + ARC welds into one exact rational SPLINE");
    let (degree, _, _, weights) = spline_cps(&app);
    assert_eq!(degree, 2);
    assert!(
        weights.iter().any(|weight| (weight - 1.0).abs() > 1e-6),
        "both rational pieces keep their weights: {weights:?}"
    );
    assert!(near(evaluate_weld(&app, 0.0), point(1.5, 0.0)));
    assert!(near(evaluate_weld(&app, 1.0), point(0.0, 1.5)));
    assert!(near(evaluate_weld(&app, 2.0), point(-3.0, 0.0)));
    for step in 0..=10 {
        let u = step as f64 / 10.0;
        let welded = evaluate_weld(&app, 1.0 + u);
        let angle =
            rational_ellipse_angle(std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2, u);
        let expected = point(3.0 * angle.cos(), 1.5 * angle.sin());
        assert!(
            near(welded, expected),
            "ellipse span at u={u}: {welded:?} vs {expected:?}"
        );
    }
    assert_eq!(undo_depth(&app), 1);
}

/// A degenerate elliptical arc in a mixed selection is refused by name; the
/// refusal writes nothing and leaves no undo entry.
#[test]
fn multiple_mode_refuses_a_degenerate_ellipse_mix_by_name() {
    let ellipse = |major_axis: Point3, ratio: f64, sweep: f64| SemanticGeometry::Ellipse {
        center: point(0.0, 0.0),
        normal: p3(0.0, 0.0, 1.0),
        major_axis,
        ratio,
        start: 0.0,
        sweep,
    };
    let cases = [
        (
            ellipse(p3(0.0, 0.0, 0.0), 0.5, 1.0),
            "join.degenerate",
            "zero major axis",
        ),
        (
            ellipse(p3(5.0, 0.0, 0.0), 0.0, 1.0),
            "join.degenerate",
            "zero ratio",
        ),
        (
            ellipse(p3(5.0, 0.0, 0.0), 0.5, 0.0),
            "join.degenerate",
            "zero sweep",
        ),
        (
            ellipse(p3(5.0, 0.0, 0.0), 0.5, std::f64::consts::TAU + 0.5),
            "join.closed_object",
            "more than a full turn",
        ),
    ];
    for (geometry, code, label) in cases {
        let (mut app, mut session) = app_with(vec![
            part(1, geometry, LayerId(0), SpaceId::Model, "AcDbEllipse", 0),
            line_entity(2, LayerId(0), line_geometry(0.0, 0.0, 5.0, 0.0), 1),
        ]);
        let before_revision = revision(&app);
        select(&mut app, &mut session, &[1, 2]);
        let error = join_with(&mut app, &mut session, false, JoinMode::Multiple)
            .expect_err("the degenerate ellipse is refused");
        assert!(
            format!("{error:?}").contains(code),
            "{label}: expected {code}, got {error:?}"
        );
        assert_eq!(revision(&app), before_revision, "{label}: no revision bump");
        assert_eq!(entity_count(&app), 2, "{label}: no partial write");
        assert_eq!(undo_depth(&app), 0, "{label}: no undo entry");
    }
}

#[test]
fn multiple_mode_refuses_a_closed_mixed_weld() {
    // JOIN CLOSE on a mixed weld is refused by name (no closed spline flag).
    let (mut app, mut session) = app_with(vec![
        line_entity(1, LayerId(0), line_geometry(0.0, 0.0, 4.0, 0.0), 0),
        part(
            2,
            spline_geometry(
                vec![
                    point(4.0, 0.0),
                    point(5.0, 2.0),
                    point(7.0, 2.0),
                    point(0.0, 0.0),
                ],
                3,
            ),
            LayerId(0),
            SpaceId::Model,
            "AcDbSpline",
            1,
        ),
    ]);
    let before_revision = revision(&app);
    select(&mut app, &mut session, &[1, 2]);
    let error = join_with(&mut app, &mut session, true, JoinMode::Multiple).unwrap_err();
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert!(
        format!("{error:?}").contains("join.close_spline"),
        "{error:?}"
    );
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn multiple_mode_refuses_a_gap_in_a_mixed_weld() {
    let (mut app, mut session) = app_with(vec![
        line_entity(1, LayerId(0), line_geometry(0.0, 0.0, 4.0, 0.0), 0),
        part(
            2,
            spline_geometry(
                vec![
                    point(5.0, 0.0),
                    point(6.0, 2.0),
                    point(8.0, 2.0),
                    point(9.0, 0.0),
                ],
                3,
            ),
            LayerId(0),
            SpaceId::Model,
            "AcDbSpline",
            1,
        ),
    ]);
    let before_revision = revision(&app);
    select(&mut app, &mut session, &[1, 2]);
    let error = join_with(&mut app, &mut session, false, JoinMode::Multiple).unwrap_err();
    assert!(
        format!("{error:?}").contains("join.not_contiguous"),
        "{error:?}"
    );
    assert_eq!(revision(&app), before_revision);
    assert_eq!(entity_count(&app), 2);
    assert_eq!(undo_depth(&app), 0);
}

// --- closed / construction / non-coplanar refusals ---------------------------

#[test]
fn a_circle_is_a_named_closed_object_refusal() {
    let (mut app, mut session) = app_with(vec![
        part(
            1,
            SemanticGeometry::Circle {
                center: point(0.0, 0.0),
                normal: p3(0.0, 0.0, 1.0),
                radius: 5.0,
            },
            LayerId(0),
            SpaceId::Model,
            "AcDbCircle",
            0,
        ),
        arc_entity(
            2,
            LayerId(0),
            arc_geometry(point(0.0, 0.0), 5.0, 0.0, 0.5),
            1,
        ),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    let before = revision(&app);
    let error = join(&mut app, &mut session).unwrap_err();
    assert!(
        format!("{error:?}").contains("join.closed_object"),
        "{error:?}"
    );
    assert_eq!(revision(&app), before);
    assert_eq!(entity_count(&app), 2);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn a_construction_line_type_key_is_refused_even_though_it_is_line_geometry() {
    let (mut app, mut session) = app_with(vec![
        part(
            1,
            line_geometry(0.0, 0.0, 10.0, 0.0),
            LayerId(0),
            SpaceId::Model,
            "AcDbXline",
            0,
        ),
        line_entity(2, LayerId(0), line_geometry(10.0, 0.0, 20.0, 0.0), 1),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    let before = revision(&app);
    let error = join(&mut app, &mut session).unwrap_err();
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert!(
        format!("{error:?}").contains("join.construction_line"),
        "{error:?}"
    );
    assert_eq!(revision(&app), before);
    assert_eq!(entity_count(&app), 2);
}

#[test]
fn a_non_coplanar_polyline_source_is_an_unsupported_3d_boundary() {
    let (mut app, mut session) = app_with(vec![
        part(
            1,
            polyline_geometry(vec![p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 3.0)], vec![0.0], false),
            LayerId(0),
            SpaceId::Model,
            "AcDbPolyline",
            0,
        ),
        line_entity(2, LayerId(0), line_geometry(4.0, 0.0, 9.0, 0.0), 1),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    let before = revision(&app);
    let error = join(&mut app, &mut session).unwrap_err();
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert!(
        format!("{error:?}").contains("join.noncoplanar"),
        "{error:?}"
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

// The source-object SPLINE (mixed LINE/ARC continuation) contract lives in its
// own file (code-size limit); it is a private helper module of these tests.
#[path = "join_spline_source.rs"]
mod spline_source;
