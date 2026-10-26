//! Grip model contract tests for the curved geometries (`CIRCLE`, `ARC`).
//!
//! Split from `tests::grips` (LINE/POINT, hit-testing and the refusal list) so
//! both files stay small; the shared fixtures (`p3`, `ent`, `grip_id`,
//! `assert_reason`, `assert_bits_eq`) are imported from there.
//!
//! The model is pure, so "nothing else moved" is a **bit** comparison and the
//! only tolerance appears where a distance is recomputed independently.

use super::*;

use super::grips::{assert_bits_eq, assert_reason, ent, grip_id, p3, reason};
use crate::grips::{
    apply_grip, grip_hit, grip_hit_planar, grips_for_entity, Grip, GripKind, REASON_DEGENERATE_ARC,
    REASON_DEGENERATE_RADIUS, REASON_STALE_HANDLE, REASON_UNSUPPORTED_KIND,
    REASON_ZERO_DISPLACEMENT,
};
use cad_db::DbEntity;

fn circle(center: Point3, normal: Point3, radius: f64) -> SemanticGeometry {
    SemanticGeometry::Circle {
        center,
        normal,
        radius,
    }
}

fn arc(center: Point3, normal: Point3, radius: f64, start: f64, sweep: f64) -> SemanticGeometry {
    SemanticGeometry::Arc {
        center,
        normal,
        radius,
        start,
        sweep,
    }
}

fn assert_close(actual: Point3, expected: Point3, what: &str) {
    for (axis, (a, e)) in [
        ("x", (actual.x, expected.x)),
        ("y", (actual.y, expected.y)),
        ("z", (actual.z, expected.z)),
    ] {
        assert!((a - e).abs() < 1e-12, "{what}: {axis} is {a}, expected {e}");
    }
}

fn dot3(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn len3(v: Point3) -> f64 {
    dot3(v, v).sqrt()
}

fn sub3(a: Point3, b: Point3) -> Point3 {
    p3(a.x - b.x, a.y - b.y, a.z - b.z)
}

// ---------------------------------------------------------------------------
// 1. Which handles exist: centre first, then the four documented quadrants
// ---------------------------------------------------------------------------

#[test]
fn circle_exposes_a_centre_then_four_quadrants() {
    // Normal +Z is the common case: the shared arbitrary-axis frame is (+X, +Y),
    // so the documented quadrant order is +X, +Y, -X, -Y in the circle's plane.
    let entity = ent(31, circle(p3(1.0, 2.0, 3.0), p3(0.0, 0.0, 1.0), 5.0));
    let grips = grips_for_entity(&entity);
    assert_eq!(grips.len(), 5, "a CIRCLE has a centre and four quadrants");
    assert_eq!(
        grips[0],
        Grip {
            id: grip_id(31, GripKind::Center, 0),
            position: p3(1.0, 2.0, 3.0),
        },
        "index 0 of Center is the stored centre"
    );
    for (index, expected) in [
        p3(6.0, 2.0, 3.0),  // 0 = +ax (+X here)
        p3(1.0, 7.0, 3.0),  // 1 = +ay (+Y here)
        p3(-4.0, 2.0, 3.0), // 2 = -ax
        p3(1.0, -3.0, 3.0), // 3 = -ay
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            grips[index + 1],
            Grip {
                id: grip_id(31, GripKind::Quadrant, index as u8),
                position: expected,
            },
            "quadrant index {index} = centre + radius * basis"
        );
    }
}

#[test]
fn circle_quadrant_grips_lie_in_the_circles_own_plane_frame() {
    // A tilted, unnormalized normal: the positions must come from the shared
    // arbitrary-axis frame, not from a second (x/y) frame invented for grips.
    let center = p3(1.0, 2.0, 3.0);
    let normal = p3(1.0, 2.0, -3.0);
    let radius = 5.0;
    let entity = ent(31, circle(center, normal, radius));
    let grips = grips_for_entity(&entity);
    let (ax, ay, unit_normal) = cad_geometry::arbitrary_axis(normal);
    let basis = [ax, ay, p3(-ax.x, -ax.y, -ax.z), p3(-ay.x, -ay.y, -ay.z)];
    for (index, direction) in basis.into_iter().enumerate() {
        let position = grips[index + 1].position;
        assert_close(
            position,
            p3(
                center.x + radius * direction.x,
                center.y + radius * direction.y,
                center.z + radius * direction.z,
            ),
            &format!("quadrant {index} position"),
        );
        // In the circle's plane, at the radius, with the stored 3D coordinates.
        assert!(
            dot3(sub3(position, center), unit_normal).abs() < 1e-12,
            "quadrant {index} must lie in the circle plane"
        );
        assert!(
            (len3(sub3(position, center)) - radius).abs() < 1e-12,
            "quadrant {index} must be `radius` from the centre"
        );
    }
}

