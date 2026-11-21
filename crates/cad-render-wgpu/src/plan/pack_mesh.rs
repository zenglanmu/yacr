use super::*;

pub(crate) fn position_only_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: 12,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![0 => Float32x3],
    }
}

pub(crate) fn mesh_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: 12,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![0 => Float32x3],
    }
}

/// Per-vertex normals travel in their own vertex buffer (slot 1, see
/// `draw_batch`). `mesh.wgsl` reads them at `@location(1)`, so the pipeline must
/// declare this layout too or `create_render_pipeline` fails validation with
/// "Location[1] ... is not provided by the previous stage outputs".
pub(crate) fn mesh_normal_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: 12,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![1 => Float32x3],
    }
}

/// Per-vertex colours travel in their own vertex buffer (slot 2, see
/// `draw_batch`). The buffer is always present for a mesh — white when the
/// batch has no gradient — so `mesh.wgsl`'s `@location(2)` is always supplied.
pub(crate) fn mesh_color_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: 12,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![2 => Float32x3],
    }
}

/// Per-vertex UV0 travels in its own vertex buffer (slot 3), and **only** for a
/// batch that carries a base-colour texture (ADR-0038 D9).
///
/// The untextured `mesh` pipelines deliberately keep their three-buffer layout:
/// a batch without a texture uploads no UV buffer and is drawn through exactly
/// the same pipeline state as before this feature existed.
pub(crate) fn mesh_uv_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: 8,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![3 => Float32x2],
    }
}

/// The packed bytes of a batch's UV0 stream (`Float32x2` per vertex; 8 bytes).
pub(crate) fn pack_uvs(uvs: &[[f32; 2]]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(uvs.len() * 8);
    for uv in uvs {
        bytes.extend_from_slice(&uv[0].to_le_bytes());
        bytes.extend_from_slice(&uv[1].to_le_bytes());
    }
    bytes
}

/// Per-vertex tangent frame travels in its own vertex buffer (slot 4), and
/// **only** for a PBR batch that needs one: the `mesh_pbr` pipeline's full
/// variant (ADR-0039 D6).
///
/// `Float32x4`: `xyz` is the unit tangent, `w` the bitangent handedness
/// (`+1`/`-1`) so the shader can rebuild `B = cross(N, T) * w` without a second
/// attribute.
pub(crate) fn mesh_tangent_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: u64::from(cad_scene::TANGENT_STRIDE),
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![4 => Float32x4],
    }
}

/// The packed bytes of a batch's tangent stream (`Float32x4` per vertex).
pub(crate) fn pack_tangents(tangents: &[[f32; 4]]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(tangents.len() * 16);
    for tangent in tangents {
        for component in tangent {
            bytes.extend_from_slice(&component.to_le_bytes());
        }
    }
    bytes
}

/// A normal shorter than this is degenerate; stored normals are expected to be
/// ~unit length, so this is an absolute floor, not a scale-relative one.
const NORMAL_LENGTH_EPSILON: f32 = 1e-8;

/// Absolute UV-area floor, relative to the mesh's **squared UV extent**.
///
/// `f32` UVs quantise at ~1e-7 of their own magnitude, so a UV triangle area
/// below `1e-12 · extent²` is subtraction noise from the determinant, not a
/// parameterisation. Making it relative keeps the test honest for a mesh whose
/// UVs are in millimetres and for one in the unit square.
const UV_AREA_RELATIVE_EPSILON: f32 = 1e-12;

/// Largest `|det(duv1, duv2)| / (|duv1| · |duv2|)` — the sine of the angle
/// between the two UV edges — a triangle may have and still be usable.
///
/// Below it the two UV edges are numerically collinear: the gradient solve would
/// divide by rounding noise and produce a huge, meaningless direction.
const UV_SLIVER_SINE: f32 = 1e-6;

/// A vertex's accumulated tangent counts as **cancelled out** when the part
/// orthogonal to its normal is below this fraction of the strongest
/// single-triangle tangent the vertex received. Relative, so it tracks mesh
/// scale; far above the `f32` accumulation noise (~n · 6e-8 of the summands) and
/// far below any real area-weighted average.
const TANGENT_CANCEL_RATIO: f32 = 1e-5;

/// A raw tangent whose orthogonal component is below this fraction of its own
/// length is (numerically) parallel to the normal and carries no direction.
const TANGENT_PARALLEL_RATIO: f32 = 1e-6;

