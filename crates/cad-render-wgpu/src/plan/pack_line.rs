use super::*;

/// Byte stride of one [`LineInstance`]. Two `vec4<f32>` (`p0`+width, `p1`+pad),
/// one `vec4<f32>` tint and one `vec2<u32>` pick key.
pub(crate) const LINE_INSTANCE_STRIDE: u32 = 56;

/// Default upper bound on instances per stream buffer. `1 << 20` instances is
/// 56 MiB, well inside every backend's `max_buffer_size`, and keeps a stream
/// draw's instance range legal. The bound is enforced **within** a batch too: a
/// single batch with more segments is split across several streams (and so
/// several draws), never uploaded as one oversized buffer. The value is a
/// `Renderer` field (injectable for tests); this is the default.
pub(crate) const MAX_INSTANCES_PER_STREAM: usize = 1 << 20;

/// Maximum distance (mm) a stored instance position may be from its stream's
/// origin. Each instance coordinate is stored as `(world - origin) as f32`, so
/// this bounds the f32 ulp of a stored coordinate: at `2^13` mm the ulp is
/// `2^-10` mm ≈ 0.98 µm, i.e. below the ~1 µm absolute precision floor the
/// `f64` fixed-point-relative contract is written to. Chunks split when their
/// world AABB grows past this radius, so stream positions stay small exactly
/// like the legacy per-batch path (which stores batch-local coordinates).
pub(crate) const MAX_STREAM_RADIUS: f64 = 8192.0;

/// Reserved instance flags, packed in the second component of
/// [`LineInstance::id_flags`] / [`MeshVertex::entity_flags`] and in
/// `Camera::key_flags.y` of the per-batch uniform. This slice sets
/// [`INSTANCE_FLAG_OPAQUE`], [`INSTANCE_FLAG_HAS_SOURCE`] and
/// [`INSTANCE_FLAG_LINEWEIGHT`] from the batch's own fields; they are
/// descriptive metadata (the selection bits live in the separate flags table,
/// see `FLAG_SELECTED`), never a pick key.
pub(crate) const INSTANCE_FLAG_OPAQUE: u32 = 1 << 0;

pub(crate) const INSTANCE_FLAG_HAS_SOURCE: u32 = 1 << 1;

pub(crate) const INSTANCE_FLAG_LINEWEIGHT: u32 = 1 << 2;

/// The [`INSTANCE_FLAG_*`] bitfield describing one scene batch.
///
/// Shared by the instanced/merged packers and the per-batch uniform, so the
/// three carriers cannot disagree.
pub(crate) fn batch_instance_flags(batch: &cad_scene::RenderBatch) -> u32 {
    INSTANCE_FLAG_OPAQUE
        | if batch.sources.is_empty() {
            0
        } else {
            INSTANCE_FLAG_HAS_SOURCE
        }
        | if batch.lineweight > 0.0 {
            INSTANCE_FLAG_LINEWEIGHT
        } else {
            0
        }
}

/// One instanced line segment (a single 2-vertex `LineList` pair).
///
/// `#[repr(C)]` + `Pod` so the packed bytes are exactly the GPU layout; a unit
/// test fences the stride against `size_of`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct LineInstance {
    /// `xyz` = segment start in the stream's fixed-point-relative world space,
    /// `w` = requested lineweight mm (carried, never rasterized).
    pub(crate) p0_width: [f32; 4],
    /// `xyz` = segment end in the same space, `w` reserved (0).
    pub(crate) p1_pad: [f32; 4],
    /// Sanitised linear sRGB colour (`rgb`) and clamped alpha (`a`).
    pub(crate) rgba: [f32; 4],
    /// `[0]` = the batch's renderer **pick key** (see [`PickTable`]; `0` = none);
    /// `[1]` = the batch's [`INSTANCE_FLAG_*`] bits. The key is deliberately a
    /// renderer key rather than a truncated `EntityId`, so the CPU-side table can
    /// preserve the full `u128` identity and report truncation collisions instead
    /// of mis-picking.
    pub(crate) id_flags: [u32; 2],
}

