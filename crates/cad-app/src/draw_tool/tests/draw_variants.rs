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