/// Generate one tangent frame per vertex from positions + UV0 + triangles.
///
/// The domain stores **no** tangents (glTF's `TANGENT` accessor is not imported),
/// so they are derived here at pack time with a MikkTSpace-style simplification:
///
/// 1. per triangle, solve the UV gradient for a tangent and bitangent and
///    accumulate both on each of its three vertices. A **UV-degenerate**
///    triangle — zero UV area at `f32` precision ([`UV_AREA_RELATIVE_EPSILON`])
///    or two numerically collinear UV edges ([`UV_SLIVER_SINE`]) — is **skipped**
///    and contributes nothing. Skipping is not inventing: that triangle holds no
///    UV gradient to read, so the frame is built only from the triangles that do
///    have one. A sliver triangle is extremely common in game assets, and
///    refusing the whole batch for it used to cost the asset its whole normal
///    map;
/// 2. per vertex, remove the normal component of the accumulated tangent and
///    normalise it. When the accumulation **cancels** (see
///    [`TANGENT_CANCEL_RATIO`]), the frame falls back to the **largest-magnitude
///    single-triangle tangent that vertex actually received** — still a real
///    triangle's tangent, with that triangle's bitangent deciding `w`; never a
///    fabricated direction;
/// 3. a vertex whose triangles were *all* UV-degenerate received no UV gradient
///    at all. Its frame is inherited from a vertex at the **identical** stored
///    position (exact `f32` bit pattern: a UV-seam duplicate, i.e. the same
///    surface point on another chart) that did resolve, re-orthonormalised
///    against this vertex's normal and keeping the donor chart's handedness.
///    That is again a real measured tangent at the same point, not an invented
///    one;
/// 4. `w` is the handedness `sign(dot(cross(N, T), B))`, `+1` when the frame
///    degenerates to zero.
///
/// `None` means the batch has **no usable tangent frame**: a structural mismatch
/// (UV0/normal stream not exactly one entry per vertex, no triangles, an
/// out-of-range index, a zero-length normal), or **any** vertex still without a
/// resolved frame — it has no non-degenerate triangle and no coincident vertex
/// to inherit from. A per-vertex attribute cannot be partial, so one undetermined
/// vertex refuses the batch. The caller must then treat the normal-map channel as
/// absent and record `render.pbr.tangent_unavailable` — a tangent is **never**
/// invented (ADR-0039 D6).
pub(crate) fn generate_tangents(
    vertices: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    indices: &[[u32; 3]],
) -> Option<Vec<[f32; 4]>> {
    if vertices.is_empty()
        || uvs.len() != vertices.len()
        || normals.len() != vertices.len()
        || indices.is_empty()
    {
        return None;
    }
    let mut unit_normals = Vec::with_capacity(normals.len());
    for normal in normals {
        unit_normals.push(tangent_unit(*normal)?);
    }
    let area_floor = uv_area_floor(uvs);
    let mut tangent_accum = vec![[0.0f32; 3]; vertices.len()];
    let mut bitangent_accum = vec![[0.0f32; 3]; vertices.len()];
    // Per vertex: the strongest single-triangle tangent it actually received
    // (largest raw magnitude) with that triangle's bitangent, plus that
    // magnitude as the scale reference for the cancellation test.
    let mut strongest: Vec<Option<([f32; 3], [f32; 3])>> = vec![None; vertices.len()];
    let mut strongest_length = vec![0.0f32; vertices.len()];
    for triangle in indices {
        let [i0, i1, i2] = triangle.map(|index| index as usize);
        if i0 >= vertices.len() || i1 >= vertices.len() || i2 >= vertices.len() {
            return None;
        }
        let p0 = vertices[i0];
        let p1 = vertices[i1];
        let p2 = vertices[i2];
        let uv0 = uvs[i0];
        let uv1 = uvs[i1];
        let uv2 = uvs[i2];
        let edge1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let edge2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
        let duv1 = [uv1[0] - uv0[0], uv1[1] - uv0[1]];
        let duv2 = [uv2[0] - uv0[0], uv2[1] - uv0[1]];
        let determinant = duv1[0] * duv2[1] - duv2[0] * duv1[1];
        // `|det| / (|duv1|·|duv2|)` is `sin` of the angle between the UV edges,
        // so the sliver test is scale-free; the area floor covers the case where
        // both edges are short enough that their determinant is pure noise.
        let edge_product = (duv1[0] * duv1[0] + duv1[1] * duv1[1]).sqrt()
            * (duv2[0] * duv2[0] + duv2[1] * duv2[1]).sqrt();
        if !determinant.is_finite()
            || !edge_product.is_finite()
            || determinant.abs() <= area_floor.max(UV_SLIVER_SINE * edge_product)
        {
            continue;
        }
        let reciprocal = 1.0 / determinant;
        let tangent = [
            (edge1[0] * duv2[1] - edge2[0] * duv1[1]) * reciprocal,
            (edge1[1] * duv2[1] - edge2[1] * duv1[1]) * reciprocal,
            (edge1[2] * duv2[1] - edge2[2] * duv1[1]) * reciprocal,
        ];
        let bitangent = [
            (edge2[0] * duv1[0] - edge1[0] * duv2[0]) * reciprocal,
            (edge2[1] * duv1[0] - edge1[1] * duv2[0]) * reciprocal,
            (edge2[2] * duv1[0] - edge1[2] * duv2[0]) * reciprocal,
        ];
        if !tangent.iter().all(|value| value.is_finite())
            || !bitangent.iter().all(|value| value.is_finite())
        {
            // A non-finite vertex/UV stream is refused by the upload gate before
            // this point; a triangle that still lands here contributes nothing
            // instead of poisoning every vertex it touches.
            continue;
        }
        let magnitude = tangent_length(tangent);
        for index in [i0, i1, i2] {
            for axis in 0..3 {
                tangent_accum[index][axis] += tangent[axis];
                bitangent_accum[index][axis] += bitangent[axis];
            }
            if magnitude > strongest_length[index] {
                strongest_length[index] = magnitude;
                strongest[index] = Some((tangent, bitangent));
            }
        }
    }
    let mut frames: Vec<Option<[f32; 4]>> = Vec::with_capacity(vertices.len());
    for index in 0..vertices.len() {
        let normal = unit_normals[index];
        let accumulated = tangent_accum[index];
        // Only average when the average still carries a direction: compare the
        // part of the sum that remains orthogonal to the normal (a sum that is
        // mostly along the normal has no tangent direction) against the strongest
        // real contribution, so the ratio tracks mesh scale. Below it the sum is
        // cancellation noise and the strongest real triangle tangent is the
        // honest answer.
        let mut frame = None;
        if orthogonal_length(normal, accumulated) > TANGENT_CANCEL_RATIO * strongest_length[index] {
            frame = tangent_frame(normal, accumulated, bitangent_accum[index]);
        }
        if frame.is_none() {
            if let Some((tangent, bitangent)) = strongest[index] {
                frame = tangent_frame(normal, tangent, bitangent);
            }
        }
        frames.push(frame);
    }
    // Vertices whose triangles were all skipped: inherit the frame of a vertex
    // at the *identical* stored position (a UV-seam duplicate of the same
    // surface point). Exact bit equality only — no tolerance is guessed.
    let mut by_position: std::collections::HashMap<[u32; 3], usize> =
        std::collections::HashMap::new();
    for (index, frame) in frames.iter().enumerate() {
        if frame.is_some() {
            by_position
                .entry(position_bits(vertices[index]))
                .or_insert(index);
        }
    }
    for index in 0..frames.len() {
        if frames[index].is_some() {
            continue;
        }
        let Some(&donor) = by_position.get(&position_bits(vertices[index])) else {
            continue;
        };
        let Some(donor_frame) = frames[donor] else {
            continue;
        };
        let donor_normal = unit_normals[donor];
        let donor_tangent = [donor_frame[0], donor_frame[1], donor_frame[2]];
        // Rebuild the donor's bitangent from its stored handedness so the
        // inherited frame keeps that chart's orientation.
        let donor_bitangent = tangent_cross(donor_normal, donor_tangent);
        let donor_bitangent = [
            donor_bitangent[0] * donor_frame[3],
            donor_bitangent[1] * donor_frame[3],
            donor_bitangent[2] * donor_frame[3],
        ];
        frames[index] = tangent_frame(unit_normals[index], donor_tangent, donor_bitangent);
    }
    let mut tangents = Vec::with_capacity(vertices.len());
    for frame in frames {
        tangents.push(frame?);
    }
    Some(tangents)
}

