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
pub(crate) fn mesh_batch_is_mergeable(batch: &cad_scene::RenderBatch) -> bool {
    batch.topology == RenderTopology::Mesh
        && !batch.indices.is_empty()
        && classify_alpha(batch.alpha) == AlphaClass::Opaque
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
