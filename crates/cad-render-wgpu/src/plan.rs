//! Batch planning, GPU packing and pipeline layout helpers.

use super::*;
use std::collections::HashSet;

pub(crate) struct GpuPlan {
    pub(crate) accepted: Vec<usize>,
    pub(crate) usage: cad_scene::FrameUsage,
    pub(crate) over_budget: Option<OverBudgetReason>,
}

/// The ordered, budget-accepted batch indices split into the two draw passes.
pub(crate) struct DrawPasses {
    pub(crate) opaque: Vec<usize>,
    pub(crate) transparent: Vec<usize>,
    pub(crate) invisible: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MeshDrawPlan {
    triangles: bool,
    edge_overlay: bool,
}

fn mesh_draw_plan(has_triangles: bool, has_edge_indices: bool) -> MeshDrawPlan {
    MeshDrawPlan {
        triangles: has_triangles,
        edge_overlay: has_triangles && has_edge_indices,
    }
}

/// Turn the pure [`DrawOrderPlan`] over the accepted sub-set back into global
/// batch indices, preserving the plan's order.
///
/// `accepted` are the global indices the budget kept (upload order); `entries`
/// are their ordering keys in the same order. Splitting this out keeps the index
/// remapping testable without a GPU.
pub(crate) fn draw_passes(
    accepted: &[usize],
    entries: &[BatchOrderEntry],
    camera_position: Option<[f32; 3]>,
) -> DrawPasses {
    debug_assert_eq!(accepted.len(), entries.len());
    let order = plan_draw_order(entries, camera_position);
    let remap = |positions: &[usize]| -> Vec<usize> {
        positions.iter().map(|&pos| accepted[pos]).collect()
    };
    DrawPasses {
        opaque: remap(&order.opaque),
        transparent: remap(&order.transparent),
        invisible: order.invisible.len(),
    }
}

/// Bytes of the per-batch uniform struct: `mat4x4<f32>` (64) + `vec4<f32>` (16)
/// in `shaders/{line,mesh,image}.wgsl`. The shaders are unchanged, so this is
/// fixed at 80.
pub(crate) const UNIFORM_STRUCT_BYTES: u32 = 80;

/// Hard ceiling on slots per uniform page. `max_uniform_buffer_binding_size` is
/// only 64 KiB by default, but an adapter may report far more; this keeps the
/// page buffer bounded regardless.
pub(crate) const MAX_UNIFORM_SLOTS_PER_PAGE: usize = 16_384;

/// Derive `(slot_size, slots_per_page)` for the shared uniform arena.
///
/// `slot_size` is the uniform struct rounded up to the device's
/// `min_uniform_buffer_offset_alignment`, because consecutive dynamic offsets
/// must be aligned. A page's total bytes are kept within
/// `max_uniform_buffer_binding_size`, so every page is a legal uniform binding
/// even where the limit is the WebGPU default 64 KiB (256 slots at 256 bytes).
///
/// Pure so it is unit-testable without a GPU.
pub(crate) fn uniform_layout_params(limits: &wgpu::Limits) -> (u32, usize) {
    // Uniform structs are 16-byte aligned in WGSL; never go below that even if
    // the platform reports a smaller offset alignment.
    let alignment = limits.min_uniform_buffer_offset_alignment.max(16);
    let slot_size = UNIFORM_STRUCT_BYTES.div_ceil(alignment) * alignment;
    let slots = (limits.max_uniform_buffer_binding_size / u64::from(slot_size))
        .clamp(1, MAX_UNIFORM_SLOTS_PER_PAGE as u64) as usize;
    (slot_size, slots)
}

/// Pure `(page, byte offset)` for uniform `slot`.
///
/// Split out so the multi-page/multi-buffer offset selection is tested without
/// a GPU.
pub(crate) fn slot_location(slot: usize, slots_per_page: usize, slot_size: u32) -> (usize, u32) {
    (
        slot / slots_per_page,
        (slot % slots_per_page) as u32 * slot_size,
    )
}

/// Per-frame count of render-pass state-setting calls actually issued.
///
/// The counters are incremented at the call site (never derived from batch
/// topology), so a frame's encode work is measured rather than inferred. When
/// [`EncodeState`] elides a redundant call the counter does not move, which is
/// how state-change dedup is proven to have happened.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EncodeTally {
    pub(crate) set_pipeline_calls: u64,
    pub(crate) set_bind_group_calls: u64,
    pub(crate) set_vertex_buffer_calls: u64,
    pub(crate) set_index_buffer_calls: u64,
}

/// Render-pass encode state for one frame: the per-call tally plus the last
/// bound pipeline/bind group/vertex/index handles.
///
/// The handles let a batch skip a redundant state-setting call when the
/// requested state already equals what is bound. Draw order is never changed:
/// only calls whose state is *identical* to the bound state are elided, so the
/// rendered result is byte-identical.
///
/// Note: wgpu-core already elides redundant `set_pipeline`/`set_bind_group`
/// internally (its pass tracks the current pipeline and bind groups), so this
/// outer dedup mostly saves our own call overhead there; `set_vertex_buffer`
/// and `set_index_buffer` are **not** deduped by wgpu-core, so those are the
/// calls this layer genuinely removes.
pub(crate) struct EncodeState {
    pub(crate) tally: EncodeTally,
    last_pipeline: Option<wgpu::RenderPipeline>,
    last_bind_group0: Option<wgpu::BindGroup>,
    last_bind_offset0: u32,
    last_bind_group1: Option<wgpu::BindGroup>,
    last_vertex: [Option<wgpu::Buffer>; 3],
    last_index: Option<wgpu::Buffer>,
}

impl EncodeState {
    pub(crate) fn new() -> Self {
        EncodeState {
            tally: EncodeTally::default(),
            last_pipeline: None,
            last_bind_group0: None,
            last_bind_offset0: u32::MAX,
            last_bind_group1: None,
            last_vertex: [None, None, None],
            last_index: None,
        }
    }

    fn set_pipeline(&mut self, pass: &mut wgpu::RenderPass<'_>, pipeline: &wgpu::RenderPipeline) {
        // Skip when the identical pipeline is already bound. Draw order is not
        // changed; only a redundant state-setting call is elided.
        if self.last_pipeline.as_ref() == Some(pipeline) {
            return;
        }
        pass.set_pipeline(pipeline);
        self.last_pipeline = Some(pipeline.clone());
        self.tally.set_pipeline_calls += 1;
    }

    fn set_bind_group0(
        &mut self,
        pass: &mut wgpu::RenderPass<'_>,
        bind_group: &wgpu::BindGroup,
        offset: u32,
    ) {
        // Same bind group *and* dynamic offset: nothing to change.
        if self.last_bind_group0.as_ref() == Some(bind_group) && self.last_bind_offset0 == offset {
            return;
        }
        pass.set_bind_group(0, bind_group, &[offset]);
        self.last_bind_group0 = Some(bind_group.clone());
        self.last_bind_offset0 = offset;
        self.tally.set_bind_group_calls += 1;
    }

    fn set_bind_group1(&mut self, pass: &mut wgpu::RenderPass<'_>, bind_group: &wgpu::BindGroup) {
        if self.last_bind_group1.as_ref() == Some(bind_group) {
            return;
        }
        pass.set_bind_group(1, bind_group, &[]);
        self.last_bind_group1 = Some(bind_group.clone());
        self.tally.set_bind_group_calls += 1;
    }

    fn set_vertex_buffer(
        &mut self,
        pass: &mut wgpu::RenderPass<'_>,
        slot: u32,
        buffer: &wgpu::Buffer,
    ) {
        let slot_index = slot as usize;
        if self.last_vertex[slot_index].as_ref() == Some(buffer) {
            return;
        }
        pass.set_vertex_buffer(slot, buffer.slice(..));
        self.last_vertex[slot_index] = Some(buffer.clone());
        self.tally.set_vertex_buffer_calls += 1;
    }