/// Absolute UV-area floor for one mesh's parameterisation: [`UV_AREA_RELATIVE_EPSILON`]
/// times the squared UV extent (the larger side of the UV bounding box).
fn uv_area_floor(uvs: &[[f32; 2]]) -> f32 {
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for uv in uvs {
        for axis in 0..2 {
            min[axis] = min[axis].min(uv[axis]);
            max[axis] = max[axis].max(uv[axis]);
        }
    }
    let extent = (max[0] - min[0]).max(max[1] - min[1]);
    if !extent.is_finite() || extent <= 0.0 {
        return 0.0;
    }
    UV_AREA_RELATIVE_EPSILON * extent * extent
}

/// Orthonormalise `raw_tangent` against the unit `normal` and derive the
/// handedness from `raw_bitangent`.
///
/// `None` when the raw tangent is zero, non-finite, or (numerically) parallel to
/// the normal: it then carries no usable direction in the tangent plane.
fn tangent_frame(
    normal: [f32; 3],
    raw_tangent: [f32; 3],
    raw_bitangent: [f32; 3],
) -> Option<[f32; 4]> {
    let raw_length = tangent_length(raw_tangent);
    if !raw_length.is_finite() || raw_length <= 0.0 {
        return None;
    }
    // Gram-Schmidt: remove the normal component, then normalise.
    let projection = tangent_dot(raw_tangent, normal);
    let orthogonal = [
        raw_tangent[0] - normal[0] * projection,
        raw_tangent[1] - normal[1] * projection,
        raw_tangent[2] - normal[2] * projection,
    ];
    let length = tangent_length(orthogonal);
    if !length.is_finite() || length <= TANGENT_PARALLEL_RATIO * raw_length {
        return None;
    }
    let tangent = [
        orthogonal[0] / length,
        orthogonal[1] / length,
        orthogonal[2] / length,
    ];
    // `cross(N, T)` points along +B when w = +1. A zero bitangent (no
    // information) keeps +1.
    let cross = tangent_cross(normal, tangent);
    let w = if tangent_dot(cross, raw_bitangent) < 0.0 {
        -1.0
    } else {
        1.0
    };
    Some([tangent[0], tangent[1], tangent[2], w])
}

