//! Grip model contract tests for `SPLINE` control-point handles.
//!
//! Pins the surface this lane added to `crates/cad-app/src/grips/`: one
//! [`GripKind::ControlPoint`] per **stored** NURBS control point, the
//! "definition point only" drag rule (`degree`/`knots`/`weights` bit-identical,
//! no re-fit), the `u8` addressability cap, the "no grips for an inconsistent
//! definition" rule, the refusals and the hit-test/tie-break.
//!
//! A control point is **not** a point on the drawn curve (only a clamped spline's
//! first/last one is), so "the grip position is the stored point" is asserted
//! against the stored `Point3`, never against an evaluated curve point — nothing
//! in the grip model evaluates the NURBS.
//!
//! Documented choices pinned here (ours, **not AutoCAD claims**): a
//! control-point drag never recomputes a knot or a weight, so it stores a
//! **different** NURBS; the curve therefore no longer passes through the points it
//! used to.

use super::grips::{assert_bits_eq, assert_reason, ent, grip_id, p3, reason};

use super::*;

use crate::grips::{
    apply_grip, grip_hit, grip_hit_planar, grips_for_entity, Grip, GripKind,
    REASON_NON_FINITE_TARGET, REASON_STALE_HANDLE, REASON_ZERO_DISPLACEMENT,
};
use cad_db::DbEntity;

/// A simple 4-point cubic in `z = 0`-ish space: degree 3 with 4 control points
/// needs `4 + 3 + 1 = 8` knots, which is exactly the clamped knot vector below.
/// The control points are deliberately not collinear and not at one elevation.
fn cubic() -> SemanticGeometry {
    SemanticGeometry::Spline {
        degree: 3,
        knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
        control_points: vec![
            p3(0.0, 0.0, 0.0),
            p3(1.0, 2.0, 0.5),
            p3(3.0, 2.0, -1.5),
            p3(4.0, 0.0, 0.25),
        ],
        weights: vec![1.0, 1.0, 1.0, 1.0],
    }
}

/// A degree-1 spline through two control points: `(degree 1) + (2 control
/// points) + 1 = 4` knots. Used where the fixture must be trivially short.
fn two_point() -> SemanticGeometry {
    SemanticGeometry::Spline {
        degree: 1,
        knots: vec![0.0, 0.0, 1.0, 1.0],
        control_points: vec![p3(0.0, 0.0, 0.0), p3(2.0, 0.0, 0.0)],
        weights: vec![1.0, 1.0],
    }
}

/// The stored `(degree, knots, control_points, weights)` of a spline result.
fn parts(geometry: &SemanticGeometry) -> (u32, Vec<f64>, Vec<Point3>, Vec<f64>) {
    match geometry {
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => (
            *degree,
            knots.clone(),
            control_points.clone(),
            weights.clone(),
        ),
        other => panic!("expected a SPLINE, got {other:?}"),
    }
}

fn points_of(geometry: &SemanticGeometry) -> Vec<Point3> {
    parts(geometry).2
}

/// Compare two `f64` vectors bit for bit (order included).
fn assert_bits_vec(actual: &[f64], expected: &[f64], what: &str) {
    assert_eq!(actual.len(), expected.len(), "{what}: length");
    for (index, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
        assert_eq!(
            a.to_bits(),
            e.to_bits(),
            "{what}: entry {index} is {a} but expected {e}"
        );
    }
}

// ---------------------------------------------------------------------------
// 1. Handle list: one ControlPoint per stored control point, stored order
// ---------------------------------------------------------------------------

#[test]
fn a_spline_exposes_one_control_point_grip_per_stored_control_point() {
    let geometry = cubic();
    let stored = points_of(&geometry);
    let grips = grips_for_entity(&ent(61, geometry.clone()));
    assert_eq!(
        grips.len(),
        stored.len(),
        "exactly one ControlPoint grip per stored control point"
    );
    for (index, point) in stored.iter().enumerate() {
        assert_eq!(
            grips[index],
            Grip {
                id: grip_id(61, GripKind::ControlPoint, index as u8),
                position: *point,
            },
            "control point {index} is the stored point, in stored order"
        );
        // Verbatim, never an evaluated curve point: these `f64` bits are the ones
        // the NURBS definition stores.
        assert_bits_eq(
            grips[index].position,
            *point,
            &format!("control point {index} position"),
        );
    }
    assert_eq!(
        grips[0].id.kind,
        GripKind::ControlPoint,
        "the handle family is ControlPoint, never reused as Vertex"
    );
}

