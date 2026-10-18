use super::*;

/// Decide whether triangle winding must be flipped for back-face culling.
///
/// A negative-determinant transform (a mirror, e.g. a negative `INSERT` scale)
/// reverses the on-screen winding of every triangle, so a front-face setting
/// derived from the un-mirrored data would cull the visible side. This is the
/// explicit decision the audit (F14 "镜像绕序") found missing.
pub fn winding_is_flipped(mirrored: bool) -> bool {
    mirrored
}

/// Whether the GPU front face should be `Ccw` (`true`) or `Cw` (`false`).
///
/// Normal, un-mirrored CAD meshes are wound counter-clockwise, so the default is
/// `Ccw`. A mirrored batch flips this.
pub fn front_face_ccw(mirrored: bool) -> bool {
    !winding_is_flipped(mirrored)
}

/// Recompute usable per-vertex normals for a batch's mesh topology.
///
/// Rules (documented, F14 "缺失/不一致法向"):
/// * a normal vector is used as-is when it is finite and non-degenerate and the
///   batch has one per vertex;
/// * otherwise (missing, wrong count, NaN, or a zero normal) the vertex normal
///   is recomputed from the indexed triangles via [`cad_geometry::compute_vertex_normals`];
/// * if recomputation still yields a zero normal (a vertex touched only by
///   degenerate triangles), a fixed fallback normal is used so the shader never
///   sees NaN or a zero-length vector.
///
/// The returned vector is always `batch.vertices.len()` long.
pub fn repaired_normals(batch: &RenderBatch) -> Vec<[f32; 3]> {
    const FALLBACK: [f32; 3] = [0.0, 0.0, 1.0];
    let n = batch.vertices.len();
    let consistent = batch.normals.len() == n
        && batch
            .normals
            .iter()
            .all(|v| vector_is_usable([v[0] as f64, v[1] as f64, v[2] as f64]));
    if consistent {
        return batch
            .normals
            .iter()
            .map(|v| normalize3(*v).unwrap_or(FALLBACK))
            .collect();
    }
    // Rebuild an f64 `Mesh` from the batch's own f32 relative coordinates and
    // let the shared geometry helper do the area-weighted average.
    let mesh = Mesh {
        vertices: batch
            .vertices
            .iter()
            .map(|v| Point3 {
                x: v[0] as f64,
                y: v[1] as f64,
                z: v[2] as f64,
            })
            .collect(),
        triangles: batch.indices.clone(),
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
    };
    let computed = cad_geometry::compute_vertex_normals(&mesh);
    computed
        .iter()
        .map(|nn| normalize3([nn.x as f32, nn.y as f32, nn.z as f32]).unwrap_or(FALLBACK))
        .collect()
}

/// Whether a batch needs its normals repaired (missing, wrong length or bad).
pub fn normals_need_repair(batch: &RenderBatch) -> bool {
    batch.normals.len() != batch.vertices.len()
        || batch
            .normals
            .iter()
            .any(|v| !vector_is_usable([v[0] as f64, v[1] as f64, v[2] as f64]))
}

/// Per-vertex colours for the mesh pipeline, always one entry per vertex.
///
/// The returned value is the vertex-colour attribute the shader multiplies into
/// the batch tint:
/// * when the batch carries exactly one colour per vertex, each is sanitised and
///   used as-is;
/// * otherwise (the default, a non-gradient mesh) every vertex is white, so the
///   tint is unchanged and existing geometry renders identically.
///
/// A non-finite or out-of-range channel is clamped rather than dropped, matching
/// [`cad_scene::sanitize_color`].
pub fn repaired_colors(batch: &RenderBatch) -> Vec<[f32; 3]> {
    const WHITE: [f32; 3] = [1.0, 1.0, 1.0];
    if batch.colors.len() == batch.vertices.len() {
        return batch
            .colors
            .iter()
            .map(|c| cad_scene::sanitize_color(*c))
            .collect();
    }
    vec![WHITE; batch.vertices.len()]
}

/// Whether a batch carries usable per-vertex colours.
pub fn has_vertex_colors(batch: &RenderBatch) -> bool {
    batch.colors.len() == batch.vertices.len() && !batch.colors.is_empty()
}

/// Deterministic (first, second) vertex indices for every edge of a batch's
/// triangle topology.
///
/// Used to build a `LineList` index buffer on top of the shared mesh vertex
/// buffer, so the wireframe overlay does not duplicate vertex data. The pair for
/// an edge is ordered `min < max` and the list is sorted and de-duplicated, so
/// the same mesh always produces the same index buffer.
pub fn sorted_edge_indices(indices: &[[u32; 3]]) -> Vec<u32> {
    let mut edges: Vec<(u32, u32)> = Vec::new();
    for tri in indices {
        for (a, b) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
            if a == b {
                continue;
            }
            edges.push((a.min(b), a.max(b)));
        }
    }
    edges.sort_unstable();
    edges.dedup();
    let mut out = Vec::with_capacity(edges.len() * 2);
    for (a, b) in edges {
        out.push(a);
        out.push(b);
    }
    out
}

/// Build wireframe indices only when the scene batch explicitly carries edge
/// overlay geometry. Empty `edges` is the opt-out signal for a filled mesh.
pub(crate) fn mesh_edge_indices(batch: &RenderBatch) -> Vec<u32> {
    if batch.topology == RenderTopology::Mesh && !batch.edges.is_empty() {
        sorted_edge_indices(&batch.indices)
    } else {
        Vec::new()
    }
}