/// `value` scaled to unit length, or `None` when it is degenerate.
fn tangent_unit(value: [f32; 3]) -> Option<[f32; 3]> {
    let length = tangent_length(value);
    if !length.is_finite() || length <= NORMAL_LENGTH_EPSILON {
        return None;
    }
    Some([value[0] / length, value[1] / length, value[2] / length])
}

/// Length of the part of `raw` orthogonal to the unit `normal`.
fn orthogonal_length(normal: [f32; 3], raw: [f32; 3]) -> f32 {
    let projection = tangent_dot(raw, normal);
    tangent_length([
        raw[0] - normal[0] * projection,
        raw[1] - normal[1] * projection,
        raw[2] - normal[2] * projection,
    ])
}

fn tangent_length(value: [f32; 3]) -> f32 {
    (value[0] * value[0] + value[1] * value[1] + value[2] * value[2]).sqrt()
}

fn tangent_dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn tangent_cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Exact bit pattern of a stored position: the key for "the same surface point"
/// (`+0.0` and `-0.0` are folded together so a seam duplicate is still found).
fn position_bits(position: [f32; 3]) -> [u32; 3] {
    position.map(|value| {
        if value == 0.0 {
            0.0f32.to_bits()
        } else {
            value.to_bits()
        }
    })
}

// ---------------------------------------------------------------------------
// Flat-merged mesh runs ("entity = data", second slice)
// ---------------------------------------------------------------------------
//
// The second dominant draw class on real scenes is the tessellated glyph mesh
// (`MeshWithoutEdgeOverlay`): thousands of small opaque triangle lists, one draw
// each. Instead of one draw per batch, adjacent same-pipeline opaque `Mesh`
// batches are concatenated into one static vertex buffer at upload and drawn as
// one indexed draw. The per-batch uniform tint is baked into the per-vertex
// colour (see `shaders/mesh_merged.wgsl`); that is mathematically equal to the
// legacy per-pixel product but only **bit**-equal for a constant-colour mesh or a
// white tint, because a varying colour re-associates the f32 product (fenced by
// a tolerance pixel test, not byte-identity). A per-vertex `entity_flags`
// attribute carries the low 32 bits of the source `EntityId` (from the batch's
// first source) so a later picking phase can resolve fragments; the legacy
// per-batch draw carried no explicit pick key. The layout is WebGL2-safe: one
// interleaved buffer of plain vertex attributes, no vertex-stage storage buffer.

/// Byte stride of one merged-mesh vertex: position (`vec3`), normal (`vec3`),
/// baked colour (`vec3`) and `entity_flags` (`vec2<u32>`).
pub(crate) const MESH_VERTEX_STRIDE: u32 = 44;

/// Default upper bound on vertices per merged mesh chunk. `1 << 20` vertices is
/// 44 MiB, well inside every backend's `max_buffer_size`, and keeps a merged
/// draw's index range legal. A batch with more vertices is split across several
/// chunks (one run per piece), never uploaded as one oversized buffer. Injectable
/// on the `Renderer` for tests.
pub(crate) const MAX_VERTICES_PER_MESH_STREAM: usize = 1 << 20;

/// One merged-mesh vertex, `#[repr(C)]` + `Pod` so the packed bytes are exactly
/// the GPU layout; a unit test fences the stride against `size_of`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct MeshVertex {
    /// Position in the chunk's fixed-point-relative space.
    pub(crate) position: [f32; 3],
    /// Per-vertex normal (repaired at upload, exactly as the legacy path).
    pub(crate) normal: [f32; 3],
    /// Per-vertex colour *already multiplied by the source batch's tint*, so the
    /// merged draw's shared uniform tint can stay opaque white. Bit-identical to
    /// the legacy per-pixel product only when the colour is constant across the
    /// mesh or the tint is white; a varying-colour mesh is equal only within a
    /// documented tolerance.
    pub(crate) color: [f32; 3],
    /// `[0]` = the renderer **pick key** of the vertex's source batch (see
    /// [`PickTable`]; `0` = none); `[1]` = the batch's [`INSTANCE_FLAG_*`] bits.
    ///
    /// This is a *renderer* key, deliberately not a truncated `EntityId`: the
    /// key indexes the CPU-side [`PickTable`], which preserves the full `u128`
    /// identity (and detects truncation collisions), so an id-pass readback can
    /// never silently mis-pick between two entities. A multi-source batch
    /// resolves to its whole source list and is reported as ambiguous rather
    /// than attributed to its first source. The legacy per-batch draw carried no
    /// explicit pick key at all; the key rides the per-batch uniform instead.
    pub(crate) entity_flags: [u32; 2],
}