#[test]
fn arc_exposes_the_centre_then_both_endpoint_grips() {
    // Documented order: `Center 0, Endpoint 0 (start), Endpoint 1 (end)`.
    let center = p3(4.0, -1.0, 2.0);
    let radius = 5.5;
    let (start, sweep) = (0.3, 2.7);
    let entity = ent(32, arc(center, p3(0.0, 0.0, 1.0), radius, start, sweep));
    let grips = grips_for_entity(&entity);
    assert_eq!(
        grips.len(),
        3,
        "an ARC exposes the centre and its two endpoints"
    );
    assert_eq!(
        grips[0],
        Grip {
            id: grip_id(32, GripKind::Center, 0),
            position: center,
        },
        "index 0 is still the stored centre"
    );
    // Normal +Z: the shared arbitrary-axis frame is (+X, +Y), so the endpoint
    // position is `centre + radius * (cos angle, sin angle)` in that frame.
    for (index, angle) in [(0u8, start), (1, start + sweep)] {
        assert_bits_eq(
            grips[usize::from(index) + 1].position,
            p3(
                center.x + radius * angle.cos(),
                center.y + radius * angle.sin(),
                center.z,
            ),
            &format!("endpoint {index} position"),
        );
        assert_eq!(
            grips[usize::from(index) + 1].id,
            grip_id(32, GripKind::Endpoint, index)
        );
    }
}

#[test]
fn arc_endpoint_grips_lie_in_the_arcs_own_plane_frame() {
    // A tilted, unnormalized normal: the endpoint positions must come from the
    // shared arbitrary-axis frame, the same one the tessellator measures the
    // stored `start` / `sweep` angles in.
    let center = p3(1.0, 2.0, 3.0);
    let normal = p3(1.0, 2.0, -3.0);
    let radius = 4.25;
    let (start, sweep) = (0.7, -1.3);
    let entity = ent(32, arc(center, normal, radius, start, sweep));
    let grips = grips_for_entity(&entity);
    let (ax, ay, unit_normal) = cad_geometry::arbitrary_axis(normal);
    for (index, angle) in [(0u8, start), (1, start + sweep)] {
        let position = grips[usize::from(index) + 1].position;
        assert_close(
            position,
            p3(
                center.x + radius * (angle.cos() * ax.x + angle.sin() * ay.x),
                center.y + radius * (angle.cos() * ax.y + angle.sin() * ay.y),
                center.z + radius * (angle.cos() * ax.z + angle.sin() * ay.z),
            ),
            &format!("endpoint {index} position"),
        );
        assert!(
            dot3(sub3(position, center), unit_normal).abs() < 1e-12,
            "endpoint {index} must lie in the arc plane"
        );
        assert!(
            (len3(sub3(position, center)) - radius).abs() < 1e-12,
            "endpoint {index} must be `radius` from the centre"
        );
    }
}

// ---------------------------------------------------------------------------
// 2. Dragging
// ---------------------------------------------------------------------------

#[test]
fn dragging_the_circle_centre_moves_it_and_keeps_radius_and_normal() {
    let geometry = circle(p3(1.0, 2.0, 3.0), p3(0.0, 0.0, 1.0), 5.0);
    let dragged = apply_grip(
        &geometry,
        &grip_id(31, GripKind::Center, 0),
        p3(-4.0, 8.5, -2.0),
    )
    .expect("a circle centre drag is supported");
    match dragged {
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => {
            assert_bits_eq(center, p3(-4.0, 8.5, -2.0), "the dragged centre");
            assert_bits_eq(normal, p3(0.0, 0.0, 1.0), "the normal is untouched");
            assert_eq!(
                radius.to_bits(),
                5.0f64.to_bits(),
                "the radius is untouched"
            );
        }
        other => panic!("expected a CIRCLE, got {other:?}"),
    }
}