#[test]
fn a_spline_with_more_than_256_control_points_offers_the_first_256_only() {
    // `GripId::index` is a `u8`, so control points 256.. are not addressable and
    // are therefore **not offered** (never wrapped onto a lower index).
    let control_points: Vec<Point3> = (0..300).map(|i| p3(f64::from(i), 0.0, 0.0)).collect();
    let knots: Vec<f64> = (0..302).map(f64::from).collect();
    let geometry = SemanticGeometry::Spline {
        degree: 1,
        knots,
        control_points: control_points.clone(),
        weights: vec![1.0; 300],
    };
    let grips = grips_for_entity(&ent(62, geometry.clone()));
    assert_eq!(
        grips.len(),
        256,
        "indices 0..=255 are the addressable range"
    );
    let indices: Vec<u8> = grips.iter().map(|grip| grip.id.index).collect();
    assert_eq!(
        indices,
        (0..=255u8).collect::<Vec<u8>>(),
        "the offered indices are exactly 0..=255, once each"
    );
    assert_bits_eq(
        grips[255].position,
        control_points[255],
        "grip 255 is the 256th stored control point",
    );
    // 255 is still a real drag target; 256 cannot even be expressed in the id.
    let dragged = apply_grip(
        &geometry,
        &grip_id(62, GripKind::ControlPoint, 255),
        p3(255.0, -7.0, 0.0),
    )
    .expect("control point 255 is addressable");
    assert_eq!(points_of(&dragged)[255], p3(255.0, -7.0, 0.0));
    assert_eq!(
        points_of(&dragged).len(),
        300,
        "the definition keeps every stored control point"
    );
}

// ---------------------------------------------------------------------------
// 2. Dragging one control point: a different NURBS, same parameterisation
// ---------------------------------------------------------------------------

#[test]
fn a_control_point_drag_moves_exactly_one_point_and_keeps_the_nurbs_definition() {
    let geometry = cubic();
    let (degree, knots, before, weights) = parts(&geometry);
    let target = p3(-2.5, 9.75, 4.0);
    let dragged = apply_grip(&geometry, &grip_id(61, GripKind::ControlPoint, 2), target)
        .expect("a spline control-point drag is supported");
    let (new_degree, new_knots, after, new_weights) = parts(&dragged);

    assert_eq!(new_degree, degree, "the degree is untouched");
    assert_bits_vec(&new_knots, &knots, "the knot vector");
    assert_bits_vec(&new_weights, &weights, "the weight vector");
    assert_eq!(
        after.len(),
        before.len(),
        "no control point is added or dropped"
    );
    // The target is stored in all three coordinates: a control point is a free 3D
    // definition point, so nothing is projected onto a plane or onto z = 0.
    assert_bits_eq(after[2], target, "the dragged control point");
    for index in [0usize, 1, 3] {
        assert_bits_eq(
            after[index],
            before[index],
            &format!("control point {index} must stay bit-identical"),
        );
    }
    // The definition really changed: the curve is a different NURBS that keeps the
    // stored parameterisation (no re-fit, no knot recomputation).
    assert_ne!(after[2], before[2], "the fixture moves the handle");
}

// ---------------------------------------------------------------------------
// 3. Refusals (handle identity first, then the target, then the rule)
// ---------------------------------------------------------------------------

#[test]
fn a_control_point_index_outside_the_stored_points_is_stale() {
    let geometry = cubic();
    for index in [4u8, 5, u8::MAX] {
        let error = apply_grip(
            &geometry,
            &grip_id(61, GripKind::ControlPoint, index),
            p3(9.0, 9.0, 0.0),
        )
        .expect_err("a control-point index past the stored points is stale");
        assert_reason(error, REASON_STALE_HANDLE);
    }
}

#[test]
fn a_non_control_point_grip_on_a_spline_is_stale() {
    let geometry = cubic();
    for (kind, index) in [
        (GripKind::Endpoint, 0u8),
        (GripKind::Endpoint, 1),
        (GripKind::Midpoint, 2),
        (GripKind::Center, 0),
        (GripKind::Quadrant, 0),
        (GripKind::Vertex, 0),
        (GripKind::Point, 0),
    ] {
        let error = apply_grip(&geometry, &grip_id(61, kind, index), p3(1.0, 1.0, 0.0))
            .expect_err("a SPLINE exposes ControlPoint grips only");
        assert_reason(error, REASON_STALE_HANDLE);
    }
    // The handle identity is validated before the target, so a wrong kind never
    // reaches the finiteness check (and never looks like a zero displacement).
    let error = apply_grip(
        &geometry,
        &grip_id(61, GripKind::Vertex, 0),
        p3(f64::NAN, 0.0, 0.0),
    )
    .expect_err("a wrong kind is stale regardless of the target");
    assert_reason(error, REASON_STALE_HANDLE);
}

