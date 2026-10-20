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
            DrawToolKind::Arc
            | DrawToolKind::Ellipse
            | DrawToolKind::Scale
            | DrawToolKind::DimLinear
            | DrawToolKind::DimAligned
            | DrawToolKind::DimAngular => 3,
            DrawToolKind::Spline | DrawToolKind::Align | DrawToolKind::Stretch => 4,
            DrawToolKind::Offset
            | DrawToolKind::Point
            | DrawToolKind::ArrayPolar
            | DrawToolKind::DimCenter
            // TEXT captures one insertion point; its text is a typed string, not a
            // point (ADR 0019 decision A.1). TEXTEDIT's one step is its entity
            // pick (ADR 0020 amendment A).
            | DrawToolKind::Text
            | DrawToolKind::TextEdit => 1,
            // ARRAYRECT captures no points: it is defined entirely by the session
            // typed parameters plus the selection snapshot. HATCH has the same
            // pre-selection shape: the selected entities are the boundary and the
            // pattern parameters are the typed channel.
            DrawToolKind::ArrayRect | DrawToolKind::Hatch => 0,
            _ => 2,
        };
        assert_eq!(kind.required_points(), expected, "{kind:?}");
    }
    // The arity model: PLINE/SPLINE and the baseline/continue pair are open-ended.
    assert_eq!(DrawToolKind::Polyline.arity(), PointArity::AtLeast(2));
    assert_eq!(DrawToolKind::Spline.arity(), PointArity::AtLeast(4));
    assert_eq!(DrawToolKind::DimBaseline.arity(), PointArity::AtLeast(2));
    assert_eq!(DrawToolKind::DimContinue.arity(), PointArity::AtLeast(2));
    assert_eq!(DrawToolKind::Point.arity(), PointArity::Exact(1));
    for kind in DrawToolKind::ALL {
        if kind.arity().is_open_ended() {
            continue;
        }
        assert_eq!(
            kind.arity(),
            PointArity::Exact(kind.required_points()),
            "{kind:?} must stay fixed-arity"
        );
    }
    assert!(PointArity::AtLeast(2).is_open_ended());
    assert!(!PointArity::Exact(2).is_open_ended());
    assert_eq!(PointArity::AtLeast(2).minimum(), 2);
    assert_eq!(PointArity::Exact(3).minimum(), 3);
    assert!(!DrawToolKind::Line.requires_selection());
    assert!(!DrawToolKind::Circle.requires_selection());
    assert!(!DrawToolKind::Arc.requires_selection());
    assert!(!DrawToolKind::Polyline.requires_selection());
    assert!(!DrawToolKind::Polyline.needs_picked_entities());
    assert!(DrawToolKind::Move.requires_selection());
    assert!(!DrawToolKind::Trim.requires_selection());
    assert!(DrawToolKind::Copy.requires_selection());
    assert!(!DrawToolKind::Extend.requires_selection());
    assert!(DrawToolKind::Rotate.requires_selection());
    assert!(DrawToolKind::Scale.requires_selection());
    assert!(DrawToolKind::Mirror.requires_selection());
    assert!(DrawToolKind::Offset.requires_selection());
    // STRETCH is Exact(4) with no entity picks; the window itself defines the set.
    assert_eq!(DrawToolKind::Stretch.arity(), PointArity::Exact(4));
    assert!(!DrawToolKind::Stretch.requires_selection());
    assert!(!DrawToolKind::Stretch.needs_picked_entities());
}

#[test]
fn offset_captures_one_side_point() {
    let mut tool = DrawTool::new(DrawToolKind::Offset, 1);
    assert_eq!(tool.remaining(), Some(1));
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
    assert_eq!(tool.remaining(), Some(3));
    tool.push_point(p(0.0, 0.0)).unwrap();
    tool.push_point(p(5.0, 5.0)).unwrap();
    assert_eq!(tool.remaining(), Some(1));
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

mod array;
mod dimension;
mod hatch;
mod mleader;
mod point_spline;
mod text;

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