#[test]
fn each_quadrant_drag_sets_the_in_plane_radius() {
    // One target per quadrant index; the radius rule is the same for all four,
    // and the centre/normal never move.
    let geometry = circle(p3(0.0, 0.0, 0.0), p3(0.0, 0.0, 1.0), 5.0);
    let expected = (8.0f64 * 8.0 + 3.0 * 3.0f64).sqrt();
    for index in 0..=3u8 {
        let dragged = apply_grip(
            &geometry,
            &grip_id(31, GripKind::Quadrant, index),
            p3(8.0, 3.0, 0.0),
        )
        .unwrap_or_else(|error| panic!("quadrant {index} drag refused: {error:?}"));
        match dragged {
            SemanticGeometry::Circle {
                center,
                normal,
                radius,
            } => {
                assert_eq!(
                    radius, expected,
                    "quadrant {index} takes the in-plane distance"
                );
                assert_bits_eq(center, p3(0.0, 0.0, 0.0), "the centre stays put");
                assert_bits_eq(normal, p3(0.0, 0.0, 1.0), "the normal stays put");
            }
            other => panic!("expected a CIRCLE, got {other:?}"),
        }
    }
}

/// The key anti-bug test: the radius is measured **in the circle's plane**, so a
/// target lifted off the plane must not inflate it (3D distance) and a tilted
/// circle must not be measured in x/y.
#[test]
fn a_quadrant_drag_measures_the_radius_in_the_circle_plane_not_in_3d() {
    // Case A: XY circle, target 7 units above the plane.
    // in-plane distance = 10, 3D distance = sqrt(10^2 + 7^2) ~ 12.207, x/y = 10.
    let flat = circle(p3(0.0, 0.0, 0.0), p3(0.0, 0.0, 1.0), 5.0);
    let target = p3(6.0, 8.0, 7.0);
    let three_d = len3(target);
    assert!(
        (three_d - 149.0f64.sqrt()).abs() < 1e-12,
        "sanity: sqrt(6^2 + 8^2 + 7^2) is the 3D distance"
    );
    let dragged = apply_grip(&flat, &grip_id(31, GripKind::Quadrant, 0), target)
        .expect("an off-plane quadrant drag is supported");
    match dragged {
        SemanticGeometry::Circle { radius, .. } => {
            assert_eq!(radius, 10.0, "the in-plane distance, not the 3D distance");
            assert_ne!(
                radius, three_d,
                "a 3D distance would have tilted the circle"
            );
        }
        other => panic!("expected a CIRCLE, got {other:?}"),
    }

    // Case B: circle in the XZ plane (normal +Y). For this normal the shared
    // frame is ax = -X, ay = +Z, so the in-plane distance of (6, 9, 8) is 10
    // while the x/y distance is sqrt(117) ~ 10.817 — a second, x/y frame would
    // give a different radius.
    let tilted = circle(p3(1.0, 2.0, 3.0), p3(0.0, 1.0, 0.0), 5.0);
    let target = p3(1.0 + 6.0, 2.0 + 9.0, 3.0 + 8.0);
    let dragged = apply_grip(&tilted, &grip_id(31, GripKind::Quadrant, 1), target)
        .expect("a quadrant drag on a tilted circle is supported");
    match dragged {
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => {
            assert_eq!(radius, 10.0, "the in-plane distance of the tilted plane");
            assert!(
                (radius - 117.0f64.sqrt()).abs() > 1e-3,
                "never the x/y distance"
            );
            assert_bits_eq(center, p3(1.0, 2.0, 3.0), "the centre stays put");
            assert_bits_eq(normal, p3(0.0, 1.0, 0.0), "the normal stays put");
        }
        other => panic!("expected a CIRCLE, got {other:?}"),
    }
}

