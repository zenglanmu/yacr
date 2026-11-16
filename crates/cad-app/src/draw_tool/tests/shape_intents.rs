//! Fixed-shape kinds: LINE, CIRCLE, RECTANG, ELLIPSE, POLYGON and POLYLINE —
//! their captured intents, status lines and the pure polygon vertex geometry.
//!
//! Split out of `draw_tool/tests.rs` to keep that file under the code-size limit.

use super::*;

#[test]
fn line_captures_two_points_and_confirms_a_line_intent() {
    let mut tool = DrawTool::new(DrawToolKind::Line, 0);
    assert_eq!(tool.remaining(), Some(2));
    assert!(!tool.is_ready());
    tool.push_point(p(0.0, 0.0)).unwrap();
    assert_eq!(tool.remaining(), Some(1));
    assert!(!tool.is_ready());
    tool.push_point(p(4.0, 3.0)).unwrap();
    assert!(tool.is_ready());
    assert!(tool.preview().can_confirm());
    assert_eq!(
        tool.intent().unwrap(),
        DrawIntent::Line {
            start: p(0.0, 0.0),
            end: p(4.0, 3.0)
        }
    );
}

#[test]
fn circle_intent_uses_centre_and_edge_and_radius_is_positive() {
    let mut tool = DrawTool::new(DrawToolKind::Circle, 0);
    tool.push_point(p(1.0, 1.0)).unwrap();
    tool.push_point(p(4.0, 5.0)).unwrap();
    assert_eq!(
        tool.intent().unwrap(),
        DrawIntent::Circle {
            center: p(1.0, 1.0),
            edge: p(4.0, 5.0)
        }
    );
    assert_eq!(circle_radius(p(1.0, 1.0), p(4.0, 5.0)).unwrap(), 5.0);
    // Degenerate radius is explicit, not a fabricated zero circle.
    assert!(matches!(
        circle_radius(p(1.0, 1.0), p(1.0, 1.0)),
        Err(CadError::InvalidInput(_))
    ));
}

#[test]
fn polyline_captures_two_and_three_points_into_an_exact_intent() {
    let mut two = DrawTool::new(DrawToolKind::Polyline, 0);
    two.push_point(p(0.0, 0.0)).unwrap();
    two.push_point(p(3.0, 4.0)).unwrap();
    assert_eq!(
        two.intent().unwrap(),
        DrawIntent::Polyline {
            points: vec![p(0.0, 0.0), p(3.0, 4.0)]
        }
    );

    let mut three = DrawTool::new(DrawToolKind::Polyline, 0);
    three.push_point(p(0.0, 0.0)).unwrap();
    three.push_point(p(5.0, 0.0)).unwrap();
    three.push_point(p(5.0, 5.0)).unwrap();
    assert_eq!(
        three.intent().unwrap(),
        DrawIntent::Polyline {
            points: vec![p(0.0, 0.0), p(5.0, 0.0), p(5.0, 5.0)]
        }
    );
}

#[test]
fn rectang_captures_two_corners_exactly_and_ready_at_two() {
    // RECTANG is fixed arity 2 (like LINE): ready exactly at the second corner,
    // and the (2 + 1)-th click is refused. No selection and no entity pick.
    let mut tool = DrawTool::new(DrawToolKind::Rectang, 0);
    assert_eq!(DrawToolKind::Rectang.arity(), PointArity::Exact(2));
    assert_eq!(tool.remaining(), Some(2));
    assert!(!tool.is_ready());
    assert!(!DrawToolKind::Rectang.requires_selection());
    assert!(!DrawToolKind::Rectang.needs_picked_entities());
    assert!(!tool.requires_entity_pick());

    tool.push_point(p(0.0, 0.0)).unwrap();
    assert_eq!(tool.remaining(), Some(1));
    assert!(!tool.is_ready());
    // An incomplete capture has no intent.
    assert!(matches!(tool.intent(), Err(CadError::InvalidInput(_))));

    tool.push_point(p(4.0, 3.0)).unwrap();
    assert_eq!(tool.remaining(), Some(0));
    assert!(tool.is_ready());
    assert!(tool.preview().can_confirm());
    assert_eq!(tool.preview().remaining_points(), Some(0));
    assert!(!tool.preview().is_open_ended());
    // The intent carries the two captured corners **exactly**, in click order
    // (the winding is canonicalised by the command layer, not here).
    assert_eq!(
        tool.intent().unwrap(),
        DrawIntent::Rect {
            corner1: p(0.0, 0.0),
            corner2: p(4.0, 3.0),
        }
    );

    // The third point is refused and does not disturb the capture.
    let error = tool.push_point(p(9.0, 9.0)).unwrap_err();
    assert!(matches!(error, CadError::InvalidInput(_)));
    assert_eq!(tool.points().len(), 2);
    assert_eq!(
        tool.intent().unwrap(),
        DrawIntent::Rect {
            corner1: p(0.0, 0.0),
            corner2: p(4.0, 3.0),
        }
    );

    // A non-finite corner is refused at capture, never silently stored.
    let mut fresh = DrawTool::new(DrawToolKind::Rectang, 0);
    assert!(fresh.push_point(p(f64::NAN, 0.0)).is_err());
    assert!(fresh.points().is_empty());
}

