use super::*;

/// Byte stride of one [`PulledLine`]: three `vec4<f32>` plus one `vec4<u32>`.
pub(crate) const PULLED_LINE_STRIDE: u32 = 64;

/// Byte stride of one [`PulledMeshVertex`]: three `vec4<f32>` plus one
/// `vec4<u32>`.
pub(crate) const PULLED_MESH_VERTEX_STRIDE: u32 = 64;

/// Byte size of the per-class pulled camera uniform: four `vec4<f32>` columns
/// (the fourth carries the world units per pixel for the AA line SDF in its `x`).
pub(crate) const PULLED_CAMERA_BYTES: u32 = 64;

/// One pulled line segment: the universal [`LineInstance`] fields plus the
/// per-chunk origin index. `vec4` members (not `vec3`/`uvec2`) so the storage
/// array element stride is exactly [`PULLED_LINE_STRIDE`] with no implicit
/// padding.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct PulledLine {
    pub(crate) p0_width: [f32; 4],
    pub(crate) p1_pad: [f32; 4],
    pub(crate) rgba: [f32; 4],
    /// `[0]` = renderer pick key, `[1]` = [`INSTANCE_FLAG_*`] bits,
    /// `[2]` = per-chunk origin index, `[3]` = reserved (0).
    pub(crate) meta: [u32; 4],
}

/// One pulled merged-mesh vertex: the universal [`MeshVertex`] fields plus the
/// per-chunk origin index, in a storage-friendly `vec4` layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct PulledMeshVertex {
    /// `xyz` = chunk-relative position, `w` reserved (0).
    pub(crate) position: [f32; 4],
    /// `xyz` = repaired normal, `w` reserved (0).
    pub(crate) normal: [f32; 4],
    /// `rgb` = baked per-vertex colour, `a` reserved (0).
    pub(crate) color: [f32; 4],
    /// `[0]` = renderer pick key, `[1]` = [`INSTANCE_FLAG_*`] bits,
    /// `[2]` = per-chunk origin index, `[3]` = reserved (0).
    pub(crate) meta: [u32; 4],
}

/// The exact packed bytes of `lines` (one contiguous storage payload).
pub(crate) fn pulled_line_bytes(lines: &[PulledLine]) -> Vec<u8> {
    bytemuck::cast_slice(lines).to_vec()
}

/// The exact packed bytes of `vertices` (one contiguous storage payload).
pub(crate) fn pulled_mesh_vertex_bytes(vertices: &[PulledMeshVertex]) -> Vec<u8> {
    bytemuck::cast_slice(vertices).to_vec()
}

/// The packed `f32` bytes of a per-chunk translation-column table.
pub(crate) fn chunk_transform_bytes(columns: &[[f32; 4]]) -> Vec<u8> {
    bytemuck::cast_slice(columns).to_vec()
}

/// Concatenate `streams` into one pulled line storage payload, stamping each
/// record with `chunk_base + its stream index`.
///
/// Streams are visited in index order, which is the converted-batch order, so
/// the concatenated sequence preserves the exact draw order the per-stream runs
/// had: one draw over `0..total` rasterizes the segments in the same order.
/// `chunk_base` is the index of `streams[0]` in the whole-scene chunk table, so
/// a class split into binding-sized segments still addresses the shared table.
pub(crate) fn build_pulled_lines(streams: &[&[LineInstance]], chunk_base: u32) -> Vec<PulledLine> {
    let total: usize = streams.iter().map(|s| s.len()).sum();
    let mut out = Vec::with_capacity(total);
    for (offset, instances) in streams.iter().enumerate() {
        let chunk = chunk_base + offset as u32;
        for inst in instances.iter() {
            out.push(PulledLine {
                p0_width: inst.p0_width,
                p1_pad: inst.p1_pad,
                rgba: inst.rgba,
                meta: [inst.id_flags[0], inst.id_flags[1], chunk, 0],
            });
        }
    }
    out
}

