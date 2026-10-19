//! Dimension-family pure draw-tool state-machine tests (ADR 0017 decision C).
//!
//! Split out of `draw_tool/tests.rs` to keep that file under the code-size limit.
//! The linear/aligned pair captures three free points; the radial pair and
//! DIMCENTER start with an **entity pick** (`picked_step_count == 1`) — the radial
//! pair then takes a free location point, DIMCENTER is done after its pick.

use super::*;

#[test]
fn dimension_tools_capture_three_points_and_produce_their_intent() {
    for kind in [DrawToolKind::DimLinear, DrawToolKind::DimAligned] {
        let mut tool = DrawTool::new(kind, 0);
        assert_eq!(tool.remaining(), Some(3), "{kind:?}");
        assert!(!kind.requires_selection(), "{kind:?}");
        assert!(!kind.needs_picked_entities(), "{kind:?}");
        tool.push_point(p(0.0, 0.0)).unwrap();
        tool.push_point(p(10.0, 0.0)).unwrap();
        assert!(!tool.is_ready(), "{kind:?} still needs the location pick");
        tool.push_point(p(5.0, 4.0)).unwrap();
        assert!(tool.is_ready(), "{kind:?}");
        assert_eq!(tool.remaining(), Some(0));
        let expected = match kind {
            DrawToolKind::DimLinear => DrawIntent::DimLinear {
                first: p(0.0, 0.0),
                second: p(10.0, 0.0),
                location: p(5.0, 4.0),
            },
            _ => DrawIntent::DimAligned {
                first: p(0.0, 0.0),
                second: p(10.0, 0.0),
                location: p(5.0, 4.0),
            },
        };
        assert_eq!(tool.intent().unwrap(), expected);
    }
}

#[test]
fn radial_tools_start_with_an_entity_pick_then_take_the_location() {
    for kind in [DrawToolKind::DimRadius, DrawToolKind::DimDiameter] {
        let mut tool = DrawTool::new(kind, 0);
        assert_eq!(kind.arity(), PointArity::Exact(2), "{kind:?}");
        assert!(kind.needs_picked_entities(), "{kind:?}");
        assert_eq!(kind.picked_step_count(), 1, "{kind:?}");
        assert!(!kind.requires_selection(), "{kind:?}");
        // The first step is an entity pick: a free point is refused, and only a
        // resolved pick advances.
        assert!(tool.requires_entity_pick(), "{kind:?}");
        assert!(tool.push_point(p(1.0, 0.0)).is_err(), "{kind:?}");
        tool.push_pick(p(5.0, 0.0), pick(7)).unwrap();
        assert!(!tool.requires_entity_pick(), "{kind:?} pick captured");
        assert!(!tool.is_ready(), "{kind:?} still needs the location");
        assert_eq!(tool.remaining(), Some(1), "{kind:?}");
        // A second entity pick is refused: the pending step is a free point.
        assert!(tool.push_pick(p(0.0, 0.0), pick(8)).is_err(), "{kind:?}");
        tool.push_point(p(0.0, 6.0)).unwrap();
        assert!(tool.is_ready(), "{kind:?}");
        assert_eq!(tool.remaining(), Some(0), "{kind:?}");
        let expected = match kind {
            DrawToolKind::DimRadius => DrawIntent::DimRadius {
                target: pick(7),
                location: p(0.0, 6.0),
            },
            _ => DrawIntent::DimDiameter {
                target: pick(7),
                location: p(0.0, 6.0),
            },
        };
        assert_eq!(tool.intent().unwrap(), expected);
    }

    // DIMCENTER captures exactly one entity pick and nothing else.
    let kind = DrawToolKind::DimCenter;
    assert_eq!(kind.arity(), PointArity::Exact(1));
    assert!(kind.needs_picked_entities());
    assert_eq!(kind.picked_step_count(), 1);
    assert!(!kind.requires_selection());
    let mut center = DrawTool::new(kind, 0);
    assert!(center.push_point(p(0.0, 0.0)).is_err());
    center.push_pick(p(5.0, 0.0), pick(9)).unwrap();
    assert!(center.is_ready());
    // A second pick or point is refused once the complement is complete.
    assert!(center.push_pick(p(0.0, 0.0), pick(8)).is_err());
    assert_eq!(
        center.intent().unwrap(),
        DrawIntent::DimCenter { target: pick(9) }
    );
}