#[test]
fn a_quadrant_drag_that_would_zero_the_radius_is_refused_by_name() {
    let geometry = circle(p3(0.0, 0.0, 0.0), p3(0.0, 0.0, 1.0), 5.0);
    // The centre itself: in-plane distance 0.
    let error = apply_grip(
        &geometry,
        &grip_id(31, GripKind::Quadrant, 0),
        p3(0.0, 0.0, 0.0),
    )
    .expect_err("a zero radius is refused, never clamped");
    assert_reason(error, REASON_DEGENERATE_RADIUS);

    // Purely along the normal through the centre: also 0 in the plane.
    let error = apply_grip(
        &geometry,
        &grip_id(31, GripKind::Quadrant, 1),
        p3(0.0, 0.0, 7.0),
    )
    .expect_err("an off-plane target over the centre is a zero radius");
    assert_reason(error, REASON_DEGENERATE_RADIUS);

    // A corrupt normal makes the in-plane radius non-finite; that is refused
    // with the same code instead of storing `NaN`.
    let corrupt = circle(p3(0.0, 0.0, 0.0), p3(f64::NAN, 0.0, 1.0), 5.0);
    let error = apply_grip(
        &corrupt,
        &grip_id(31, GripKind::Quadrant, 2),
        p3(8.0, 0.0, 0.0),
    )
    .expect_err("a non-finite radius is refused");
    assert_reason(error, REASON_DEGENERATE_RADIUS);

    // Never clamped: a tiny but non-zero radius is stored exactly as computed.
    let dragged = apply_grip(
        &geometry,
        &grip_id(31, GripKind::Quadrant, 3),
        p3(1e-9, 0.0, 0.0),
    )
    .expect("a tiny positive radius is a real edit, not a degenerate one");
    match dragged {
        SemanticGeometry::Circle { radius, .. } => {
            assert_eq!(radius, 1e-9, "stored exactly, with no minimum radius");
        }
        other => panic!("expected a CIRCLE, got {other:?}"),
    }
}

#[test]
fn a_quadrant_drag_that_would_not_change_the_radius_writes_nothing() {
    let geometry = circle(p3(0.0, 0.0, 0.0), p3(0.0, 0.0, 1.0), 5.0);
    // Exactly at the handle.
    let error = apply_grip(
        &geometry,
        &grip_id(31, GripKind::Quadrant, 0),
        p3(5.0, 0.0, 0.0),
    )
    .expect_err("a drag onto the handle is nothing to write");
    assert_reason(error, REASON_ZERO_DISPLACEMENT);

    // Off the plane at the stored radius: the radius rule ignores the
    // out-of-plane offset, so this is equally a no-op (never an empty update).
    let error = apply_grip(
        &geometry,
        &grip_id(31, GripKind::Quadrant, 0),
        p3(5.0, 0.0, 3.0),
    )
    .expect_err("an off-plane target at the stored radius writes nothing");
    assert_reason(error, REASON_ZERO_DISPLACEMENT);
}

