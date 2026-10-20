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

#[test]
fn dimangular_picks_two_lines_then_takes_the_arc_location() {
    let kind = DrawToolKind::DimAngular;
    assert_eq!(kind.arity(), PointArity::Exact(3));
    assert!(kind.needs_picked_entities());
    assert_eq!(kind.picked_step_count(), 2);
    assert!(!kind.requires_selection());
    let mut tool = DrawTool::new(kind, 0);
    // Both leading steps are entity picks: free points are refused.
    assert!(tool.push_point(p(0.0, 0.0)).is_err());
    tool.push_pick(p(5.0, 0.0), pick(21)).unwrap();
    assert!(tool.push_point(p(0.0, 0.0)).is_err());
    tool.push_pick(p(0.0, 5.0), pick(22)).unwrap();
    assert!(!tool.requires_entity_pick());
    assert!(!tool.is_ready(), "the arc location is still needed");
    assert_eq!(tool.remaining(), Some(1));
    // A third pick is refused: the pending step is the free location point.
    assert!(tool.push_pick(p(0.0, 0.0), pick(23)).is_err());
    tool.push_point(p(4.0, 4.0)).unwrap();
    assert!(tool.is_ready());
    assert_eq!(tool.remaining(), Some(0));
    assert_eq!(
        tool.intent().unwrap(),
        DrawIntent::DimAngular {
            first: pick(21),
            second: pick(22),
            location: p(4.0, 4.0),
        }
    );
}

#[test]
fn dimordinate_captures_the_feature_point_and_leader_endpoint() {
    let kind = DrawToolKind::DimOrdinate;
    assert_eq!(kind.arity(), PointArity::Exact(2));
    assert!(!kind.needs_picked_entities());
    assert_eq!(kind.picked_step_count(), 0);
    assert!(!kind.requires_selection());
    let mut tool = DrawTool::new(kind, 0);
    assert_eq!(tool.remaining(), Some(2));
    tool.push_point(p(3.0, 4.0)).unwrap();
    assert!(!tool.is_ready());
    assert_eq!(tool.remaining(), Some(1));
    tool.push_point(p(3.0, 9.0)).unwrap();
    assert!(tool.is_ready());
    assert_eq!(
        tool.intent().unwrap(),
        DrawIntent::DimOrdinate {
            feature: p(3.0, 4.0),
            leader: p(3.0, 9.0),
        }
    );
}

#[test]
fn baseline_and_continue_pick_the_base_then_take_open_ended_points() {
    for kind in [DrawToolKind::DimBaseline, DrawToolKind::DimContinue] {
        // The base pick is a captured step, so the arity minimum is the pick
        // **plus one measured point**: a base pick alone must never be
        // confirmable (ADR 0017 decision C, P5f).
        assert_eq!(kind.arity(), PointArity::AtLeast(2), "{kind:?}");
        assert!(kind.needs_picked_entities(), "{kind:?}");
        assert_eq!(kind.picked_step_count(), 1, "{kind:?}");
        assert!(!kind.requires_selection(), "{kind:?}");
        assert!(!kind.requires_text(), "{kind:?}");
        assert_eq!(kind.required_points(), 2, "{kind:?}");

        let mut tool = DrawTool::new(kind, 0);
        assert!(tool.requires_entity_pick(), "{kind:?}");
        // The base step is an entity pick: a blind free point is refused.
        assert!(tool.push_point(p(5.0, -10.0)).is_err(), "{kind:?}");
        assert!(tool.points().is_empty(), "{kind:?}");
        // An incomplete capture refuses to produce an intent.
        assert!(tool.intent().is_err(), "{kind:?}");

        tool.push_pick(p(5.0, -10.0), pick(31)).unwrap();
        assert!(!tool.requires_entity_pick(), "{kind:?}");
        assert!(
            !tool.is_ready(),
            "{kind:?} needs at least one measured point"
        );
        assert_eq!(tool.remaining(), Some(1), "{kind:?}");
        assert!(
            tool.preview().status_line().contains("after the base"),
            "{}",
            tool.preview().status_line()
        );
        // A second entity pick is refused: the pending step is a free point.
        assert!(tool.push_pick(p(0.0, 0.0), pick(32)).is_err(), "{kind:?}");

        tool.push_point(p(10.0, 0.0)).unwrap();
        assert!(tool.is_ready(), "{kind:?}");
        assert!(tool.preview().is_open_ended(), "{kind:?}");
        assert_eq!(tool.preview().remaining_points(), None, "{kind:?}");
        // More points are accepted; the capture is finished with Enter.
        tool.push_point(p(20.0, 0.0)).unwrap();
        assert!(tool.is_ready(), "{kind:?}");

        let points = vec![p(10.0, 0.0), p(20.0, 0.0)];
        let expected = if kind == DrawToolKind::DimBaseline {
            DrawIntent::DimBaseline {
                base: pick(31),
                points,
            }
        } else {
            DrawIntent::DimContinue {
                base: pick(31),
                points,
            }
        };
        assert_eq!(tool.intent().unwrap(), expected, "{kind:?}");
        // A non-finite measured point is refused without disturbing the capture.
        assert!(tool.push_point(p(f64::NAN, 0.0)).is_err(), "{kind:?}");
        assert_eq!(tool.points().len(), 3, "{kind:?}");
    }

    // Without the base pick there is no intent at all: the base is required, and
    // it is never carried over from a previous command.
    for kind in [DrawToolKind::DimBaseline, DrawToolKind::DimContinue] {
        let mut bare = DrawTool::new(kind, 0);
        assert!(bare.push_point(p(10.0, 0.0)).is_err(), "{kind:?}");
        assert!(bare.intent().is_err(), "{kind:?}");
    }
}