/// The exact packed bytes of `instances` (one contiguous GPU buffer payload).
pub(crate) fn line_instance_bytes(instances: &[LineInstance]) -> Vec<u8> {
    bytemuck::cast_slice(instances).to_vec()
}

/// Per-instance vertex-buffer layout (slot 1 of the instanced line pipeline).
///
/// `shader_location` 1..=4 so the shared unit segment keeps `@location(0)`.
pub(crate) fn line_instance_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
        1 => Float32x4,
        2 => Float32x4,
        3 => Float32x4,
        4 => Uint32x2
    ];
    wgpu::VertexBufferLayout {
        array_stride: u64::from(LINE_INSTANCE_STRIDE),
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &ATTRIBUTES,
    }
}

/// Per-instance layout of the **accelerated** line buffer: the same attributes at
/// the same offsets, but a 64-byte stride so the same buffer is also a legal
/// `array<PulledLine>` storage binding (WGSL rounds the 56-byte struct up to its
/// 16-byte alignment). The extra 8 bytes are the per-chunk index the pulled
/// shader reads; the universal shaders ignore them.
pub(crate) fn line_instance_layout_64() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
        1 => Float32x4,
        2 => Float32x4,
        3 => Float32x4,
        4 => Uint32x2
    ];
    wgpu::VertexBufferLayout {
        array_stride: u64::from(PULLED_LINE_STRIDE),
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &ATTRIBUTES,
    }
}

/// Byte stride of one [`LineQuadVertex`]: two `vec3<f32>` (`p0`, `p1`) plus one
/// `vec2<f32>` (quad corner).
pub(crate) const LINE_QUAD_STRIDE: u32 = 32;

/// One vertex of a pre-expanded anti-aliased line quad (the legacy per-batch
/// line path). One 2-vertex segment expands to six of these (two triangles); the
/// vertex stage rebuilds the world-space quad and the fragment stage computes the
/// distance to the segment — the same SDF the instanced/pulled paths use.
///
/// `#[repr(C)]` + `Pod` so the packed bytes are exactly the GPU layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct LineQuadVertex {
    /// `xyz` = segment start (batch-local, the same values the instanced path
    /// stores relative to its stream origin).
    pub(crate) p0: [f32; 3],
    /// `xyz` = segment end (batch-local).
    pub(crate) p1: [f32; 3],
    /// Quad corner in `[0, 1]^2`.
    pub(crate) corner: [f32; 2],
}

/// The shared unit quad's six corners (two triangles), matching the shared
/// `unit_quad_buffer` and the constant table in `line_pulled.wgsl`.
const LINE_QUAD_CORNERS: [[f32; 2]; 6] = [
    [0.0, 0.0],
    [1.0, 0.0],
    [1.0, 1.0],
    [0.0, 0.0],
    [1.0, 1.0],
    [0.0, 1.0],
];

/// Per-vertex layout of the pre-expanded line-quad buffer (slot 0 of the legacy
/// line pipeline). `shader_location` 0..=2, matching `line.wgsl` / `pick_line.wgsl`.
pub(crate) fn line_quad_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![
        0 => Float32x3,
        1 => Float32x3,
        2 => Float32x2
    ];
    wgpu::VertexBufferLayout {
        array_stride: u64::from(LINE_QUAD_STRIDE),
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRIBUTES,
    }
}

/// The exact packed bytes of `vertices` (one contiguous GPU buffer payload).
pub(crate) fn line_quad_bytes(vertices: &[LineQuadVertex]) -> Vec<u8> {
    bytemuck::cast_slice(vertices).to_vec()
}

