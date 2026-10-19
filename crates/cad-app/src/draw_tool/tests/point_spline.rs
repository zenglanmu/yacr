//! POINT/SPLINE pure draw-tool state-machine tests (ADR 0015 decisions A/F).
//!
//! Split out of `draw_tool/tests.rs` to keep that file under the code-size limit.
//! These pin the pure capture machine: POINT is exact arity 1 (single placement),
//! SPLINE is open-ended (at least four fit points), and neither needs a selection
//! or an entity pick.

use super::*;

#[test]
fn point_captures_one_position_and_is_ready_at_one() {
    // POINT is exact arity 1 (single placement): ready exactly at the one click,
    // and the (1 + 1)-th click is refused. No selection and no entity pick.
    let mut tool = DrawTool::new(DrawToolKind::Point, 0);
    assert_eq!(DrawToolKind::Point.arity(), PointArity::Exact(1));
    assert_eq!(tool.remaining(), Some(1));
    assert!(!tool.is_ready());
    assert!(!DrawToolKind::Point.requires_selection());
    assert!(!DrawToolKind::Point.needs_picked_entities());
    assert!(!tool.requires_entity_pick());
    assert!(matches!(tool.intent(), Err(CadError::InvalidInput(_))));

    tool.push_point(p(3.0, 4.0)).unwrap();
    assert_eq!(tool.remaining(), Some(0));
    assert!(tool.is_ready());
    assert_eq!(
        tool.intent().unwrap(),
        DrawIntent::Point {
            position: p(3.0, 4.0)
        }
    );

    // The second point is refused and does not disturb the capture.
    assert!(matches!(
        tool.push_point(p(9.0, 9.0)),
        Err(CadError::InvalidInput(_))
    ));
    assert_eq!(tool.points().len(), 1);

    // A non-finite point is refused at capture, never silently stored.
    let mut fresh = DrawTool::new(DrawToolKind::Point, 0);
    assert!(fresh.push_point(p(f64::NAN, 0.0)).is_err());
    assert!(fresh.points().is_empty());
}

#[test]
fn spline_captures_fit_points_and_is_ready_at_four() {
    // SPLINE is open-ended (at least four fit points): not ready below four, ready
    // at four with more still accepted, and the intent carries every fit point.
    let mut tool = DrawTool::new(DrawToolKind::Spline, 0);
    assert_eq!(DrawToolKind::Spline.arity(), PointArity::AtLeast(4));
    assert_eq!(tool.remaining(), Some(4));
    assert!(!tool.is_ready());
    assert!(!DrawToolKind::Spline.requires_selection());
    assert!(!DrawToolKind::Spline.needs_picked_entities());
    assert!(!tool.requires_entity_pick());

    for i in 0..4usize {
        assert_eq!(tool.remaining(), Some(4 - i));
        tool.push_point(p(i as f64, (i % 2) as f64)).unwrap();
    }
    // At the minimum: ready, and there is **no** fixed remaining count (never a
    // fabricated 0) because more fit points are still accepted.
    assert_eq!(tool.remaining(), None);
    assert!(tool.is_ready());
    assert!(tool.preview().is_open_ended());
    assert_eq!(tool.preview().remaining_points(), None);

    tool.push_point(p(4.0, 1.0)).unwrap();
    assert!(tool.is_ready());
    match tool.intent().unwrap() {
        DrawIntent::Spline { fit_points } => {
            assert_eq!(fit_points.len(), 5, "every fit point travels in order");
            assert_eq!(fit_points[0], p(0.0, 0.0));
            assert_eq!(fit_points[4], p(4.0, 1.0));
        }
        other => panic!("expected Spline, got {other:?}"),
    }
}
