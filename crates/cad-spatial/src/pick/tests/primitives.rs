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
        uvs: Vec::new(),
        texture: None,
        pbr: None,
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

/// A triangle at z = 2 facing -z, so a ray travelling +z always sees its front.
fn body_triangle_mesh() -> Mesh {
    Mesh {
        vertices: vec![p(-1.0, -1.0, 2.0), p(0.0, 1.0, 2.0), p(1.0, -1.0, 2.0)],
        triangles: vec![[0, 1, 2]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
        uvs: Vec::new(),
        texture: None,
        pbr: None,
    }
}

#[test]
fn body_is_its_own_pick_family_and_its_mesh_is_hit() {
    use super::super::hit::geometry_family_name;
    let geometry = body(BodyKind::Solid, body_triangle_mesh(), Vec::new());
    assert_eq!(geometry_family_name(&geometry), "body");
    assert_ne!(geometry_family_name(&geometry), "mesh");

    let r = ray(p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
    let options = PickOptions::new(1e-6).unwrap();
    match hit_geometry(&r, &geometry, &Transform3::identity(), &options).unwrap() {
        PickOutcome::Hit(hit) => {
            assert!((hit.distance - 2.0).abs() < 1e-9, "the display face is hit");
            assert_eq!(hit.geometry_source, GeometrySource::DirectMesh);
            // The stored (approximate) display precision is carried through, not
            // upgraded to analytic.
            assert_eq!(
                hit.precision,
                Precision::Approximate {
                    error_bound: Some(0.01)
                }
            );
            // The display triangle carries no face map, so no `SubElementId` is
            // invented; the helper's absent-sources reason says why.
            assert!(hit.sub_element.is_none());
            assert_eq!(hit.sub_element_reason, Some(REASON_FACE_SOURCES_ABSENT));
        }
        other => panic!("expected a body face hit, got {other:?}"),
    }
}

#[test]
fn wire_body_with_only_edges_is_hit_testable() {
    // No tessellation at all: only one boundary polyline at z = 3, running
    // along +x. A body must still be pickable through its display edges.
    let edge = vec![p(1.0, -2.0, 3.0), p(1.0, 2.0, 3.0)];
    let geometry = body(
        BodyKind::Wire,
        Mesh {
            vertices: Vec::new(),
            triangles: Vec::new(),
            normals: Vec::new(),
            face_sources: Vec::new(),
            colors: Vec::new(),
            uvs: Vec::new(),
            texture: None,
            pbr: None,
        },
        vec![edge],
    );
    let r = ray(p(1.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
    let options = PickOptions::new(1e-6).unwrap();
    match hit_geometry(&r, &geometry, &Transform3::identity(), &options).unwrap() {
        PickOutcome::Hit(hit) => {
            assert!((hit.distance - 3.0).abs() < 1e-9, "the display edge is hit");
            assert!((hit.point.y).abs() < 1e-9);
            assert_eq!(hit.geometry_source, GeometrySource::DirectMesh);
            assert_eq!(
                hit.precision,
                Precision::Approximate {
                    error_bound: Some(0.01)
                }
            );
            assert!(hit.sub_element.is_none());
            assert_eq!(hit.sub_element_reason, None);
        }
        other => panic!("expected a wire-body edge hit, got {other:?}"),
    }
}

#[test]
fn body_with_an_empty_display_honestly_misses() {
    let geometry = body(
        BodyKind::Unknown,
        Mesh {
            vertices: Vec::new(),
            triangles: Vec::new(),
            normals: Vec::new(),
            face_sources: Vec::new(),
            colors: Vec::new(),
            uvs: Vec::new(),
            texture: None,
            pbr: None,
        },
        Vec::new(),
    );
    let r = ray(p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
    let options = PickOptions::new(1e-6).unwrap();
    assert_eq!(
        hit_geometry(&r, &geometry, &Transform3::identity(), &options).unwrap(),
        PickOutcome::Miss
    );
}