/// The exact packed bytes of `vertices` (one contiguous GPU buffer payload).
pub(crate) fn mesh_vertex_bytes(vertices: &[MeshVertex]) -> Vec<u8> {
    bytemuck::cast_slice(vertices).to_vec()
}

/// Interleaved vertex-buffer layout of the merged-mesh pipeline.
pub(crate) fn mesh_merged_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
        0 => Float32x3,
        1 => Float32x3,
        2 => Float32x3,
        3 => Uint32x2
    ];
    wgpu::VertexBufferLayout {
        array_stride: u64::from(MESH_VERTEX_STRIDE),
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRIBUTES,
    }
}

/// Whether a scene batch's triangle list is eligible for the flat-merge path.
///
/// Only opaque `Mesh` batches with a non-empty triangle list are merged: a
/// transparent mesh keeps its camera-dependent back-to-front order, and a mesh
/// with no triangles has nothing to draw. `mirrored` is a merge boundary (a
/// different pipeline), enforced in [`build_mesh_chunks`], not here.
///
/// A **textured** batch is never mergeable: the merged path bakes the per-batch
/// tint into one vertex stream and binds a single uniform for the whole chunk,
/// which cannot express a per-batch texture (ADR-0038 D8). That exclusion is a
/// documented batching regression — it costs one draw per textured mesh — not a
/// silent behaviour change. The pulled mesh class applies the same predicate, so
/// textured batches stay on the per-batch `mesh_textured` path on every tier.
///
/// A **PBR** batch is never mergeable either (ADR-0039 D6): it draws through the
/// separate `mesh_pbr` channel with its own five texture bindings and per-batch
/// factors, none of which the merged vertex stream can carry. The same predicate
/// keeps it out of the pulled class on every tier.
pub(crate) fn mesh_batch_is_mergeable(batch: &cad_scene::RenderBatch) -> bool {
    batch.topology == RenderTopology::Mesh
        && !batch.indices.is_empty()
        && classify_alpha(batch.alpha) == AlphaClass::Opaque
        && batch.texture.is_none()
        && batch.pbr.is_none()
}

/// One member batch's triangle range inside a merged mesh chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MeshRun {
    /// Index into `Renderer::mesh_chunks`.
    pub(crate) chunk: u32,
    /// First triangle index of this member within its chunk's index buffer.
    pub(crate) index_offset: u32,
    /// Number of triangle indices (3 per triangle) this member contributes.
    pub(crate) index_count: u32,
    /// Number of chunk vertices this member contributes (the distinct vertices
    /// its triangles use, in first-use order).
    pub(crate) vertex_count: u32,
}

/// One merged mesh chunk, ready to upload: interleaved vertices in original
/// batch order, a triangle index list rebased onto those vertices, and the
/// single `f64` reference origin its stored positions are measured from.
pub(crate) struct MeshChunkData {
    pub(crate) origin: [f64; 3],
    pub(crate) mirrored: bool,
    pub(crate) vertices: Vec<MeshVertex>,
    pub(crate) indices: Vec<u32>,
    /// World-space AABB of the chunk's vertices, retained from the builder for
    /// per-frame view culling (see [`cull`]). A source split across several
    /// chunks carries the whole-batch AABB in every piece (conservative).
    pub(crate) aabb: WorldAabb,
}

/// A single batch's contribution to a chunk: the subset of its vertices used by
/// a slice of its triangles, plus those triangles rebased onto the subset.
///
/// Most batches are one whole piece; a batch larger than the vertex cap is split
/// by triangles into several pieces (each a separate run), so no chunk is
/// oversized and no triangle is lost.
pub(crate) struct MeshPiece {
    batch: usize,
    /// Vertex indices into `batches[batch].vertices`, in first-use order.
    vertex_indices: Vec<u32>,
    /// Triangles rebased onto `vertex_indices`.
    triangles: Vec<[u32; 3]>,
    origin: [f64; 3],
    min: [f64; 3],
    max: [f64; 3],
    mirrored: bool,
}

