//! The entity-pick capture path (TRIM/EXTEND and the cursor candidate), which is
//! disjoint from the free-point path.
//!
//! Split out of `draw_tool/tests.rs` to keep that file under the code-size limit.

use super::*;

#[test]
fn extend_walks_target_then_boundary_with_entity_picks() {
    let mut tool = DrawTool::new(DrawToolKind::Extend, 0);
    assert!(tool.requires_entity_pick());
    assert_eq!(tool.preview().status_line(), "延伸：请选择要延伸的目标");
    // A blind free point is refused: an EXTEND step is an entity pick.
    assert!(matches!(
        tool.push_point(p(9.0, 0.0)),
        Err(CadError::InvalidInput(_))
    ));
    assert!(tool.points().is_empty());
    assert!(tool.picks().is_empty());

    tool.push_pick(p(9.0, 0.0), pick(101)).unwrap();
    assert_eq!(tool.preview().status_line(), "延伸：请选择边界");
    assert!(!tool.is_ready());
    assert_eq!(tool.picks(), &[pick(101)]);
    tool.push_pick(p(20.0, 0.0), pick(102)).unwrap();
    assert!(tool.is_ready());
    assert!(tool.preview().can_confirm());
    assert_eq!(
        tool.intent().unwrap(),
        DrawIntent::Extend {
            target: pick(101),
            boundary: pick(102),
            pick_point: p(9.0, 0.0),
        }
    );
}

#[test]
fn trim_walks_target_then_boundary_with_entity_picks() {
    let mut tool = DrawTool::new(DrawToolKind::Trim, 0);
    assert!(tool.requires_entity_pick());
    assert_eq!(tool.preview().status_line(), "修剪：请选择要修剪的目标");
    // A blind free point is refused: a TRIM step is an entity pick.
    assert!(matches!(
        tool.push_point(p(1.0, 0.0)),
        Err(CadError::InvalidInput(_))
    ));
    tool.push_pick(p(1.0, 0.0), pick(1)).unwrap();
    assert_eq!(tool.preview().status_line(), "修剪：请选择边界");
    tool.push_pick(p(1.0, 5.0), pick(2)).unwrap();
    assert!(tool.is_ready());
    assert_eq!(
        tool.intent().unwrap(),
        DrawIntent::Trim {
            target: pick(1),
            boundary: pick(2),
            pick_point: p(1.0, 0.0),
        }
    );
}

#[test]
fn trim_extend_are_not_ready_from_points_alone_and_refuse_free_pick_on_other_kinds() {
    // Readiness is "both steps resolved to real entities", not "two points".
    let mut trim = DrawTool::new(DrawToolKind::Trim, 0);
    trim.push_pick(p(0.0, 0.0), pick(1)).unwrap();
    assert!(!trim.is_ready());
    assert!(matches!(trim.intent(), Err(CadError::InvalidInput(_))));
    // A free-point kind refuses an entity pick: the two capture paths are
    // disjoint, so neither can silently stand in for the other.
    let mut line = DrawTool::new(DrawToolKind::Line, 0);
    assert!(matches!(
        line.push_pick(p(0.0, 0.0), pick(1)),
        Err(CadError::InvalidInput(_))
    ));
    assert!(line.points().is_empty());
}

#[test]
fn pick_kinds_expose_the_cursor_candidate_and_clear_it_on_capture() {
    let mut tool = DrawTool::new(DrawToolKind::Trim, 0);
    assert_eq!(tool.candidate(), None);
    tool.set_candidate(Some(pick(7)));
    assert_eq!(tool.preview().candidate, Some(pick(7)));
    // Capturing the step clears the candidate: the next step starts absent
    // rather than carrying the previous step's highlight.
    tool.push_pick(p(0.0, 0.0), pick(1)).unwrap();
    assert_eq!(tool.candidate(), None);
    assert_eq!(tool.preview().candidate, None);
}

#[test]
fn preview_counts_resolved_picks_for_the_pick_kinds() {
    let mut tool = DrawTool::new(DrawToolKind::Extend, 0);
    tool.push_pick(p(1.0, 0.0), pick(1)).unwrap();
    let preview = tool.preview();
    assert_eq!(preview.picks, vec![pick(1)]);
    assert_eq!(preview.remaining, 1);
    assert!(!preview.can_confirm());
}

#[test]
fn a_click_after_both_pick_steps_is_refused_as_ready_not_as_a_free_point() {
    // Once both TRIM steps are resolved the tool is ready and waiting for the
    // Enter confirmation. A further click cannot advance the capture, so it is
    // refused with that reason — not the misleading "captures entity picks, not
    // free points", which describes a step that is still un-captured.
    let mut tool = DrawTool::new(DrawToolKind::Trim, 0);
    tool.push_pick(p(1.0, 0.0), pick(1)).unwrap();
    tool.push_pick(p(1.0, 5.0), pick(2)).unwrap();
    assert!(tool.is_ready());

    let message = match tool.push_point(p(3.0, 3.0)) {
        Err(CadError::InvalidInput(message)) => message,
        other => panic!("expected a ready refusal, got {other:?}"),
    };
    assert!(message.contains("ready"), "{message}");
    assert!(message.contains("Enter"), "{message}");
    assert!(message.contains(DrawToolKind::Trim.key()), "{message}");

    // The refusal does not disturb the completed capture.
    assert_eq!(tool.picks().len(), 2);
    assert_eq!(tool.points().len(), 2);
    assert!(tool.is_ready());
}
