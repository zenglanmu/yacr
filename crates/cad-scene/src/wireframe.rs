//! Wireframe overlay sources for a [`RenderBatch`](crate::RenderBatch).
//!
//! Two independent producers feed `RenderBatch::edges`, both as flat lists of
//! **independent endpoint pairs** (two consecutive positions = one segment):
//!
//! - [`mesh_boundary_edge_positions`]: the surface **boundary** of a plain mesh,
//!   derived from triangle topology by the odd-use rule. Tessellation diagonals
//!   cancel because the two triangles of one face use them twice.
//! - [`stored_edge_positions`]: a body's own stored display edges, copied
//!   verbatim (relative to the batch origin).
//!
//! [`triangle_edge_positions`] is the legacy "one segment per unique triangle
//! edge" overlay; it survives only as the defensive fallback for a
//! `MeshWithStoredEdges` that carries no usable stored segment.

use cad_domain::Point3;

/// The mesh's **surface boundary** as independent endpoint pairs: an edge is a
/// boundary edge exactly when the kept triangles use it an odd number of times.
///
/// Edges are matched by vertex **index**, never by floating-point position, so a
/// tessellation diagonal (used twice by the two triangles of one face) cancels
/// and only the outer border of a face — or of a hole in it — is emitted. A
/// non-manifold edge (three uses) stays in the boundary rather than being
/// dropped. Same rule as `cad_representation`'s `FILL OFF` outline
/// (`mesh_boundary_pairs`); this copy works on the batch's reduced `f32`
/// coordinates.
pub(crate) fn mesh_boundary_edge_positions(kept: &[[u32; 3]], verts: &[[f32; 3]]) -> Vec<[f32; 3]> {
    use std::collections::BTreeMap;
    let mut uses: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    for triangle in kept {
        for (first, second) in [
            (triangle[0], triangle[1]),
            (triangle[1], triangle[2]),
            (triangle[2], triangle[0]),
        ] {
            if first == second {
                continue;
            }
            let key = if first < second {
                (first, second)
            } else {
                (second, first)
            };
            *uses.entry(key).or_insert(0) += 1;
        }
    }
    let mut pairs = Vec::new();
    for ((first, second), count) in uses {
        if count % 2 == 0 {
            continue;
        }
        let (Some(a), Some(b)) = (verts.get(first as usize), verts.get(second as usize)) else {
            // An index outside the reduced vertex list is malformed input; drop
            // the segment instead of inventing a position.
            continue;
        };
        pairs.push(*a);
        pairs.push(*b);
    }
    pairs
}

/// The legacy triangle-derived edge overlay: one segment per **unique**
/// triangle edge.
///
/// The directed pairs of every kept triangle are de-duplicated through
/// [`dedup_pairs`] (an unordered endpoint pair is one segment), so a shared
/// diagonal is emitted once. It is also the defensive fallback for a
/// `MeshWithStoredEdges` that carries no usable stored segment.
pub(crate) fn triangle_edge_positions(kept: &[[u32; 3]], verts: &[[f32; 3]]) -> Vec<[f32; 3]> {
    let mut pairs = Vec::with_capacity(kept.len() * 6);
    for tri in kept {
        for &(i, j) in &[(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
            pairs.push(verts[i as usize]);
            pairs.push(verts[j as usize]);
        }
    }
    dedup_pairs(pairs)
}

/// Stored body edge positions relative to `origin`, reduced to `f32`, with
/// duplicate segments removed.
///
/// The input is a flat list of independent endpoint pairs (the
/// [`cad_representation::DisplayPrimitive::MeshWithStoredEdges`] contract). A
/// non-finite endpoint is dropped, then [`dedup_pairs`] removes a repeated
/// unordered pair (`-0.0` normalized to `0.0`) while preserving first-occurrence
/// order and dropping zero-length segments.
///
/// Defensive only: `cad-db` validates a body's edge polylines (at least two
/// points, finite), so a release-path cache cannot contain a bad endpoint. There
/// is no diagnostics channel in this scene build, so the drop is not reported
/// here; the provider's `representation.body_empty` is the missing-geometry
/// signal.
pub(crate) fn stored_edge_positions(points: &[Point3], origin: Point3) -> Vec<[f32; 3]> {
    let mut pairs = Vec::with_capacity(points.len());
    for pair in points.as_chunks::<2>().0 {
        let (Some(a), Some(b)) = (relative_f32(pair[0], origin), relative_f32(pair[1], origin))
        else {
            continue;
        };
        pairs.push(a);
        pairs.push(b);
    }
    dedup_pairs(pairs)
}

/// Drop duplicate and degenerate segments from a flat endpoint-pair list.
///
/// A segment is an **unordered** endpoint pair compared bitwise
/// ([`canonical_segment`]), so the same edge emitted in either direction is one
/// segment. The first occurrence keeps its order; a zero-length pair is dropped.
/// Both wireframe sources (stored body edges and the triangle-derived overlay)
/// go through this one rule.
fn dedup_pairs(pairs: Vec<[f32; 3]>) -> Vec<[f32; 3]> {
    let mut seen: std::collections::BTreeSet<[[u32; 3]; 2]> = std::collections::BTreeSet::new();
    let mut out = Vec::with_capacity(pairs.len());
    for pair in pairs.as_chunks::<2>().0 {
        let key = canonical_segment(pair[0], pair[1]);
        if key[0] == key[1] {
            // Degenerate: both endpoints are the same reduced position.
            continue;
        }
        if seen.insert(key) {
            out.push(pair[0]);
            out.push(pair[1]);
        }
    }
    out
}

/// One world point reduced against `origin` and narrowed to `f32`, `None` when a
/// component is not finite.
fn relative_f32(point: Point3, origin: Point3) -> Option<[f32; 3]> {
    let values = [
        (point.x - origin.x) as f32,
        (point.y - origin.y) as f32,
        (point.z - origin.z) as f32,
    ];
    values
        .iter()
        .all(|value| value.is_finite())
        .then_some(values)
}

/// A bitwise key for one `f32` position, with `-0.0` folded onto `0.0`.
fn f32_key(value: f32) -> u32 {
    if value == 0.0 {
        0
    } else {
        value.to_bits()
    }
}

/// The unordered bitwise key of one segment: endpoints are sorted so both
/// directions produce the same key.
fn canonical_segment(a: [f32; 3], b: [f32; 3]) -> [[u32; 3]; 2] {
    let a = a.map(f32_key);
    let b = b.map(f32_key);
    if a <= b {
        [a, b]
    } else {
        [b, a]
    }
}