/// Expand one batch's line geometry into anti-aliased quad vertices.
///
/// With `indices = None` the segments are consecutive pairs of `batch.vertices`
/// (a `Lines`/`MeshEdges` batch). With `indices = Some(list)` each pair is
/// `(list[2k], list[2k + 1])` indexed into `batch.vertices` (a mesh edge
/// overlay); the caller validates the indices are in range.
///
/// The pairing rule is exactly [`push_line_instances`]'s, so the legacy per-batch
/// path and the instanced path rasterize the same segments (conversion-path
/// equivalence). A trailing unpaired vertex is dropped by both paths.
pub(crate) fn expand_line_quads(
    out: &mut Vec<LineQuadVertex>,
    batch: &cad_scene::RenderBatch,
    indices: Option<&[u32]>,
) {
    let push_pair = |out: &mut Vec<LineQuadVertex>, a: [f32; 3], b: [f32; 3]| {
        for corner in LINE_QUAD_CORNERS {
            out.push(LineQuadVertex {
                p0: a,
                p1: b,
                corner,
            });
        }
    };
    match indices {
        None => {
            for pair in batch.vertices.chunks_exact(2) {
                push_pair(out, pair[0], pair[1]);
            }
        }
        Some(list) => {
            for pair in list.chunks_exact(2) {
                push_pair(
                    out,
                    batch.vertices[pair[0] as usize],
                    batch.vertices[pair[1] as usize],
                );
            }
        }
    }
}

/// Whether a scene batch is eligible for the instanced line path.
///
/// Opaque `Lines` **and** `MeshEdges` batches with a whole number of 2-vertex
/// segments are converted (an odd trailing vertex stays legacy so the pairing
/// rule is never reinterpreted; it is counted as a `line_odd_vertex_skip`).
/// Transparent batches stay legacy (their back-to-front order is camera-dependent
/// and is drawn per-batch); the legacy per-batch path rasterizes the same
/// anti-aliased quad strokes (`plan::expand_line_quads`), so every line class
/// carries identical AA ink either way.
///
/// `MeshEdges` used to be a documented silent no-op (the legacy `draw_batch`
/// read `GpuBatch::indices`, which is empty for `MeshEdges`). It is now converted
/// here — and its legacy fallback renders the same AA quads — so this topology
/// finally draws (backlog item G3).
pub(crate) fn line_batch_is_instanced(batch: &cad_scene::RenderBatch) -> bool {
    matches!(
        batch.topology,
        RenderTopology::Lines | RenderTopology::MeshEdges
    ) && batch.vertices.len() >= 2
        && batch.vertices.len() % 2 == 0
        && classify_alpha(batch.alpha) == AlphaClass::Opaque
}

/// One batch converted to the instanced line path, plus how its 2-vertex
/// segments are derived.
///
/// `Lines` batches contribute consecutive pairs of `batch.vertices`; `MeshEdges`
/// batches and mesh edge overlays contribute explicit index pairs into
/// `batch.vertices`. Either way the packed instance is a plain 2-vertex segment,
/// so the coalesced run reproduces the legacy `LineList` draw exactly.
pub(crate) struct ConvertedLine {
    /// Index into the full batch slice.
    pub(crate) batch: usize,
    /// Number of 2-vertex segments this batch contributes.
    pub(crate) segments: u32,
    /// Explicit index list (exactly `2 * segments` entries) into
    /// `batch.vertices`; `None` means consecutive pairs of `batch.vertices`.
    pub(crate) indices: Option<Vec<u32>>,
}

