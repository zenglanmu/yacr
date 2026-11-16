//! ARC intent capture and the three-point circumcircle parameters.
//!
//! Split out of `draw_tool/tests.rs` to keep that file under the code-size limit.

use super::*;

#[test]
fn arc_captures_three_points_and_confirms_an_arc_intent() {
    let mut tool = DrawTool::new(DrawToolKind::Arc, 0);
    assert_eq!(tool.remaining(), Some(3));
    tool.push_point(p(0.0, 0.0)).unwrap();
    tool.push_point(p(5.0, 5.0)).unwrap();
    assert_eq!(tool.remaining(), Some(1));
    assert!(!tool.is_ready());
    tool.push_point(p(10.0, 0.0)).unwrap();
    assert!(tool.is_ready());
    assert_eq!(
        tool.intent().unwrap(),
        DrawIntent::Arc {
            points: [p(0.0, 0.0), p(5.0, 5.0), p(10.0, 0.0)]
        }
    );
}

#[test]
fn arc_parameters_resolve_a_circumcircle_and_refuse_degenerate_input() {
    // Semicircle through (0,0), (5,5), (10,0): centre (5,0), radius 5.
    let parameters = arc_parameters(p(0.0, 0.0), p(5.0, 5.0), p(10.0, 0.0)).unwrap();
    assert!((parameters.center.x - 5.0).abs() < 1e-9, "{parameters:?}");
    assert!((parameters.center.y - 0.0).abs() < 1e-9, "{parameters:?}");
    assert!((parameters.radius - 5.0).abs() < 1e-9, "{parameters:?}");
    assert!((parameters.start - std::f64::consts::PI).abs() < 1e-9);
    // Through the upper point, the arc is clockwise: a negative sweep.
    assert!((parameters.sweep + std::f64::consts::PI).abs() < 1e-9);

    // Collinear points have no circumcircle.
    assert!(matches!(
        arc_parameters(p(0.0, 0.0), p(1.0, 0.0), p(2.0, 0.0)),
        Err(CadError::InvalidInput(_))
    ));
    // Coincident points are refused, never a degenerate arc.
    assert!(matches!(
        arc_parameters(p(0.0, 0.0), p(0.0, 0.0), p(1.0, 0.0)),
        Err(CadError::InvalidInput(_))
    ));
    // Non-finite points are refused.
    assert!(matches!(
        arc_parameters(p(0.0, 0.0), p(f64::NAN, 1.0), p(2.0, 0.0)),
        Err(CadError::InvalidInput(_))
    ));
}

#[test]
fn arc_parameters_are_translation_invariant_at_large_coordinates() {
    // The semicircle fixture translated by (1e6, 1e6): centre (1e6+5, 1e6), r=5.
    let base = 1.0e6;
    let parameters = arc_parameters(
        p(base, base),
        p(base + 5.0, base + 5.0),
        p(base + 10.0, base),
    )
    .unwrap();
    assert!(
        (parameters.center.x - (base + 5.0)).abs() < 1e-9,
        "{parameters:?}"
    );
    assert!((parameters.center.y - base).abs() < 1e-9, "{parameters:?}");
    assert!((parameters.radius - 5.0).abs() < 1e-9, "{parameters:?}");
    assert!((parameters.start - std::f64::consts::PI).abs() < 1e-9);
    assert!((parameters.sweep + std::f64::consts::PI).abs() < 1e-9);
    // The arc passes through the picked points: each is exactly one radius out.
    for point in [
        p(base, base),
        p(base + 5.0, base + 5.0),
        p(base + 10.0, base),
    ] {
        let distance = (point.x - parameters.center.x).hypot(point.y - parameters.center.y);
        assert!(
            (distance - parameters.radius).abs() < 1e-9,
            "{point:?} is not on the circumcircle"
        );
    }
}

#[test]
fn arc_parameters_accept_a_unit_arc_at_large_coordinates() {
    // A unit-radius upper semicircle centred at (1e6, 1e6) must be accepted,
    // not refused by a translation-dependent degeneracy test.
    let base = 1.0e6;
    let parameters = arc_parameters(
        p(base + 1.0, base),
        p(base, base + 1.0),
        p(base - 1.0, base),
    )
    .unwrap();
    assert!((parameters.center.x - base).abs() < 1e-9, "{parameters:?}");
    assert!((parameters.center.y - base).abs() < 1e-9, "{parameters:?}");
    assert!((parameters.radius - 1.0).abs() < 1e-9, "{parameters:?}");
    // right -> top -> left is a counter-clockwise half turn (positive sweep).
    assert!(
        (parameters.sweep - std::f64::consts::PI).abs() < 1e-9,
        "{parameters:?}"
    );
}

#[test]
fn arc_parameters_supports_a_near_full_arc() {
    let tau = std::f64::consts::TAU;
    let end_angle = tau - 0.1;
    let parameters = arc_parameters(
        p(1.0, 0.0),
        p(-1.0, 0.0),
        Point3 {
            x: end_angle.cos(),
            y: end_angle.sin(),
            z: 0.0,
        },
    )
    .unwrap();
    assert!((parameters.center.x).abs() < 1e-9, "{parameters:?}");
    assert!((parameters.center.y).abs() < 1e-9, "{parameters:?}");
    assert!((parameters.radius - 1.0).abs() < 1e-9, "{parameters:?}");
    assert!(
        (parameters.sweep - (tau - 0.1)).abs() < 1e-9,
        "{parameters:?}"
    );
}

#[test]
fn arc_parameters_refuse_non_coplanar_points() {
    // A point off the start plane must be refused, never silently projected.
    let through = Point3 {
        x: 1.0,
        y: 0.0,
        z: 5.0,
    };
    assert!(matches!(
        arc_parameters(p(0.0, 0.0), through, p(0.0, 1.0)),
        Err(CadError::InvalidInput(_))
    ));
}