#[test]
fn dragging_the_arc_centre_moves_it_rigidly() {
    let geometry = arc(p3(4.0, -1.0, 2.0), p3(0.0, 0.0, 1.0), 5.5, 0.3, 2.7);
    let dragged = apply_grip(
        &geometry,
        &grip_id(32, GripKind::Center, 0),
        p3(7.25, -1.5, 2.0),
    )
    .expect("an arc centre drag is supported");
    match dragged {
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => {
            assert_bits_eq(center, p3(7.25, -1.5, 2.0), "the dragged centre");
            assert_bits_eq(normal, p3(0.0, 0.0, 1.0), "the normal is untouched");
            assert_eq!(
                radius.to_bits(),
                5.5f64.to_bits(),
                "the radius is untouched"
            );
            assert_eq!(
                start.to_bits(),
                0.3f64.to_bits(),
                "the start angle is untouched"
            );
            assert_eq!(sweep.to_bits(), 2.7f64.to_bits(), "the sweep is untouched");
        }
        other => panic!("expected an ARC, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 3. Refusals specific to the curved geometries
// ---------------------------------------------------------------------------

#[test]
fn dragging_the_arc_start_endpoint_moves_only_its_angle() {
    // Centre/radius/normal are bit-identical; the start angle becomes the
    // in-plane direction of the target and the sweep is recomputed so the end
    // angle (the stored `start + sweep`) stays put.
    let center = p3(1.0, 2.0, 3.0);
    let normal = p3(0.0, 0.0, 1.0);
    let radius = 5.0;
    let (start, sweep) = (0.0, 1.5);
    let geometry = arc(center, normal, radius, start, sweep);
    let (old_start_grip, old_end_grip) = (
        grips_for_entity(&ent(32, geometry.clone()))[1].position,
        grips_for_entity(&ent(32, geometry.clone()))[2].position,
    );
    // A target off the arc's plane (z = 9): the angle rule ignores that offset.
    let target = p3(
        center.x + radius * 0.5f64.cos(),
        center.y + radius * 0.5f64.sin(),
        9.0,
    );

    let dragged = apply_grip(&geometry, &grip_id(32, GripKind::Endpoint, 0), target)
        .expect("an arc start endpoint drag is supported");
    match dragged {
        SemanticGeometry::Arc {
            center: new_center,
            normal: new_normal,
            radius: new_radius,
            start: new_start,
            sweep: new_sweep,
        } => {
            assert_bits_eq(new_center, center, "the centre stays put");
            assert_bits_eq(new_normal, normal, "the normal stays put");
            assert_eq!(
                new_radius.to_bits(),
                radius.to_bits(),
                "the radius stays put"
            );
            assert!(
                (new_start - 0.5).abs() < 1e-12,
                "the start angle follows the in-plane direction, got {new_start}"
            );
            assert!(
                (new_sweep - 1.0).abs() < 1e-12,
                "the sweep shrinks to end - new start, got {new_sweep}"
            );
            // The anchored endpoint keeps its angle, so it keeps its position.
            let anchored = p3(
                new_center.x + new_radius * (new_start + new_sweep).cos(),
                new_center.y + new_radius * (new_start + new_sweep).sin(),
                new_center.z,
            );
            for (axis, (a, e)) in [
                ("x", (anchored.x, old_end_grip.x)),
                ("y", (anchored.y, old_end_grip.y)),
                ("z", (anchored.z, old_end_grip.z)),
            ] {
                assert!(
                    (a - e).abs() < 1e-9,
                    "the opposite endpoint keeps its angle: {axis} {a} vs {e}"
                );
            }
            assert!(
                old_start_grip != target,
                "the fixture drags the handle somewhere else"
            );
        }
        other => panic!("expected an ARC, got {other:?}"),
    }
}

#[test]
fn dragging_the_arc_end_endpoint_keeps_the_start_angle() {
    let center = p3(-2.0, 0.5, 7.0);
    let normal = p3(0.0, 0.0, 1.0);
    let radius = 3.0;
    let (start, sweep) = (0.4, 1.1);
    let geometry = arc(center, normal, radius, start, sweep);
    let target = p3(
        center.x + radius * 2.5f64.cos(),
        center.y + radius * 2.5f64.sin(),
        0.0,
    );

    let dragged = apply_grip(&geometry, &grip_id(32, GripKind::Endpoint, 1), target)
        .expect("an arc end endpoint drag is supported");
    match dragged {
        SemanticGeometry::Arc {
            center: new_center,
            normal: new_normal,
            radius: new_radius,
            start: new_start,
            sweep: new_sweep,
        } => {
            assert_bits_eq(new_center, center, "the centre stays put");
            assert_bits_eq(new_normal, normal, "the normal stays put");
            assert_eq!(
                new_radius.to_bits(),
                radius.to_bits(),
                "the radius stays put"
            );
            assert_eq!(
                new_start.to_bits(),
                start.to_bits(),
                "the start angle is bit-identical"
            );
            assert!(
                (new_sweep - (2.5 - start)).abs() < 1e-12,
                "the sweep grows to the new end angle, got {new_sweep}"
            );
        }
        other => panic!("expected an ARC, got {other:?}"),
    }
}

#[test]
fn an_arc_endpoint_drag_keeps_the_stored_sweep_direction() {
    // A clockwise / reflected arc (negative sweep) must stay clockwise: dragging
    // the start past the end gives the small clockwise extent, never the positive
    // complement.
    let center = p3(0.0, 0.0, 0.0);
    let normal = p3(0.0, 0.0, 1.0);
    let radius = 4.0;
    let geometry = arc(center, normal, radius, 2.0, -1.0);
    let target = p3(radius * 1.5f64.cos(), radius * 1.5f64.sin(), 0.0);
    let dragged = apply_grip(&geometry, &grip_id(32, GripKind::Endpoint, 0), target)
        .expect("a clockwise arc start drag is supported");
    match dragged {
        SemanticGeometry::Arc { start, sweep, .. } => {
            assert!((start - 1.5).abs() < 1e-12, "the new start angle");
            assert!(
                sweep < 0.0,
                "the sweep must stay negative (same direction), got {sweep}"
            );
            assert!(
                (sweep + 0.5).abs() < 1e-12,
                "the clockwise extent, got {sweep}"
            );
        }
        other => panic!("expected an ARC, got {other:?}"),
    }

    // A counter-clockwise arc dragged "behind" its anchored end keeps the
    // positive direction: the extent is the complement, not a negative sweep.
    let geometry = arc(center, normal, radius, 0.0, 1.0);
    let target = p3(radius * 2.0f64.cos(), radius * 2.0f64.sin(), 0.0);
    let dragged = apply_grip(&geometry, &grip_id(32, GripKind::Endpoint, 0), target)
        .expect("a counter-clockwise arc start drag is supported");
    match dragged {
        SemanticGeometry::Arc { sweep, .. } => {
            assert!(sweep > 0.0, "the sweep must stay positive, got {sweep}");
            assert!(
                (sweep - (std::f64::consts::TAU - 1.0)).abs() < 1e-12,
                "the counter-clockwise extent of the complement, got {sweep}"
            );
        }
        other => panic!("expected an ARC, got {other:?}"),
    }
}

#[test]
fn an_arc_endpoint_drag_that_collapses_the_arc_is_refused_by_name() {
    let center = p3(0.0, 0.0, 0.0);
    let normal = p3(0.0, 0.0, 1.0);
    let radius = 5.0;
    let geometry = arc(center, normal, radius, 0.0, 1.0);
    let at = |angle: f64| p3(radius * angle.cos(), radius * angle.sin(), 0.0);

    // ~0: the start almost reaches the anchored end angle.
    let error = apply_grip(
        &geometry,
        &grip_id(32, GripKind::Endpoint, 0),
        at(1.0 - 1e-12),
    )
    .expect_err("a near-zero sweep is refused, never clamped");
    assert_reason(error, REASON_DEGENERATE_ARC);

    // ~2π: the start almost makes the whole circle.
    let error = apply_grip(
        &geometry,
        &grip_id(32, GripKind::Endpoint, 0),
        at(1.0 + 1e-12),
    )
    .expect_err("a near-2pi sweep is refused, never clamped");
    assert_reason(error, REASON_DEGENERATE_ARC);

    // Exactly the anchored angle: the extent is a full turn (or exactly zero).
    let error = apply_grip(&geometry, &grip_id(32, GripKind::Endpoint, 1), at(0.0))
        .expect_err("an end drag onto the start is a full turn");
    assert_reason(error, REASON_DEGENERATE_ARC);

    // No in-plane direction at all: the centre itself, and a point on the normal
    // through it.
    for target in [center, p3(0.0, 0.0, 12.0)] {
        let error = apply_grip(&geometry, &grip_id(32, GripKind::Endpoint, 0), target)
            .expect_err("a target without an in-plane direction is refused");
        assert_reason(error, REASON_DEGENERATE_ARC);
    }

    // Never clamped: a tiny but real extent is stored as computed.
    let dragged = apply_grip(
        &geometry,
        &grip_id(32, GripKind::Endpoint, 0),
        at(1.0 - 1e-6),
    )
    .expect("a tiny positive sweep is a real edit, not a degenerate one");
    match dragged {
        SemanticGeometry::Arc { sweep, .. } => assert!((sweep - 1e-6).abs() < 1e-12),
        other => panic!("expected an ARC, got {other:?}"),
    }
}

#[test]
fn an_arc_endpoint_drag_with_no_geometric_effect_writes_nothing() {
    let center = p3(0.0, 0.0, 0.0);
    let normal = p3(0.0, 0.0, 1.0);
    let radius = 5.0;
    let geometry = arc(center, normal, radius, 0.0, 1.0);

    // Exactly at the handle.
    let error = apply_grip(
        &geometry,
        &grip_id(32, GripKind::Endpoint, 0),
        p3(radius, 0.0, 0.0),
    )
    .expect_err("a drag onto the handle is nothing to write");
    assert_reason(error, REASON_ZERO_DISPLACEMENT);

    // Off the plane along the same in-plane ray: the angle rule ignores the
    // out-of-plane offset, so the stored start/sweep would be unchanged.
    let error = apply_grip(
        &geometry,
        &grip_id(32, GripKind::Endpoint, 0),
        p3(radius, 0.0, 3.0),
    )
    .expect_err("an off-plane target along the same ray writes nothing");
    assert_reason(error, REASON_ZERO_DISPLACEMENT);
}

#[test]
fn arc_refuses_a_kind_or_index_it_has_no_handle_for() {
    let geometry = arc(p3(0.0, 0.0, 0.0), p3(0.0, 0.0, 1.0), 5.0, 0.0, 1.0);
    for (kind, index) in [
        (GripKind::Quadrant, 0u8),
        (GripKind::Midpoint, 2),
        (GripKind::Point, 0),
        (GripKind::Vertex, 0),
        (GripKind::ControlPoint, 0),
        (GripKind::Center, 1),
        (GripKind::Endpoint, 2),
        (GripKind::Endpoint, u8::MAX),
    ] {
        let error = apply_grip(&geometry, &grip_id(32, kind, index), p3(1.0, 1.0, 0.0))
            .expect_err("an ARC has no handle for this kind/index");
        assert_reason(error, REASON_STALE_HANDLE);
    }
}

#[test]
fn circle_refuses_a_kind_or_index_it_has_no_handle_for() {
    let geometry = circle(p3(0.0, 0.0, 0.0), p3(0.0, 0.0, 1.0), 5.0);
    for (kind, index) in [
        (GripKind::Endpoint, 0u8),
        (GripKind::Endpoint, 1),
        (GripKind::Midpoint, 2),
        (GripKind::Point, 0),
        (GripKind::ControlPoint, 0),
        (GripKind::Center, 1),
        (GripKind::Quadrant, 4),
        (GripKind::Quadrant, u8::MAX),
    ] {
        let error = apply_grip(&geometry, &grip_id(31, kind, index), p3(1.0, 1.0, 0.0))
            .expect_err("a CIRCLE has no handle for this kind/index");
        assert_reason(error, REASON_STALE_HANDLE);
    }
}

#[test]
fn a_curve_id_is_stale_even_when_the_target_is_the_handle() {
    // The handle identity is validated before the geometry rules, so an id of
    // the wrong kind never reaches the zero-displacement check.
    let geometry = arc(p3(0.0, 0.0, 0.0), p3(0.0, 0.0, 1.0), 5.0, 0.0, 1.0);
    let error = apply_grip(
        &geometry,
        &grip_id(32, GripKind::Center, 3),
        p3(0.0, 0.0, 0.0),
    )
    .expect_err("a center index above 0 is stale");
    assert_reason(error, REASON_STALE_HANDLE);
}

#[test]
fn a_non_finite_target_is_refused_before_the_radius_rule() {
    let geometry = circle(p3(0.0, 0.0, 0.0), p3(0.0, 0.0, 1.0), 5.0);
    for target in [
        p3(f64::NAN, 0.0, 0.0),
        p3(0.0, f64::INFINITY, 0.0),
        p3(0.0, 0.0, f64::NEG_INFINITY),
    ] {
        let error = apply_grip(&geometry, &grip_id(31, GripKind::Quadrant, 0), target)
            .expect_err("a non-finite target is refused");
        assert_reason(error, crate::grips::REASON_NON_FINITE_TARGET);
    }
}

#[test]
fn a_geometry_with_no_grip_model_still_names_its_class() {
    // `INSERT` stays on the refusal list: no grip model at all. (`SPLINE` used to
    // be this fixture; it now has control-point grips, pinned in
    // `tests::grips_spline`.)
    let insert = SemanticGeometry::Insert {
        block: BlockId(0),
        transform: Transform3::translation(p3(5.0, 5.0, 0.0)),
    };
    let error = apply_grip(
        &insert,
        &grip_id(33, GripKind::Center, 0),
        p3(1.0, 1.0, 0.0),
    )
    .expect_err("an INSERT exposes no grip model");
    assert_reason(error.clone(), REASON_UNSUPPORTED_KIND);
    assert!(reason(error).contains("AcDbBlockReference"));
}

// ---------------------------------------------------------------------------
// 4. Hit-testing the new handles (the tie-break with the centre)
// ---------------------------------------------------------------------------

#[test]
fn grip_hit_finds_the_circle_centre_and_quadrant() {
    let entity: DbEntity = ent(31, circle(p3(0.0, 0.0, 0.0), p3(0.0, 0.0, 1.0), 5.0));
    let hit = grip_hit([entity.clone()].iter(), p3(0.2, 0.1, 0.0), 1.0).expect("a hit");
    assert_eq!(hit.id, grip_id(31, GripKind::Center, 0));
    let hit = grip_hit([entity].iter(), p3(-0.2, 5.1, 0.0), 1.0).expect("a hit");
    assert_eq!(hit.id, grip_id(31, GripKind::Quadrant, 1));
}

#[test]
fn an_exact_tie_between_the_centre_and_a_quadrant_goes_to_the_centre() {
    // (2.5, 0) is 2.5 from the centre and 2.5 from quadrant 0 at (5, 0): the
    // centre is earlier in the list, so it wins — in both hit rules.
    let entity = ent(31, circle(p3(0.0, 0.0, 0.0), p3(0.0, 0.0, 1.0), 5.0));
    let query = p3(2.5, 0.0, 0.0);
    let hit = grip_hit([entity.clone()].iter(), query, 3.0).expect("a hit");
    assert_eq!(
        hit.id,
        grip_id(31, GripKind::Center, 0),
        "centre wins the tie"
    );
    let hit = grip_hit_planar([entity].iter(), query, 3.0).expect("a hit");
    assert_eq!(
        hit.id,
        grip_id(31, GripKind::Center, 0),
        "centre wins the tie"
    );
}

#[test]
fn a_quadrant_is_hit_in_the_plan_view_at_another_elevation() {
    // The circle lives at z = 5; the plan-view pointer resolves onto z = 0, so
    // only the plan-view rule can arm its quadrant (3D grip editing is NOT
    // IMPLEMENTED, the host keeps the handle's own z).
    let entity = ent(31, circle(p3(0.0, 0.0, 5.0), p3(0.0, 0.0, 1.0), 5.0));
    let plan = p3(0.0, 5.1, 0.0);
    assert!(grip_hit([entity.clone()].iter(), plan, 1.0).is_none());
    let hit = grip_hit_planar([entity].iter(), plan, 1.0).expect("the plan-view rule hits");
    assert_eq!(hit.id, grip_id(31, GripKind::Quadrant, 1));
    assert_bits_eq(
        hit.position,
        p3(0.0, 5.0, 5.0),
        "the position is the stored 3D handle, never a projection",
    );
}

#[test]
fn a_circle_with_a_non_finite_handle_is_skipped_by_both_hit_rules() {
    // A radius of `NaN` makes every quadrant position non-finite; the centre is
    // still a valid handle, so it must remain hittable while the quadrants are
    // not.
    let entity = ent(31, circle(p3(0.0, 0.0, 0.0), p3(0.0, 0.0, 1.0), f64::NAN));
    // The centre is 5 away, so only a corrupt quadrant could match: neither rule
    // reports one (a `NaN` distance/position is never a hit).
    assert!(grip_hit([entity.clone()].iter(), p3(5.0, 0.0, 0.0), 1.0).is_none());
    assert!(grip_hit_planar([entity.clone()].iter(), p3(5.0, 0.0, 0.0), 1.0).is_none());
    let hit = grip_hit_planar([entity].iter(), p3(0.1, 0.1, 0.0), 1.0).expect("the centre");
    assert_eq!(hit.id, grip_id(31, GripKind::Center, 0));
}