/// The edge-overlay index list of an opaque `Mesh` batch converted to the
/// instanced line path, or `None` when the batch has no overlay.
///
/// A mergeable opaque `Mesh` batch always instantiates its wireframe overlay when
/// it carries one: the triangles go to a merged mesh chunk and the overlay to an
/// instance stream, so no legacy per-batch draw remains for it.
pub(crate) fn overlay_is_instanced(batch: &cad_scene::RenderBatch) -> Option<Vec<u32>> {
    if batch.topology != RenderTopology::Mesh || batch.edges.is_empty() {
        return None;
    }
    if classify_alpha(batch.alpha) != AlphaClass::Opaque {
        return None;
    }
    let edges = geometry::mesh_edge_indices(batch);
    // `sorted_edge_indices` emits whole `(a, b)` pairs by construction, so a
    // non-empty list is always even. A merged batch owns no per-batch vertex
    // buffer, so `draw_batch` early-returns for it: a malformed (odd) list could
    // *not* be routed back to the legacy overlay draw and would be silently
    // dropped. Assert the invariant rather than drop geometry, and convert only a
    // non-empty list.
    debug_assert_eq!(
        edges.len() % 2,
        0,
        "mesh edge index list must be whole (a, b) pairs"
    );
    (!edges.is_empty()).then_some(edges)
}

/// Build the instanced-line conversion list for a batch set.
///
/// `Lines` batches are converted by [`line_batch_is_instanced`]. When
/// `mesh_merging` is on, a mergeable `Mesh` batch's edge overlay is converted
/// too: its triangles go to a merged chunk, so the overlay cannot remain a legacy
/// per-batch draw without either duplicating the mesh buffers or dropping the
/// wireframe.
pub(crate) fn convert_line_batches(
    batches: &[cad_scene::RenderBatch],
    line_instancing: bool,
    mesh_merging: bool,
) -> Vec<ConvertedLine> {
    let mut converted = Vec::new();
    for (index, batch) in batches.iter().enumerate() {
        if line_instancing && line_batch_is_instanced(batch) {
            converted.push(ConvertedLine {
                batch: index,
                segments: (batch.vertices.len() / 2) as u32,
                indices: None,
            });
        } else if mesh_merging && mesh_batch_is_mergeable(batch) {
            if let Some(edges) = overlay_is_instanced(batch) {
                converted.push(ConvertedLine {
                    batch: index,
                    segments: (edges.len() / 2) as u32,
                    indices: Some(edges),
                });
            }
        }
    }
    converted
}

/// Fixed-point reference for one static instance stream (one chunk).
///
/// A chunk groups consecutive converted batches that are spatially close (see
/// [`build_instance_chunks`]); this picks the single `f64` reference the chunk's
/// stored positions are measured from. When every batch in the chunk shares one
/// `local_origin`, that origin is used verbatim: each batch then stores its own
/// `f32` vertex values unchanged (see [`push_line_instances`]), so the
/// instanced stream reproduces the legacy per-batch path. Otherwise (mixed
/// origins) the `f64` midpoint of the chunk's world-space AABB is used, so
/// stream-relative positions stay small. On the **orthographic** path the
/// camera-relative term is still subtracted in `f64` on the CPU when the
/// per-frame matrix is built (see `FrameProjection::batch_matrix_f64`); the
/// **perspective** path narrows the origin to `f32` before `translate_left`, so
/// that `f64` subtraction claim is Ortho-only.
///
/// Byte-identity is only claimed for an `f32`-representable `local_origin`: the
/// legacy path narrows `local_origin` to `f32` before building its matrix, while
/// this keeps the exact `f64` origin (strictly better, not identical).
pub(crate) fn choose_instance_origin(batches: &[&cad_scene::RenderBatch]) -> [f64; 3] {
    let Some(first) = batches.first() else {
        return [0.0; 3];
    };
    if batches
        .iter()
        .all(|batch| batch.local_origin == first.local_origin)
    {
        return [
            first.local_origin.x,
            first.local_origin.y,
            first.local_origin.z,
        ];
    }
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for batch in batches {
        let o = [
            batch.local_origin.x,
            batch.local_origin.y,
            batch.local_origin.z,
        ];
        for vertex in &batch.vertices {
            for axis in 0..3 {
                let world = o[axis] + f64::from(vertex[axis]);
                min[axis] = min[axis].min(world);
                max[axis] = max[axis].max(world);
            }
        }
    }
    [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ]
}