/// Split a batch's triangle list into pieces of at most `cap` distinct vertices,
/// preserving triangle order.
///
/// Triangles are taken greedily until adding the next one would exceed `cap`
/// distinct vertices; that triangle starts the next piece. The result is
/// deterministic (a function of the batch's own vertex/triangle order) and never
/// empty for a non-empty triangle list. `cap` is clamped to at least 3.
pub(super) fn mesh_pieces(
    batch: &cad_scene::RenderBatch,
    batch_index: usize,
    cap: usize,
) -> Vec<MeshPiece> {
    let cap = cap.max(3);
    let origin = [
        batch.local_origin.x,
        batch.local_origin.y,
        batch.local_origin.z,
    ];
    let mut pieces = Vec::new();
    let mut vertex_indices: Vec<u32> = Vec::new();
    let mut triangles: Vec<[u32; 3]> = Vec::new();
    let mut remap: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();

    let flush = |vertex_indices: &mut Vec<u32>,
                 triangles: &mut Vec<[u32; 3]>,
                 remap: &mut std::collections::HashMap<u32, u32>,
                 pieces: &mut Vec<MeshPiece>| {
        if triangles.is_empty() {
            return;
        }
        let (min, max) = piece_world_aabb(batch, vertex_indices, origin);
        pieces.push(MeshPiece {
            batch: batch_index,
            vertex_indices: std::mem::take(vertex_indices),
            triangles: std::mem::take(triangles),
            origin,
            min,
            max,
            mirrored: batch.mirrored,
        });
        remap.clear();
    };

    for tri in &batch.indices {
        // Distinct new vertices this triangle would add.
        let new_vertices = tri.iter().filter(|v| !remap.contains_key(v)).count();
        if !triangles.is_empty() && vertex_indices.len() + new_vertices > cap {
            flush(&mut vertex_indices, &mut triangles, &mut remap, &mut pieces);
        }
        let mut rebased = [0u32; 3];
        for (slot, v) in tri.iter().enumerate() {
            let mapped = *remap.entry(*v).or_insert_with(|| {
                let next = vertex_indices.len() as u32;
                vertex_indices.push(*v);
                next
            });
            rebased[slot] = mapped;
        }
        triangles.push(rebased);
    }
    flush(&mut vertex_indices, &mut triangles, &mut remap, &mut pieces);
    pieces
}

/// World-space AABB of the vertices `vertex_indices` selects from `batch`.
pub(super) fn piece_world_aabb(
    batch: &cad_scene::RenderBatch,
    vertex_indices: &[u32],
    origin: [f64; 3],
) -> ([f64; 3], [f64; 3]) {
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for &v in vertex_indices {
        let vertex = batch.vertices[v as usize];
        for axis in 0..3 {
            let world = origin[axis] + f64::from(vertex[axis]);
            min[axis] = min[axis].min(world);
            max[axis] = max[axis].max(world);
        }
    }
    (min, max)
}

/// A merged mesh chunk under construction.
pub(super) struct MeshChunkBuilder {
    /// Distinct batch indices contributing to this chunk (for the origin).
    batch_indices: Vec<usize>,
    /// `Some(origin)` while every piece shares that exact `local_origin`;
    /// `None` once two differ (the chunk is then "mixed").
    shared_origin: Option<[f64; 3]>,
    min: [f64; 3],
    max: [f64; 3],
    vertices: usize,
    mirrored: bool,
    /// Positions into the flat piece list, in append order.
    slices: Vec<usize>,
}

impl MeshChunkBuilder {
    pub(super) fn new(piece: usize, builder: &MeshPiece) -> Self {
        Self {
            batch_indices: vec![builder.batch],
            shared_origin: Some(builder.origin),
            min: builder.min,
            max: builder.max,
            vertices: builder.vertex_indices.len(),
            mirrored: builder.mirrored,
            slices: vec![piece],
        }
    }

    /// Would adding `piece` keep the chunk inside `cap` vertices, the same
    /// `mirrored` pipeline and — only for a mixed-origin chunk — inside
    /// `max_radius` of its midpoint?
    pub(super) fn accepts(&self, piece: &MeshPiece, cap: usize, max_radius: f64) -> bool {
        if self.mirrored != piece.mirrored {
            return false;
        }
        if self.vertices + piece.vertex_indices.len() > cap {
            return false;
        }
        if self.shared_origin == Some(piece.origin) {
            return true;
        }
        let union_min = [
            self.min[0].min(piece.min[0]),
            self.min[1].min(piece.min[1]),
            self.min[2].min(piece.min[2]),
        ];
        let union_max = [
            self.max[0].max(piece.max[0]),
            self.max[1].max(piece.max[1]),
            self.max[2].max(piece.max[2]),
        ];
        aabb_half_diagonal(union_min, union_max) <= max_radius
    }

    pub(super) fn push(&mut self, piece: usize, builder: &MeshPiece) {
        if self.shared_origin != Some(builder.origin) {
            self.shared_origin = None;
        }
        for axis in 0..3 {
            self.min[axis] = self.min[axis].min(builder.min[axis]);
            self.max[axis] = self.max[axis].max(builder.max[axis]);
        }
        self.batch_indices.push(builder.batch);
        self.vertices += builder.vertex_indices.len();
        self.slices.push(piece);
    }
}

