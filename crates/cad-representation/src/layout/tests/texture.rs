//! UV0 and the base-colour texture key through the two rebuild paths
//! (transform + viewport clip), plus what the clippers must *not* invent.
//!
//! Authority: `docs/adr/0038-mesh-base-color-texture.md` D7 and invariant 9
//! (a transform carries UVs verbatim, a mirror does not flip U; clipping keeps
//! one UV per surviving vertex).

use super::*;

use crate::layout::{clip_mesh_to_polygon, PaperClip};

// ---- UV0 / base-colour texture through the rebuild paths (ADR-0038) -----

/// A canonical document texture key (`<32 lowercase hex>.png`).
const TEXTURE_KEY: &str = "4c2f2e0a5f3d9b7c1e8a6d4b2c0f9e71.png";

fn textured_triangle(corners: [Point3; 3], uvs: [[f64; 2]; 3]) -> Mesh {
    Mesh {
        vertices: corners.to_vec(),
        triangles: vec![[0, 1, 2]],
        normals: vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            };
            3
        ],
        face_sources: Vec::new(),
        colors: Vec::new(),
        uvs: uvs.to_vec(),
        texture: Some(TEXTURE_KEY.to_owned()),
    }
}

#[test]
fn transformed_textured_mesh_keeps_uvs_and_texture() {
    let mesh = textured_triangle(
        [p(0.0, 0.0), p(2.0, 0.0), p(0.0, 2.0)],
        [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
    );
    // A translation and a mirror: positions move/flip, UV0 does not (ADR-0038
    // invariant 9: UV belongs to the surface, a mirror does not flip U).
    let mut mirrored = Transform3::translation(p(10.0, 5.0)).matrix;
    mirrored[0][0] = -1.0;
    let transformed = crate::transform_mesh(&mesh, &Transform3 { matrix: mirrored });

    assert_eq!(transformed.vertices[0], p(10.0, 5.0));
    assert_eq!(transformed.vertices[1], p(8.0, 5.0));
    assert_eq!(transformed.uvs, mesh.uvs, "UV0 must pass through unchanged");
    assert_eq!(transformed.texture.as_deref(), Some(TEXTURE_KEY));
}

#[test]
fn clipped_textured_mesh_keeps_uvs_and_texture() {
    let mesh = textured_triangle(
        [p(-0.5, -0.5), p(0.5, -0.5), p(0.0, 0.5)],
        [[0.25, 0.25], [0.75, 0.25], [0.5, 0.75]],
    );
    let clipped = clip_mesh_to_rect(&mesh, [0.0, 0.0], [1.0, 1.0])
        .unwrap()
        .expect("the triangle is inside the window");
    assert_eq!(clipped.uvs, mesh.uvs, "a verbatim triangle keeps its UVs");
    assert_eq!(clipped.texture.as_deref(), Some(TEXTURE_KEY));
}

#[test]
fn clipped_textured_mesh_interpolates_uv_at_new_vertices() {
    // Triangle (-2,0),(2,0),(0,2) ∩ [-1,1]²: the x = -1 cut vertex sits halfway
    // along the (-2,0)->(2,0) edge, so its U must be halfway too.
    let mesh = textured_triangle(
        [p(-2.0, 0.0), p(2.0, 0.0), p(0.0, 2.0)],
        [[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]],
    );
    let clipped = clip_mesh_to_rect(&mesh, [0.0, 0.0], [1.0, 1.0])
        .unwrap()
        .expect("overlaps the window");
    assert_eq!(clipped.uvs.len(), clipped.vertices.len());
    assert_eq!(clipped.texture.as_deref(), Some(TEXTURE_KEY));
    let left = clipped
        .vertices
        .iter()
        .position(|v| (v.x + 1.0).abs() < 1e-9 && v.y.abs() < 1e-9)
        .expect("left crossing");
    let uv = clipped.uvs[left];
    assert!((uv[0] - 0.25).abs() < 1e-9, "interpolated u {uv:?}");
    assert!(uv[1].abs() < 1e-9, "interpolated v {uv:?}");
}

#[test]
fn clipped_textured_mesh_keeps_uvs_aligned_when_triangles_are_dropped() {
    // Two triangles with distinct UV ramps; the second lies fully outside and is
    // dropped. What survives must be the *first* triangle's own UVs, still 1:1
    // with the surviving vertices.
    let mesh = Mesh {
        vertices: vec![
            p(-0.5, -0.5),
            p(0.5, -0.5),
            p(0.0, 0.5),
            p(5.0, 5.0),
            p(6.0, 5.0),
            p(5.5, 6.0),
        ],
        triangles: vec![[0, 1, 2], [3, 4, 5]],
        normals: vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            };
            6
        ],
        face_sources: Vec::new(),
        colors: Vec::new(),
        uvs: vec![
            [0.0, 0.0],
            [1.0, 0.0],
            [0.5, 1.0],
            [9.0, 9.0],
            [9.5, 9.0],
            [9.25, 9.5],
        ],
        texture: Some(TEXTURE_KEY.to_owned()),
    };
    let clipped = clip_mesh_to_rect(&mesh, [0.0, 0.0], [1.0, 1.0])
        .unwrap()
        .expect("the first triangle survives");
    assert_eq!(
        clipped.triangles.len(),
        1,
        "the outside triangle is dropped"
    );
    assert_eq!(clipped.vertices.len(), 3);
    assert_eq!(clipped.uvs.len(), 3);
    // The surviving UVs are the source ones of vertices 0..3, in order.
    assert_eq!(clipped.uvs[0], [0.0, 0.0]);
    assert_eq!(clipped.uvs[1], [1.0, 0.0]);
    assert_eq!(clipped.uvs[2], [0.5, 1.0]);
    assert_eq!(clipped.texture.as_deref(), Some(TEXTURE_KEY));
}

#[test]
fn clipped_textured_mesh_to_a_polygon_keeps_uvs_and_texture() {
    let mesh = textured_triangle(
        [p(-2.0, 0.0), p(2.0, 0.0), p(0.0, 2.0)],
        [[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]],
    );
    let clip = PaperClip::rect([-1.0, -1.0], [1.0, 1.0]);
    let clipped = clip_mesh_to_polygon(&mesh, &clip)
        .unwrap()
        .expect("overlaps the clip");
    assert_eq!(clipped.uvs.len(), clipped.vertices.len());
    assert_eq!(clipped.texture.as_deref(), Some(TEXTURE_KEY));
}

#[test]
fn untextured_mesh_clip_carries_no_uvs_or_texture() {
    let mesh = triangle_mesh([p(-0.5, -0.5), p(0.5, -0.5), p(0.0, 0.5)]);
    let clipped = clip_mesh_to_rect(&mesh, [0.0, 0.0], [1.0, 1.0])
        .unwrap()
        .expect("inside");
    assert!(clipped.uvs.is_empty());
    assert!(clipped.texture.is_none());
}
