//! Draw-tool contracts for the AutoCAD-2026 draw variants (WIPEOUT and the
//! circumscribed POLYGON).
//!
//! Split out of `draw_tool/tests.rs` (which is at the code-size limit). Each test
//! pins the pure capture shape only: arity, readiness, and the intent the state
//! machine produces. The geometry construction and every refusal live in the
//! command layer and are tested there.

use super::*;

#[test]
fn wipeout_is_an_open_ended_polygon_capture() {
    let mut tool = DrawTool::new(DrawToolKind::Wipeout, 0);
    assert_eq!(DrawToolKind::Wipeout.arity(), PointArity::AtLeast(3));
    assert_eq!(DrawToolKind::Wipeout.key(), "wipeout");
    assert_eq!(DrawToolKind::Wipeout.label(), "WIPEOUT");
    assert!(!DrawToolKind::Wipeout.requires_selection());
    assert!(!DrawToolKind::Wipeout.needs_picked_entities());
    assert!(!DrawToolKind::Wipeout.requires_text());
    assert!(!tool.requires_entity_pick());
    assert!(matches!(tool.intent(), Err(CadError::InvalidInput(_))));

    // Two points are not a ring and the exact arity model reports it.
    tool.push_point(p(0.0, 0.0)).unwrap();
    tool.push_point(p(10.0, 0.0)).unwrap();
    assert_eq!(tool.remaining(), Some(1));
    assert!(!tool.is_ready());
    assert!(!tool.preview().can_confirm());

    tool.push_point(p(10.0, 10.0)).unwrap();
    assert_eq!(tool.remaining(), None, "open-ended past the minimum");
    assert!(tool.is_ready());
    assert!(tool.preview().is_open_ended());
    assert_eq!(
        tool.intent().unwrap(),
        DrawIntent::Wipeout {
            points: vec![p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0)],
        }
    );

    // More vertices stay accepted and are all carried by the intent.
    tool.push_point(p(0.0, 10.0)).unwrap();
    assert_eq!(
        tool.intent().unwrap(),
        DrawIntent::Wipeout {
            points: vec![p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0), p(0.0, 10.0),],
        }
    );
    // The pure tool never invents a side count for WIPEOUT.
    assert_eq!(tool.preview().polygon_sides, None);
}

#[test]
fn wipeout_status_line_reports_the_three_point_minimum() {
    let mut tool = DrawTool::new(DrawToolKind::Wipeout, 0);
    tool.push_point(p(0.0, 0.0)).unwrap();
    assert_eq!(
        tool.preview().status_line(),
        "wipeout: 1 point(s) captured, at least 3 needed"
    );
    tool.push_point(p(1.0, 0.0)).unwrap();
    tool.push_point(p(1.0, 1.0)).unwrap();
    assert_eq!(
        tool.preview().status_line(),
        "wipeout: 3 point(s) captured, press Enter to finish"
    );
}

#[test]
fn polygon_circumscribed_is_a_two_point_capture_with_its_own_key() {
    let mut tool = DrawTool::new(DrawToolKind::PolygonCircumscribed, 0);
    assert_eq!(
        DrawToolKind::PolygonCircumscribed.arity(),
        PointArity::Exact(2)
    );
    assert_eq!(
        DrawToolKind::PolygonCircumscribed.key(),
        "polygoncircumscribed"
    );
    assert_eq!(
        DrawToolKind::PolygonCircumscribed.label(),
        "POLYGON CIRCUMSCRIBED"
    );
    assert!(!DrawToolKind::PolygonCircumscribed.requires_selection());
    assert!(!DrawToolKind::PolygonCircumscribed.needs_picked_entities());
    assert!(!DrawToolKind::PolygonCircumscribed.requires_text());
    assert!(!tool.requires_entity_pick());
    assert!(matches!(tool.intent(), Err(CadError::InvalidInput(_))));

    tool.push_point(p(0.0, 0.0)).unwrap();
    assert_eq!(tool.remaining(), Some(1));
    assert!(!tool.is_ready());
    tool.push_point(p(3.0, 4.0)).unwrap();
    assert!(tool.is_ready());
    assert_eq!(
        tool.intent().unwrap(),
        DrawIntent::PolygonCircumscribed {
            center: p(0.0, 0.0),
            radius_point: p(3.0, 4.0),
        }
    );
    // A third click is refused: the capture is a fixed two-point complement.
    assert!(tool.push_point(p(9.0, 9.0)).is_err());
    assert_eq!(tool.points().len(), 2);
    // The pure tool never invents a sysvar; the session annotates the side count.
    assert_eq!(tool.preview().polygon_sides, None);
}

#[test]
fn polygon_circumscribed_vertices_are_tangent_and_counter_clockwise() {
    // n = 4, centre (0,0), tangent radius 2 along +X: the picked direction is the
    // midpoint of the vertical edge at x = 2, so the four vertices are the
    // square's corners and every edge midpoint is exactly 2 from the centre.
    let points =
        crate::draw_tool::circumscribed_polygon_vertices(p(0.0, 0.0), p(2.0, 0.0), 4).unwrap();
    assert_eq!(points.len(), 4);
    let expected = [(2.0, -2.0), (2.0, 2.0), (-2.0, 2.0), (-2.0, -2.0)];
    for (point, (x, y)) in points.iter().zip(expected) {
        assert!((point.x - x).abs() <= 1e-9, "{point:?}");
        assert!((point.y - y).abs() <= 1e-9, "{point:?}");
        assert_eq!(point.z, 0.0);
    }
    for index in 0..4 {
        let a = points[index];
        let b = points[(index + 1) % 4];
        let midpoint = ((a.x + b.x) * 0.5, (a.y + b.y) * 0.5);
        assert!((midpoint.0.hypot(midpoint.1) - 2.0).abs() <= 1e-9);
    }
    // Counter-clockwise: the signed shoelace sum is positive.
    let mut twice_area = 0.0;
    for index in 0..4 {
        let a = points[index];
        let b = points[(index + 1) % 4];
        twice_area += a.x * b.y - b.x * a.y;
    }
    assert!(twice_area > 0.0, "winding must be counter-clockwise");
}

#[test]
fn polygon_circumscribed_helper_refuses_bad_input_by_name() {
    let cases = [
        (
            p(f64::NAN, 0.0),
            p(1.0, 0.0),
            4,
            "polygoncircumscribed.invalid_point",
        ),
        (
            p(0.0, 0.0),
            p(f64::INFINITY, 0.0),
            4,
            "polygoncircumscribed.invalid_point",
        ),
        (
            p(0.0, 0.0),
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 5.0,
            },
            4,
            "polygoncircumscribed.not_planar",
        ),
        (p(0.0, 0.0), p(1.0, 0.0), 2, "polygoncircumscribed.sides"),
    ];
    for (center, tangent, sides, reason) in cases {
        let error =
            crate::draw_tool::circumscribed_polygon_vertices(center, tangent, sides).unwrap_err();
        let CadError::InvalidInput(message) = error else {
            panic!("expected an InvalidInput refusal, got {error:?}");
        };
        assert!(message.contains(reason), "expected {reason}, got {message}");
    }
}
