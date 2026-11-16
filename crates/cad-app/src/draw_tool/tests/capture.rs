//! The capture state machine: arity refusal, readiness, status lines and the
//! open-ended (point-capped) capture path.
//!
//! Split out of `draw_tool/tests.rs` to keep that file under the code-size limit.

use super::*;

#[test]
fn a_third_point_and_a_non_finite_point_are_refused_without_capture() {
    let mut tool = DrawTool::new(DrawToolKind::Line, 0);
    tool.push_point(p(0.0, 0.0)).unwrap();
    tool.push_point(p(1.0, 0.0)).unwrap();
    assert!(matches!(
        tool.push_point(p(2.0, 0.0)),
        Err(CadError::InvalidInput(_))
    ));
    assert_eq!(tool.points().len(), 2);

    let mut fresh = DrawTool::new(DrawToolKind::Line, 0);
    assert!(fresh.push_point(p(f64::NAN, 0.0)).is_err());
    assert!(fresh.points().is_empty());
}

#[test]
fn intent_refuses_incomplete_capture() {
    let tool = DrawTool::new(DrawToolKind::Circle, 0);
    assert!(matches!(tool.intent(), Err(CadError::InvalidInput(_))));
}

#[test]
fn preview_status_and_cursor_follow_the_state_machine() {
    let mut tool = DrawTool::new(DrawToolKind::Line, 0);
    assert_eq!(tool.preview().status_line(), "直线：已选 0 点，还需 2 点");
    tool.set_cursor(Some(p(5.0, 5.0)));
    assert_eq!(tool.cursor(), Some(p(5.0, 5.0)));
    // The cursor never counts as a captured point.
    assert!(tool.points().is_empty());
    tool.push_point(p(0.0, 0.0)).unwrap();
    assert_eq!(tool.preview().status_line(), "直线：已选 1 点，还需 1 点");
    tool.push_point(p(1.0, 1.0)).unwrap();
    assert_eq!(tool.preview().status_line(), "直线：已选 2 点，可确认");
}

#[test]
fn fixed_arity_still_refuses_the_n_plus_one_point_and_ready_exactly_at_n() {
    // Byte-identical contract for every fixed kind: ready exactly at n, and the
    // (n + 1)-th point is refused without capturing. The entity-picking steps
    // (TRIM/EXTEND/CHAMFER/FILLET, and the leading step of the radial family)
    // capture through `push_pick` (the point and the resolved ref together); any
    // later step of a mixed capture is a free point.
    for kind in DrawToolKind::ALL {
        if kind.arity().is_open_ended() {
            continue;
        }
        let n = kind.required_points();
        // A selection-requiring kind needs a selection only for readiness, not
        // for the point-complement refusal this test pins.
        let mut tool = DrawTool::new(kind, 1);
        let capture = |tool: &mut DrawTool, i: usize, point: Point3| {
            if i < kind.picked_step_count() {
                tool.push_pick(point, pick(i as u128 + 100))
            } else {
                tool.push_point(point)
            }
        };
        for i in 0..n {
            assert_eq!(tool.remaining(), Some(n - i), "{kind:?}");
            capture(&mut tool, i, p(i as f64, 0.0)).unwrap();
            assert_eq!(tool.remaining(), Some(n - 1 - i), "{kind:?}");
        }
        assert_eq!(tool.remaining(), Some(0), "{kind:?}");
        assert!(
            tool.is_ready(),
            "{kind:?} must be ready at exactly n points"
        );
        let error = capture(&mut tool, 99, p(99.0, 99.0)).unwrap_err();
        assert!(matches!(error, CadError::InvalidInput(_)), "{kind:?}");
        assert_eq!(
            tool.points().len(),
            n,
            "{kind:?} refused point not captured"
        );
        // Every fixed-arity preview still reports a numeric 0 remainder.
        assert_eq!(tool.preview().remaining, 0);
        assert!(!tool.preview().is_open_ended());
        assert_eq!(tool.preview().remaining_points(), Some(0));
    }
}

#[test]
fn open_ended_polyline_accepts_more_points_and_is_ready_at_its_minimum() {
    let mut tool = DrawTool::new(DrawToolKind::Polyline, 0);
    assert_eq!(DrawToolKind::Polyline.arity(), PointArity::AtLeast(2));
    assert!(tool.remaining().is_some());
    assert_eq!(tool.remaining(), Some(2));
    assert!(!tool.is_ready());
    // Below the minimum the capture is not confirmable and reports a count.
    assert!(matches!(tool.intent(), Err(CadError::InvalidInput(_))));

    tool.push_point(p(0.0, 0.0)).unwrap();
    assert_eq!(tool.remaining(), Some(1));
    assert!(!tool.is_ready());

    tool.push_point(p(1.0, 0.0)).unwrap();
    // At the minimum: ready, and there is **no** fixed remaining count (never a
    // fabricated 0) because more points are still accepted.
    assert_eq!(tool.remaining(), None);
    assert!(tool.is_ready());
    assert!(tool.preview().can_confirm());
    assert!(tool.preview().is_open_ended());
    assert_eq!(tool.preview().remaining_points(), None);
    assert_eq!(
        tool.preview().status_line(),
        "polyline: 2 point(s) captured, press Enter to finish"
    );

    // More points are accepted indefinitely (up to the cap) and the tool stays
    // ready; the intent carries every captured vertex in order.
    tool.push_point(p(2.0, 0.0)).unwrap();
    tool.push_point(p(2.0, 1.0)).unwrap();
    assert_eq!(tool.remaining(), None);
    assert!(tool.is_ready());
    assert_eq!(
        tool.intent().unwrap(),
        DrawIntent::Polyline {
            points: vec![p(0.0, 0.0), p(1.0, 0.0), p(2.0, 0.0), p(2.0, 1.0)]
        }
    );

    // A non-finite point is refused mid-capture without disturbing the capture.
    assert!(tool.push_point(p(f64::NAN, 0.0)).is_err());
    assert_eq!(tool.points().len(), 4);
}

#[test]
fn open_ended_capture_refuses_past_the_hard_cap_explicitly() {
    let mut tool = DrawTool::new(DrawToolKind::Polyline, 0);
    for i in 0..MAX_OPEN_CAPTURE_POINTS {
        tool.push_point(p(i as f64, 0.0)).unwrap();
    }
    assert_eq!(tool.points().len(), MAX_OPEN_CAPTURE_POINTS);
    let message = match tool.push_point(p(1.0e9, 1.0e9)) {
        Err(CadError::InvalidInput(message)) => message,
        other => panic!("expected a cap refusal, got {other:?}"),
    };
    assert!(message.contains("maximum"), "{message}");
    assert!(
        message.contains(&MAX_OPEN_CAPTURE_POINTS.to_string()),
        "{message}"
    );
    assert_eq!(tool.points().len(), MAX_OPEN_CAPTURE_POINTS);
    assert!(tool.is_ready());
}
