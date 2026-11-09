//! Extended `JOIN` modes: gaps, `Close`, ELLIPSE/POLYLINE/SPLINE sources and MULTIPLE.

use super::*;

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