/// Concatenate `chunks` into one pulled vertex storage payload, stamping each
/// vertex with `chunk_base + its chunk index`.
pub(crate) fn build_pulled_mesh_vertices(
    chunks: &[&[MeshVertex]],
    chunk_base: u32,
) -> Vec<PulledMeshVertex> {
    let total: usize = chunks.iter().map(|c| c.len()).sum();
    let mut out = Vec::with_capacity(total);
    for (offset, vertices) in chunks.iter().enumerate() {
        let chunk = chunk_base + offset as u32;
        for v in vertices.iter() {
            out.push(PulledMeshVertex {
                position: [v.position[0], v.position[1], v.position[2], 0.0],
                normal: [v.normal[0], v.normal[1], v.normal[2], 0.0],
                color: [v.color[0], v.color[1], v.color[2], 0.0],
                meta: [v.entity_flags[0], v.entity_flags[1], chunk, 0],
            });
        }
    }
    out
}

/// Build the whole-scene triangle index list, rebased onto the concatenated
/// vertex buffer and split into a **non-mirrored** then a **mirrored** range.
///
/// The mirrored range is emitted after the non-mirrored one so a single draw per
/// pipeline can cover each winding class (a mirrored chunk culls the opposite
/// face, which is fixed-function state). The returned value is
/// `(indices, mirror_split)` where `mirror_split` is the number of non-mirrored
/// indices.
///
/// `chunks` is `(local_indices, vertex_count, mirrored)` per resident chunk, in
/// chunk order; each chunk's vertices occupy
/// `[vertex_base, vertex_base + vertex_count)` of the concatenated buffer.
pub(crate) fn build_pulled_mesh_indices_with_bases(
    chunks: &[(&[u32], u32, bool)],
) -> (Vec<u32>, u32) {
    let mut vertex_base = Vec::with_capacity(chunks.len());
    let mut base = 0u32;
    for (_, vertex_count, _) in chunks {
        vertex_base.push(base);
        base += *vertex_count;
    }
    let mut indices = Vec::new();
    for (i, (local, _, mirrored)) in chunks.iter().enumerate() {
        if *mirrored {
            continue;
        }
        let vb = vertex_base[i];
        indices.extend(local.iter().map(|v| v + vb));
    }
    let mirror_split = indices.len() as u32;
    for (i, (local, _, mirrored)) in chunks.iter().enumerate() {
        if !*mirrored {
            continue;
        }
        let vb = vertex_base[i];
        indices.extend(local.iter().map(|v| v + vb));
    }
    (indices, mirror_split)
}

/// The per-chunk translation column of `projection.batch_matrix_f64(origin)`.
///
/// The first three columns are origin-independent, so one camera uniform serves
/// every chunk; only this last column (the `f64`-reduced translation) is per
/// chunk. Reusing the exact column the universal per-stream uniform carries
/// makes the pulled multiply byte-identical.
pub(crate) fn chunk_transform_column(projection: FrameProjection, origin: [f64; 3]) -> [f32; 4] {
    let m = projection.batch_matrix_f64(origin);
    [m[12], m[13], m[14], m[15]]
}

/// The four pulled-camera columns: the three origin-independent columns of the
/// per-chunk transform, plus a fourth whose `x` carries the world units per pixel
/// (the AA line SDF ramp width). `yzw` are reserved.
pub(crate) fn pulled_camera_columns(projection: FrameProjection) -> [[f32; 4]; 4] {
    let m = projection.batch_matrix_f64([0.0, 0.0, 0.0]);
    [
        [m[0], m[1], m[2], m[3]],
        [m[4], m[5], m[6], m[7]],
        [m[8], m[9], m[10], m[11]],
        [projection.world_per_px() as f32, 0.0, 0.0, 0.0],
    ]
}

/// The packed `f32` bytes of the pulled camera uniform (four columns).
pub(crate) fn pulled_camera_bytes(columns: &[[f32; 4]; 4]) -> [u8; PULLED_CAMERA_BYTES as usize] {
    let mut bytes = [0u8; PULLED_CAMERA_BYTES as usize];
    for (col, column) in columns.iter().enumerate() {
        for (row, value) in column.iter().enumerate() {
            let at = (col * 4 + row) * 4;
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
    bytes
}
