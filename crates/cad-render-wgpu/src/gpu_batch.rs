//! The resident GPU batch record and its small accessors.
//!
//! Split out of `lib.rs` to keep the crate root within the file-size gate.

use super::*;

pub(crate) struct GpuBatch {
    /// Per-batch position vertex buffer (`None` when the batch was converted to
    /// an instanced line stream, which owns the geometry instead).
    pub(crate) vertices: Option<wgpu::Buffer>,
    pub(crate) vertex_count: u32,
    /// Per-vertex normals (mesh topology only); kept alive for binding.
    pub(crate) normals: Option<wgpu::Buffer>,
    /// Per-vertex colours (mesh topology only); kept alive for binding. Always
    /// bound for a mesh (white when the batch has no gradient), so the shader's
    /// `@location(2)` attribute is always provided.
    pub(crate) colors: Option<wgpu::Buffer>,
    /// Per-vertex UV0 (`@location(3)`) for a **textured** mesh batch; `None` for
    /// every untextured batch (only `mesh_textured` binds slot 3).
    pub(crate) uvs: Option<wgpu::Buffer>,
    /// The base-colour texture key; `Some` routes the draw through
    /// `mesh_textured`. A non-resident key falls back to the untextured pipeline
    /// and is surfaced, never bound to a placeholder texture.
    pub(crate) texture: Option<ResourceKey>,
    /// The declared PBR channel set; `Some` routes the draw through `mesh_pbr`
    /// (never the merged/pulled paths). `None` keeps the existing channels.
    pub(crate) pbr: Option<cad_scene::RenderPbr>,
    /// CPU-generated tangent stream (`Float32x4`, slot 4) for a PBR batch whose
    /// normal map needs a frame. `None` when the material needs none, or when the
    /// UVs are degenerate and the channel was disabled instead of inventing one.
    pub(crate) tangents: Option<wgpu::Buffer>,
    /// `true` when [`Self::tangents`] is present, i.e. the normal-map channel can
    /// use a real tangent frame.
    pub(crate) has_tangents: bool,
    pub(crate) topology: RenderTopology,
    /// `true` for the text-glyph display class (indexed mesh, no edge overlay).
    pub(crate) text_glyph: bool,
    /// Triangle index buffer (mesh topology only).
    pub(crate) indices: Option<wgpu::Buffer>,
    pub(crate) index_count: u32,
    /// Pre-expanded anti-aliased line quads for this batch's line geometry: the
    /// `Lines`/`MeshEdges` line list, or a non-merged mesh's wireframe overlay.
    /// `None` when the batch owns no per-batch line geometry (a converted
    /// instance stream, or a mesh with no overlay).
    pub(crate) line_quads: Option<wgpu::Buffer>,
    /// Vertex count of [`GpuBatch::line_quads`] (six per segment).
    pub(crate) line_quad_count: u32,
    /// `true` when the batch's transform mirrors winding.
    pub(crate) mirrored: bool,
    /// Constant per-object alpha.
    pub(crate) alpha: f32,
    /// Constant per-object colour (normalized sRGB), sanitized at upload.
    pub(crate) color: [f32; 3],
    /// Requested lineweight in millimetres; carried but not drawn.
    pub(crate) lineweight: f32,
    /// Paint order relative to sibling batches (larger is later/on top). The
    /// frame's draw plan performs a stable sort on this key.
    pub(crate) draw_order: i64,
    /// World-space centroid, the transparent pass's back-to-front sort key.
    pub(crate) centroid: [f32; 3],
    pub(crate) origin: [f32; 3],
    /// Empty unless this batch's segments live in instanced line streams. The
    /// frame path draws each `[offset, offset + count)` instance slice in order
    /// (coalescing against adjacent slices in the same stream) instead of a
    /// per-batch draw. Usually one run; a batch larger than a stream's instance
    /// cap contributes one run per chunk it was split across.
    pub(crate) instanced: Vec<InstanceRun>,
    /// Empty unless this batch's triangles live in merged mesh chunks (see
    /// [`MeshChunkData`]). The frame path draws each `[index_offset,
    /// index_offset + index_count)` triangle range in order (coalescing against
    /// adjacent ranges in the same chunk) instead of a per-batch draw. Usually
    /// one run; a batch larger than the vertex cap contributes one run per piece.
    pub(crate) mesh_runs: Vec<MeshRun>,
    /// Empty unless this batch's conics live in SDF instance streams (see
    /// [`ArcChunk`]). The frame path draws each `[offset, offset + count)`
    /// instance slice in order (coalescing against adjacent slices in the same
    /// stream). Usually one run; a batch larger than the instance cap contributes
    /// one run per chunk.
    pub(crate) arc_runs: Vec<ArcRun>,
    /// Exact bytes this batch uploads, cached at upload time so the per-frame
    /// byte budget does not recompute the edge index list every frame.
    pub(crate) upload_bytes: usize,
    /// This batch's renderer pick key (see [`PickTable`]): `1 + <ordinal>`, never
    /// `0`. Packed into every instance/merged vertex the batch contributes and
    /// into `Camera::key_flags.x` of its per-batch uniform, so the id pass and
    /// the selection-flags table can resolve any fragment of it.
    pub(crate) pick_key: u32,
    /// World-space z range `[min, max]` of this batch's geometry, captured at
    /// upload. The orthographic frame derives its depth band from the union over
    /// resident batches (see [`plan::OrthoDepth`]), so a solid's world-z extent
    /// maps into the depth range instead of being clipped by it.
    pub(crate) z_range: [f32; 2],
    /// The full `SelectionRef` list this batch's fragments can belong to,
    /// preserved so the CPU-side pick table can return the whole `u128` identity
    /// (the scene builder may concatenate several entities into one batch).
    pub(crate) sources: Vec<SelectionRef>,
    /// GPU memory charge of this batch's per-vertex buffers (positions, normals,
    /// colours, anti-aliased line quads). `None` for a batch whose geometry lives
    /// in a shared instance stream / merged chunk / conic stream, which owns the
    /// charge instead. Released when the batch is dropped (see
    /// [`gpu_memory::GpuMemoryLedger`]).
    // Held only for its `Drop` impl, which releases the ledger charge.
    #[allow(dead_code)]
    pub(crate) vertex_charge: Option<GpuCharge>,
    /// GPU memory charge of this batch's triangle index buffer.
    // Held only for its `Drop` impl, which releases the ledger charge.
    #[allow(dead_code)]
    pub(crate) index_charge: Option<GpuCharge>,
}

impl GpuBatch {
    /// Exact bytes this batch uploads to the device.
    ///
    /// Positions/normals are 12 bytes per vertex, edge indices and the triangle
    /// index buffer are `u32` (4 bytes) each. This matches the byte count
    /// [`Renderer::upload`] accumulates, so the frame byte budget and the
    /// reported `FrameStats::uploaded_bytes` cannot disagree.
    pub(crate) fn upload_size_bytes(&self) -> usize {
        self.upload_bytes
    }

    /// The [`INSTANCE_FLAG_*`] bitfield of this resident batch, for
    /// `Camera::key_flags.y` of its per-batch uniform. Mirrors
    /// [`batch_instance_flags`] on the packed form (converted batches are always
    /// opaque, a legacy batch keeps its own alpha class).
    pub(crate) fn instance_flags(&self) -> u32 {
        (if classify_alpha(self.alpha) == AlphaClass::Opaque {
            INSTANCE_FLAG_OPAQUE
        } else {
            0
        }) | if self.sources.is_empty() {
            0
        } else {
            INSTANCE_FLAG_HAS_SOURCE
        } | if self.lineweight > 0.0 {
            INSTANCE_FLAG_LINEWEIGHT
        } else {
            0
        }
    }
}
