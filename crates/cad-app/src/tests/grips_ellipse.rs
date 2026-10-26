//! Grip model contract tests for `ELLIPSE` (`crates/cad-app/src/grips/`).
//!
//! Pins the whole implemented ellipse surface: the handle list and its order, the
//! exact handle positions in the ellipse's **stored** plane frame, the two
//! quadrant drag rules (major and minor), the centre's rigidity, the refusals and
//! the partial-ellipse (elliptical arc) decision.
//!
//! The model is pure, so "nothing else moved" is a **bit** comparison; tolerances
//! appear only where a position or a radius is recomputed independently from the
//! stored values.
//!
//! Documented choices pinned here (ours, **not AutoCAD claims**):
//!
//! * quadrant order within the ellipse's own frame is `0` = `+major`,
//!   `1` = `+minor`, `2` = `-major`, `3` = `-minor`, i.e. the circle's
//!   `+, +, -, -` ordering applied to the stored axes;
//! * a quadrant drag sets **that** radius from the target's **in-plane** distance
//!   to the centre and **never rotates** the axes (a major drag scales the stored
//!   vector; a minor drag leaves `major_axis` bit-identical);
//! * there is **no** `minor <= major` invariant in the domain model, so a drag may
//!   cross the two radii and is neither refused nor silently re-normalised;
//! * a **partial** ellipse (stored `|sweep|` not a full turn) exposes the centre
//!   grip only; quadrant ids are refused as `grip.partial_ellipse`.

use super::*;

use super::grips::{assert_bits_eq, assert_reason, ent, grip_id, p3};
use crate::grips::{
    apply_grip, grip_hit, grip_hit_planar, grips_for_entity, Grip, GripKind,
    REASON_DEGENERATE_RADIUS, REASON_NON_FINITE_TARGET, REASON_PARTIAL_ELLIPSE,
    REASON_STALE_HANDLE, REASON_ZERO_DISPLACEMENT,
};
use cad_db::DbEntity;

/// A full ellipse with the given frame (`normal` need not be unit-length).
fn ellipse(
    center: Point3,
    normal: Point3,
    major_axis: Point3,
    ratio: f64,
    start: f64,
    sweep: f64,
) -> SemanticGeometry {
    SemanticGeometry::Ellipse {
        center,
        normal,
        major_axis,
        ratio,
        start,
        sweep,
    }
}

/// The common fixture: a full ellipse in the z = 0 plane.
fn flat() -> SemanticGeometry {
    ellipse(
        p3(0.0, 0.0, 0.0),
        p3(0.0, 0.0, 1.0),
        p3(5.0, 0.0, 0.0),
        0.5,
        0.0,
        std::f64::consts::TAU,
    )
}

/// The **rotated** fixture: an in-plane major axis at 3-4-5, so the minor
/// direction is not an x/y axis.
fn rotated() -> SemanticGeometry {
    ellipse(
        p3(1.0, 2.0, 3.0),
        p3(0.0, 0.0, 1.0),
        p3(3.0, 4.0, 0.0),
        0.5,
        0.0,
        std::f64::consts::TAU,
    )
}