#[test]
fn rectang_status_line_reports_the_corner_progress() {
    let mut tool = DrawTool::new(DrawToolKind::Rectang, 0);
    assert_eq!(tool.preview().status_line(), "矩形：已选 0 点，还需 2 点");
    tool.push_point(p(0.0, 0.0)).unwrap();
    assert_eq!(tool.preview().status_line(), "矩形：已选 1 点，还需 1 点");
    tool.push_point(p(2.0, 2.0)).unwrap();
    assert_eq!(tool.preview().status_line(), "矩形：已选 2 点，可确认");
}

#[test]
fn ellipse_captures_three_picks_exactly_and_ready_at_three() {
    // ELLIPSE is fixed arity 3 (axis-endpoint method): ready exactly at the third
    // pick, and the (3 + 1)-th click is refused. No selection and no entity pick.
    let mut tool = DrawTool::new(DrawToolKind::Ellipse, 0);
    assert_eq!(DrawToolKind::Ellipse.arity(), PointArity::Exact(3));
    assert_eq!(tool.remaining(), Some(3));
    assert!(!tool.is_ready());
    assert!(!DrawToolKind::Ellipse.requires_selection());
    assert!(!DrawToolKind::Ellipse.needs_picked_entities());
    assert!(!tool.requires_entity_pick());

    tool.push_point(p(0.0, 0.0)).unwrap();
    assert_eq!(tool.remaining(), Some(2));
    tool.push_point(p(4.0, 0.0)).unwrap();
    assert_eq!(tool.remaining(), Some(1));
    assert!(!tool.is_ready());
    // An incomplete capture has no intent.
    assert!(matches!(tool.intent(), Err(CadError::InvalidInput(_))));

    tool.push_point(p(2.0, 3.0)).unwrap();
    assert_eq!(tool.remaining(), Some(0));
    assert!(tool.is_ready());
    assert!(tool.preview().can_confirm());
    assert_eq!(tool.preview().remaining_points(), Some(0));
    assert!(!tool.preview().is_open_ended());
    // The intent carries the three captured picks **exactly**, in click order (the
    // geometry construction and axis swap are the command layer's job, not here).
    assert_eq!(
        tool.intent().unwrap(),
        DrawIntent::Ellipse {
            p1: p(0.0, 0.0),
            p2: p(4.0, 0.0),
            p3: p(2.0, 3.0),
        }
    );

    // The fourth pick is refused and does not disturb the capture.
    let error = tool.push_point(p(9.0, 9.0)).unwrap_err();
    assert!(matches!(error, CadError::InvalidInput(_)));
    assert_eq!(tool.points().len(), 3);

    // A non-finite pick is refused at capture, never silently stored.
    let mut fresh = DrawTool::new(DrawToolKind::Ellipse, 0);
    assert!(fresh.push_point(p(f64::NAN, 0.0)).is_err());
    assert!(fresh.points().is_empty());
}

#[test]
fn ellipse_status_line_reports_the_pick_progress() {
    let mut tool = DrawTool::new(DrawToolKind::Ellipse, 0);
    assert_eq!(tool.preview().status_line(), "椭圆：已选 0 点，还需 3 点");
    tool.push_point(p(0.0, 0.0)).unwrap();
    tool.push_point(p(4.0, 0.0)).unwrap();
    assert_eq!(tool.preview().status_line(), "椭圆：已选 2 点，还需 1 点");
    tool.push_point(p(2.0, 3.0)).unwrap();
    assert_eq!(tool.preview().status_line(), "椭圆：已选 3 点，可确认");
}

#[test]
fn polyline_status_line_reports_open_ended_progress_without_a_fixed_count() {
    let mut tool = DrawTool::new(DrawToolKind::Polyline, 0);
    assert_eq!(
        tool.preview().status_line(),
        "polyline: 0 point(s) captured, at least 2 needed"
    );
    tool.push_point(p(0.0, 0.0)).unwrap();
    assert_eq!(
        tool.preview().status_line(),
        "polyline: 1 point(s) captured, at least 2 needed"
    );
    tool.push_point(p(1.0, 0.0)).unwrap();
    assert_eq!(
        tool.preview().status_line(),
        "polyline: 2 point(s) captured, press Enter to finish"
    );
}