/// Partition the mergeable mesh batches into merged chunks and pack them.
///
/// Pieces are produced in converted order (the order `converted` lists batches,
/// i.e. upload order) and appended to the current chunk while that keeps it
/// within `max_vertices`, the same `mirrored` pipeline and — for a mixed-origin
/// chunk — within `max_radius` of its AABB midpoint; otherwise the piece starts
/// a new chunk. Vertices and indices are concatenated in that converted order.
/// That order is *not* by itself the safety argument: the frame loop only
/// coalesces runs that are contiguous in the same chunk and emits them at the
/// position of the first member batch, so equal-depth ties are broken by
/// in-order rasterization exactly as the sequential draws were (see the frame
/// loop's contiguity rule). A batch larger than the cap is split into several
/// pieces (one run each). No empty chunk is ever produced.
///
/// Returns the chunks and, per input batch, its run list (`None` for a batch that
/// was not converted).
///
/// `keys[i]` is `batches[i]`'s renderer pick key, packed into every vertex it
/// contributes so the id pass and the flag tint can resolve a fragment.
pub(crate) fn build_mesh_chunks(
    batches: &[cad_scene::RenderBatch],
    converted: &[usize],
    keys: &[u32],
    max_vertices: usize,
    max_radius: f64,
) -> (Vec<MeshChunkData>, Vec<Option<Vec<MeshRun>>>) {
    debug_assert_eq!(batches.len(), keys.len());
    let cap = max_vertices.max(3);
    let mut pieces: Vec<MeshPiece> = Vec::new();
    for &index in converted {
        pieces.extend(mesh_pieces(&batches[index], index, cap));
    }

    let mut builders: Vec<MeshChunkBuilder> = Vec::new();
    let mut current: Option<MeshChunkBuilder> = None;
    for (pos, piece) in pieces.iter().enumerate() {
        if let Some(chunk) = current.as_mut() {
            if chunk.accepts(piece, cap, max_radius) {
                chunk.push(pos, piece);
                continue;
            }
        }
        if let Some(chunk) = current.take() {
            builders.push(chunk);
        }
        current = Some(MeshChunkBuilder::new(pos, piece));
    }
    if let Some(chunk) = current.take() {
        builders.push(chunk);
    }

    let mut runs: Vec<Option<Vec<MeshRun>>> = vec![None; batches.len()];
    let mut chunks = Vec::with_capacity(builders.len());
    for (chunk_index, builder) in builders.iter().enumerate() {
        let refs: Vec<&cad_scene::RenderBatch> = builder
            .batch_indices
            .iter()
            .map(|&index| &batches[index])
            .collect();
        let origin = choose_instance_origin(&refs);
        let mut vertices: Vec<MeshVertex> = Vec::with_capacity(builder.vertices);
        let mut indices: Vec<u32> = Vec::new();
        for &pos in &builder.slices {
            let piece = &pieces[pos];
            let batch = &batches[piece.batch];
            let vertex_base = vertices.len() as u32;
            let index_offset = indices.len() as u32;
            push_mesh_vertices(&mut vertices, batch, piece, origin, keys[piece.batch]);
            for tri in &piece.triangles {
                indices.push(tri[0] + vertex_base);
                indices.push(tri[1] + vertex_base);
                indices.push(tri[2] + vertex_base);
            }
            runs[piece.batch]
                .get_or_insert_with(Vec::new)
                .push(MeshRun {
                    chunk: chunk_index as u32,
                    index_offset,
                    index_count: (piece.triangles.len() * 3) as u32,
                    vertex_count: piece.vertex_indices.len() as u32,
                });
        }
        chunks.push(MeshChunkData {
            origin,
            mirrored: builder.mirrored,
            vertices,
            indices,
            // Retained from the builder (no per-frame computation). An oversized
            // source split across chunks carries the whole-batch box in every
            // piece; a chunk mixing distant batches has a large box that never
            // culls — both conservative.
            aabb: WorldAabb {
                min: builder.min,
                max: builder.max,
            },
        });
    }
    (chunks, runs)
}

/// Append one [`MeshVertex`] per vertex of `piece`, in first-use order.
///
/// Positions are stored as the batch-local `f32` value when the batch's origin
/// is the chunk `origin` (exact), and otherwise as `world - origin` reduced in
/// `f64` before narrowing, mirroring [`push_line_instances`]. The colour is the
/// sanitized per-vertex colour multiplied by the sanitized batch tint, so the
/// merged draw's uniform tint can stay opaque white. `entity_flags` carries the
/// batch's renderer pick key and its [`INSTANCE_FLAG_*`] bits.
pub(crate) fn push_mesh_vertices(
    out: &mut Vec<MeshVertex>,
    batch: &cad_scene::RenderBatch,
    piece: &MeshPiece,
    origin: [f64; 3],
    pick_key: u32,
) {
    let normals = geometry::repaired_normals(batch);
    let colors = geometry::repaired_colors(batch);
    let tint = renderer_color(batch.color);
    let flags = batch_instance_flags(batch);
    let o = [
        batch.local_origin.x,
        batch.local_origin.y,
        batch.local_origin.z,
    ];
    let exact = o == origin;
    for &v in &piece.vertex_indices {
        let index = v as usize;
        let position = batch.vertices[index];
        let stored = if exact {
            position
        } else {
            [
                (o[0] + f64::from(position[0]) - origin[0]) as f32,
                (o[1] + f64::from(position[1]) - origin[1]) as f32,
                (o[2] + f64::from(position[2]) - origin[2]) as f32,
            ]
        };
        let vertex_color = colors[index];
        out.push(MeshVertex {
            position: stored,
            normal: normals[index],
            color: [
                tint[0] * vertex_color[0],
                tint[1] * vertex_color[1],
                tint[2] * vertex_color[2],
            ],
            entity_flags: [pick_key, flags],
        });
    }
}