fn dot3(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn sub3(a: Point3, b: Point3) -> Point3 {
    p3(a.x - b.x, a.y - b.y, a.z - b.z)
}

fn len3(v: Point3) -> f64 {
    dot3(v, v).sqrt()
}

fn close(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() < 1e-12,
        "{what}: {actual} vs {expected}"
    );
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

/// The stored ellipse fields of an `apply_grip` result.
fn parts(geometry: &SemanticGeometry) -> (Point3, Point3, Point3, f64, f64, f64) {
    match geometry {
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => (*center, *normal, *major_axis, *ratio, *start, *sweep),
        other => panic!("expected an ELLIPSE, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 1. Handle list: centre first, then the four stored-axis quadrants
// ---------------------------------------------------------------------------

#[test]
fn a_full_ellipse_exposes_a_centre_then_four_axis_quadrants() {
    let entity = ent(51, rotated());
    let grips = grips_for_entity(&entity);
    assert_eq!(
        grips.len(),
        5,
        "a full ELLIPSE has a centre and four axis quadrants"
    );
    assert_eq!(
        grips[0],
        Grip {
            id: grip_id(51, GripKind::Center, 0),
            position: p3(1.0, 2.0, 3.0),
        },
        "index 0 of Center is the stored centre (bit-identical)"
    );
    let (center, _, major_axis, ratio, ..) = parts(&rotated());
    // `0` and `2` are exactly `centre + major_axis` / `centre - major_axis`: the
    // tessellator's `center + major_axis * cos t + minor * sin t` at `t = 0, π`.
    assert_eq!(
        grips[1],
        Grip {
            id: grip_id(51, GripKind::Quadrant, 0),
            position: p3(center.x + major_axis.x, center.y + major_axis.y, center.z),
        },
        "quadrant 0 is `+major`"
    );
    assert_eq!(
        grips[3],
        Grip {
            id: grip_id(51, GripKind::Quadrant, 2),
            position: p3(center.x - major_axis.x, center.y - major_axis.y, center.z),
        },
        "quadrant 2 is `-major`"
    );
    // The minor semi-axis in the stored frame: `unit(minor) * |major| * ratio`
    // with `unit(minor) = normalize(cross(unit(normal), unit(major)))`; for a
    // 3-4-5 major axis and `+Z` that is `2.5 * (-0.8, 0.6, 0) = (-2, 1.5, 0)`.
    let minor_len = len3(major_axis) * ratio;
    assert_eq!(minor_len, 2.5, "the fixture's minor radius");
    assert_close(
        grips[2].position,
        p3(center.x - 2.0, center.y + 1.5, center.z),
        "quadrant 1 is `+minor`",
    );
    assert_close(
        grips[4].position,
        p3(center.x + 2.0, center.y - 1.5, center.z),
        "quadrant 3 is `-minor`",
    );
    for index in 0..=3u8 {
        assert_eq!(
            grips[usize::from(index) + 1].id,
            grip_id(51, GripKind::Quadrant, index),
            "the list order is quadrants 0..=3, right after the centre"
        );
    }
}

#[test]
fn ellipse_quadrants_lie_in_the_stored_plane_frame_and_on_the_curve() {
    // A tilted, unnormalised normal with a major axis perpendicular to it: the
    // positions must come from the ellipse's own stored frame, never from a
    // second (arbitrary-axis or x/y) frame.
    let normal = p3(1.0, 2.0, -3.0);
    // `cross(normal, +Z) = (2, -1, 0)`, already perpendicular to `normal`;
    // scaled to length 4.
    let perp = p3(2.0, -1.0, 0.0);
    let perp_len = len3(perp);
    let center = p3(-2.0, 0.5, 4.0);
    let ratio = 0.75;
    let major_axis = p3(perp.x * 4.0 / perp_len, perp.y * 4.0 / perp_len, 0.0);
    let entity = ent(
        52,
        ellipse(
            center,
            normal,
            major_axis,
            ratio,
            0.0,
            std::f64::consts::TAU,
        ),
    );
    let grips = grips_for_entity(&entity);
    assert_eq!(grips.len(), 5);
    let unit_normal = {
        let n = len3(normal);
        p3(normal.x / n, normal.y / n, normal.z / n)
    };
    let unit_major = {
        let m = len3(major_axis);
        p3(major_axis.x / m, major_axis.y / m, major_axis.z / m)
    };
    let minor_dir = {
        let c = p3(
            unit_normal.y * unit_major.z - unit_normal.z * unit_major.y,
            unit_normal.z * unit_major.x - unit_normal.x * unit_major.z,
            unit_normal.x * unit_major.y - unit_normal.y * unit_major.x,
        );
        let l = len3(c);
        p3(c.x / l, c.y / l, c.z / l)
    };
    assert_bits_eq(
        grips[1].position,
        p3(
            center.x + major_axis.x,
            center.y + major_axis.y,
            center.z + major_axis.z,
        ),
        "quadrant 0 is `+major_axis`",
    );
    assert_close(
        grips[2].position,
        p3(
            center.x + minor_dir.x * 4.0 * ratio,
            center.y + minor_dir.y * 4.0 * ratio,
            center.z + minor_dir.z * 4.0 * ratio,
        ),
        "quadrant 1 is `+minor`",
    );
    for (index, position) in grips[1..].iter().enumerate() {
        let offset = sub3(position.position, center);
        assert!(
            dot3(offset, unit_normal).abs() < 1e-12,
            "quadrant {index} must lie in the stored ellipse plane"
        );
        // On the ellipse: (offset·u / a)^2 + (offset·m / b)^2 == 1.
        let along = dot3(offset, unit_major) / 4.0;
        let across = dot3(offset, minor_dir) / (4.0 * ratio);
        close(along * along + across * across, 1.0, "on the ellipse");
        // The stored axes stay orthogonal — the frame is never re-derived.
        close(dot3(unit_major, minor_dir), 0.0, "axes are orthogonal");
    }
    // The four handles are the parameterisation's quarter points: 1 and 3 sit on
    // the opposite side of the major axis from each other.
    assert_close(
        grips[4].position,
        p3(
            center.x - minor_dir.x * 4.0 * ratio,
            center.y - minor_dir.y * 4.0 * ratio,
            center.z - minor_dir.z * 4.0 * ratio,
        ),
        "quadrant 3 is `-minor`",
    );
}

// ---------------------------------------------------------------------------
// 2. Dragging the centre: rigid, every other field bit-identical
// ---------------------------------------------------------------------------

#[test]
fn dragging_the_ellipse_centre_moves_it_rigidly() {
    let geometry = rotated();
    let (_, normal, major_axis, ratio, start, sweep) = parts(&geometry);
    let dragged = apply_grip(
        &geometry,
        &grip_id(51, GripKind::Center, 0),
        p3(-4.0, 8.5, -2.0),
    )
    .expect("an ellipse centre drag is supported");
    let (center, new_normal, new_major, new_ratio, new_start, new_sweep) = parts(&dragged);
    assert_bits_eq(center, p3(-4.0, 8.5, -2.0), "the dragged centre");
    assert_bits_eq(new_normal, normal, "the normal is untouched");
    assert_bits_eq(new_major, major_axis, "the major axis is untouched");
    assert_eq!(
        new_ratio.to_bits(),
        ratio.to_bits(),
        "the ratio is untouched"
    );
    assert_eq!(new_start.to_bits(), start.to_bits(), "start is untouched");
    assert_eq!(new_sweep.to_bits(), sweep.to_bits(), "sweep is untouched");
}

// ---------------------------------------------------------------------------
// 3. Quadrant drags: the corresponding radius, measured in the ellipse plane
// ---------------------------------------------------------------------------

#[test]
fn each_major_quadrant_drag_sets_the_major_radius_and_keeps_the_minor_one() {
    let geometry = flat();
    let expected = (8.0f64 * 8.0 + 3.0 * 3.0f64).sqrt();
    for index in [0u8, 2] {
        let dragged = apply_grip(
            &geometry,
            &grip_id(51, GripKind::Quadrant, index),
            p3(8.0, 3.0, 0.0),
        )
        .unwrap_or_else(|error| panic!("quadrant {index} drag refused: {error:?}"));
        let (center, normal, major_axis, ratio, start, sweep) = parts(&dragged);
        assert_bits_eq(center, p3(0.0, 0.0, 0.0), "the centre stays put");
        assert_bits_eq(normal, p3(0.0, 0.0, 1.0), "the normal stays put");
        close(
            len3(major_axis),
            expected,
            "the major radius is the in-plane distance",
        );
        assert!(
            major_axis.y == 0.0 && major_axis.z == 0.0,
            "the major direction never rotates: {major_axis:?}"
        );
        // The stored minor radius (5 * 0.5 = 2.5) is preserved, so `ratio` is
        // recomputed as 2.5 / distance.
        close(
            len3(major_axis) * ratio,
            2.5,
            "the minor radius is preserved",
        );
        close(ratio, 2.5 / expected, "the recomputed ratio");
        assert_eq!(start.to_bits(), 0.0f64.to_bits(), "start is untouched");
        assert_eq!(
            sweep.to_bits(),
            std::f64::consts::TAU.to_bits(),
            "sweep is untouched"
        );
    }
}

#[test]
fn each_minor_quadrant_drag_sets_the_ratio_and_keeps_the_major_axis() {
    let geometry = flat();
    let expected = (8.0f64 * 8.0 + 3.0 * 3.0f64).sqrt();
    for index in [1u8, 3] {
        let dragged = apply_grip(
            &geometry,
            &grip_id(51, GripKind::Quadrant, index),
            p3(8.0, 3.0, 0.0),
        )
        .unwrap_or_else(|error| panic!("quadrant {index} drag refused: {error:?}"));
        let (center, normal, major_axis, ratio, start, sweep) = parts(&dragged);
        assert_bits_eq(center, p3(0.0, 0.0, 0.0), "the centre stays put");
        assert_bits_eq(normal, p3(0.0, 0.0, 1.0), "the normal stays put");
        assert_bits_eq(
            major_axis,
            p3(5.0, 0.0, 0.0),
            "the major axis vector is bit-identical",
        );
        close(ratio, expected / 5.0, "ratio = distance / |major_axis|");
        assert_eq!(start.to_bits(), 0.0f64.to_bits(), "start is untouched");
        assert_eq!(
            sweep.to_bits(),
            std::f64::consts::TAU.to_bits(),
            "sweep is untouched"
        );
    }
}

/// The key anti-bug test: the radius is measured **in the ellipse plane**, so a
/// target lifted off the plane must not inflate it (3D distance) and a tilted
/// ellipse must not be measured in x/y.
#[test]
fn a_quadrant_drag_measures_the_radius_in_the_ellipse_plane_not_in_3d() {
    // Case A: z = 0 ellipse, target 7 units above the plane.
    // in-plane distance = 10, 3D distance = sqrt(149) ~ 12.207, x/y = 10.
    let geometry = flat();
    let target = p3(6.0, 8.0, 7.0);
    let three_d = len3(target);
    close(three_d, 149.0f64.sqrt(), "sanity: the 3D distance");
    let dragged = apply_grip(&geometry, &grip_id(51, GripKind::Quadrant, 0), target)
        .expect("an off-plane major drag is supported");
    let (_, _, major_axis, ratio, ..) = parts(&dragged);
    close(
        len3(major_axis),
        10.0,
        "the in-plane distance, not the 3D one",
    );
    assert!(
        (len3(major_axis) - three_d).abs() > 1.0,
        "a 3D distance would have produced {three_d}"
    );
    close(ratio, 0.25, "the minor radius 2.5 over the new major 10");

    let dragged = apply_grip(&geometry, &grip_id(51, GripKind::Quadrant, 1), target)
        .expect("an off-plane minor drag is supported");
    let (_, _, major_axis, ratio, ..) = parts(&dragged);
    assert_bits_eq(major_axis, p3(5.0, 0.0, 0.0), "the major axis is untouched");
    close(ratio, 2.0, "the minor radius is 10, ratio 10 / 5");

    // Case B: ellipse in the XZ plane (normal +Y). For this frame the major
    // direction is `(0.6, 0, 0.8)` and the minor `(0.8, 0, -0.6)`, so the offset
    // `(6, 9, 8)` is 10 in-plane while the x/y distance is sqrt(117) ~ 10.817 and
    // the 3D distance sqrt(181) ~ 13.454.
    let tilted = ellipse(
        p3(1.0, 2.0, 3.0),
        p3(0.0, 1.0, 0.0),
        p3(3.0, 0.0, 4.0),
        0.5,
        0.0,
        std::f64::consts::TAU,
    );
    let target = p3(1.0 + 6.0, 2.0 + 9.0, 3.0 + 8.0);
    let xy = 117.0f64.sqrt();
    let space = 181.0f64.sqrt();
    let dragged = apply_grip(&tilted, &grip_id(51, GripKind::Quadrant, 0), target)
        .expect("a major drag on a tilted ellipse is supported");
    let (center, normal, major_axis, ..) = parts(&dragged);
    close(
        len3(major_axis),
        10.0,
        "the in-plane distance of the tilted plane",
    );
    assert!(
        (len3(major_axis) - xy).abs() > 1e-3,
        "never the x/y distance ({xy})"
    );
    assert!(
        (len3(major_axis) - space).abs() > 1e-3,
        "never the 3D distance ({space})"
    );
    assert_bits_eq(center, p3(1.0, 2.0, 3.0), "the centre stays put");
    assert_bits_eq(normal, p3(0.0, 1.0, 0.0), "the normal stays put");
    // The stored major direction is preserved (no rotation).
    assert!(
        major_axis.y == 0.0 && (major_axis.x / major_axis.z - 0.75).abs() < 1e-12,
        "the stored major direction is scaled, never re-derived: {major_axis:?}"
    );
}

#[test]
fn an_ellipse_drag_may_cross_the_two_radii_without_being_clamped() {
    // The domain model has no `minor <= major` invariant (`cad-db` only requires a
    // positive finite ratio, and the renderer draws `|major| * |ratio|`), so this
    // is neither refused nor re-normalised: the stored roles simply swap visually.
    let geometry = flat();
    // Minor handle dragged far out: ratio 3 (> 1), major axis untouched.
    let dragged = apply_grip(
        &geometry,
        &grip_id(51, GripKind::Quadrant, 1),
        p3(15.0, 0.0, 0.0),
    )
    .expect("a ratio above 1 is representable");
    let (_, _, major_axis, ratio, ..) = parts(&dragged);
    assert_bits_eq(major_axis, p3(5.0, 0.0, 0.0), "the major axis is untouched");
    close(ratio, 3.0, "ratio = 15 / 5");

    // Major handle dragged inside the stored minor radius: ratio 2.5 / 1 = 2.5.
    let dragged = apply_grip(
        &geometry,
        &grip_id(51, GripKind::Quadrant, 0),
        p3(1.0, 0.0, 0.0),
    )
    .expect("shrinking the major radius below the minor one is representable");
    let (_, _, major_axis, ratio, ..) = parts(&dragged);
    close(
        len3(major_axis),
        1.0,
        "the major radius is the in-plane distance",
    );
    close(ratio, 2.5, "the minor radius 2.5 over the new major 1");
}

// ---------------------------------------------------------------------------
// 4. Refusals
// ---------------------------------------------------------------------------

#[test]
fn a_quadrant_drag_that_would_zero_a_radius_is_refused_by_name() {
    let geometry = flat();
    for index in 0..=3u8 {
        // The centre itself: in-plane distance 0.
        let error = apply_grip(
            &geometry,
            &grip_id(51, GripKind::Quadrant, index),
            p3(0.0, 0.0, 0.0),
        )
        .expect_err("a zero radius is refused, never clamped");
        assert_reason(error, REASON_DEGENERATE_RADIUS);
    }
    // Purely along the normal through the centre: also 0 in the plane.
    let error = apply_grip(
        &geometry,
        &grip_id(51, GripKind::Quadrant, 0),
        p3(0.0, 0.0, 7.0),
    )
    .expect_err("an off-plane target over the centre is a zero radius");
    assert_reason(error, REASON_DEGENERATE_RADIUS);

    // Never clamped: a tiny but non-zero radius is stored exactly as computed.
    let dragged = apply_grip(
        &geometry,
        &grip_id(51, GripKind::Quadrant, 1),
        p3(1e-9, 0.0, 0.0),
    )
    .expect("a tiny positive radius is a real edit, not a degenerate one");
    let (_, _, major_axis, ratio, ..) = parts(&dragged);
    assert_bits_eq(major_axis, p3(5.0, 0.0, 0.0), "the major axis is untouched");
    close(ratio, 1e-9 / 5.0, "stored exactly, with no minimum ratio");
}

#[test]
fn a_quadrant_drag_on_a_geometry_with_no_derivable_frame_is_refused_by_name() {
    // A zero-length major axis: no plane frame, so no quadrant handle exists
    // (the entity exposes the centre only) and the drag is refused by name.
    let broken = ellipse(
        p3(0.0, 0.0, 0.0),
        p3(0.0, 0.0, 1.0),
        p3(0.0, 0.0, 0.0),
        0.5,
        0.0,
        std::f64::consts::TAU,
    );
    assert_eq!(
        grips_for_entity(&ent(53, broken.clone())).len(),
        1,
        "a frame-less ellipse exposes the centre grip only"
    );
    let error = apply_grip(
        &broken,
        &grip_id(53, GripKind::Quadrant, 0),
        p3(4.0, 0.0, 0.0),
    )
    .expect_err("no in-plane radius can be derived");
    assert_reason(error, REASON_DEGENERATE_RADIUS);

    // A normal parallel to the major axis: same story.
    let parallel = ellipse(
        p3(0.0, 0.0, 0.0),
        p3(1.0, 0.0, 0.0),
        p3(5.0, 0.0, 0.0),
        0.5,
        0.0,
        std::f64::consts::TAU,
    );
    assert_eq!(grips_for_entity(&ent(54, parallel.clone())).len(), 1);
    let error = apply_grip(
        &parallel,
        &grip_id(54, GripKind::Quadrant, 1),
        p3(0.0, 4.0, 0.0),
    )
    .expect_err("the normal is parallel to the major axis");
    assert_reason(error, REASON_DEGENERATE_RADIUS);
}

#[test]
fn a_quadrant_drag_that_would_not_change_a_radius_writes_nothing() {
    let geometry = flat();
    // Exactly at the major handle.
    let error = apply_grip(
        &geometry,
        &grip_id(51, GripKind::Quadrant, 0),
        p3(5.0, 0.0, 0.0),
    )
    .expect_err("a drag onto the handle is nothing to write");
    assert_reason(error, REASON_ZERO_DISPLACEMENT);

    // Off the plane at the stored major radius: the radius rule ignores the
    // out-of-plane offset, so this is equally a no-op.
    let error = apply_grip(
        &geometry,
        &grip_id(51, GripKind::Quadrant, 0),
        p3(5.0, 0.0, 3.0),
    )
    .expect_err("an off-plane target at the stored radius writes nothing");
    assert_reason(error, REASON_ZERO_DISPLACEMENT);

    // Exactly at the minor handle, and at any point of the same in-plane radius.
    for target in [p3(0.0, 2.5, 0.0), p3(1.5, 2.0, 0.0), p3(0.0, 2.5, 4.0)] {
        let error = apply_grip(&geometry, &grip_id(51, GripKind::Quadrant, 1), target)
            .expect_err("the minor radius would stay 2.5");
        assert_reason(error, REASON_ZERO_DISPLACEMENT);
    }
}

#[test]
fn ellipse_refuses_a_kind_or_index_it_has_no_handle_for() {
    let geometry = flat();
    for (kind, index) in [
        (GripKind::Endpoint, 0u8),
        (GripKind::Endpoint, 1),
        (GripKind::Midpoint, 2),
        (GripKind::Vertex, 0),
        (GripKind::ControlPoint, 0),
        (GripKind::Point, 0),
        (GripKind::Center, 1),
        (GripKind::Quadrant, 4),
        (GripKind::Quadrant, u8::MAX),
    ] {
        let error = apply_grip(&geometry, &grip_id(51, kind, index), p3(1.0, 1.0, 0.0))
            .expect_err("an ELLIPSE has no handle for this kind/index");
        assert_reason(error, REASON_STALE_HANDLE);
    }
}

#[test]
fn an_ellipse_id_is_stale_even_when_the_target_is_the_handle() {
    // Handle identity is validated before the geometry rules, so a wrong kind
    // never reaches the zero-displacement check.
    let geometry = flat();
    let error = apply_grip(
        &geometry,
        &grip_id(51, GripKind::Center, 3),
        p3(0.0, 0.0, 0.0),
    )
    .expect_err("a centre index above 0 is stale");
    assert_reason(error, REASON_STALE_HANDLE);
}

#[test]
fn a_non_finite_target_is_refused_before_the_radius_rule() {
    let geometry = flat();
    for target in [
        p3(f64::NAN, 0.0, 0.0),
        p3(0.0, f64::INFINITY, 0.0),
        p3(0.0, 0.0, f64::NEG_INFINITY),
    ] {
        let error = apply_grip(&geometry, &grip_id(51, GripKind::Quadrant, 0), target)
            .expect_err("a non-finite target is refused");
        assert_reason(error, REASON_NON_FINITE_TARGET);
        let error = apply_grip(&geometry, &grip_id(51, GripKind::Center, 0), target)
            .expect_err("a non-finite centre target is refused");
        assert_reason(error, REASON_NON_FINITE_TARGET);
    }
}

// ---------------------------------------------------------------------------
// 5. Partial ellipses (elliptical arcs): centre grip only
// ---------------------------------------------------------------------------

#[test]
fn a_partial_ellipse_exposes_the_centre_grip_only() {
    // The same frame, but a stored parameter range that is not a full turn.
    let partial = ellipse(
        p3(1.0, 2.0, 3.0),
        p3(0.0, 0.0, 1.0),
        p3(5.0, 0.0, 0.0),
        0.5,
        0.25,
        1.5,
    );
    let grips = grips_for_entity(&ent(55, partial.clone()));
    assert_eq!(
        grips.len(),
        1,
        "a partial ellipse exposes the centre grip only"
    );
    assert_eq!(grips[0].id, grip_id(55, GripKind::Center, 0));
    assert_bits_eq(grips[0].position, p3(1.0, 2.0, 3.0), "the stored centre");

    // A stored range one ulp short of a full turn is still partial; the slack is
    // the same one the tessellator uses to call a sweep "full".
    let just_short = ellipse(
        p3(0.0, 0.0, 0.0),
        p3(0.0, 0.0, 1.0),
        p3(5.0, 0.0, 0.0),
        0.5,
        0.0,
        std::f64::consts::TAU - 1e-6,
    );
    assert_eq!(
        grips_for_entity(&ent(56, just_short)).len(),
        1,
        "a range that is not a full turn is a partial ellipse"
    );
    // Within the shared slack it is a full ellipse: same handle set as the curve
    // the renderer draws.
    let nearly_full = ellipse(
        p3(0.0, 0.0, 0.0),
        p3(0.0, 0.0, 1.0),
        p3(5.0, 0.0, 0.0),
        0.5,
        0.0,
        std::f64::consts::TAU - 1e-12,
    );
    assert_eq!(
        grips_for_entity(&ent(57, nearly_full)).len(),
        5,
        "a sweep within the shared 1e-9 slack is a full ellipse"
    );
    // A negative (clockwise) full turn is full too.
    let clockwise = ellipse(
        p3(0.0, 0.0, 0.0),
        p3(0.0, 0.0, 1.0),
        p3(5.0, 0.0, 0.0),
        0.5,
        0.0,
        -std::f64::consts::TAU,
    );
    assert_eq!(grips_for_entity(&ent(58, clockwise)).len(), 5);
}

#[test]
fn a_partial_ellipse_refuses_quadrant_drags_by_name() {
    let partial = ellipse(
        p3(1.0, 2.0, 3.0),
        p3(0.0, 0.0, 1.0),
        p3(5.0, 0.0, 0.0),
        0.5,
        0.25,
        1.5,
    );
    for index in 0..=3u8 {
        let error = apply_grip(
            &partial,
            &grip_id(55, GripKind::Quadrant, index),
            p3(9.0, 0.0, 0.0),
        )
        .expect_err("a partial ellipse exposes no quadrant handle");
        assert_reason(error, REASON_PARTIAL_ELLIPSE);
    }
    // The refusal is the handle's, so it wins over a target problem (the handle
    // identity is checked before the target's finiteness).
    let error = apply_grip(
        &partial,
        &grip_id(55, GripKind::Quadrant, 0),
        p3(f64::NAN, 0.0, 0.0),
    )
    .expect_err("the handle is missing regardless of the target");
    assert_reason(error, REASON_PARTIAL_ELLIPSE);
    // A wrong kind is still stale, not the partial refusal.
    let error = apply_grip(
        &partial,
        &grip_id(55, GripKind::Endpoint, 0),
        p3(9.0, 0.0, 0.0),
    )
    .expect_err("an ELLIPSE has no endpoint handle");
    assert_reason(error, REASON_STALE_HANDLE);
}

#[test]
fn a_partial_ellipse_centre_drag_is_rigid_and_keeps_the_parameter_range() {
    let partial = ellipse(
        p3(1.0, 2.0, 3.0),
        p3(0.0, 0.0, 1.0),
        p3(5.0, 0.0, 0.0),
        0.5,
        0.25,
        1.5,
    );
    let dragged = apply_grip(
        &partial,
        &grip_id(55, GripKind::Center, 0),
        p3(-1.0, 4.0, 3.0),
    )
    .expect("the centre grip of a partial ellipse is draggable");
    let (center, normal, major_axis, ratio, start, sweep) = parts(&dragged);
    assert_bits_eq(center, p3(-1.0, 4.0, 3.0), "the dragged centre");
    assert_bits_eq(normal, p3(0.0, 0.0, 1.0), "the normal is untouched");
    assert_bits_eq(major_axis, p3(5.0, 0.0, 0.0), "the major axis is untouched");
    assert_eq!(ratio.to_bits(), 0.5f64.to_bits(), "the ratio is untouched");
    assert_eq!(start.to_bits(), 0.25f64.to_bits(), "the start is untouched");
    assert_eq!(sweep.to_bits(), 1.5f64.to_bits(), "the sweep is untouched");
}

// ---------------------------------------------------------------------------
// 6. Hit-testing the ellipse handles
// ---------------------------------------------------------------------------

#[test]
fn grip_hit_finds_the_ellipse_centre_and_axis_points() {
    let entity: DbEntity = ent(51, flat());
    let hit = grip_hit([entity.clone()].iter(), p3(0.2, 0.1, 0.0), 1.0).expect("a hit");
    assert_eq!(hit.id, grip_id(51, GripKind::Center, 0));
    let hit = grip_hit([entity.clone()].iter(), p3(5.1, 0.0, 0.0), 1.0).expect("a hit");
    assert_eq!(hit.id, grip_id(51, GripKind::Quadrant, 0));
    let hit = grip_hit([entity.clone()].iter(), p3(0.0, 2.4, 0.0), 1.0).expect("a hit");
    assert_eq!(hit.id, grip_id(51, GripKind::Quadrant, 1));
    // The plan-view rule hits the axis point at another elevation.
    let elevated: DbEntity = ent(
        51,
        ellipse(
            p3(0.0, 0.0, 5.0),
            p3(0.0, 0.0, 1.0),
            p3(5.0, 0.0, 0.0),
            0.5,
            0.0,
            std::f64::consts::TAU,
        ),
    );
    assert!(grip_hit([elevated.clone()].iter(), p3(0.0, 2.5, 0.0), 1.0).is_none());
    let hit = grip_hit_planar([elevated].iter(), p3(0.0, 2.5, 0.0), 1.0).expect("a hit");
    assert_eq!(hit.id, grip_id(51, GripKind::Quadrant, 1));
    assert_bits_eq(
        hit.position,
        p3(0.0, 2.5, 5.0),
        "the stored 3D handle, never a projection",
    );
}

#[test]
fn an_exact_tie_between_the_ellipse_centre_and_a_quadrant_goes_to_the_centre() {
    // (2.5, 0) is 2.5 from the centre and 2.5 from quadrant 0 at (5, 0): the
    // centre is earlier in the list, so it wins — in both hit rules.
    let entity = ent(51, flat());
    let query = p3(2.5, 0.0, 0.0);
    let hit = grip_hit([entity.clone()].iter(), query, 3.0).expect("a hit");
    assert_eq!(
        hit.id,
        grip_id(51, GripKind::Center, 0),
        "centre wins the tie"
    );
    let hit = grip_hit_planar([entity].iter(), query, 3.0).expect("a hit");
    assert_eq!(
        hit.id,
        grip_id(51, GripKind::Center, 0),
        "centre wins the tie"
    );
}

#[test]
fn a_partial_ellipse_has_no_quadrant_hit() {
    let partial = ellipse(
        p3(0.0, 0.0, 0.0),
        p3(0.0, 0.0, 1.0),
        p3(5.0, 0.0, 0.0),
        0.5,
        0.0,
        1.5,
    );
    let entity = ent(55, partial);
    // The full ellipse's quadrant 0 point is not a handle on a partial ellipse.
    assert!(grip_hit([entity.clone()].iter(), p3(5.0, 0.0, 0.0), 1.0).is_none());
    assert!(grip_hit_planar([entity.clone()].iter(), p3(5.0, 0.0, 0.0), 1.0).is_none());
    let hit = grip_hit([entity].iter(), p3(0.1, 0.1, 0.0), 1.0).expect("the centre is still there");
    assert_eq!(hit.id, grip_id(55, GripKind::Center, 0));
}