#[test]
fn a_control_point_drag_onto_its_own_point_writes_nothing() {
    let geometry = cubic();
    // Exactly at the handle: nothing to write, never an empty update.
    let error = apply_grip(
        &geometry,
        &grip_id(61, GripKind::ControlPoint, 2),
        p3(3.0, 2.0, -1.5),
    )
    .expect_err("a drag onto the control point is nothing to write");
    assert_reason(error, REASON_ZERO_DISPLACEMENT);
    // Unlike a polyline vertex (which keeps its own `z`), a `z`-only target here is
    // a real edit: a control-point drag stores all three coordinates verbatim.
    let dragged = apply_grip(
        &geometry,
        &grip_id(61, GripKind::ControlPoint, 2),
        p3(3.0, 2.0, 7.5),
    )
    .expect("a z-only control-point drag is a real edit");
    assert_bits_eq(points_of(&dragged)[2], p3(3.0, 2.0, 7.5), "the new point");
    // A sub-epsilon-sized but real edit is a real edit too: no tolerance is applied.
    let dragged = apply_grip(
        &geometry,
        &grip_id(61, GripKind::ControlPoint, 2),
        p3(3.0, 2.0, -1.5 + f64::EPSILON),
    )
    .expect("a tiny drag is a real edit");
    assert_ne!(points_of(&dragged)[2], p3(3.0, 2.0, -1.5));
}

#[test]
fn a_control_point_drag_refuses_a_non_finite_target() {
    let geometry = cubic();
    for target in [
        p3(f64::NAN, 0.0, 0.0),
        p3(0.0, f64::INFINITY, 0.0),
        p3(0.0, 0.0, f64::NEG_INFINITY),
    ] {
        let error = apply_grip(&geometry, &grip_id(61, GripKind::ControlPoint, 0), target)
            .expect_err("a non-finite control point cannot be stored");
        assert_reason(error, REASON_NON_FINITE_TARGET);
    }
}

// ---------------------------------------------------------------------------
// 4. An internally inconsistent definition exposes no grips (never a guess)
// ---------------------------------------------------------------------------

#[test]
fn a_spline_with_a_weights_length_mismatch_exposes_no_grips() {
    // `weights` is one entry short. `cad-db` validation is the authority on shape
    // validity; this model only refuses to index into a definition it cannot read
    // (padding the vector would silently rewrite the curve).
    let geometry = SemanticGeometry::Spline {
        degree: 3,
        knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
        control_points: vec![
            p3(0.0, 0.0, 0.0),
            p3(1.0, 2.0, 0.0),
            p3(3.0, 2.0, 0.0),
            p3(4.0, 0.0, 0.0),
        ],
        weights: vec![1.0, 1.0, 1.0],
    };
    let entity = ent(63, geometry.clone());
    assert!(
        grips_for_entity(&entity).is_empty(),
        "a definition with a weight mismatch exposes no handle"
    );
    // No marker means no hit, even exactly at a stored control point.
    assert!(grip_hit([entity.clone()].iter(), p3(0.0, 0.0, 0.0), 1.0).is_none());
    assert!(grip_hit_planar([entity].iter(), p3(0.0, 0.0, 0.0), 1.0).is_none());
    let error = apply_grip(
        &geometry,
        &grip_id(63, GripKind::ControlPoint, 0),
        p3(5.0, 5.0, 0.0),
    )
    .expect_err("an inconsistent spline has no control-point handle");
    assert_reason(error.clone(), REASON_STALE_HANDLE);
    assert!(
        reason(error).contains("AcDbSpline"),
        "the refusal names the DXF class"
    );
}