#[cfg(test)]
mod tangent_frame_tests {
    //! Contracts for the *robustness* rules of [`generate_tangents`]: a
    //! UV-degenerate triangle is skipped instead of costing the batch its frame,
    //! a cancelled accumulation falls back to a real triangle tangent, and a
    //! frame with no usable gradient at all is still refused (never invented).

    use super::*;

    type Streams = (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<[u32; 3]>);

    /// Unit quad in the XY plane, `+Z` normals, full `[0,1]²` UV square.
    fn quad() -> Streams {
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

    fn assert_frames_are_orthonormal(tangents: &[[f32; 4]], normals: &[[f32; 3]]) {
        for (tangent, normal) in tangents.iter().zip(normals) {
            let length = tangent_length([tangent[0], tangent[1], tangent[2]]);
            assert!(close(length, 1.0), "tangent must be unit: {tangent:?}");
            let dot = tangent_dot([tangent[0], tangent[1], tangent[2]], *normal);
            assert!(dot.abs() < 1e-4, "tangent must be ⟂ normal: {tangent:?}");
            assert!(tangent[3] == 1.0 || tangent[3] == -1.0);
        }
    }

    #[test]
    fn a_degenerate_uv_triangle_is_skipped_and_its_vertex_inherits_the_coincident_frame() {
        let (vertices, normals, uvs, indices) = quad();
        let control = generate_tangents(&vertices, &normals, &uvs, &indices)
            .expect("the control quad has a frame");

        // Append one vertex at the *same* position as vertex 0 with vertex 0's
        // UV, so triangle `[4, 0, 1]` has two identical UVs: zero UV area, no
        // gradient. It must not cost the batch its frame.
        let mut vertices = vertices;
        let mut normals = normals;
        let mut uvs = uvs;
        let mut indices = indices;
        vertices.push(vertices[0]);
        normals.push(normals[0]);
        uvs.push(uvs[0]);
        indices.push([4, 0, 1]);
        let tangents = generate_tangents(&vertices, &normals, &uvs, &indices)
            .expect("one UV-degenerate triangle must not disable the whole frame");
        assert_eq!(tangents.len(), vertices.len());
        // The valid triangles' frames are bit-for-bit the control's.
        for (index, (with, without)) in tangents.iter().zip(&control).enumerate().take(4) {
            assert_eq!(with, without, "vertex {index} must be unaffected");
        }
        // The duplicate vertex inherits the coincident vertex's real tangent.
        assert!(close(tangents[4][0], 1.0), "{:?}", tangents[4]);
        assert_eq!(tangents[4][3], 1.0);
        assert_frames_are_orthonormal(&tangents, &normals);
    }

    #[test]
    fn a_cancelled_accumulation_falls_back_to_a_real_triangle_tangent() {
        // Vertex 0 belongs to two triangles whose UV gradients point at +X and
        // -X with equal magnitude, so the area-weighted sum cancels exactly.
        let vertices = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, -1.0, 0.0],
        ];
        let normals = vec![[0.0, 0.0, 1.0]; 5];
        let uvs = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 0.0], [0.0, 1.0]];
        let indices = vec![[0, 1, 2], [0, 3, 4]];
        let tangents = generate_tangents(&vertices, &normals, &uvs, &indices)
            .expect("a cancellation must fall back, not disable the frame");
        // The fallback is the strongest *real* triangle tangent (+X), not an
        // arbitrary direction in the tangent plane.
        assert!(close(tangents[0][0].abs(), 1.0), "{:?}", tangents[0]);
        assert!(close(tangents[0][1], 0.0), "{:?}", tangents[0]);
        assert_frames_are_orthonormal(&tangents, &normals);
    }

    #[test]
    fn a_frame_with_no_usable_gradient_is_still_refused() {
        let (vertices, normals, uvs, indices) = quad();
        // Every triangle UV-degenerate and every position distinct: nothing to
        // derive and nobody to inherit from.
        assert!(generate_tangents(&vertices, &normals, &[[0.5, 0.5]; 4], &indices).is_none());
        // Structural mismatches are refused, never repaired.
        assert!(generate_tangents(&vertices, &normals, &uvs, &[]).is_none());
        assert!(generate_tangents(&vertices, &normals, &uvs[..3], &indices).is_none());
        assert!(generate_tangents(&vertices, &normals, &uvs, &[[0, 1, 9]]).is_none());
        let mut zero_normals = normals.clone();
        zero_normals[0] = [0.0, 0.0, 0.0];
        assert!(generate_tangents(&vertices, &zero_normals, &uvs, &indices).is_none());
    }
}