#[test]
fn polygon_intent_carries_the_two_picks_exactly_and_is_ready_at_two() {
    // POLYGON is fixed arity 2 (centre + radius point), like LINE/RECTANG: ready
    // exactly at the second pick, and the third click is refused. No selection
    // and no entity pick. The side count is the session sysvar, never a field of
    // the intent.
    let mut tool = DrawTool::new(DrawToolKind::Polygon, 0);
    assert_eq!(DrawToolKind::Polygon.arity(), PointArity::Exact(2));
    assert_eq!(tool.remaining(), Some(2));
    assert!(!DrawToolKind::Polygon.requires_selection());
    assert!(!DrawToolKind::Polygon.needs_picked_entities());
    assert!(!tool.requires_entity_pick());
    assert!(matches!(tool.intent(), Err(CadError::InvalidInput(_))));

    tool.push_point(p(0.0, 0.0)).unwrap();
    assert_eq!(tool.remaining(), Some(1));
    assert!(!tool.is_ready());
    tool.push_point(p(3.0, 4.0)).unwrap();
    assert!(tool.is_ready());
    assert!(tool.preview().can_confirm());
    assert_eq!(
        tool.intent().unwrap(),
        DrawIntent::Polygon {
            center: p(0.0, 0.0),
            radius_point: p(3.0, 4.0),
        }
    );

    // The third click is refused and does not disturb the capture.
    let error = tool.push_point(p(9.0, 9.0)).unwrap_err();
    assert!(matches!(error, CadError::InvalidInput(_)));
    assert_eq!(tool.points().len(), 2);

    // The pure tool never invents a sysvar: the preview carries no side count
    // until the session annotates it.
    assert_eq!(tool.preview().polygon_sides, None);
}

#[test]
fn polygon_vertices_are_inscribed_with_the_first_on_the_radius_ray() {
    // The AutoCAD default (inscribed) construction: the first vertex lies on the
    // centre→radius ray at the exact radius; the rest step by 2π/n about +Z; all
    // vertices share one circumradius on one +Z plane.
    let center = p(1.0, 2.0);
    let radius_point = p(4.0, 6.0); // dx = 3, dy = 4 ⇒ radius 5
    let radius = 5.0;
    for sides in [3u32, 4, 6, 12] {
        let vertices = polygon_vertices(center, radius_point, sides).unwrap();
        assert_eq!(vertices.len(), sides as usize, "{sides} vertices");
        // The first vertex is exactly the radius point (it lies on the ray).
        assert!(
            (vertices[0].x - radius_point.x).abs() < 1e-9
                && (vertices[0].y - radius_point.y).abs() < 1e-9,
            "{sides}: first vertex must be on the radius ray: {vertices:?}"
        );
        // Every vertex is at the same circumradius, on the centre's +Z plane.
        for vertex in &vertices {
            let r = (vertex.x - center.x).hypot(vertex.y - center.y);
            assert!((r - radius).abs() < 1e-9, "{sides}: {vertex:?} r={r}");
            assert_eq!(vertex.z, center.z, "{sides}: the ring is on the +Z plane");
        }
        // Consecutive vertices step by exactly 2π/n about the centre.
        let step = std::f64::consts::TAU / f64::from(sides);
        for pair in vertices.windows(2) {
            let a0 = (pair[0].y - center.y).atan2(pair[0].x - center.x);
            let a1 = (pair[1].y - center.y).atan2(pair[1].x - center.x);
            let mut delta = a1 - a0;
            while delta < 0.0 {
                delta += std::f64::consts::TAU;
            }
            assert!((delta - step).abs() < 1e-9, "{sides}: {delta} vs {step}");
        }
    }
    // Changing the side count changes the vertex count.
    let three = polygon_vertices(center, radius_point, 3).unwrap();
    let six = polygon_vertices(center, radius_point, 6).unwrap();
    assert_eq!(three.len(), 3);
    assert_eq!(six.len(), 6);
    assert_ne!(three.len(), six.len());
}

#[test]
fn polygon_vertices_refuse_degenerate_or_out_of_range_input() {
    let center = p(0.0, 0.0);
    let radius_point = p(2.0, 0.0);
    // Non-finite pick.
    assert!(matches!(
        polygon_vertices(p(f64::NAN, 0.0), radius_point, 4),
        Err(CadError::InvalidInput(_))
    ));
    // Zero radius (the radius point coincides with the centre).
    assert!(matches!(
        polygon_vertices(center, center, 4),
        Err(CadError::InvalidInput(_))
    ));
    // An out-of-range stored side count is refused (defensive), never clamped.
    for sides in [0u32, 1, 2, MAX_POLYGON_SIDES + 1] {
        assert!(
            matches!(
                polygon_vertices(center, radius_point, sides),
                Err(CadError::InvalidInput(_))
            ),
            "sides={sides} must be refused"
        );
    }
    // The legal bounds themselves are accepted.
    assert!(polygon_vertices(center, radius_point, MIN_POLYGON_SIDES).is_ok());
    assert!(polygon_vertices(center, radius_point, MAX_POLYGON_SIDES).is_ok());
    // A non-coplanar radius point is refused, never silently projected.
    assert!(matches!(
        polygon_vertices(
            center,
            Point3 {
                x: 2.0,
                y: 0.0,
                z: 5.0
            },
            4
        ),
        Err(CadError::InvalidInput(_))
    ));
}