    fn set_index_buffer(&mut self, pass: &mut wgpu::RenderPass<'_>, buffer: &wgpu::Buffer) {
        if self.last_index.as_ref() == Some(buffer) {
            return;
        }
        pass.set_index_buffer(buffer.slice(..), wgpu::IndexFormat::Uint32);
        self.last_index = Some(buffer.clone());
        self.tally.set_index_buffer_calls += 1;
    }
}

/// Submit one batch, dispatching on its topology.
///
/// `bind_group`/`uniform_offset` are group 0's shared uniform page and this
/// batch's dynamic slot offset (see [`super::UniformArena`]).
///
/// The caller chooses the mesh pipeline (opaque vs transparent, mirrored vs
/// normal); line and `MeshEdges` topologies always use the line pipeline. A mesh
/// batch also draws its optional wireframe overlay.
pub(crate) fn draw_batch(
    pass: &mut wgpu::RenderPass<'_>,
    state: &mut EncodeState,
    batch: &GpuBatch,
    bind_group: &wgpu::BindGroup,
    uniform_offset: u32,
    line_pipeline: &wgpu::RenderPipeline,
    mesh_pipeline: &wgpu::RenderPipeline,
) {
    // A batch converted to the instanced line path owns no per-batch vertex
    // buffer; it is never routed through `draw_batch` (the frame path coalesces
    // it into an instanced run instead). Legacy batches always carry one.
    let Some(vertices) = batch.vertices.as_ref() else {
        return;
    };
    match batch.topology {
        RenderTopology::Lines => {
            state.set_pipeline(pass, line_pipeline);
            state.set_bind_group0(pass, bind_group, uniform_offset);
            state.set_vertex_buffer(pass, 0, vertices);
            if let Some(indices) = &batch.indices {
                state.set_index_buffer(pass, indices);
                pass.draw_indexed(0..batch.index_count, 0, 0..1);
            } else {
                pass.draw(0..batch.vertex_count, 0..1);
            }
        }
        RenderTopology::Mesh => {
            state.set_pipeline(pass, mesh_pipeline);
            state.set_bind_group0(pass, bind_group, uniform_offset);
            state.set_vertex_buffer(pass, 0, vertices);
            if let Some(normals) = &batch.normals {
                state.set_vertex_buffer(pass, 1, normals);
            }
            if let Some(colors) = &batch.colors {
                state.set_vertex_buffer(pass, 2, colors);
            }
            let draw_plan = mesh_draw_plan(batch.indices.is_some(), batch.edge_indices.is_some());
            if !draw_plan.triangles {
                return;
            }
            let indices = batch
                .indices
                .as_ref()
                .expect("mesh draw plan requires triangle indices");
            state.set_index_buffer(pass, indices);
            pass.draw_indexed(0..batch.index_count, 0, 0..1);
            // Optional wireframe overlay over the shared mesh vertices.
            if draw_plan.edge_overlay {
                let edges = batch
                    .edge_indices
                    .as_ref()
                    .expect("draw plan requires edge indices");
                state.set_pipeline(pass, line_pipeline);
                state.set_vertex_buffer(pass, 0, vertices);
                state.set_index_buffer(pass, edges);
                pass.draw_indexed(0..batch.edge_index_count, 0, 0..1);
            }
        }
        RenderTopology::MeshEdges => {
            state.set_pipeline(pass, line_pipeline);
            state.set_bind_group0(pass, bind_group, uniform_offset);
            state.set_vertex_buffer(pass, 0, vertices);
            let Some(indices) = batch.indices.as_ref() else {
                return;
            };
            state.set_index_buffer(pass, indices);
            pass.draw_indexed(0..batch.index_count, 0, 0..1);
        }
    }
}

/// Submit one coalesced instanced line run: `2` unit vertices expanded once per
/// instance in `start..start + count` of `stream`.
///
/// `stream` is the instance buffer (slot 1); `unit` is the shared
/// `(0,0,0)-(1,0,0)` segment (slot 0). Instances are contiguous and in their
/// original upload order, so the run reproduces exactly the draw order the same
/// batches would have had as individual `LineList` draws.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_instance_run(
    pass: &mut wgpu::RenderPass<'_>,
    state: &mut EncodeState,
    unit: &wgpu::Buffer,
    stream: &wgpu::Buffer,
    bind_group: &wgpu::BindGroup,
    uniform_offset: u32,
    start: u32,
    count: u32,
    pipeline: &wgpu::RenderPipeline,
) {
    if count == 0 {
        return;
    }
    state.set_pipeline(pass, pipeline);
    state.set_bind_group0(pass, bind_group, uniform_offset);
    state.set_vertex_buffer(pass, 0, unit);
    state.set_vertex_buffer(pass, 1, stream);
    pass.draw(0..2, start..start + count);
}

/// Draw and clear a pending coalesced instanced run.
///
/// `pending` is `(stream index, first instance, one-past-last instance)`. Returns
/// whether a draw was issued (so the caller can count draw calls). Split out so
/// the frame path's three flush sites share one implementation.
#[allow(clippy::too_many_arguments)]
pub(crate) fn flush_pending_run(
    pass: &mut wgpu::RenderPass<'_>,
    state: &mut EncodeState,
    pending: &mut Option<(usize, u32, u32)>,
    uniforms: &UniformArena,
    stream_slots: &[u32],
    streams: &[InstanceStream],
    unit: &wgpu::Buffer,
    pipeline: &wgpu::RenderPipeline,
) -> bool {
    let Some((stream, start, end)) = pending.take() else {
        return false;
    };
    let (bind_group, offset) = uniforms.binding(stream_slots[stream] as usize);
    draw_instance_run(
        pass,
        state,
        unit,
        &streams[stream].buffer,
        bind_group,
        offset,
        start,
        end - start,
        pipeline,
    );
    true
}

/// Draw and clear a pending merged-mesh run.
///
/// `pending` is `(chunk index, first triangle index, one-past-last index)` into
/// that chunk's index buffer. The chunk's `mirrored` flag selects the pipeline
/// (a merged chunk never spans both). Returns whether a draw was issued.
#[allow(clippy::too_many_arguments)]
pub(crate) fn flush_pending_mesh(
    pass: &mut wgpu::RenderPass<'_>,
    state: &mut EncodeState,
    pending: &mut Option<(usize, u32, u32)>,
    uniforms: &UniformArena,
    chunk_slots: &[u32],
    chunks: &[GpuMeshChunk],
    pipeline: &wgpu::RenderPipeline,
    pipeline_mirrored: &wgpu::RenderPipeline,
) -> bool {
    let Some((chunk, start, end)) = pending.take() else {
        return false;
    };
    if end <= start {
        return false;
    }
    let mesh = &chunks[chunk];
    debug_assert!(
        end <= mesh.index_count,
        "merged mesh run must stay inside its chunk's index buffer"
    );
    let (bind_group, offset) = uniforms.binding(chunk_slots[chunk] as usize);
    state.set_pipeline(
        pass,
        if mesh.mirrored {
            pipeline_mirrored
        } else {
            pipeline
        },
    );
    state.set_bind_group0(pass, bind_group, offset);
    state.set_vertex_buffer(pass, 0, &mesh.vertex_buffer);
    state.set_index_buffer(pass, &mesh.index_buffer);
    pass.draw_indexed(start..end, 0, 0..1);
    true
}