/// World-space AABB of one batch (`local_origin + vertex`), as `(min, max)`.
pub(super) fn batch_world_aabb(batch: &cad_scene::RenderBatch) -> ([f64; 3], [f64; 3]) {
    let o = [
        batch.local_origin.x,
        batch.local_origin.y,
        batch.local_origin.z,
    ];
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for vertex in &batch.vertices {
        for axis in 0..3 {
            let world = o[axis] + f64::from(vertex[axis]);
            min[axis] = min[axis].min(world);
            max[axis] = max[axis].max(world);
        }
    }
    (min, max)
}

/// Half the AABB diagonal: the radius of the sphere about the AABB midpoint
/// that contains the whole box, and so the largest `|world - midpoint|`.
pub(super) fn aabb_half_diagonal(min: [f64; 3], max: [f64; 3]) -> f64 {
    let dx = max[0] - min[0];
    let dy = max[1] - min[1];
    let dz = max[2] - min[2];
    0.5 * (dx * dx + dy * dy + dz * dz).sqrt()
}

/// One memory- and precision-bounded static instance stream, ready to upload.
pub(crate) struct InstanceChunk {
    /// The stream's fixed-point reference ([`choose_instance_origin`]).
    pub(crate) origin: [f64; 3],
    /// Packed instances, in original batch order.
    pub(crate) instances: Vec<LineInstance>,
    /// World-space AABB of the chunk's segments, retained from the builder for
    /// per-frame view culling (see [`cull`]). A source split across several
    /// chunks carries the whole-batch AABB in every piece (conservative).
    pub(crate) aabb: WorldAabb,
}

/// A chunk under construction.
pub(super) struct ChunkBuilder {
    /// Distinct batch indices contributing to this chunk (for the origin).
    batch_indices: Vec<usize>,
    /// `Some(origin)` while every batch shares that exact `local_origin`;
    /// `None` once two differ (the chunk is then "mixed").
    shared_origin: Option<[f64; 3]>,
    min: [f64; 3],
    max: [f64; 3],
    instances: usize,
    /// `(converted position, first segment, segment count)` in append order.
    slices: Vec<(usize, u32, u32)>,
}

impl ChunkBuilder {
    pub(super) fn new(
        pos: usize,
        batch: usize,
        first_segment: u32,
        count: u32,
        origin: [f64; 3],
        min: [f64; 3],
        max: [f64; 3],
    ) -> Self {
        Self {
            batch_indices: vec![batch],
            shared_origin: Some(origin),
            min,
            max,
            instances: count as usize,
            slices: vec![(pos, first_segment, count)],
        }
    }

    /// Would adding `count` segments of a batch with this origin/AABB keep the
    /// chunk inside `cap` instances and — only when the result would be a
    /// mixed-origin chunk — inside `max_radius` of its midpoint? A shared-origin
    /// chunk stores batch-local coordinates, so it is never split on radius.
    pub(super) fn accepts(
        &self,
        count: u32,
        origin: [f64; 3],
        min: [f64; 3],
        max: [f64; 3],
        cap: usize,
        max_radius: f64,
    ) -> bool {
        if self.instances + count as usize > cap {
            return false;
        }
        if self.shared_origin == Some(origin) {
            return true;
        }
        let union_min = [
            self.min[0].min(min[0]),
            self.min[1].min(min[1]),
            self.min[2].min(min[2]),
        ];
        let union_max = [
            self.max[0].max(max[0]),
            self.max[1].max(max[1]),
            self.max[2].max(max[2]),
        ];
        aabb_half_diagonal(union_min, union_max) <= max_radius
    }

    pub(super) fn push(
        &mut self,
        pos: usize,
        batch: usize,
        count: u32,
        origin: [f64; 3],
        min: [f64; 3],
        max: [f64; 3],
    ) {
        if self.shared_origin != Some(origin) {
            self.shared_origin = None;
        }
        for axis in 0..3 {
            self.min[axis] = self.min[axis].min(min[axis]);
            self.max[axis] = self.max[axis].max(max[axis]);
        }
        self.batch_indices.push(batch);
        self.slices.push((pos, 0, count));
        self.instances += count as usize;
    }
}

