use super::*;
use cad_domain::{DocumentId, EntityId, InstancePath, SelectionRef};

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

/// A synthetic ref for the draw-tool-level tests (identity only; the tool never
/// reads the database).
fn pick(id: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(id),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

#[test]
fn keys_indices_and_labels_round_trip_and_reject_unknowns() {
    for (index, kind) in DrawToolKind::ALL.iter().copied().enumerate() {
        assert_eq!(kind.index(), index);
        assert_eq!(DrawToolKind::from_index(index as i32), Some(kind));
        assert_eq!(DrawToolKind::from_key(kind.key()), Some(kind));
        assert_eq!(DrawToolKind::from_label(kind.label()), Some(kind));
    }
    assert_eq!(DrawToolKind::from_key("arc"), Some(DrawToolKind::Arc));
    assert_eq!(DrawToolKind::from_key("elbow"), None);
    assert_eq!(DrawToolKind::from_index(-1), None);
    assert_eq!(
        DrawToolKind::from_index(DrawToolKind::ALL.len() as i32),
        None
    );
    assert_eq!(DrawToolKind::from_label("半径"), None);
}

#[test]
fn required_points_and_selection_are_explicit_per_kind() {
    for kind in DrawToolKind::ALL {
        let expected = match kind {
            DrawToolKind::Arc | DrawToolKind::Scale => 3,
            DrawToolKind::Offset => 1,
            _ => 2,
        };
        assert_eq!(kind.required_points(), expected, "{kind:?}");
    }
    assert!(!DrawToolKind::Line.requires_selection());
    assert!(!DrawToolKind::Circle.requires_selection());
    assert!(!DrawToolKind::Arc.requires_selection());
    assert!(DrawToolKind::Move.requires_selection());
    assert!(!DrawToolKind::Trim.requires_selection());
    assert!(DrawToolKind::Copy.requires_selection());
    assert!(!DrawToolKind::Extend.requires_selection());
    assert!(DrawToolKind::Rotate.requires_selection());
    assert!(DrawToolKind::Scale.requires_selection());
    assert!(DrawToolKind::Mirror.requires_selection());
    assert!(DrawToolKind::Offset.requires_selection());
}

#[test]
fn offset_captures_one_side_point() {
    let mut tool = DrawTool::new(DrawToolKind::Offset, 1);
    assert_eq!(tool.remaining(), 1);
    assert!(!tool.is_ready());
    tool.push_point(p(3.0, 4.0)).unwrap();
    assert!(tool.is_ready());
    assert_eq!(
        tool.intent().unwrap(),
        DrawIntent::Offset {
            side_point: p(3.0, 4.0)
        }
    );
    // A selection-requiring tool with no selection is not confirmable.
    let none = DrawTool::new(DrawToolKind::Offset, 0);
    assert!(matches!(none.intent(), Err(CadError::InvalidInput(_))));
}

#[test]
fn rotate_scale_mirror_capture_their_parameters() {
    // ROTATE: angle is the absolute direction from base to the second point.
    let mut rotate = DrawTool::new(DrawToolKind::Rotate, 2);
    rotate.push_point(p(0.0, 0.0)).unwrap();
    rotate.push_point(p(0.0, 1.0)).unwrap();
    match rotate.intent().unwrap() {
        DrawIntent::Rotate { base, angle } => {
            assert_eq!(base, p(0.0, 0.0));
            assert!(
                (angle - std::f64::consts::FRAC_PI_2).abs() < 1e-12,
                "{angle}"
            );
        }
        other => panic!("expected Rotate, got {other:?}"),
    }
    // A zero-length direction (or one differing only in Z) has no defined
    // angle: refuse it rather than silently committing a 0° no-op.
    let mut degenerate_rotate = DrawTool::new(DrawToolKind::Rotate, 2);
    degenerate_rotate.push_point(p(2.0, 2.0)).unwrap();
    degenerate_rotate.push_point(p(2.0, 2.0)).unwrap();
    assert!(matches!(
        degenerate_rotate.intent(),
        Err(CadError::InvalidInput(_))
    ));
    let mut z_only_rotate = DrawTool::new(DrawToolKind::Rotate, 2);
    z_only_rotate.push_point(p(2.0, 2.0)).unwrap();
    z_only_rotate
        .push_point(Point3 {
            x: 2.0,
            y: 2.0,
            z: 5.0,
        })
        .unwrap();
    assert!(matches!(
        z_only_rotate.intent(),
        Err(CadError::InvalidInput(_))
    ));

    // SCALE: factor = |target − base| / |reference − base|.
    let mut scale = DrawTool::new(DrawToolKind::Scale, 1);
    scale.push_point(p(0.0, 0.0)).unwrap();
    scale.push_point(p(1.0, 0.0)).unwrap();
    scale.push_point(p(3.0, 0.0)).unwrap();
    match scale.intent().unwrap() {
        DrawIntent::Scale { base, factor } => {
            assert_eq!(base, p(0.0, 0.0));
            assert!((factor - 3.0).abs() < 1e-12, "{factor}");
        }
        other => panic!("expected Scale, got {other:?}"),
    }
    // A reference coincident with the base is refused, not an infinite factor.
    let mut degenerate = DrawTool::new(DrawToolKind::Scale, 1);
    degenerate.push_point(p(2.0, 2.0)).unwrap();
    degenerate.push_point(p(2.0, 2.0)).unwrap();
    degenerate.push_point(p(5.0, 2.0)).unwrap();
    assert!(matches!(
        degenerate.intent(),
        Err(CadError::InvalidInput(_))
    ));

    // MIRROR: the axis is the two captured points; equal points are refused.
    let mut mirror = DrawTool::new(DrawToolKind::Mirror, 1);
    mirror.push_point(p(0.0, 0.0)).unwrap();
    mirror.push_point(p(1.0, 1.0)).unwrap();
    assert_eq!(
        mirror.intent().unwrap(),
        DrawIntent::Mirror {
            p1: p(0.0, 0.0),
            p2: p(1.0, 1.0)
        }
    );
    let mut flat = DrawTool::new(DrawToolKind::Mirror, 1);
    flat.push_point(p(3.0, 3.0)).unwrap();
    flat.push_point(p(3.0, 3.0)).unwrap();
    assert!(matches!(flat.intent(), Err(CadError::InvalidInput(_))));
}

#[test]
fn rotate_scale_mirror_transforms_are_correct() {
    // Rotation by 90° about the origin sends (1,0) to (0,1).
    let r = rotation_transform(p(0.0, 0.0), std::f64::consts::FRAC_PI_2);
    let rotated = r.apply_point(p(1.0, 0.0));
    assert!(
        rotated.x.abs() < 1e-12 && (rotated.y - 1.0).abs() < 1e-12,
        "{rotated:?}"
    );

    // Scale by 2 about (1,1): (2,1) -> (3,1).
    let s = scale_transform(p(1.0, 1.0), 2.0);
    let scaled = s.apply_point(p(2.0, 1.0));
    assert!(
        (scaled.x - 3.0).abs() < 1e-12 && (scaled.y - 1.0).abs() < 1e-12,
        "{scaled:?}"
    );

    // Mirror across the diagonal y = x swaps x and y.
    let m = mirror_transform(p(0.0, 0.0), p(1.0, 1.0)).unwrap();
    let mirrored = m.apply_point(p(2.0, 0.0));
    assert!(
        (mirrored.x - 0.0).abs() < 1e-12 && (mirrored.y - 2.0).abs() < 1e-12,
        "{mirrored:?}"
    );
    // A degenerate axis is refused.
    assert!(matches!(
        mirror_transform(p(1.0, 1.0), p(1.0, 1.0)),
        Err(CadError::InvalidInput(_))
    ));
}

#[test]
fn arc_captures_three_points_and_confirms_an_arc_intent() {
    let mut tool = DrawTool::new(DrawToolKind::Arc, 0);
    assert_eq!(tool.remaining(), 3);
    tool.push_point(p(0.0, 0.0)).unwrap();
    tool.push_point(p(5.0, 5.0)).unwrap();
    assert_eq!(tool.remaining(), 1);
    assert!(!tool.is_ready());
    tool.push_point(p(10.0, 0.0)).unwrap();
    assert!(tool.is_ready());
    assert_eq!(
        tool.intent().unwrap(),
        DrawIntent::Arc {
            points: [p(0.0, 0.0), p(5.0, 5.0), p(10.0, 0.0)]
        }
    );
}

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
fn arc_parameters_resolve_a_circumcircle_and_refuse_degenerate_input() {
    // Semicircle through (0,0), (5,5), (10,0): centre (5,0), radius 5.
    let parameters = arc_parameters(p(0.0, 0.0), p(5.0, 5.0), p(10.0, 0.0)).unwrap();
    assert!((parameters.center.x - 5.0).abs() < 1e-9, "{parameters:?}");
    assert!((parameters.center.y - 0.0).abs() < 1e-9, "{parameters:?}");
    assert!((parameters.radius - 5.0).abs() < 1e-9, "{parameters:?}");
    assert!((parameters.start - std::f64::consts::PI).abs() < 1e-9);
    // Through the upper point, the arc is clockwise: a negative sweep.
    assert!((parameters.sweep + std::f64::consts::PI).abs() < 1e-9);

    // Collinear points have no circumcircle.
    assert!(matches!(
        arc_parameters(p(0.0, 0.0), p(1.0, 0.0), p(2.0, 0.0)),
        Err(CadError::InvalidInput(_))
    ));
    // Coincident points are refused, never a degenerate arc.
    assert!(matches!(
        arc_parameters(p(0.0, 0.0), p(0.0, 0.0), p(1.0, 0.0)),
        Err(CadError::InvalidInput(_))
    ));
    // Non-finite points are refused.
    assert!(matches!(
        arc_parameters(p(0.0, 0.0), p(f64::NAN, 1.0), p(2.0, 0.0)),
        Err(CadError::InvalidInput(_))
    ));
}

#[test]
fn arc_parameters_are_translation_invariant_at_large_coordinates() {
    // The semicircle fixture translated by (1e6, 1e6): centre (1e6+5, 1e6), r=5.
    let base = 1.0e6;
    let parameters = arc_parameters(
        p(base, base),
        p(base + 5.0, base + 5.0),
        p(base + 10.0, base),
    )
    .unwrap();
    assert!(
        (parameters.center.x - (base + 5.0)).abs() < 1e-9,
        "{parameters:?}"
    );
    assert!((parameters.center.y - base).abs() < 1e-9, "{parameters:?}");
    assert!((parameters.radius - 5.0).abs() < 1e-9, "{parameters:?}");
    assert!((parameters.start - std::f64::consts::PI).abs() < 1e-9);
    assert!((parameters.sweep + std::f64::consts::PI).abs() < 1e-9);
    // The arc passes through the picked points: each is exactly one radius out.
    for point in [
        p(base, base),
        p(base + 5.0, base + 5.0),
        p(base + 10.0, base),
    ] {
        let distance = (point.x - parameters.center.x).hypot(point.y - parameters.center.y);
        assert!(
            (distance - parameters.radius).abs() < 1e-9,
            "{point:?} is not on the circumcircle"
        );
    }
}

#[test]
fn arc_parameters_accept_a_unit_arc_at_large_coordinates() {
    // A unit-radius upper semicircle centred at (1e6, 1e6) must be accepted,
    // not refused by a translation-dependent degeneracy test.
    let base = 1.0e6;
    let parameters = arc_parameters(
        p(base + 1.0, base),
        p(base, base + 1.0),
        p(base - 1.0, base),
    )
    .unwrap();
    assert!((parameters.center.x - base).abs() < 1e-9, "{parameters:?}");
    assert!((parameters.center.y - base).abs() < 1e-9, "{parameters:?}");
    assert!((parameters.radius - 1.0).abs() < 1e-9, "{parameters:?}");
    // right -> top -> left is a counter-clockwise half turn (positive sweep).
    assert!(
        (parameters.sweep - std::f64::consts::PI).abs() < 1e-9,
        "{parameters:?}"
    );
}

#[test]
fn arc_parameters_supports_a_near_full_arc() {
    let tau = std::f64::consts::TAU;
    let end_angle = tau - 0.1;
    let parameters = arc_parameters(
        p(1.0, 0.0),
        p(-1.0, 0.0),
        Point3 {
            x: end_angle.cos(),
            y: end_angle.sin(),
            z: 0.0,
        },
    )
    .unwrap();
    assert!((parameters.center.x).abs() < 1e-9, "{parameters:?}");
    assert!((parameters.center.y).abs() < 1e-9, "{parameters:?}");
    assert!((parameters.radius - 1.0).abs() < 1e-9, "{parameters:?}");
    assert!(
        (parameters.sweep - (tau - 0.1)).abs() < 1e-9,
        "{parameters:?}"
    );
}

#[test]
fn arc_parameters_refuse_non_coplanar_points() {
    // A point off the start plane must be refused, never silently projected.
    let through = Point3 {
        x: 1.0,
        y: 0.0,
        z: 5.0,
    };
    assert!(matches!(
        arc_parameters(p(0.0, 0.0), through, p(0.0, 1.0)),
        Err(CadError::InvalidInput(_))
    ));
}

#[test]
fn line_captures_two_points_and_confirms_a_line_intent() {
    let mut tool = DrawTool::new(DrawToolKind::Line, 0);
    assert_eq!(tool.remaining(), 2);
    assert!(!tool.is_ready());
    tool.push_point(p(0.0, 0.0)).unwrap();
    assert_eq!(tool.remaining(), 1);
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
fn move_requires_a_selection_and_computes_the_delta() {
    let mut tool = DrawTool::new(DrawToolKind::Move, 0);
    tool.push_point(p(1.0, 2.0)).unwrap();
    tool.push_point(p(4.0, 6.0)).unwrap();
    // Even with both points, a missing selection is not ready.
    assert!(!tool.is_ready());
    assert!(matches!(tool.intent(), Err(CadError::InvalidInput(_))));
    assert_eq!(tool.preview().status_line(), "移动：请先选择对象");

    tool.set_selection_count(2);
    assert!(tool.is_ready());
    assert_eq!(
        tool.intent().unwrap(),
        DrawIntent::Move { delta: p(3.0, 4.0) }
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