pub(crate) fn plan_from_gpu(batches: &[GpuBatch], budget: &FrameBudget) -> GpuPlan {
    let mut usage = cad_scene::FrameUsage::default();
    let mut accepted = Vec::with_capacity(batches.len());
    for (i, batch) in batches.iter().enumerate() {
        let vertices = batch.vertex_count as usize;
        let triangles = if batch.topology == RenderTopology::Mesh {
            batch.index_count as usize / 3
        } else {
            0
        };
        // The GPU bytes actually submitted: position, normal and edge-index
        // buffers plus the triangle index buffer. Derived from the packed GpuBatch
        // so `upload_bytes_per_frame` is charged the same numbers the device sees.
        let bytes = batch.upload_size_bytes();
        let outcome = budget
            .charge_bytes(&mut usage, bytes)
            .and_then(|()| budget.charge(&mut usage, vertices, triangles));
        match outcome {
            Ok(()) => accepted.push(i),
            Err(exceeded) => {
                let report = OverBudget {
                    category: exceeded.category,
                    requested: exceeded.requested,
                    limit: exceeded.limit,
                    skipped_batches: batches.len() - accepted.len(),
                };
                return GpuPlan {
                    accepted,
                    usage,
                    over_budget: Some(OverBudgetReason {
                        reason: over_budget_reason(&report),
                        report,
                    }),
                };
            }
        }
    }
    GpuPlan {
        accepted,
        usage,
        over_budget: None,
    }
}

/// Budget one consecutive page of a globally ordered scene. Unlike the legacy
/// one-frame planner, callers retain the cursor and never discard the suffix.
pub(crate) fn plan_ordered_page(
    batches: &[GpuBatch],
    ordered: &[usize],
    cursor: usize,
    budget: &FrameBudget,
) -> GpuPlan {
    let mut usage = cad_scene::FrameUsage::default();
    let mut accepted = Vec::new();
    for &index in &ordered[cursor..] {
        let batch = &batches[index];
        let mut next = usage;
        let triangles = if batch.topology == RenderTopology::Mesh {
            batch.index_count as usize / 3
        } else {
            0
        };
        let result = budget
            .charge_bytes(&mut next, batch.upload_size_bytes())
            .and_then(|()| budget.charge(&mut next, batch.vertex_count as usize, triangles));
        if let Err(exceeded) = result {
            let report = OverBudget {
                category: exceeded.category,
                requested: exceeded.requested,
                limit: exceeded.limit,
                skipped_batches: ordered.len() - cursor - accepted.len(),
            };
            return GpuPlan {
                accepted,
                usage,
                over_budget: Some(OverBudgetReason {
                    reason: over_budget_reason(&report),
                    report,
                }),
            };
        }
        usage = next;
        accepted.push(index);
    }
    GpuPlan {
        accepted,
        usage,
        over_budget: None,
    }
}

pub(crate) fn over_budget_reason(report: &OverBudget) -> DiagnosticReason {
    DiagnosticReason::partial(
        codes::RENDER_FRAME_OVER_BUDGET,
        vec![
            DiagnosticParameter::Identifier(report.category.to_string()),
            DiagnosticParameter::Count(report.requested as u64),
            DiagnosticParameter::Limit(report.limit as u64),
            DiagnosticParameter::Count(report.skipped_batches as u64),
        ],
    )
}

pub(crate) fn pack_positions(vectors: &[[f32; 3]]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vectors.len() * 12);
    for v in vectors {
        bytes.extend_from_slice(&v[0].to_le_bytes());
        bytes.extend_from_slice(&v[1].to_le_bytes());
        bytes.extend_from_slice(&v[2].to_le_bytes());
    }
    bytes
}

/// Sanitise a batch colour for the uniform: finite `[0, 1]` channels.
///
/// Mirrors `cad_scene::sanitize_color` so a value that bypassed the scene cache
/// still cannot put a NaN into the shader.
pub(crate) fn renderer_color(color: [f32; 3]) -> [f32; 3] {
    let channel = |c: f32| {
        if c.is_finite() {
            c.clamp(0.0, 1.0)
        } else {
            1.0
        }
    };
    [channel(color[0]), channel(color[1]), channel(color[2])]
}

/// Aggregate the "lineweight carried but not drawn" batches into one stable
/// diagnostic reason. `None` when nothing asked for a non-zero weight.
pub(crate) fn lineweight_reason(not_drawn: &[LineweightNotDrawn]) -> Option<DiagnosticReason> {
    if not_drawn.is_empty() {
        return None;
    }
    let max_mm = not_drawn
        .iter()
        .map(|l| l.millimeters)
        .fold(0.0f32, f32::max);
    Some(DiagnosticReason::partial(
        codes::RENDER_LINEWEIGHT_NOT_DRAWN,
        vec![
            DiagnosticParameter::Count(not_drawn.len() as u64),
            DiagnosticParameter::Identifier(format!("{max_mm:.2}mm")),
        ],
    ))
}