/// Partition the converted line sources into instance-stream chunks and pack
/// them.
///
/// Chunks are built in converted order (original batch order among the converted
/// batches), so instance order preserves the draw order and adjacent runs stay
/// coalescible, but each chunk gets its own [`choose_instance_origin`] reference.
/// A source joins the current chunk only while that keeps the chunk within
/// `max_instances` segments and — for a mixed-origin chunk — within `max_radius`
/// of its AABB midpoint; otherwise it starts a new chunk. A single source with
/// more than `max_instances` segments is itself split across several chunks (one
/// run per piece). No empty chunk is ever produced.
///
/// Returns the chunks (each with its chosen origin and packed instances) and,
/// per input batch, its run list (`None` for a batch that was not converted).
///
/// `keys[i]` is `batches[i]`'s renderer pick key, packed into every instance it
/// contributes.
pub(crate) fn build_instance_chunks(
    batches: &[cad_scene::RenderBatch],
    converted: &[ConvertedLine],
    keys: &[u32],
    max_instances: usize,
    max_radius: f64,
) -> (Vec<InstanceChunk>, Vec<Option<Vec<InstanceRun>>>) {
    debug_assert_eq!(batches.len(), keys.len());
    let cap = max_instances.max(1);
    let mut builders: Vec<ChunkBuilder> = Vec::new();
    let mut current: Option<ChunkBuilder> = None;

    for (pos, source) in converted.iter().enumerate() {
        let index = source.batch;
        let batch = &batches[index];
        let count = source.segments;
        let origin = [
            batch.local_origin.x,
            batch.local_origin.y,
            batch.local_origin.z,
        ];
        let (min, max) = batch_world_aabb(batch);

        if count as usize > cap {
            // A single source beyond the cap: split it into dedicated chunks of
            // at most `cap` segments, so no stream is oversized. Each piece is a
            // separate run of the same batch.
            if let Some(chunk) = current.take() {
                builders.push(chunk);
            }
            let mut first = 0u32;
            while first < count {
                let take = (count - first).min(cap as u32);
                builders.push(ChunkBuilder::new(pos, index, first, take, origin, min, max));
                first += take;
            }
            continue;
        }

        if let Some(chunk) = current.as_mut() {
            if chunk.accepts(count, origin, min, max, cap, max_radius) {
                chunk.push(pos, index, count, origin, min, max);
                continue;
            }
        }
        if let Some(chunk) = current.take() {
            builders.push(chunk);
        }
        current = Some(ChunkBuilder::new(pos, index, 0, count, origin, min, max));
    }
    if let Some(chunk) = current.take() {
        builders.push(chunk);
    }

    let mut runs: Vec<Option<Vec<InstanceRun>>> = vec![None; batches.len()];
    let mut chunks = Vec::with_capacity(builders.len());
    for (stream, chunk) in builders.iter().enumerate() {
        let refs: Vec<&cad_scene::RenderBatch> = chunk
            .batch_indices
            .iter()
            .map(|&index| &batches[index])
            .collect();
        let origin = choose_instance_origin(&refs);
        let mut instances = Vec::with_capacity(chunk.instances);
        for &(pos, first, count) in &chunk.slices {
            let source = &converted[pos];
            let offset = instances.len() as u32;
            push_line_instances(
                &mut instances,
                &batches[source.batch],
                source.indices.as_deref(),
                origin,
                first,
                count,
                keys[source.batch],
            );
            runs[source.batch]
                .get_or_insert_with(Vec::new)
                .push(InstanceRun {
                    stream: stream as u32,
                    offset,
                    count,
                });
        }
        chunks.push(InstanceChunk {
            origin,
            instances,
            aabb: WorldAabb {
                min: chunk.min,
                max: chunk.max,
            },
        });
    }
    (chunks, runs)
}

