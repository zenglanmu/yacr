//! Grip model contract tests for `POLYLINE` vertex handles (LWPOLYLINE and the
//! other polyline classes the importer maps onto the same semantic variant).
//!
//! Split from `tests::grips` (LINE/POINT, hit-testing and the refusal list) and
//! `tests::grips_curves` (CIRCLE/ARC) so every file stays small; the shared
//! fixtures (`p3`, `ent`, `grip_id`, `assert_reason`, `assert_bits_eq`) are
//! imported from `tests::grips`.
//!
//! The model is pure, so "nothing else moved" is a **bit** comparison: the other
//! vertices and the whole bulge vector must come back unchanged, not merely
//! close.

use super::*;

use super::grips::{assert_bits_eq, assert_reason, ent, grip_id, p3};
use crate::grips::{
    apply_grip, grip_hit, grip_hit_planar, grips_for_entity, Grip, GripKind, REASON_STALE_HANDLE,
    REASON_ZERO_DISPLACEMENT,
};
use cad_db::DbEntity;

fn polyline(points: Vec<Point3>, bulges: Vec<f64>, closed: bool) -> SemanticGeometry {
    SemanticGeometry::Polyline {
        points,
        bulges,
        closed,
    }
}

/// A four-vertex open polyline with non-zero, non-round bulges — including a
/// negative one — so "the bulges are untouched" is a real assertion.
fn fixture() -> SemanticGeometry {
    polyline(
        vec![
            p3(0.0, 0.0, 1.0),
            p3(10.0, 0.0, 1.0),
            p3(10.0, 7.5, 1.0),
            p3(-2.5, 7.5, 1.0),
        ],
        vec![0.0, 0.375, -0.125, 0.0],
        false,
    )
}

fn points_of(geometry: &SemanticGeometry) -> Vec<Point3> {
    match geometry {
        SemanticGeometry::Polyline { points, .. } => points.clone(),
        other => panic!("expected a POLYLINE, got {other:?}"),
    }
}

