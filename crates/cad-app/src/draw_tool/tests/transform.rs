//! OFFSET/ROTATE/SCALE/MIRROR/MOVE parameter capture and the pure transforms
//! behind them.
//!
//! Split out of `draw_tool/tests.rs` to keep that file under the code-size limit.

use super::*;

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