/// Append one [`LineInstance`] per 2-vertex pair in the segment range
/// `[first_segment, first_segment + segment_count)` of a batch's line geometry.
///
/// With `indices = None` the pairs are consecutive entries of `batch.vertices`
/// (a `Lines` batch). With `indices = Some(list)` each pair is
/// `(list[2k], list[2k + 1])`, indexed into `batch.vertices` (a `MeshEdges` batch
/// or a mesh edge overlay); the caller validates the indices are in range.
///
/// A vertex is stored as the batch-local `f32` value when the batch's origin is
/// the stream `origin` (exact), and otherwise as `world - origin` reduced in
/// `f64` before narrowing to `f32`. Either way the camera-relative subtraction
/// never happens in the shader at large magnitudes. `origin` is the chunk's
/// [`choose_instance_origin`]. The range lets a batch larger than a stream's
/// instance cap be split across chunks. `pick_key` is the batch's renderer pick
/// key, packed per instance so the id pass and the flag tint can resolve a
/// fragment without a per-batch draw.
pub(crate) fn push_line_instances(
    out: &mut Vec<LineInstance>,
    batch: &cad_scene::RenderBatch,
    indices: Option<&[u32]>,
    origin: [f64; 3],
    first_segment: u32,
    segment_count: u32,
    pick_key: u32,
) {
    let [r, g, b] = renderer_color(batch.color);
    let rgba = [r, g, b, clamp_alpha(batch.alpha)];
    let width = if batch.lineweight.is_finite() {
        batch.lineweight.max(0.0)
    } else {
        0.0
    };
    let flags = batch_instance_flags(batch);
    let o = [
        batch.local_origin.x,
        batch.local_origin.y,
        batch.local_origin.z,
    ];
    // When the batch's own origin is the stream origin, the batch-local `f32`
    // vertex is already the stream-relative position and is stored verbatim
    // (`(o + v - o)` would round at large `o` for sub-ULP `v`). Otherwise the
    // world position is reduced against the stream origin in `f64` before
    // narrowing, so the camera-relative subtraction never happens in the shader.
    let exact = o == origin;
    let position = |axis: usize, v: f32| -> f32 {
        if exact {
            v
        } else {
            (o[axis] + f64::from(v) - origin[axis]) as f32
        }
    };
    let start = first_segment as usize * 2;
    let end = start + segment_count as usize * 2;
    let push_pair = |out: &mut Vec<LineInstance>, a: [f32; 3], b: [f32; 3]| {
        let p0 = [position(0, a[0]), position(1, a[1]), position(2, a[2])];
        let p1 = [position(0, b[0]), position(1, b[1]), position(2, b[2])];
        out.push(LineInstance {
            p0_width: [p0[0], p0[1], p0[2], width],
            p1_pad: [p1[0], p1[1], p1[2], 0.0],
            rgba,
            id_flags: [pick_key, flags],
        });
    };
    match indices {
        None => {
            for pair in batch.vertices[start..end].chunks_exact(2) {
                push_pair(out, pair[0], pair[1]);
            }
        }
        Some(list) => {
            for pair in list[start..end].chunks_exact(2) {
                push_pair(
                    out,
                    batch.vertices[pair[0] as usize],
                    batch.vertices[pair[1] as usize],
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Instanced SDF conic stream (slice B: analytic circles/arcs)
// ---------------------------------------------------------------------------
//
// An eligible circle/arc travels as a `RenderTopology::Conics` batch and is
// packed into a static instance stream exactly like the instanced line path: one
// `ArcInstance` (72 B) per conic, packed as **instance-rate vertex attributes**
// (WebGL2-safe, no vertex-stage storage buffer). The vertex shader expands a
// shared unit quad to the conic's full-circle extent; the fragment shader
// computes the signed distance and discards outside the anti-aliased band (see
// `shaders/arc.wgsl` and `docs/adr/0008-sdf-conic-rendering.md`).