#[test]
fn a_spline_with_a_knots_length_mismatch_exposes_no_grips() {
    // One knot short (`7` instead of `4 + 3 + 1 = 8`), then one knot too many.
    for knots in [
        vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        vec![0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0],
    ] {
        let geometry = SemanticGeometry::Spline {
            degree: 3,
            knots,
            control_points: vec![
                p3(0.0, 0.0, 0.0),
                p3(1.0, 2.0, 0.0),
                p3(3.0, 2.0, 0.0),
                p3(4.0, 0.0, 0.0),
            ],
            weights: vec![1.0; 4],
        };
        assert!(
            grips_for_entity(&ent(64, geometry.clone())).is_empty(),
            "a definition with a knot-count mismatch exposes no handle"
        );
        let error = apply_grip(
            &geometry,
            &grip_id(64, GripKind::ControlPoint, 1),
            p3(9.0, 9.0, 0.0),
        )
        .expect_err("an inconsistent spline has no control-point handle");
        assert_reason(error, REASON_STALE_HANDLE);
    }
    // The consistent control: the same shape with the right knot count does expose
    // handles, so the mismatch check is what refused above.
    assert_eq!(grips_for_entity(&ent(65, cubic())).len(), 4);
}

// ---------------------------------------------------------------------------
// 5. Hit-testing: a stored control point is hit, and ties go to the lower index
// ---------------------------------------------------------------------------

#[test]
fn grip_hit_finds_a_spline_control_point_and_misses_outside_the_tolerance() {
    let entity: DbEntity = ent(66, cubic());
    let hit = grip_hit([entity.clone()].iter(), p3(2.9, 2.1, -1.5), 1.0).expect("a hit");
    assert_eq!(hit.id, grip_id(66, GripKind::ControlPoint, 2));
    assert_bits_eq(
        hit.position,
        p3(3.0, 2.0, -1.5),
        "the hit position is the stored control point",
    );
    // A query exactly at a control point is that control point's hit.
    let hit = grip_hit([entity.clone()].iter(), p3(1.0, 2.0, 0.5), 1.0).expect("a hit");
    assert_eq!(hit.id, grip_id(66, GripKind::ControlPoint, 1));
    // Outside every handle's tolerance there is nothing to grab.
    assert!(grip_hit([entity.clone()].iter(), p3(20.0, 20.0, 0.0), 1.0).is_none());
    assert!(grip_hit([entity].iter(), p3(2.9, 2.1, -1.5), 0.05).is_none());
}

#[test]
fn an_exact_control_point_tie_goes_to_the_lower_stored_index() {
    // (1, 0) is 1 away from control point 0 (0, 0) and control point 1 (2, 0): the
    // first candidate in list order wins, i.e. the lower stored index — in both
    // hit rules.
    let entity: DbEntity = ent(67, two_point());
    let query = p3(1.0, 0.0, 0.0);
    let hit = grip_hit([entity.clone()].iter(), query, 5.0).expect("a hit");
    assert_eq!(
        hit.id,
        grip_id(67, GripKind::ControlPoint, 0),
        "control point 0 wins the tie"
    );
    let hit = grip_hit_planar([entity].iter(), query, 5.0).expect("a hit");
    assert_eq!(
        hit.id,
        grip_id(67, GripKind::ControlPoint, 0),
        "control point 0 wins the tie"
    );
}

#[test]
fn a_spline_control_point_is_hit_in_the_plan_view_at_another_elevation() {
    // The spline lives at z = 5; the plan-view pointer resolves onto z = 0, so only
    // the plan-view rule can arm a control point (the target the host commits keeps
    // the handle's own z).
    let geometry = SemanticGeometry::Spline {
        degree: 1,
        knots: vec![0.0, 0.0, 1.0, 2.0, 3.0, 3.0],
        control_points: vec![
            p3(0.0, 0.0, 5.0),
            p3(4.0, 0.0, 5.0),
            p3(8.0, 0.0, 5.0),
            p3(12.0, 0.0, 5.0),
        ],
        weights: vec![1.0; 4],
    };
    let entity: DbEntity = ent(68, geometry);
    let plan = p3(4.2, 0.1, 0.0);
    assert!(
        grip_hit([entity.clone()].iter(), plan, 1.0).is_none(),
        "the 3D rule measures the elevation difference"
    );
    let hit = grip_hit_planar([entity].iter(), plan, 1.0).expect("the plan-view rule hits");
    assert_eq!(hit.id, grip_id(68, GripKind::ControlPoint, 1));
    assert_bits_eq(
        hit.position,
        p3(4.0, 0.0, 5.0),
        "the position stays the stored 3D control point, never a projection",
    );
}
