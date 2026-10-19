//! HATCH pure draw-tool state-machine tests (ADR 0018 decision D, P6b).
//!
//! Split out of `draw_tool/tests.rs` to keep that file under the code-size limit.
//! HATCH uses the **pre-selection** shape (the same zero-point exact complement
//! ARRAYRECT uses): the selected entities are the boundary, the typed session
//! parameters define the fill, and the tool never captures a point or an entity
//! pick.

use super::*;

#[test]
fn hatch_zero_point_arity_is_ready_on_selection_alone_and_refuses_points() {
    assert_eq!(DrawToolKind::Hatch.arity(), PointArity::Exact(0));
    assert_eq!(DrawToolKind::Hatch.required_points(), 0);
    assert!(DrawToolKind::Hatch.requires_selection());
    // No in-command entity picks: the boundary is the pre-selection.
    assert_eq!(DrawToolKind::Hatch.picked_step_count(), 0);
    assert!(!DrawToolKind::Hatch.needs_picked_entities());
    assert_eq!(DrawToolKind::Hatch.key(), "hatch");
    assert_eq!(
        DrawToolKind::from_key("hatch"),
        Some(DrawToolKind::Hatch),
        "the machine key round-trips"
    );
    assert_eq!(DrawToolKind::Hatch.label(), "HATCH");

    // Not ready without a selection; ready with one, and no point was captured.
    let tool = DrawTool::new(DrawToolKind::Hatch, 0);
    assert_eq!(tool.remaining(), Some(0));
    assert!(!tool.is_ready());
    assert!(matches!(tool.intent(), Err(CadError::InvalidInput(_))));
    let mut tool = DrawTool::new(DrawToolKind::Hatch, 2);
    assert!(tool.is_ready());
    assert_eq!(tool.intent().unwrap(), DrawIntent::Hatch);
    assert_eq!(tool.preview().remaining_points(), Some(0));
    assert_eq!(tool.preview().selection_count, 2);
    // No point is accepted: the zero-point complement is already full.
    assert!(matches!(
        tool.push_point(p(1.0, 2.0)),
        Err(CadError::InvalidInput(_))
    ));
    // No entity pick either: every boundary entity comes from the selection.
    assert!(matches!(
        tool.push_pick(p(1.0, 2.0), pick(7)),
        Err(CadError::InvalidInput(_))
    ));
    assert_eq!(tool.points().len(), 0);
    assert_eq!(tool.picks().len(), 0);
}

#[test]
fn hatch_does_not_accept_a_point_when_the_selection_is_empty() {
    // The refusal must be the selection reason, never a fabricated point capture.
    let mut tool = DrawTool::new(DrawToolKind::Hatch, 0);
    let message = match tool.push_point(p(0.0, 0.0)) {
        Err(CadError::InvalidInput(message)) => message,
        other => panic!("expected an InvalidInput refusal, got {other:?}"),
    };
    assert!(message.contains("hatch"), "{message}");
    assert_eq!(tool.points().len(), 0);
}
