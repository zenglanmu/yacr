//! ARRAYRECT/ARRAYPOLAR pure draw-tool state-machine tests (ADR 0016 decision D).
//!
//! Split out of `draw_tool/tests.rs` to keep that file under the code-size limit.
//! ARRAYRECT captures no points (the zero-point exact complement: defined by the
//! typed session parameters plus the selection snapshot), while ARRAYPOLAR
//! captures exactly one point (the array centre).

use super::*;

#[test]
fn arrayrect_zero_point_arity_is_ready_on_selection_alone_and_refuses_points() {
    // ADR 0016 decision D: ARRAYRECT is defined entirely by the typed session
    // parameters plus the selection, so its arity is the zero-point exact
    // complement — the arity model supports it cleanly (it is ready as soon as the
    // selection is non-empty) and `intent()` never reads a point for it.
    assert_eq!(DrawToolKind::ArrayRect.arity(), PointArity::Exact(0));
    assert_eq!(DrawToolKind::ArrayRect.required_points(), 0);
    assert!(DrawToolKind::ArrayRect.requires_selection());
    assert!(!DrawToolKind::ArrayRect.needs_picked_entities());

    // Not ready without a selection; ready with one, and no point was captured.
    let tool = DrawTool::new(DrawToolKind::ArrayRect, 0);
    assert_eq!(tool.remaining(), Some(0));
    assert!(!tool.is_ready());
    assert!(matches!(tool.intent(), Err(CadError::InvalidInput(_))));
    let mut tool = DrawTool::new(DrawToolKind::ArrayRect, 1);
    assert!(tool.is_ready());
    assert_eq!(tool.intent().unwrap(), DrawIntent::ArrayRect);
    // No point is accepted: the zero-point complement is already full.
    assert!(matches!(
        tool.push_point(p(1.0, 2.0)),
        Err(CadError::InvalidInput(_))
    ));
    assert_eq!(tool.points().len(), 0);

    // ARRAYPOLAR captures exactly one point (the centre) and needs a selection.
    assert_eq!(DrawToolKind::ArrayPolar.arity(), PointArity::Exact(1));
    assert!(DrawToolKind::ArrayPolar.requires_selection());
    assert!(!DrawToolKind::ArrayPolar.needs_picked_entities());
    let mut polar = DrawTool::new(DrawToolKind::ArrayPolar, 1);
    assert!(!polar.is_ready());
    polar.push_point(p(3.0, 4.0)).unwrap();
    assert!(polar.is_ready());
    assert_eq!(
        polar.intent().unwrap(),
        DrawIntent::ArrayPolar {
            center: p(3.0, 4.0)
        }
    );
    assert!(polar.push_point(p(5.0, 6.0)).is_err());
}