fn bulges_of(geometry: &SemanticGeometry) -> Vec<f64> {
    match geometry {
        SemanticGeometry::Polyline { bulges, .. } => bulges.clone(),
        other => panic!("expected a POLYLINE, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 1. Which handles exist: one Vertex per stored vertex, in stored order
// ---------------------------------------------------------------------------

#[test]
fn polyline_exposes_one_vertex_grip_per_stored_vertex() {
    let geometry = fixture();
    let entity = ent(41, geometry.clone());
    let grips = grips_for_entity(&entity);
    let points = points_of(&geometry);
    assert_eq!(
        grips.len(),
        points.len(),
        "a POLYLINE has exactly one Vertex grip per stored vertex"
    );
    for (index, point) in points.iter().enumerate() {
        assert_eq!(
            grips[index],
            Grip {
                id: grip_id(41, GripKind::Vertex, index as u8),
                position: *point,
            },
            "vertex {index} is the stored point, in stored order"
        );
        assert_bits_eq(
            grips[index].position,
            *point,
            &format!("vertex {index} position"),
        );
    }
}

#[test]
fn a_closed_polyline_exposes_its_vertices_in_the_same_order() {
    // `closed` only adds the wrap-around segment; it must not reorder, duplicate
    // or offset the vertex handles.
    let geometry = polyline(
        vec![p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 0.0), p3(4.0, 4.0, 0.0)],
        vec![0.0, 0.0, 0.0],
        true,
    );
    let grips = grips_for_entity(&ent(42, geometry.clone()));
    assert_eq!(grips.len(), 3);
    for (index, point) in points_of(&geometry).iter().enumerate() {
        assert_eq!(grips[index].id, grip_id(42, GripKind::Vertex, index as u8));
        assert_bits_eq(grips[index].position, *point, "stored vertex");
    }
}

#[test]
fn a_polyline_without_bulges_still_exposes_vertex_grips() {
    // The importer emits an empty bulge vector for a 3D polyline (and for the
    // legacy `AcDbPolyline`); `SemanticGeometry::Polyline` is the one variant for
    // all polyline classes, so these vertices are grippable too and the empty
    // bulge vector is preserved as-is (documented in the module doc).
    let geometry = polyline(
        vec![p3(0.0, 0.0, 0.0), p3(1.0, 1.0, 2.0)],
        Vec::new(),
        false,
    );
    let grips = grips_for_entity(&ent(43, geometry.clone()));
    assert_eq!(grips.len(), 2);
    let dragged = apply_grip(
        &geometry,
        &grip_id(43, GripKind::Vertex, 1),
        p3(3.0, 4.0, 0.0),
    )
    .expect("a vertex of a bulge-less polyline is draggable");
    assert_eq!(points_of(&dragged)[1], p3(3.0, 4.0, 2.0));
    assert!(
        bulges_of(&dragged).is_empty(),
        "an empty bulge vector stays empty"
    );
}

#[test]
fn a_polyline_longer_than_256_vertices_offers_the_first_256_only() {
    // `GripId::index` is a `u8`: vertices 256.. are not addressable, so they are
    // **not offered** (never wrapped onto a different vertex index).
    let points: Vec<Point3> = (0..300).map(|i| p3(f64::from(i), 0.0, 0.0)).collect();
    let geometry = polyline(points, vec![0.0; 300], false);
    let grips = grips_for_entity(&ent(44, geometry.clone()));
    assert_eq!(
        grips.len(),
        256,
        "indices 0..=255 are the addressable range"
    );
    assert_eq!(grips[255].id, grip_id(44, GripKind::Vertex, 255));
    // The cap is the `u8` range itself: the 256 offered grips are exactly the
    // addressable vertex indices, never a wrapped repeat of an earlier vertex.
    let indices: Vec<u8> = grips.iter().map(|grip| grip.id.index).collect();
    assert_eq!(
        indices,
        (0..=255u8).collect::<Vec<u8>>(),
        "the offered indices are exactly 0..=255, once each"
    );
    assert_eq!(
        grips[255].position,
        points_last_of(&geometry),
        "grip 255 is the 256th stored vertex"
    );
    // The highest addressable vertex is still a real drag target.
    let dragged = apply_grip(
        &geometry,
        &grip_id(44, GripKind::Vertex, 255),
        p3(255.0, -1.0, 0.0),
    )
    .expect("vertex 255 is addressable");
    assert_eq!(points_of(&dragged)[255], p3(255.0, -1.0, 0.0));
}

// ---------------------------------------------------------------------------
// 2. Dragging one vertex
// ---------------------------------------------------------------------------

#[test]
fn a_vertex_drag_moves_exactly_one_vertex_and_keeps_every_other_bit() {
    let geometry = fixture();
    let before = points_of(&geometry);
    let before_bulges = bulges_of(&geometry);
    // The target is lifted well above the polyline's z: the drag keeps the
    // vertex's own z (plan-view rule), so the stored elevation never changes.
    let target = p3(9.25, -3.5, 42.0);
    let dragged = apply_grip(&geometry, &grip_id(41, GripKind::Vertex, 1), target)
        .expect("a polyline vertex drag is supported");

    let after = points_of(&dragged);
    assert_eq!(after.len(), before.len(), "no vertex is added or dropped");
    assert_bits_eq(
        after[1],
        p3(target.x, target.y, before[1].z),
        "the dragged vertex takes x/y and keeps its own z",
    );
    assert!(
        after[1].z.to_bits() == before[1].z.to_bits(),
        "the elevation is bit-identical"
    );
    for index in [0usize, 2, 3] {
        assert_bits_eq(
            after[index],
            before[index],
            &format!("vertex {index} must stay bit-identical"),
        );
    }
    assert_eq!(
        bulges_of(&dragged),
        before_bulges,
        "every stored bulge is copied verbatim"
    );
    match dragged {
        SemanticGeometry::Polyline { closed, .. } => {
            assert!(!closed, "the closed flag is untouched");
        }
        other => panic!("expected a POLYLINE, got {other:?}"),
    }
}

#[test]
fn a_vertex_drag_never_recomputes_a_bulge() {
    // The vertex sits between two arc segments (bulges 0.5 entering, -0.25
    // leaving). Neither is recomputed, cleared or normalized for the moved
    // vertex: the same `f64` bits come back.
    let geometry = polyline(
        vec![p3(0.0, 0.0, 0.0), p3(5.0, 0.0, 0.0), p3(5.0, 5.0, 0.0)],
        vec![0.5, -0.25, 0.0],
        false,
    );
    let dragged = apply_grip(
        &geometry,
        &grip_id(45, GripKind::Vertex, 1),
        p3(6.5, -2.0, 0.0),
    )
    .expect("the vertex drag is supported");
    let bulges = bulges_of(&dragged);
    assert_eq!(bulges.len(), 3);
    for (index, expected) in [0.5f64, -0.25, 0.0].into_iter().enumerate() {
        assert_eq!(
            bulges[index].to_bits(),
            expected.to_bits(),
            "bulge {index} is bit-identical (never recomputed)"
        );
    }
}

#[test]
fn a_closed_polyline_vertex_drag_keeps_it_closed() {
    let geometry = polyline(
        vec![p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 0.0), p3(4.0, 4.0, 0.0)],
        vec![0.0, 0.125, 0.0],
        true,
    );
    let dragged = apply_grip(
        &geometry,
        &grip_id(46, GripKind::Vertex, 2),
        p3(6.0, 6.0, 0.0),
    )
    .expect("a closed polyline vertex drag is supported");
    match dragged {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => {
            assert!(closed, "the closed flag is untouched");
            assert_eq!(points[2], p3(6.0, 6.0, 0.0));
            assert_eq!(bulges, vec![0.0, 0.125, 0.0]);
        }
        other => panic!("expected a POLYLINE, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 3. Refusals
// ---------------------------------------------------------------------------

#[test]
fn a_vertex_index_outside_the_stored_vertices_is_stale() {
    let geometry = polyline(
        vec![p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)],
        vec![0.0, 0.0],
        false,
    );
    for index in [2u8, 3, u8::MAX] {
        let error = apply_grip(
            &geometry,
            &grip_id(47, GripKind::Vertex, index),
            p3(9.0, 9.0, 0.0),
        )
        .expect_err("a vertex index past the stored vertices is stale");
        assert_reason(error, REASON_STALE_HANDLE);
    }
}

#[test]
fn a_non_vertex_grip_on_a_polyline_is_stale() {
    let geometry = fixture();
    for (kind, index) in [
        (GripKind::Endpoint, 0u8),
        (GripKind::Endpoint, 1),
        (GripKind::Midpoint, 2),
        (GripKind::Center, 0),
        (GripKind::Quadrant, 0),
        (GripKind::Point, 0),
    ] {
        let error = apply_grip(&geometry, &grip_id(41, kind, index), p3(1.0, 1.0, 0.0))
            .expect_err("a POLYLINE exposes Vertex grips only");
        assert_reason(error, REASON_STALE_HANDLE);
    }
}

#[test]
fn a_vertex_drag_with_no_geometric_effect_is_refused() {
    let geometry = fixture();
    // Exactly at the handle.
    let error = apply_grip(
        &geometry,
        &grip_id(41, GripKind::Vertex, 2),
        p3(10.0, 7.5, 1.0),
    )
    .expect_err("a drag onto the vertex is nothing to write");
    assert_reason(error, REASON_ZERO_DISPLACEMENT);

    // Same x/y, different z: the drag keeps the stored z, so the vertex would not
    // change at all — equally a zero displacement, never an empty transaction.
    let error = apply_grip(
        &geometry,
        &grip_id(41, GripKind::Vertex, 2),
        p3(10.0, 7.5, 99.0),
    )
    .expect_err("a z-only target writes nothing");
    assert_reason(error, REASON_ZERO_DISPLACEMENT);
}

// ---------------------------------------------------------------------------
// 4. Hit-testing the vertex handles
// ---------------------------------------------------------------------------

#[test]
fn grip_hit_finds_the_nearest_vertex() {
    let entity = ent(41, fixture());
    let hit = grip_hit([entity.clone()].iter(), p3(9.8, 0.2, 1.0), 1.0).expect("a hit");
    assert_eq!(hit.id, grip_id(41, GripKind::Vertex, 1));
    let hit = grip_hit([entity].iter(), p3(0.2, -0.2, 1.0), 1.0).expect("a hit");
    assert_eq!(hit.id, grip_id(41, GripKind::Vertex, 0));
}

#[test]
fn an_exact_vertex_tie_goes_to_the_lower_stored_index() {
    // (1, 0) is 1 away from both vertex 0 (0, 0) and vertex 1 (2, 0): the first
    // candidate in list order wins, i.e. the lower stored vertex index — in both
    // hit rules.
    let entity = ent(
        48,
        polyline(
            vec![p3(0.0, 0.0, 0.0), p3(2.0, 0.0, 0.0)],
            vec![0.0, 0.0],
            false,
        ),
    );
    let query = p3(1.0, 0.0, 0.0);
    let hit = grip_hit([entity.clone()].iter(), query, 5.0).expect("a hit");
    assert_eq!(hit.id, grip_id(48, GripKind::Vertex, 0), "vertex 0 wins");
    let hit = grip_hit_planar([entity].iter(), query, 5.0).expect("a hit");
    assert_eq!(hit.id, grip_id(48, GripKind::Vertex, 0), "vertex 0 wins");
}

#[test]
fn a_vertex_is_hit_in_the_plan_view_at_another_elevation() {
    // The polyline lives at z = 5; the plan-view pointer resolves onto z = 0.
    let entity: DbEntity = ent(41, fixture_at(5.0));
    let plan = p3(10.2, 0.1, 0.0);
    assert!(
        grip_hit([entity.clone()].iter(), plan, 1.0).is_none(),
        "the 3D rule measures the elevation difference"
    );
    let hit = grip_hit_planar([entity].iter(), plan, 1.0).expect("the plan-view rule hits");
    assert_eq!(hit.id, grip_id(41, GripKind::Vertex, 1));
    assert_bits_eq(
        hit.position,
        p3(10.0, 0.0, 5.0),
        "the position stays the stored 3D vertex, never a projection",
    );
}

fn fixture_at(z: f64) -> SemanticGeometry {
    polyline(
        vec![p3(0.0, 0.0, z), p3(10.0, 0.0, z), p3(10.0, 7.5, z)],
        vec![0.0, 0.0, 0.0],
        false,
    )
}

/// The 256th stored vertex (index 255) of a geometry, for the `u8` cap test.
fn points_last_of(geometry: &SemanticGeometry) -> Point3 {
    points_of(geometry)[255]
}
