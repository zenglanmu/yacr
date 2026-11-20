//! Pure contracts for the CPU tangent-frame generator (ADR-0039 D6).
//!
//! The domain stores no tangents, so the renderer derives them from positions +
//! UV0 at pack time. These tests pin the geometric facts without a GPU: an
//! axis-aligned quad yields the exact expected frame, and a degenerate UV
//! parameterisation yields **no** frame (a tangent is never invented).

use super::*;

/// One mesh's streams: positions, normals, UV0, triangle indices.
type QuadStreams = (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<[u32; 3]>);

/// Unit quad in the XY plane with `+Z` normals and a full `[0,1]²` UV square.
fn quad() -> QuadStreams {
    (
        vec![
            [-1.0, -1.0, 0.0],
            [1.0, -1.0, 0.0],
            [1.0, 1.0, 0.0],
            [-1.0, 1.0, 0.0],
        ],
        vec![[0.0, 0.0, 1.0]; 4],
        vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
        vec![[0, 1, 2], [0, 2, 3]],
    )
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

#[test]
fn an_axis_aligned_quad_yields_the_unit_x_tangent_with_positive_handedness() {
    let (vertices, normals, uvs, indices) = quad();
    let tangents = generate_tangents(&vertices, &normals, &uvs, &indices)
        .expect("a non-degenerate UV square has a tangent frame");
    assert_eq!(tangents.len(), vertices.len());
    for tangent in &tangents {
        assert!(close(tangent[0], 1.0), "tangent is +X, got {tangent:?}");
        assert!(close(tangent[1], 0.0));
        assert!(close(tangent[2], 0.0));
        assert_eq!(tangent[3], 1.0, "right-handed frame (w = +1)");
    }
}

#[test]
fn a_mirrored_uv_mapping_reports_the_opposite_handedness() {
    let (vertices, normals, mut uvs, indices) = quad();
    // Flip U: the surface parameterisation runs the other way, so the bitangent
    // (and therefore the handedness) flips while the geometry is unchanged.
    for uv in &mut uvs {
        uv[0] = 1.0 - uv[0];
    }
    let tangents =
        generate_tangents(&vertices, &normals, &uvs, &indices).expect("still a valid frame");
    for tangent in &tangents {
        assert!(close(tangent[0], -1.0), "tangent is -X, got {tangent:?}");
        assert_eq!(tangent[3], -1.0, "left-handed frame (w = -1)");
    }
}

#[test]
fn degenerate_uvs_yield_no_tangent_frame() {
    let (vertices, normals, _, indices) = quad();
    // All four UVs identical: every triangle has zero UV area.
    let collapsed = vec![[0.5, 0.5]; 4];
    assert!(
        generate_tangents(&vertices, &normals, &collapsed, &indices).is_none(),
        "no tangent may be invented for a zero-area UV triangle"
    );
    // A misaligned UV stream is refused the same way.
    assert!(generate_tangents(&vertices, &normals, &[[0.0, 0.0]; 2], &indices).is_none());
    // No triangles: nothing to derive a frame from.
    assert!(generate_tangents(&vertices, &normals, &[[0.0, 0.0]; 4], &[]).is_none());
}

#[test]
fn the_normal_component_is_removed_from_the_accumulated_tangent() {
    // A slanted surface (normal `(1, 0, 1)/√2`) with a UV gradient along world +X:
    // the returned tangent must be orthonormal to the normal, not just the raw
    // gradient.
    let vertices = vec![[-1.0, 0.0, 1.0], [1.0, 0.0, -1.0], [-1.0, 2.0, 1.0]];
    let normal = [
        std::f32::consts::FRAC_1_SQRT_2,
        0.0,
        std::f32::consts::FRAC_1_SQRT_2,
    ];
    let normals = vec![normal; 3];
    let uvs = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
    let tangents = generate_tangents(&vertices, &normals, &uvs, &[[0, 1, 2]])
        .expect("a non-degenerate UV triangle has a frame");
    for tangent in &tangents {
        let dot = tangent[0] * normal[0] + tangent[1] * normal[1] + tangent[2] * normal[2];
        assert!(
            dot.abs() < 1e-4,
            "tangent must be orthogonal to the normal: {dot}"
        );
        let length =
            (tangent[0] * tangent[0] + tangent[1] * tangent[1] + tangent[2] * tangent[2]).sqrt();
        assert!(
            close(length, 1.0),
            "tangent must be unit length, got {length}"
        );
    }
}