pub(crate) fn translate_left(vp: &[[f32; 4]; 4], origin: [f32; 3]) -> [f32; 16] {
    // Column-major mat4x4 * translation(origin). Translation only affects the
    // last column: out[.][3] = vp[.][0]*ox + vp[.][1]*oy + vp[.][2]*oz + vp[.][3].
    let mut m = [0.0f32; 16];
    for col in 0..4 {
        for row in 0..4 {
            m[col * 4 + row] = vp[col][row];
        }
    }
    let ox = origin[0];
    let oy = origin[1];
    let oz = origin[2];
    for row in 0..4 {
        let base = vp[0][row] * ox + vp[1][row] * oy + vp[2][row] * oz + vp[3][row];
        m[12 + row] = base;
    }
    m
}

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
    /// `[0]` = low 32 bits of the *first* source's `EntityId` (a multi-source
    /// batch exposes only that one id here); `[1]` = reserved
    /// [`INSTANCE_FLAG_*`] bits. New per-vertex metadata for a later picking
    /// phase — not a replacement for a per-batch key, which the legacy draw never
    /// carried explicitly.
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
fn mesh_pieces(batch: &cad_scene::RenderBatch, batch_index: usize, cap: usize) -> Vec<MeshPiece> {
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
fn piece_world_aabb(
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
struct MeshChunkBuilder {
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
    fn new(piece: usize, builder: &MeshPiece) -> Self {
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
    fn accepts(&self, piece: &MeshPiece, cap: usize, max_radius: f64) -> bool {
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

    fn push(&mut self, piece: usize, builder: &MeshPiece) {
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
pub(crate) fn build_mesh_chunks(
    batches: &[cad_scene::RenderBatch],
    converted: &[usize],
    max_vertices: usize,
    max_radius: f64,
) -> (Vec<MeshChunkData>, Vec<Option<Vec<MeshRun>>>) {
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
            push_mesh_vertices(&mut vertices, batch, piece, origin);
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
/// batch's source id and flags.
pub(crate) fn push_mesh_vertices(
    out: &mut Vec<MeshVertex>,
    batch: &cad_scene::RenderBatch,
    piece: &MeshPiece,
    origin: [f64; 3],
) {
    let normals = geometry::repaired_normals(batch);
    let colors = geometry::repaired_colors(batch);
    let tint = renderer_color(batch.color);
    // Only the first source's `EntityId` low 32 bits ride per vertex: a
    // multi-source batch (e.g. a glyph run sharing one mesh) exposes a single id
    // here, so a later picking phase needs a collision/multi-source story (P3).
    let entity_low = batch
        .sources
        .first()
        .map_or(0, |source| source.entity.0 as u32);
    let flags = INSTANCE_FLAG_OPAQUE
        | if batch.sources.is_empty() {
            0
        } else {
            INSTANCE_FLAG_HAS_SOURCE
        }
        | if batch.lineweight > 0.0 {
            INSTANCE_FLAG_LINEWEIGHT
        } else {
            0
        };
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
            entity_flags: [entity_low, flags],
        });
    }
}

// Shaders are checked into `shaders/` so they can be validated as files and
// stay reviewable; `include_str!` keeps a single source of truth.
pub(crate) const LINE_SHADER: &str = include_str!("../shaders/line.wgsl");

// Instanced line shader. One instance per 2-vertex `LineList` segment; the
// per-instance attributes are expanded against a shared unit segment so the
// dominant CAD primitive class collapses to one draw call per contiguous run.
pub(crate) const LINE_INSTANCED_SHADER: &str = include_str!("../shaders/line_instanced.wgsl");

// Mesh shader. Shading is intentionally explicit: a single fixed headlight
// direction, N·L diffuse plus a constant ambient term. There is no claim of
// physically based rendering, environment lighting or specular response.
pub(crate) const MESH_SHADER: &str = include_str!("../shaders/mesh.wgsl");

// Merged-mesh shader: the same shading body as `MESH_SHADER` with an extra
// per-vertex `entity_flags` attribute and a per-vertex baked colour (the
// per-batch tint is folded in at upload, see `push_mesh_vertices`).
pub(crate) const MESH_MERGED_SHADER: &str = include_str!("../shaders/mesh_merged.wgsl");

// Raster image shader. It samples a texture and modulates it by a per-image
// alpha/colour factor; there is no lighting model.
pub(crate) const IMAGE_SHADER: &str = include_str!("../shaders/image.wgsl");

// ---------------------------------------------------------------------------
// Instanced line stream ("entity = data", first slice)
// ---------------------------------------------------------------------------
//
// The dominant CAD primitive class is the 2-vertex `LineList` segment. Instead
// of one draw call per batch, segments are packed into a single static instance
// stream at upload time and expanded in the vertex shader against a shared unit
// segment. Instance order is the original batch order, so equal-depth paint
// semantics are preserved exactly (no re-sorting); the frame path coalesces
// adjacent instances of one stream into as few draws as the opaque order allows.
//
// The layout is WebGL2-safe: every field is a per-instance *vertex attribute*
// (`VertexStepMode::Instance`), never a vertex-stage storage buffer.

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

/// Reserved instance flags. This slice only ever sets [`INSTANCE_FLAG_OPAQUE`]
/// (converted batches are opaque); the rest are first-class fields the later
/// picking/selection phase will consume.
pub(crate) const INSTANCE_FLAG_OPAQUE: u32 = 1 << 0;
pub(crate) const INSTANCE_FLAG_HAS_SOURCE: u32 = 1 << 1;
pub(crate) const INSTANCE_FLAG_LINEWEIGHT: u32 = 1 << 2;

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
    /// `[0]` = low 32 bits of the source `EntityId` (further resolution goes
    /// through the batch's `sources`); `[1]` = reserved [`INSTANCE_FLAG_*`] bits.
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

/// Whether a scene batch is eligible for the instanced line path.
///
/// Only opaque `Lines` batches with a whole number of 2-vertex segments are
/// converted (an odd trailing vertex stays legacy so the pairing rule is never
/// reinterpreted). Transparent batches stay legacy (their back-to-front order is
/// camera-dependent).
///
/// `MeshEdges` is deliberately **not** converted here, and this is reported as a
/// gap rather than a silent change of behaviour: the legacy `draw_batch` reads
/// `GpuBatch::indices`, which `prepare_upload_batches` leaves `None` for a
/// `MeshEdges` batch (it stores the identity line list in `edge_indices`
/// instead), so a legacy `MeshEdges` batch currently submits **no** draw at all.
/// Instancing it would start drawing geometry that the legacy path dropped —
/// a behaviour change, not the byte-equivalence this path claims. Making
/// `MeshEdges` render is a separate, pre-existing fix outside this slice.
///
/// A `Mesh` batch is not a line batch either; its optional edge overlay is
/// instanced only when the batch is flat-merged (see [`overlay_is_instanced`]),
/// because the overlay must be drawn separately from the merged triangle run.
pub(crate) fn line_batch_is_instanced(batch: &cad_scene::RenderBatch) -> bool {
    batch.topology == RenderTopology::Lines
        && batch.vertices.len() >= 2
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
fn batch_world_aabb(batch: &cad_scene::RenderBatch) -> ([f64; 3], [f64; 3]) {
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
fn aabb_half_diagonal(min: [f64; 3], max: [f64; 3]) -> f64 {
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
}

/// A chunk under construction.
struct ChunkBuilder {
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
    fn new(
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
    fn accepts(
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

    fn push(
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
pub(crate) fn build_instance_chunks(
    batches: &[cad_scene::RenderBatch],
    converted: &[ConvertedLine],
    max_instances: usize,
    max_radius: f64,
) -> (Vec<InstanceChunk>, Vec<Option<Vec<InstanceRun>>>) {
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
            );
            runs[source.batch]
                .get_or_insert_with(Vec::new)
                .push(InstanceRun {
                    stream: stream as u32,
                    offset,
                    count,
                });
        }
        chunks.push(InstanceChunk { origin, instances });
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
/// instance cap be split across chunks.
pub(crate) fn push_line_instances(
    out: &mut Vec<LineInstance>,
    batch: &cad_scene::RenderBatch,
    indices: Option<&[u32]>,
    origin: [f64; 3],
    first_segment: u32,
    segment_count: u32,
) {
    let [r, g, b] = renderer_color(batch.color);
    let rgba = [r, g, b, clamp_alpha(batch.alpha)];
    let width = if batch.lineweight.is_finite() {
        batch.lineweight.max(0.0)
    } else {
        0.0
    };
    let flags = INSTANCE_FLAG_OPAQUE
        | if batch.sources.is_empty() {
            0
        } else {
            INSTANCE_FLAG_HAS_SOURCE
        }
        | if batch.lineweight > 0.0 {
            INSTANCE_FLAG_LINEWEIGHT
        } else {
            0
        };
    let entity_low = batch
        .sources
        .first()
        .map_or(0, |source| source.entity.0 as u32);
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
            id_flags: [entity_low, flags],
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

/// One image vertex: world-space position (`@location(0)`) plus a normalized
/// texture coordinate (`@location(1)`). The two attributes share one buffer,
/// unlike the mesh pipeline's split position/normal/colour buffers.
pub(crate) fn image_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2];
    wgpu::VertexBufferLayout {
        array_stride: 20,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRIBUTES,
    }
}

/// Submit one uploaded image: bind the camera uniform (group 0, with this
/// image's dynamic slot offset) and the texture/sampler (group 1), then draw the
/// indexed quad.
pub(crate) fn draw_image(
    pass: &mut wgpu::RenderPass<'_>,
    state: &mut EncodeState,
    image: &GpuImage,
    bind_group: &wgpu::BindGroup,
    uniform_offset: u32,
    pipeline: &wgpu::RenderPipeline,
) {
    state.set_pipeline(pass, pipeline);
    state.set_bind_group0(pass, bind_group, uniform_offset);
    state.set_bind_group1(pass, &image.texture_bind_group);
    state.set_vertex_buffer(pass, 0, &image.vertex_buffer);
    state.set_index_buffer(pass, &image.index_buffer);
    pass.draw_indexed(0..image.index_count, 0, 0..1);
}

/// The world-space geometry for one image batch.
///
/// Without a `clip` this is the unit square `(0,0),(1,0),(1,1),(0,1)` mapped
/// through `transform`, with a two-triangle index list (`0,1,2, 0,2,3`). When
/// `clip` is `Some` the surviving convex polygon (already in world space, with
/// per-vertex texture coordinates) is used verbatim and fan-triangulated
/// (`0,1,2, 0,2,3, ...`); the batch `transform` is not applied again.
///
/// Texture coordinates are emitted with `uv.y = 1 - v`. Decoded image rows are
/// stored top-down and wgpu samples texel row 0 at `v = 0`, while the CAD/DXF
/// convention has the origin at the lower-left with `v` pointing up; without the
/// flip every image would render vertically mirrored.
pub(crate) fn image_quad_vertices(batch: &ImageBatch) -> (Vec<u8>, Vec<u32>) {
    if let Some(clip) = &batch.clip {
        let mut vertices = Vec::with_capacity(clip.len() * 20);
        for vertex in clip.iter() {
            let p = vertex.position;
            let uv = [vertex.uv[0] as f32, (1.0 - vertex.uv[1]) as f32];
            for component in [p.x as f32, p.y as f32, p.z as f32, uv[0], uv[1]] {
                vertices.extend_from_slice(&component.to_le_bytes());
            }
        }
        // Fan-triangulate the convex polygon: (0,1,2), (0,2,3), ...; fewer than
        // three vertices (a fully clipped-away image) yields no triangles.
        let mut indices = Vec::with_capacity(clip.len().saturating_sub(2) * 3);
        for i in 1..clip.len().saturating_sub(1) {
            indices.extend_from_slice(&[0, i as u32, (i + 1) as u32]);
        }
        return (vertices, indices);
    }
    const CORNERS: [[f64; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let mut vertices = Vec::with_capacity(4 * 20);
    for [u, v] in CORNERS {
        let p = batch.transform.apply_point(Point3 { x: u, y: v, z: 0.0 });
        for component in [
            p.x as f32,
            p.y as f32,
            p.z as f32,
            u as f32,
            (1.0 - v) as f32,
        ] {
            vertices.extend_from_slice(&component.to_le_bytes());
        }
    }
    (vertices, vec![0, 1, 2, 0, 2, 3])
}

/// Charge the frame budget for the resident image quads and return the accepted
/// indices (in upload order). The first image that crosses a limit stops the
/// scan and is reported as over budget — images are never silently dropped.
pub(crate) fn charge_images(
    usage: &mut cad_scene::FrameUsage,
    budget: &FrameBudget,
    images: &[GpuImage],
) -> (Vec<usize>, Option<OverBudgetReason>) {
    let mut accepted = Vec::with_capacity(images.len());
    // A texture is uploaded once per resource key, so its bytes are charged only
    // for the first resident quad that references it; later quads sharing the key
    // charge only their own vertex/index buffers.
    let mut charged_textures: HashSet<&ResourceKey> = HashSet::new();
    for (i, image) in images.iter().enumerate() {
        let mut next = *usage;
        let texture_bytes = if charged_textures.insert(&image.resource) {
            image.texture_bytes
        } else {
            0
        };
        let result = budget
            .charge_bytes(&mut next, image.upload_bytes + texture_bytes)
            .and_then(|()| budget.charge(&mut next, image.vertex_count as usize, 2));
        match result {
            Ok(()) => {
                *usage = next;
                accepted.push(i);
            }
            Err(exceeded) => {
                let report = OverBudget {
                    category: exceeded.category,
                    requested: exceeded.requested,
                    limit: exceeded.limit,
                    skipped_batches: images.len() - accepted.len(),
                };
                return (
                    accepted,
                    Some(OverBudgetReason {
                        reason: over_budget_reason(&report),
                        report,
                    }),
                );
            }
        }
    }
    (accepted, None)
}

#[cfg(test)]
mod mesh_overlay_tests {
    use super::*;
    #[test]
    fn mesh_draw_plan_makes_edge_overlay_optional() {
        assert_eq!(
            mesh_draw_plan(true, false),
            MeshDrawPlan {
                triangles: true,
                edge_overlay: false
            }
        );
        assert_eq!(
            mesh_draw_plan(true, true),
            MeshDrawPlan {
                triangles: true,
                edge_overlay: true
            }
        );
        assert_eq!(
            mesh_draw_plan(false, true),
            MeshDrawPlan {
                triangles: false,
                edge_overlay: false
            }
        );
    }
}

#[cfg(test)]
mod instanced_line_tests {
    use super::*;
    use cad_domain::SelectionRef;

    fn source() -> SelectionRef {
        SelectionRef {
            document: cad_domain::DocumentId(1),
            entity: cad_domain::EntityId(42),
            instance: Default::default(),
            sub_element: None,
        }
    }

    fn batch(
        topology: RenderTopology,
        vertices: Vec<[f32; 3]>,
        alpha: f32,
    ) -> cad_scene::RenderBatch {
        cad_scene::RenderBatch {
            local_origin: cad_domain::Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            topology,
            vertices,
            normals: Vec::new(),
            colors: Vec::new(),
            indices: Vec::new(),
            edges: Vec::new(),
            mirrored: false,
            alpha,
            color: [0.25, 0.5, 0.75],
            color_unresolved: false,
            lineweight: 0.0,
            lineweight_unresolved: false,
            linetype: cad_scene::LinetypePattern::continuous(),
            linetype_unresolved: false,
            sources: vec![source()],
            draw_order: 0,
        }
    }

    /// Convert every `Lines` batch (the only class this module exercises) the
    /// way [`convert_line_batches`] would, with `line_instancing` on.
    fn converted(batches: &[cad_scene::RenderBatch]) -> Vec<ConvertedLine> {
        batches
            .iter()
            .enumerate()
            .filter(|(_, batch)| line_batch_is_instanced(batch))
            .map(|(index, batch)| ConvertedLine {
                batch: index,
                segments: (batch.vertices.len() / 2) as u32,
                indices: None,
            })
            .collect()
    }

    #[test]
    fn instance_stride_matches_the_packed_layout() {
        assert_eq!(
            LINE_INSTANCE_STRIDE as usize,
            std::mem::size_of::<LineInstance>()
        );
        let instance = LineInstance {
            p0_width: [0.0; 4],
            p1_pad: [0.0; 4],
            rgba: [0.0; 4],
            id_flags: [0, 0],
        };
        assert_eq!(
            line_instance_bytes(std::slice::from_ref(&instance)).len(),
            LINE_INSTANCE_STRIDE as usize
        );
    }

    #[test]
    fn line_batch_gate_accepts_only_opaque_even_line_lists() {
        // Opaque 2-vertex segment: the exact case.
        assert!(line_batch_is_instanced(&batch(
            RenderTopology::Lines,
            vec![[0.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
            1.0
        )));
        // Opaque multi-segment LineList (even): each pair is independent.
        assert!(line_batch_is_instanced(&batch(
            RenderTopology::Lines,
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [2.0, 0.0, 0.0],
                [3.0, 1.0, 0.0]
            ],
            1.0
        )));
        // Transparent stays legacy (camera-dependent ordering).
        assert!(!line_batch_is_instanced(&batch(
            RenderTopology::Lines,
            vec![[0.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
            0.5
        )));
        // Invisible stays legacy (reported, never silently instanced).
        assert!(!line_batch_is_instanced(&batch(
            RenderTopology::Lines,
            vec![[0.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
            0.0
        )));
        // An odd trailing vertex would change the LineList pairing rule.
        assert!(!line_batch_is_instanced(&batch(
            RenderTopology::Lines,
            vec![[0.0, 0.0, 0.0], [1.0, 1.0, 0.0], [2.0, 0.0, 0.0]],
            1.0
        )));
        // MeshEdges stays legacy: its legacy draw is a no-op (see the gate's
        // doc), so instancing it would change behaviour rather than preserve it.
        assert!(!line_batch_is_instanced(&batch(
            RenderTopology::MeshEdges,
            vec![[0.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
            1.0
        )));
        // A single vertex is not a segment.
        assert!(!line_batch_is_instanced(&batch(
            RenderTopology::Lines,
            vec![[0.0, 0.0, 0.0]],
            1.0
        )));
    }

    #[test]
    fn shared_origin_is_kept_verbatim() {
        let mut a = batch(
            RenderTopology::Lines,
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
            1.0,
        );
        let mut b = batch(
            RenderTopology::Lines,
            vec![[2.0, 0.0, 0.0], [3.0, 0.0, 0.0]],
            1.0,
        );
        a.local_origin = cad_domain::Point3 {
            x: 1.0e7,
            y: -2.0e7,
            z: 5.0,
        };
        b.local_origin = a.local_origin;
        assert_eq!(
            choose_instance_origin(&[&a, &b]),
            [1.0e7, -2.0e7, 5.0],
            "a shared origin is used verbatim so the legacy matrix is reproduced"
        );
    }

    #[test]
    fn mixed_origin_uses_the_world_aabb_midpoint() {
        let mut a = batch(
            RenderTopology::Lines,
            vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
            1.0,
        );
        let mut b = batch(
            RenderTopology::Lines,
            vec![[0.0, 0.0, 0.0], [4.0, 2.0, 0.0]],
            1.0,
        );
        a.local_origin = cad_domain::Point3 {
            x: 1.0e7,
            y: 0.0,
            z: 0.0,
        };
        b.local_origin = cad_domain::Point3 {
            x: 1.0e7 + 8.0,
            y: 0.0,
            z: 0.0,
        };
        // World x spans [1e7, 1e7 + 12]; world y spans [0, 2].
        assert_eq!(choose_instance_origin(&[&a, &b]), [1.0e7 + 6.0, 1.0, 0.0]);
    }

    #[test]
    fn push_line_instances_packs_positions_colour_width_and_pick_key() {
        let mut b = batch(
            RenderTopology::Lines,
            vec![[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]],
            1.0,
        );
        b.lineweight = 0.35;
        let origin = [10.0, 20.0, 30.0];
        let mut out = Vec::new();
        push_line_instances(&mut out, &b, None, origin, 0, 1);
        assert_eq!(out.len(), 1);
        let instance = out[0];
        // (origin-relative) positions
        assert_eq!(instance.p0_width, [-9.0, -18.0, -27.0, 0.35]);
        assert_eq!(instance.p1_pad, [-6.0, -15.0, -24.0, 0.0]);
        // sanitized colour + clamped alpha
        assert_eq!(instance.rgba, [0.25, 0.5, 0.75, 1.0]);
        // first-class pick key: low 32 bits of the source EntityId + flags
        assert_eq!(instance.id_flags[0], 42);
        assert_eq!(
            instance.id_flags[1],
            INSTANCE_FLAG_OPAQUE | INSTANCE_FLAG_HAS_SOURCE | INSTANCE_FLAG_LINEWEIGHT
        );
    }

    #[test]
    fn shared_origin_positions_are_exactly_the_batch_vertices() {
        // The byte-identity contract: with a shared origin, `(o + v - o) as f32`
        // must equal `v` exactly for arbitrary f32 vertices.
        let vertices = vec![[0.1f32, -0.2, 3.0e-3], [1.5e-4, 12345.5, -0.75]];
        let mut b = batch(RenderTopology::Lines, vertices.clone(), 1.0);
        b.local_origin = cad_domain::Point3 {
            x: 1.0e7,
            y: -5.0e6,
            z: 0.5,
        };
        let origin = [b.local_origin.x, b.local_origin.y, b.local_origin.z];
        let mut out = Vec::new();
        push_line_instances(&mut out, &b, None, origin, 0, 1);
        assert_eq!(
            [out[0].p0_width[0], out[0].p0_width[1], out[0].p0_width[2]],
            vertices[0]
        );
        assert_eq!(
            [out[0].p1_pad[0], out[0].p1_pad[1], out[0].p1_pad[2]],
            vertices[1]
        );
    }

    fn origin(x: f64, y: f64, z: f64) -> cad_domain::Point3 {
        cad_domain::Point3 { x, y, z }
    }

    fn segment(x: f32) -> Vec<[f32; 3]> {
        vec![[x, 0.0, 0.0], [x + 0.5, 1.0, 0.0]]
    }

    /// A batch with `count` disjoint segments, all relative to `local_origin`.
    fn multi_segment_batch(count: usize, local: cad_domain::Point3) -> cad_scene::RenderBatch {
        let vertices = (0..count).flat_map(|i| segment(i as f32)).collect();
        let mut b = batch(RenderTopology::Lines, vertices, 1.0);
        b.local_origin = local;
        b
    }

    #[test]
    fn build_instance_chunks_keeps_a_shared_origin_verbatim() {
        let a = multi_segment_batch(2, origin(0.0, 0.0, 0.0));
        let b = multi_segment_batch(3, origin(0.0, 0.0, 0.0));
        let batches = vec![a, b];
        let (chunks, runs) = build_instance_chunks(
            &batches,
            &converted(&batches),
            MAX_INSTANCES_PER_STREAM,
            MAX_STREAM_RADIUS,
        );
        assert_eq!(chunks.len(), 1, "a shared origin stays in one stream");
        assert_eq!(chunks[0].origin, [0.0, 0.0, 0.0]);
        assert_eq!(chunks[0].instances.len(), 5, "2 + 3 segments");
        // Contiguous runs in one stream, so the frame path coalesces them.
        assert_eq!(
            runs[0].as_ref().unwrap()[0],
            InstanceRun {
                stream: 0,
                offset: 0,
                count: 2
            }
        );
        assert_eq!(
            runs[1].as_ref().unwrap()[0],
            InstanceRun {
                stream: 0,
                offset: 2,
                count: 3
            }
        );
    }

    #[test]
    fn build_instance_chunks_splits_a_batch_beyond_the_cap() {
        // 5 segments in ONE batch with a cap of 2: split into 2 + 2 + 1 with no
        // segment lost, no empty chunk and no oversized buffer.
        let batches = vec![multi_segment_batch(5, origin(0.0, 0.0, 0.0))];
        let (chunks, runs) =
            build_instance_chunks(&batches, &converted(&batches), 2, MAX_STREAM_RADIUS);
        assert_eq!(chunks.len(), 3, "ceil(5 / 2) chunks");
        assert!(
            chunks
                .iter()
                .all(|chunk| !chunk.instances.is_empty() && chunk.instances.len() <= 2),
            "no empty and no oversized chunk: {:?}",
            chunks.iter().map(|c| c.instances.len()).collect::<Vec<_>>()
        );
        assert_eq!(
            chunks.iter().map(|c| c.instances.len()).sum::<usize>(),
            5,
            "no segment lost"
        );
        let runs = runs[0].as_ref().expect("batch converted");
        assert_eq!(runs.len(), 3, "one run per chunk piece");
        assert_eq!(runs.iter().map(|r| r.count).sum::<u32>(), 5);
        assert_eq!(
            runs.iter().map(|r| r.stream).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        // Order is preserved: segment `i` of the batch lands in slice `i / 2`.
        assert_eq!(
            chunks[0].instances[0].p0_width[0], 0.0,
            "first chunk starts at the batch's first segment"
        );
        assert_eq!(
            chunks[2].instances[0].p0_width[0], 4.0,
            "last chunk starts at segment 4"
        );
    }

    #[test]
    fn build_instance_chunks_splits_distant_origins_into_separate_streams() {
        // Two batches whose origins are far beyond the radius must not share a
        // stream (their midpoint would blow up every stored position).
        let mut a = multi_segment_batch(1, origin(0.0, 0.0, 0.0));
        a.local_origin = origin(0.0, 0.0, 0.0);
        let mut b = multi_segment_batch(1, origin(0.0, 0.0, 0.0));
        b.local_origin = origin(1.0e6, 0.0, 0.0);
        let batches = vec![a, b];
        let (chunks, runs) = build_instance_chunks(
            &batches,
            &converted(&batches),
            MAX_INSTANCES_PER_STREAM,
            MAX_STREAM_RADIUS,
        );
        assert_eq!(chunks.len(), 2, "a distant origin starts a new stream");
        assert_eq!(runs[0].as_ref().unwrap()[0].stream, 0);
        assert_eq!(runs[1].as_ref().unwrap()[0].stream, 1);
        for chunk in &chunks {
            for instance in &chunk.instances {
                for axis in 0..3 {
                    assert!(
                        instance.p0_width[axis].abs() <= 1.0,
                        "stream positions must stay small, got {:?}",
                        instance.p0_width
                    );
                }
            }
        }
    }

    #[test]
    fn build_instance_chunks_merges_nearby_mixed_origins_at_the_midpoint() {
        // Near origins still coalesce into one stream, with a midpoint origin
        // and positions bounded by the chunk radius.
        let mut a = multi_segment_batch(1, origin(0.0, 0.0, 0.0));
        a.vertices = vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
        a.local_origin = origin(0.0, 0.0, 0.0);
        let mut b = multi_segment_batch(1, origin(0.0, 0.0, 0.0));
        b.vertices = vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
        b.local_origin = origin(2.0, 0.0, 0.0);
        let batches = vec![a, b];
        let (chunks, runs) = build_instance_chunks(
            &batches,
            &converted(&batches),
            MAX_INSTANCES_PER_STREAM,
            MAX_STREAM_RADIUS,
        );
        assert_eq!(chunks.len(), 1, "near origins merge");
        assert_eq!(chunks[0].origin, [2.0, 0.0, 0.0], "AABB midpoint");
        assert_eq!(chunks[0].instances.len(), 2);
        assert_eq!(runs[0].as_ref().unwrap()[0].offset, 0);
        assert_eq!(runs[1].as_ref().unwrap()[0].offset, 1);
    }

    #[test]
    fn build_instance_chunks_never_emits_an_empty_chunk() {
        // A batch that does not fit the current chunk starts a new one; the
        // chunk it leaves behind is never emitted empty.
        let a = multi_segment_batch(2, origin(0.0, 0.0, 0.0));
        let b = multi_segment_batch(2, origin(1.0e6, 0.0, 0.0));
        let batches = vec![a, b];
        let (chunks, _) = build_instance_chunks(
            &batches,
            &converted(&batches),
            MAX_INSTANCES_PER_STREAM,
            MAX_STREAM_RADIUS,
        );
        assert!(chunks.iter().all(|chunk| !chunk.instances.is_empty()));
        assert_eq!(chunks.len(), 2);
    }
}

#[cfg(test)]
mod mesh_merge_tests {
    use super::*;
    use cad_domain::{EntityId, SelectionRef};

    fn source(entity: u128) -> SelectionRef {
        SelectionRef {
            document: cad_domain::DocumentId(1),
            entity: EntityId(entity),
            instance: Default::default(),
            sub_element: None,
        }
    }

    /// A quad mesh (4 vertices, 2 triangles) at `origin`, tint `color`.
    fn quad(origin: [f64; 3], color: [f32; 3], entity: u128) -> cad_scene::RenderBatch {
        let half = 1.0f32;
        cad_scene::RenderBatch {
            local_origin: cad_domain::Point3 {
                x: origin[0],
                y: origin[1],
                z: origin[2],
            },
            topology: RenderTopology::Mesh,
            vertices: vec![
                [-half, -half, 0.0],
                [-half, half, 0.0],
                [half, half, 0.0],
                [half, -half, 0.0],
            ],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            colors: Vec::new(),
            indices: vec![[0, 1, 2], [0, 2, 3]],
            edges: Vec::new(),
            mirrored: false,
            alpha: 1.0,
            color,
            color_unresolved: false,
            lineweight: 0.0,
            lineweight_unresolved: true,
            linetype: cad_scene::LinetypePattern::continuous(),
            linetype_unresolved: true,
            sources: vec![source(entity)],
            draw_order: 0,
        }
    }

    #[test]
    fn mesh_vertex_stride_matches_the_packed_layout() {
        assert_eq!(
            MESH_VERTEX_STRIDE as usize,
            std::mem::size_of::<MeshVertex>()
        );
        let vertex = MeshVertex {
            position: [0.0; 3],
            normal: [0.0; 3],
            color: [0.0; 3],
            entity_flags: [0, 0],
        };
        assert_eq!(
            mesh_vertex_bytes(std::slice::from_ref(&vertex)).len(),
            MESH_VERTEX_STRIDE as usize
        );
    }

    #[test]
    fn mesh_merge_gate_accepts_only_opaque_triangle_meshes() {
        assert!(mesh_batch_is_mergeable(&quad([0.0; 3], [1.0; 3], 1)));
        // Transparent stays legacy (camera-dependent order).
        let mut transparent = quad([0.0; 3], [1.0; 3], 1);
        transparent.alpha = 0.5;
        assert!(!mesh_batch_is_mergeable(&transparent));
        // A mesh with no triangle list has nothing to merge.
        let mut empty = quad([0.0; 3], [1.0; 3], 1);
        empty.indices.clear();
        assert!(!mesh_batch_is_mergeable(&empty));
        // A line batch is not a mesh.
        let mut line = quad([0.0; 3], [1.0; 3], 1);
        line.topology = RenderTopology::Lines;
        assert!(!mesh_batch_is_mergeable(&line));
    }

    #[test]
    fn build_mesh_chunks_concatenates_adjacent_batches_in_order() {
        let batches = vec![
            quad([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 11),
            quad([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 22),
        ];
        let (chunks, runs) = build_mesh_chunks(
            &batches,
            &[0, 1],
            MAX_VERTICES_PER_MESH_STREAM,
            MAX_STREAM_RADIUS,
        );
        assert_eq!(chunks.len(), 1, "adjacent same-pipeline meshes merge");
        assert_eq!(chunks[0].vertices.len(), 8, "4 + 4 vertices");
        assert_eq!(chunks[0].indices.len(), 12, "6 + 6 triangle indices");
        // Runs are contiguous in the chunk, so the frame coalesces them.
        assert_eq!(
            runs[0].as_ref().unwrap()[0],
            MeshRun {
                chunk: 0,
                index_offset: 0,
                index_count: 6,
                vertex_count: 4,
            }
        );
        assert_eq!(
            runs[1].as_ref().unwrap()[0],
            MeshRun {
                chunk: 0,
                index_offset: 6,
                index_count: 6,
                vertex_count: 4,
            }
        );
        // The second batch's indices are rebased by its vertex base (4).
        assert_eq!(&chunks[0].indices[6..], &[4, 5, 6, 4, 6, 7]);
    }

    #[test]
    fn build_mesh_chunks_splits_a_batch_beyond_the_vertex_cap() {
        // Two quads (4 vertices each) with a cap of 4: each piece is its own
        // chunk, no triangle lost and no empty chunk.
        let batches = vec![quad([0.0; 3], [1.0; 3], 1), quad([0.0; 3], [1.0; 3], 2)];
        let (chunks, runs) = build_mesh_chunks(&batches, &[0, 1], 4, MAX_STREAM_RADIUS);
        assert_eq!(chunks.len(), 2, "a full chunk starts a new one");
        assert!(chunks.iter().all(|chunk| !chunk.vertices.is_empty()));
        assert_eq!(
            chunks.iter().map(|c| c.indices.len()).sum::<usize>(),
            12,
            "no triangle lost"
        );
        assert_eq!(runs[0].as_ref().unwrap()[0].chunk, 0);
        assert_eq!(runs[1].as_ref().unwrap()[0].chunk, 1);
    }

    #[test]
    fn build_mesh_chunks_splits_one_oversized_batch_without_loss() {
        // A single batch of four triangles (6 distinct vertices) with a cap of 4:
        // greedy splitting keeps every triangle, in order, across pieces.
        let mut batch = quad([0.0; 3], [1.0; 3], 1);
        batch.vertices.push([2.0, 0.0, 0.0]);
        batch.vertices.push([2.0, 1.0, 0.0]);
        batch.normals = vec![[0.0, 0.0, 1.0]; 6];
        batch.indices = vec![[0, 1, 2], [0, 2, 3], [3, 2, 4], [2, 5, 4]];
        let batches = vec![batch];
        let (chunks, runs) = build_mesh_chunks(&batches, &[0], 4, MAX_STREAM_RADIUS);
        assert!(chunks.len() >= 2, "an oversized batch must split");
        assert!(chunks.iter().all(|chunk| !chunk.vertices.is_empty()));
        assert_eq!(
            chunks.iter().map(|c| c.indices.len()).sum::<usize>(),
            12,
            "all four triangles survive the split"
        );
        let runs = runs[0].as_ref().expect("batch converted");
        assert_eq!(runs.iter().map(|r| r.index_count).sum::<u32>(), 12);
        assert_eq!(
            runs.iter().map(|r| r.chunk).collect::<Vec<_>>(),
            (0..runs.len() as u32).collect::<Vec<_>>(),
            "one run per piece, one piece per chunk"
        );
    }

    #[test]
    fn build_mesh_chunks_is_a_mirrored_boundary() {
        let a = quad([0.0; 3], [1.0; 3], 1);
        let mut b = quad([0.0; 3], [1.0; 3], 2);
        b.mirrored = true;
        let batches = vec![a, b];
        let (chunks, _) = build_mesh_chunks(
            &batches,
            &[0, 1],
            MAX_VERTICES_PER_MESH_STREAM,
            MAX_STREAM_RADIUS,
        );
        assert_eq!(chunks.len(), 2, "a mirrored batch is a pipeline boundary");
        assert!(!chunks[0].mirrored);
        assert!(chunks[1].mirrored);
    }

    #[test]
    fn push_mesh_vertices_bakes_the_tint_and_carries_the_entity_id() {
        let mut batch = quad([100.0, 200.0, 0.0], [0.5, 0.25, 0.75], 42);
        // A per-vertex gradient: the shader multiplies the batch tint by this.
        batch.colors = vec![
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
        ];
        let piece = &mesh_pieces(&batch, 0, 64)[0];
        let origin = [
            batch.local_origin.x,
            batch.local_origin.y,
            batch.local_origin.z,
        ];
        let mut out = Vec::new();
        push_mesh_vertices(&mut out, &batch, piece, origin);
        assert_eq!(out.len(), 4);
        // Baked colour = sanitized tint * sanitized vertex colour.
        assert_eq!(out[0].color, [0.5, 0.0, 0.0]);
        assert_eq!(out[1].color, [0.0, 0.25, 0.0]);
        assert_eq!(out[2].color, [0.0, 0.0, 0.75]);
        assert_eq!(out[3].color, [0.5, 0.25, 0.75]);
        // The source id/flags ride per vertex (the legacy per-batch draw carried
        // no explicit pick key).
        for vertex in &out {
            assert_eq!(vertex.entity_flags[0], 42);
            assert_eq!(
                vertex.entity_flags[1],
                INSTANCE_FLAG_OPAQUE | INSTANCE_FLAG_HAS_SOURCE
            );
        }
    }

    #[test]
    fn merged_chunk_carries_the_right_entity_id_per_vertex() {
        // "Pick one triangle per class and read its id": two batches with
        // different source entities merge into one chunk, and every vertex keeps
        // its own batch's id, at the chunk vertex offsets the run list reports.
        let batches = vec![
            quad([0.0, 0.0, 0.0], [1.0; 3], 111),
            quad([3.0, 0.0, 0.0], [1.0; 3], 222),
        ];
        let (chunks, runs) = build_mesh_chunks(
            &batches,
            &[0, 1],
            MAX_VERTICES_PER_MESH_STREAM,
            MAX_STREAM_RADIUS,
        );
        assert_eq!(chunks.len(), 1);
        let first = runs[0].as_ref().unwrap()[0];
        let second = runs[1].as_ref().unwrap()[0];
        assert_eq!(first.vertex_count, 4);
        assert_eq!(second.vertex_count, 4);
        // The run's vertex range is implicit (runs are packed in order); walk the
        // chunk vertices and attribute each to its batch by that ordering.
        for (index, vertex) in chunks[0].vertices.iter().enumerate() {
            let expected = if index < first.vertex_count as usize {
                111
            } else {
                222
            };
            assert_eq!(
                vertex.entity_flags[0], expected,
                "vertex {index} must carry its own batch's entity id"
            );
            assert_eq!(
                vertex.entity_flags[1],
                INSTANCE_FLAG_OPAQUE | INSTANCE_FLAG_HAS_SOURCE
            );
        }
    }

    #[test]
    fn push_mesh_vertices_stores_shared_origin_positions_verbatim() {
        // With a shared origin the stored position is the batch-local f32 value,
        // so the merged draw reproduces the legacy per-batch matrix exactly.
        let vertices = vec![
            [0.1f32, -0.2, 3.0e-3],
            [1.5e-4, 12345.5, -0.75],
            [2.0, 0.0, 0.0],
        ];
        let mut batch = quad([1.0e7, -5.0e6, 0.5], [1.0; 3], 1);
        batch.vertices = vertices.clone();
        batch.normals = vec![[0.0, 0.0, 1.0]; 3];
        batch.indices = vec![[0, 1, 2]];
        let piece = &mesh_pieces(&batch, 0, 64)[0];
        let origin = [
            batch.local_origin.x,
            batch.local_origin.y,
            batch.local_origin.z,
        ];
        let mut out = Vec::new();
        push_mesh_vertices(&mut out, &batch, piece, origin);
        assert_eq!(out[0].position, vertices[0]);
        assert_eq!(out[1].position, vertices[1]);
    }

    #[test]
    fn overlay_is_instanced_only_for_opaque_meshes_with_edges() {
        let mut with_edges = quad([0.0; 3], [1.0; 3], 1);
        assert!(overlay_is_instanced(&with_edges).is_none());
        with_edges.edges = vec![[0.0; 3], [1.0, 0.0, 0.0]];
        // `mesh_edge_indices` sorts/de-duplicates the triangle edges.
        assert_eq!(
            overlay_is_instanced(&with_edges),
            Some(vec![0, 1, 0, 2, 0, 3, 1, 2, 2, 3])
        );
        // Transparent overlays stay legacy.
        let mut transparent = with_edges.clone();
        transparent.alpha = 0.5;
        assert!(overlay_is_instanced(&transparent).is_none());
    }

    #[test]
    fn convert_line_batches_instances_overlays_only_when_merging() {
        let mut mesh = quad([0.0; 3], [1.0; 3], 1);
        mesh.edges = vec![[0.0; 3], [1.0, 0.0, 0.0]];
        let batches = vec![mesh];
        // Mesh merging off: nothing converts (the overlay stays a legacy draw).
        assert!(convert_line_batches(&batches, true, false).is_empty());
        // Mesh merging on: the overlay becomes an index-pair line source.
        let converted = convert_line_batches(&batches, true, true);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].batch, 0);
        assert_eq!(converted[0].segments, 5);
        assert_eq!(
            converted[0].indices.as_deref(),
            Some(&[0, 1, 0, 2, 0, 3, 1, 2, 2, 3][..])
        );
    }

    #[test]
    fn push_line_instances_expands_index_pairs_from_the_overlay() {
        // An overlay is an index list over the batch vertices; each consecutive
        // index pair must expand to one 2-vertex instance.
        let mut batch = quad([0.0; 3], [0.25, 0.5, 0.75], 7);
        batch.edges = vec![[0.0; 3], [1.0, 0.0, 0.0]];
        let indices = overlay_is_instanced(&batch).expect("overlay");
        let origin = [0.0, 0.0, 0.0];
        let mut out = Vec::new();
        push_line_instances(&mut out, &batch, Some(&indices), origin, 0, 5);
        assert_eq!(out.len(), 5);
        // First edge (0,1): vertices[0] -> vertices[1].
        assert_eq!(
            [out[0].p0_width[0], out[0].p0_width[1], out[0].p0_width[2]],
            batch.vertices[0]
        );
        assert_eq!(
            [out[0].p1_pad[0], out[0].p1_pad[1], out[0].p1_pad[2]],
            batch.vertices[1]
        );
        // Colour comes from the batch (as the legacy line uniform did); the id is
        // new per-instance metadata the legacy draw did not carry.
        assert_eq!(out[0].rgba, [0.25, 0.5, 0.75, 1.0]);
        assert_eq!(out[0].id_flags[0], 7);
    }
}
