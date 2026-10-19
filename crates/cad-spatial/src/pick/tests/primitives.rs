//! Low-level segment and triangle intersection primitives.

use cad_domain::*;

use super::super::hit::segment_hit;
use super::super::*;
use super::common::*;

#[test]
fn segment_hit_and_miss_with_tolerance() {
    let r = ray(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0));
    // A segment crossing the ray (distance 0).
    let hit = segment_hit(&r, p(5.0, -1.0, 0.0), p(5.0, 1.0, 0.0), 1e-6);
    assert!(hit.is_some());
    // A segment running parallel to the ray, offset by 0.001: beyond a
    // 1e-6 tolerance it is a miss.
    let miss = segment_hit(&r, p(5.0, 0.001, -1.0), p(5.0, 0.001, 1.0), 1e-6);
    assert!(miss.is_none());
}

#[test]
fn tolerance_scales_the_segment_hit_window() {
    let r = ray(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0));
    let a = p(5.0, 0.25, 0.0);
    let b = p(5.0, 0.25, 1.0);
    // Distance 0.25: missed with a 0.1 tolerance, hit with 0.5.
    assert!(segment_hit(&r, a, b, 0.1).is_none());
    assert!(segment_hit(&r, a, b, 0.5).is_some());
}

#[test]
fn segment_behind_the_origin_is_a_miss() {
    let r = ray(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0));
    // The segment is behind the ray origin.
    assert!(segment_hit(&r, p(-5.0, -1.0, 0.0), p(-5.0, 1.0, 0.0), 10.0).is_none());
}

#[test]
fn mesh_triangle_hit_respects_back_face_policy() {
    // Triangle in the z=5 plane, counter-clockwise seen from -z, so its
    // outward normal points -z (toward a ray that travels +z).
    let v0 = p(-1.0, -1.0, 5.0);
    let v1 = p(0.0, 1.0, 5.0);
    let v2 = p(1.0, -1.0, 5.0);
    // A ray travelling +z from the origin sees the front face.
    let front = ray(p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
    assert!(ray_triangle(&front, v0, v1, v2, BackFacePolicy::Cull).is_some());
    // A ray travelling -z from beyond the plane sees the back face.
    let back = ray(p(0.0, 0.0, 10.0), p(0.0, 0.0, -1.0));
    assert!(ray_triangle(&back, v0, v1, v2, BackFacePolicy::Cull).is_none());
    assert!(ray_triangle(&back, v0, v1, v2, BackFacePolicy::DoubleSided).is_some());
}

#[test]
fn mesh_pick_returns_the_closest_triangle() {
    let near = p(-1.0, -1.0, 2.0);
    let nb = p(0.0, 1.0, 2.0);
    let nc = p(1.0, -1.0, 2.0);
    let far = p(-1.0, -1.0, 5.0);
    let fb = p(0.0, 1.0, 5.0);
    let fc = p(1.0, -1.0, 5.0);
    let mesh = Mesh {
        vertices: vec![near, nb, nc, far, fb, fc],
        triangles: vec![[0, 1, 2], [3, 4, 5]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
    };
    let r = ray(p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
    let options = PickOptions::new(1e-6).unwrap();
    match hit_geometry(
        &r,
        &SemanticGeometry::Mesh(mesh),
        &Transform3::identity(),
        &options,
    )
    .unwrap()
    {
        PickOutcome::Hit(hit) => {
            assert!((hit.distance - 2.0).abs() < 1e-9, "closest triangle wins");
            assert!((hit.point.z - 2.0).abs() < 1e-9);
        }
        other => panic!("expected a hit, got {other:?}"),
    }
}
