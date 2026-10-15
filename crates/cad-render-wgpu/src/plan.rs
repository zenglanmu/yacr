//! Batch planning, GPU packing and pipeline layout helpers.

use super::*;
use std::collections::{HashMap, HashSet};

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

/// Bytes of the per-batch uniform struct: `mat4x4<f32>` (64) + `vec4<f32>` tint
/// (16) + `vec4<u32> key_flags` (16) in
/// `shaders/{line,mesh,image,line_instanced,mesh_merged}.wgsl`. `key_flags.x` is
/// the batch's renderer pick key and `.y` its [`INSTANCE_FLAG_*`] bitfield; the
/// uniform is 16-byte aligned, so the struct is 96 bytes.
pub(crate) const UNIFORM_STRUCT_BYTES: u32 = 96;

/// Bytes of the selection-flags control uniform (`enabled` + row width + two
/// reserved words, 16-byte aligned for the uniform layout).
pub(crate) const FLAGS_CTL_BYTES: u32 = 16;

/// Texel format of the selection-flags table: one `u32` of bits per key, split
/// across the four `unorm8` channels. An integer format with `TEXTURE_BINDING`
/// on every tier, addressed with `textureLoad` (no sampler), so the layout stays
/// WebGL2-safe.
pub(crate) const FLAGS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Uint;

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

/// Decide the draw tier once per device.
///
/// The accelerated tier (one draw per class via vertex-stage storage-buffer
/// pulling) needs a **read-only storage buffer in the vertex stage**. That is
/// core WebGPU but absent from WebGL2, so it is capability-gated here rather
/// than assumed:
///
/// * the backend's capability snapshot must report storage buffers (WebGL2 does
///   not; `caps_for` already encodes this);
/// * `max_storage_buffers_per_shader_stage` must be at least
///   [`MIN_PULLED_STORAGE_BINDINGS`] — the true requirement, not `1`: the pulled
///   merged-mesh pipeline binds three read-only storage buffers in the vertex
///   stage (`mesh_pulled.wgsl`) and the pulled line pipeline two. A device
///   reporting only one or two bindings would pass a `< 1` gate and then fail
///   at bind-group/pipeline creation, so the gate is sized to the real demand;
/// * `max_storage_buffer_binding_size` must be large enough to bind a
///   meaningful pulled buffer.
///
/// The renderer **never requests a feature** from a host-provided device; it
/// only reads `limits()`/`features()` and downgrades silently-but-labelled (the
/// live tier is reported by [`Renderer::render_tier`] and on
/// [`FrameStats::tier`]). Pure, so the decision is unit-testable without a GPU.
pub(crate) fn tier_for(
    storage_buffers: bool,
    limits: &wgpu::Limits,
    storage_support_override: Option<bool>,
) -> RenderTier {
    // Test hook: a caller can strip storage-buffer support to exercise the
    // automatic downgrade without a WebGL2 device.
    if storage_support_override == Some(false) {
        return RenderTier::Universal;
    }
    let forced_on = storage_support_override == Some(true);
    if !forced_on {
        if !storage_buffers {
            return RenderTier::Universal;
        }
        if limits.max_storage_buffers_per_shader_stage < MIN_PULLED_STORAGE_BINDINGS {
            return RenderTier::Universal;
        }
        if limits.max_storage_buffer_binding_size < MIN_PULLED_BINDING_BYTES {
            return RenderTier::Universal;
        }
    }
    RenderTier::Accelerated
}

/// Smallest `max_storage_buffer_binding_size` a device must report for the
/// accelerated tier to be selected. Below this a pulled buffer could not hold
/// even a tiny scene, so the universal per-run path is the honest choice.
pub(crate) const MIN_PULLED_BINDING_BYTES: u64 = 64 * 1024;

/// Number of read-only vertex-stage storage buffers the accelerated tier
/// actually binds in one pipeline: three in `mesh_pulled.wgsl` (vertices,
/// indices, chunk table), two in `line_pulled.wgsl` (instances, chunk table).
/// The tier gate requires this many, so a device that reports fewer cannot
/// select a tier it would then fail to create pipelines for.
pub(crate) const MIN_PULLED_STORAGE_BINDINGS: u32 = 3;

/// Finalise the tier once the pulled resources have been created.
///
/// A device that selected [`RenderTier::Accelerated`] but whose pulled
/// pipelines/layouts could not be built (for example a missing flags layout)
/// must downgrade to [`RenderTier::Universal`]: leaving it labelled Accelerated
/// with a missing pulled path would draw the prepare-time placeholder buffers
/// instead of real geometry.
pub(crate) fn finalize_tier(preliminary: RenderTier, pulled_resources_ready: bool) -> RenderTier {
    if preliminary == RenderTier::Accelerated && !pulled_resources_ready {
        RenderTier::Universal
    } else {
        preliminary
    }
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
    last_bind_group2: Option<wgpu::BindGroup>,
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
            last_bind_group2: None,
            last_vertex: [None, None, None],
            last_index: None,
        }
    }

    pub(crate) fn set_pipeline(
        &mut self,
        pass: &mut wgpu::RenderPass<'_>,
        pipeline: &wgpu::RenderPipeline,
    ) {
        // Skip when the identical pipeline is already bound. Draw order is not
        // changed; only a redundant state-setting call is elided.
        if self.last_pipeline.as_ref() == Some(pipeline) {
            return;
        }
        pass.set_pipeline(pipeline);
        self.last_pipeline = Some(pipeline.clone());
        self.tally.set_pipeline_calls += 1;
    }

    pub(crate) fn set_bind_group0(
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

    /// Bind group 0 without a dynamic offset (the accelerated tier's shared
    /// pulled-camera uniform). Kept separate from [`EncodeState::set_bind_group0`]
    /// so the dynamic-offset dedup state cannot be confused by a static bind.
    pub(crate) fn set_bind_group0_static(
        &mut self,
        pass: &mut wgpu::RenderPass<'_>,
        bind_group: &wgpu::BindGroup,
    ) {
        if self.last_bind_group0.as_ref() == Some(bind_group) && self.last_bind_offset0 == 0 {
            return;
        }
        pass.set_bind_group(0, bind_group, &[]);
        self.last_bind_group0 = Some(bind_group.clone());
        self.last_bind_offset0 = 0;
        self.tally.set_bind_group_calls += 1;
    }

    /// Bind the sampled texture (group 2; group 1 is the selection-flags table).
    pub(crate) fn set_bind_group2(
        &mut self,
        pass: &mut wgpu::RenderPass<'_>,
        bind_group: &wgpu::BindGroup,
    ) {
        if self.last_bind_group2.as_ref() == Some(bind_group) {
            return;
        }
        pass.set_bind_group(2, bind_group, &[]);
        self.last_bind_group2 = Some(bind_group.clone());
        self.tally.set_bind_group_calls += 1;
    }

    pub(crate) fn set_vertex_buffer(
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

    pub(crate) fn set_index_buffer(
        &mut self,
        pass: &mut wgpu::RenderPass<'_>,
        buffer: &wgpu::Buffer,
    ) {
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
        // A conic batch owns no per-batch buffers; it is drawn through the SDF
        // instance-run path, never `draw_batch`.
        RenderTopology::Conics => {}
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

/// Submit one coalesced SDF conic run: `6` unit-quad vertices expanded once per
/// instance in `start..start + count` of `stream`.
///
/// `stream` is the instance buffer (slot 1); `quad` is the shared unit quad
/// `[0,1]^2` (slot 0). Instances are contiguous and in original upload order, so
/// the run reproduces exactly the draw order the same batches would have had
/// individually.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_arc_instance_run(
    pass: &mut wgpu::RenderPass<'_>,
    state: &mut EncodeState,
    quad: &wgpu::Buffer,
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
    state.set_vertex_buffer(pass, 0, quad);
    state.set_vertex_buffer(pass, 1, stream);
    pass.draw(0..6, start..start + count);
}

/// Draw and clear a pending coalesced SDF conic run.
///
/// `pending` is `(stream index, first instance, one-past-last instance)`. Returns
/// whether a draw was issued (so the caller can count draw calls).
#[allow(clippy::too_many_arguments)]
pub(crate) fn flush_pending_arc(
    pass: &mut wgpu::RenderPass<'_>,
    state: &mut EncodeState,
    pending: &mut Option<(usize, u32, u32)>,
    uniforms: &UniformArena,
    stream_slots: &[u32],
    streams: &[ArcStream],
    quad: &wgpu::Buffer,
    pipeline: &wgpu::RenderPipeline,
) -> bool {
    let Some((stream, start, end)) = pending.take() else {
        return false;
    };
    let (bind_group, offset) = uniforms.binding(stream_slots[stream] as usize);
    draw_arc_instance_run(
        pass,
        state,
        quad,
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

/// Vertices a resident batch charges the frame budget this frame.
///
/// An SDF conic batch draws `6` unit-quad vertices per instance (one instance =
/// one bounding quad); every other topology charges its packed vertex count.
/// This is the documented unit change from "vertices = stored geometry" to
/// "vertices = rasterized primitives" (see `docs/render-order.md`).
pub(crate) fn batch_budget_vertices(batch: &GpuBatch) -> usize {
    if batch.topology == RenderTopology::Conics {
        batch.arc_instance_count() as usize * 6
    } else {
        batch.vertex_count as usize
    }
}

/// Triangles a resident batch charges the frame budget this frame: `2` per SDF
/// conic instance (a quad is two triangles), the mesh triangle count, else `0`.
pub(crate) fn batch_budget_triangles(batch: &GpuBatch) -> usize {
    match batch.topology {
        RenderTopology::Conics => batch.arc_instance_count() as usize * 2,
        RenderTopology::Mesh => batch.index_count as usize / 3,
        _ => 0,
    }
}

pub(crate) fn plan_from_gpu(batches: &[GpuBatch], budget: &FrameBudget) -> GpuPlan {
    let mut usage = cad_scene::FrameUsage::default();
    let mut accepted = Vec::with_capacity(batches.len());
    for (i, batch) in batches.iter().enumerate() {
        let vertices = batch_budget_vertices(batch);
        let triangles = batch_budget_triangles(batch);
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
        let triangles = batch_budget_triangles(batch);
        let result = budget
            .charge_bytes(&mut next, batch.upload_size_bytes())
            .and_then(|()| budget.charge(&mut next, batch_budget_vertices(batch), triangles));
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

/// Pack one per-batch uniform: `mat4x4<f32>` transform, `vec4<f32>` tint and
/// `vec4<u32> key_flags` (x = renderer pick key, y = [`INSTANCE_FLAG_*`] bits).
///
/// Word-oriented so the byte layout is exactly what the shaders declare and does
/// not depend on the host's float representation or `bytemuck` alignment rules.
pub(crate) fn uniform_bytes(matrix: &[f32; 16], tint: [f32; 4], key_flags: [u32; 4]) -> [u8; 96] {
    let mut words = [0u32; 24];
    for (slot, value) in matrix.iter().enumerate() {
        words[slot] = value.to_bits();
    }
    for (slot, value) in tint.iter().enumerate() {
        words[16 + slot] = value.to_bits();
    }
    words[20..24].copy_from_slice(&key_flags);
    let mut bytes = [0u8; UNIFORM_STRUCT_BYTES as usize];
    for (slot, word) in words.iter().enumerate() {
        bytes[slot * 4..slot * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    bytes
}

/// Sanitise a batch colour for the uniform: finite `[0, 1]` channels.///
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

// Shaders are checked into `shaders/` so they can be validated as files and
// stay reviewable; `include_str!` keeps a single source of truth.
pub(crate) const LINE_SHADER: &str = include_str!("../shaders/line.wgsl");

// Instanced line shader. One instance per 2-vertex `LineList` segment; the
// per-instance attributes are expanded against a shared unit segment so the
// dominant CAD primitive class collapses to one draw call per contiguous run.
pub(crate) const LINE_INSTANCED_SHADER: &str = include_str!("../shaders/line_instanced.wgsl");

// Accelerated (vertex-pulling) instanced line shader: one draw covers every
// instanced line segment of the resident scene by pulling the per-instance
// record from a read-only vertex-stage storage buffer. Capability-gated (see
// `Renderer::initialize_with_device`); deliberately not WebGL2-safe.
pub(crate) const LINE_PULLED_SHADER: &str = include_str!("../shaders/line_pulled.wgsl");

// Mesh shader. Shading is intentionally explicit: a single fixed headlight
// direction, N·L diffuse plus a constant ambient term. There is no claim of
// physically based rendering, environment lighting or specular response.
pub(crate) const MESH_SHADER: &str = include_str!("../shaders/mesh.wgsl");

// Merged-mesh shader: the same shading body as `MESH_SHADER` with an extra
// per-vertex `entity_flags` attribute and a per-vertex baked colour (the
// per-batch tint is folded in at upload, see `push_mesh_vertices`).
pub(crate) const MESH_MERGED_SHADER: &str = include_str!("../shaders/mesh_merged.wgsl");

// Accelerated (vertex-pulling) merged-mesh shader: one indexed draw covers
// every flat-merged mesh chunk by pulling vertices and triangle indices from
// read-only vertex-stage storage buffers. Capability-gated; not WebGL2-safe.
pub(crate) const MESH_PULLED_SHADER: &str = include_str!("../shaders/mesh_pulled.wgsl");

// Raster image shader. It samples a texture and modulates it by a per-image
// alpha/colour factor; there is no lighting model.
pub(crate) const IMAGE_SHADER: &str = include_str!("../shaders/image.wgsl");

// SDF conic shader (slice B). One instance per analytic circle/arc; the shared
// unit quad is expanded to the conic's extent and the fragment shader computes
// the signed distance (see `docs/adr/0008-sdf-conic-rendering.md`).
pub(crate) const ARC_SHADER: &str = include_str!("../shaders/arc.wgsl");

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

/// Byte stride of one [`ArcInstance`]: four `vec4<f32>` (`center_r`, `uvec`,
/// `vvec`, `rgba`) plus one `vec2<u32>` (`id_flags`).
pub(crate) const ARC_INSTANCE_STRIDE: u32 = 72;

// The scene's public `CONIC_INSTANCE_BYTES` and this stride must never drift.
const _: () = assert!(ARC_INSTANCE_STRIDE == cad_scene::CONIC_INSTANCE_BYTES);

/// Reused instance-flag bit: this conic is a closed full circle. Lives in the
/// second component of [`ArcInstance::id_flags`], alongside the
/// [`INSTANCE_FLAG_*`] bits.
pub(crate) const ARC_FLAG_FULL_CIRCLE: u32 = 1 << 3;

/// One SDF conic instance (a slice-1 circle/arc).
///
/// `#[repr(C)]` + `Pod` so the packed bytes are exactly the GPU layout; a unit
/// test fences the stride against `size_of`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ArcInstance {
    /// `xyz` = center in the stream's fixed-point-relative world space,
    /// `w` = the major radius (a slice-1 circle's radius).
    pub(crate) center_r: [f32; 4],
    /// `xyz` = unit major direction, `w` = start angle in `[0, τ)`.
    pub(crate) uvec: [f32; 4],
    /// `xyz` = minor semi-diameter vector (conjugate), `w` = signed sweep.
    pub(crate) vvec: [f32; 4],
    /// Sanitised linear sRGB colour (`rgb`) and clamped alpha (`a`).
    pub(crate) rgba: [f32; 4],
    /// `[0]` = renderer pick key, `[1]` = `INSTANCE_FLAG_*` |
    /// [`ARC_FLAG_FULL_CIRCLE`].
    pub(crate) id_flags: [u32; 2],
}

/// The exact packed bytes of `instances` (one contiguous GPU buffer payload).
pub(crate) fn arc_instance_bytes(instances: &[ArcInstance]) -> Vec<u8> {
    bytemuck::cast_slice(instances).to_vec()
}

/// Per-instance vertex-buffer layout (slot 1 of the SDF conic pipeline).
///
/// `shader_location` 1..=5 so the shared unit quad keeps `@location(0)`.
pub(crate) fn arc_instance_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
        1 => Float32x4,
        2 => Float32x4,
        3 => Float32x4,
        4 => Float32x4,
        5 => Uint32x2
    ];
    wgpu::VertexBufferLayout {
        array_stride: u64::from(ARC_INSTANCE_STRIDE),
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &ATTRIBUTES,
    }
}

/// Whether a scene batch's conics are converted to the SDF instance path.
///
/// Only an opaque `Conics` batch with at least one conic is converted. A
/// **transparent** conic batch has no supported draw path (the SDF conic
/// pipeline is opaque-only and `draw_batch` has no conic arm), so it is
/// **rejected at upload validation** rather than silently dropped. An invisible
/// (`alpha <= 0`) conic is reported as invisible and never drawn.
pub(crate) fn arc_batch_is_instanced(batch: &cad_scene::RenderBatch) -> bool {
    batch.topology == RenderTopology::Conics
        && !batch.conics.is_empty()
        && classify_alpha(batch.alpha) == AlphaClass::Opaque
}

/// One batch converted to the SDF conic path.
pub(crate) struct ConvertedArc {
    /// Index into the full batch slice.
    pub(crate) batch: usize,
}

/// Build the SDF conic conversion list for a batch set.
pub(crate) fn convert_arc_batches(batches: &[cad_scene::RenderBatch]) -> Vec<ConvertedArc> {
    batches
        .iter()
        .enumerate()
        .filter(|(_, batch)| arc_batch_is_instanced(batch))
        .map(|(index, _)| ConvertedArc { batch: index })
        .collect()
}

/// One memory- and precision-bounded static SDF conic stream, ready to upload.
pub(crate) struct ArcChunk {
    /// The stream's fixed-point reference.
    pub(crate) origin: [f64; 3],
    /// Packed instances, in original batch order.
    pub(crate) instances: Vec<ArcInstance>,
}

/// One conic's contribution to a chunk: its batch, index within the batch, and
/// the world-space AABB of its full extent (used for the radius split).
struct ArcItem {
    batch: usize,
    conic: usize,
    min: [f64; 3],
    max: [f64; 3],
    batch_origin: [f64; 3],
}

struct ArcChunkBuilder {
    item_start: usize,
    count: usize,
    shared_origin: Option<[f64; 3]>,
    min: [f64; 3],
    max: [f64; 3],
}

impl ArcChunkBuilder {
    fn accepts(&self, item: &ArcItem, cap: usize, max_radius: f64) -> bool {
        if self.count + 1 > cap {
            return false;
        }
        if self.shared_origin == Some(item.batch_origin) {
            return true;
        }
        let union_min = [
            self.min[0].min(item.min[0]),
            self.min[1].min(item.min[1]),
            self.min[2].min(item.min[2]),
        ];
        let union_max = [
            self.max[0].max(item.max[0]),
            self.max[1].max(item.max[1]),
            self.max[2].max(item.max[2]),
        ];
        aabb_half_diagonal(union_min, union_max) <= max_radius
    }

    fn push(&mut self, item: &ArcItem) {
        if self.shared_origin != Some(item.batch_origin) {
            self.shared_origin = None;
        }
        for axis in 0..3 {
            self.min[axis] = self.min[axis].min(item.min[axis]);
            self.max[axis] = self.max[axis].max(item.max[axis]);
        }
        self.count += 1;
    }
}

/// Partition the converted conic sources into instance-stream chunks and pack
/// them.
///
/// Mirrors [`build_instance_chunks`] with the conic's full-extent AABB: chunks
/// are built in converted order (original batch order), each conic joins the
/// current chunk only while that keeps it within `max_instances` and — for a
/// mixed-origin chunk — within `max_radius` of its AABB midpoint, otherwise it
/// starts a new chunk. An oversized batch is split across several chunks (one run
/// per piece). No empty chunk is ever produced.
///
/// Returns the chunks, per input batch its run list (`None` for a batch that was
/// not converted), and the number of conics skipped as degenerate at pack (a
/// defensive guard the provider already excludes; counted, never silent).
/// `keys[i]` is `batches[i]`'s renderer pick key.
pub(crate) fn build_arc_chunks(
    batches: &[cad_scene::RenderBatch],
    converted: &[ConvertedArc],
    keys: &[u32],
    max_instances: usize,
    max_radius: f64,
) -> (Vec<ArcChunk>, Vec<Option<Vec<ArcRun>>>, u64) {
    debug_assert_eq!(batches.len(), keys.len());
    let cap = max_instances.max(1);
    let mut items: Vec<ArcItem> = Vec::new();
    for source in converted {
        let batch = &batches[source.batch];
        let batch_origin = [
            batch.local_origin.x,
            batch.local_origin.y,
            batch.local_origin.z,
        ];
        for (conic_index, conic) in batch.conics.iter().enumerate() {
            let Some((min, max)) = conic.world_aabb() else {
                continue;
            };
            items.push(ArcItem {
                batch: source.batch,
                conic: conic_index,
                min,
                max,
                batch_origin,
            });
        }
    }

    let mut builders: Vec<ArcChunkBuilder> = Vec::new();
    let mut current: Option<ArcChunkBuilder> = None;
    for (pos, item) in items.iter().enumerate() {
        if let Some(chunk) = current.as_mut() {
            if chunk.accepts(item, cap, max_radius) {
                chunk.push(item);
                continue;
            }
        }
        if let Some(chunk) = current.take() {
            builders.push(chunk);
        }
        current = Some(ArcChunkBuilder {
            item_start: pos,
            count: 1,
            shared_origin: Some(item.batch_origin),
            min: item.min,
            max: item.max,
        });
    }
    if let Some(chunk) = current.take() {
        builders.push(chunk);
    }

    let mut runs: Vec<Option<Vec<ArcRun>>> = vec![None; batches.len()];
    let mut chunks = Vec::with_capacity(builders.len());
    let mut degenerate_skips = 0u64;
    for (stream, builder) in builders.iter().enumerate() {
        let origin = match builder.shared_origin {
            Some(origin) => origin,
            None => [
                (builder.min[0] + builder.max[0]) * 0.5,
                (builder.min[1] + builder.max[1]) * 0.5,
                (builder.min[2] + builder.max[2]) * 0.5,
            ],
        };
        let mut instances = Vec::with_capacity(builder.count);
        let mut previous_batch: Option<usize> = None;
        for item in &items[builder.item_start..builder.item_start + builder.count] {
            let offset = instances.len() as u32;
            let pushed_ok = push_arc_instance(
                &mut instances,
                &batches[item.batch],
                item.conic,
                origin,
                keys[item.batch],
            );
            if !pushed_ok {
                degenerate_skips += 1;
            }
            let pushed = instances.len() as u32 - offset;
            if pushed == 0 {
                continue;
            }
            if previous_batch == Some(item.batch) {
                if let Some(run) = runs[item.batch].as_mut().and_then(|list| list.last_mut()) {
                    if run.stream == stream as u32 && run.offset + run.count == offset {
                        run.count += pushed;
                        previous_batch = Some(item.batch);
                        continue;
                    }
                }
            }
            runs[item.batch].get_or_insert_with(Vec::new).push(ArcRun {
                stream: stream as u32,
                offset,
                count: pushed,
            });
            previous_batch = Some(item.batch);
        }
        chunks.push(ArcChunk { origin, instances });
    }
    (chunks, runs, degenerate_skips)
}

/// Normalize an angle into `[0, τ)`.
fn normalize_tau(angle: f64) -> f64 {
    let t = angle.rem_euclid(std::f64::consts::TAU);
    if t >= std::f64::consts::TAU {
        0.0
    } else {
        t
    }
}

/// Append one [`ArcInstance`] for `batch.conics[conic_index]`.
///
/// The center is stored relative to the chunk `origin` (reduced in `f64` before
/// narrowing); the major direction is a unit vector and the minor is carried as
/// its full semi-diameter vector, so the parameterization
/// `p(t) = center + (r·u)·cos t + v·sin t` is exact. `pick_key` is the batch's
/// renderer pick key. Returns `false` (pushing nothing) for a degenerate conic
/// (`sweep == 0 && !full`, or a zero-length major axis); the caller counts the
/// skip so it is never silent, though the scene/provider already exclude it.
pub(crate) fn push_arc_instance(
    out: &mut Vec<ArcInstance>,
    batch: &cad_scene::RenderBatch,
    conic_index: usize,
    origin: [f64; 3],
    pick_key: u32,
) -> bool {
    let Some(conic) = batch.conics.get(conic_index) else {
        return false;
    };
    if !conic.full && conic.sweep == 0.0 {
        return false;
    }
    let major_len = (conic.major.x * conic.major.x
        + conic.major.y * conic.major.y
        + conic.major.z * conic.major.z)
        .sqrt();
    if !(major_len.is_finite() && major_len > 0.0) {
        return false;
    }
    let u = [
        conic.major.x / major_len,
        conic.major.y / major_len,
        conic.major.z / major_len,
    ];
    let center = [
        (conic.center.x - origin[0]) as f32,
        (conic.center.y - origin[1]) as f32,
        (conic.center.z - origin[2]) as f32,
    ];
    let sweep = if conic.full {
        std::f64::consts::TAU
    } else {
        conic
            .sweep
            .clamp(-std::f64::consts::TAU, std::f64::consts::TAU)
    };
    let full = conic.full || sweep.abs() >= std::f64::consts::TAU - 1e-9;
    let mut flags = batch_instance_flags(batch);
    if full {
        flags |= ARC_FLAG_FULL_CIRCLE;
    }
    let [r, g, b] = renderer_color(batch.color);
    out.push(ArcInstance {
        center_r: [center[0], center[1], center[2], major_len as f32],
        uvec: [
            u[0] as f32,
            u[1] as f32,
            u[2] as f32,
            normalize_tau(conic.start) as f32,
        ],
        vvec: [
            conic.minor.x as f32,
            conic.minor.y as f32,
            conic.minor.z as f32,
            sweep as f32,
        ],
        rgba: [r, g, b, clamp_alpha(batch.alpha)],
        id_flags: [pick_key, flags],
    });
    true
}

// ---------------------------------------------------------------------------
// Accelerated tier: vertex pulling (capability-gated)
// ---------------------------------------------------------------------------
//
// The universal path coalesces draws per *chunk* (a per-chunk `f64` origin
// bounds every stored coordinate to `MAX_STREAM_RADIUS`, so a stream buffer is
// never oversized and the f32 ulp stays under ~1 µm). That still leaves one
// draw per contiguous chunk run. The accelerated tier collapses the converted
// opaque classes to **one draw per class** by concatenating every chunk into a
// single storage buffer and pulling each primitive with
// `@builtin(instance_index)` / `@builtin(vertex_index)`.
//
// What is *not* relaxed: the per-chunk `f64` reduction still happens on the CPU
// at upload (stored coordinate = `(world - origin) as f32`), and the per-chunk
// translation column is still `FrameProjection::batch_matrix_f64(origin)`'s last
// column. The shader only adds a radius-bounded `f32` position to that column,
// so the precision contract is byte-identical to the universal path.
//
// The tier requires a read-only storage buffer in the vertex stage, which
// WebGL2 does not provide. It is chosen once per device in
// `Renderer::initialize_with_device` from the device's own limits/backend and
// never requests a feature from a host-provided device.

/// Byte stride of one [`PulledLine`]: three `vec4<f32>` plus one `vec4<u32>`.
pub(crate) const PULLED_LINE_STRIDE: u32 = 64;

/// Byte stride of one [`PulledMeshVertex`]: three `vec4<f32>` plus one
/// `vec4<u32>`.
pub(crate) const PULLED_MESH_VERTEX_STRIDE: u32 = 64;

/// Byte size of the per-class pulled camera uniform: three `vec4<f32>` columns.
pub(crate) const PULLED_CAMERA_BYTES: u32 = 48;

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

/// The three origin-independent columns of the per-chunk transform.
pub(crate) fn pulled_camera_columns(projection: FrameProjection) -> [[f32; 4]; 3] {
    let m = projection.batch_matrix_f64([0.0, 0.0, 0.0]);
    [
        [m[0], m[1], m[2], m[3]],
        [m[4], m[5], m[6], m[7]],
        [m[8], m[9], m[10], m[11]],
    ]
}

/// The packed `f32` bytes of the pulled camera uniform (three columns).
pub(crate) fn pulled_camera_bytes(columns: &[[f32; 4]; 3]) -> [u8; PULLED_CAMERA_BYTES as usize] {
    let mut bytes = [0u8; PULLED_CAMERA_BYTES as usize];
    for (col, column) in columns.iter().enumerate() {
        for (row, value) in column.iter().enumerate() {
            let at = (col * 4 + row) * 4;
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
    bytes
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
/// image's dynamic slot offset) and the texture/sampler (group 2), then draw the
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
    state.set_bind_group2(pass, &image.texture_bind_group);
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

// ---------------------------------------------------------------------------
// Pick keys and the CPU-side id → entity resolution table
// ---------------------------------------------------------------------------
//
// The id pass (see `pick.rs`) writes one 32-bit **pick key** per fragment. The
// key is a renderer key, `1 + <batch ordinal>`: the first resident batch is key
// 1, and 0 means "no geometry" (the background). `PickTable` is the CPU half of
// that contract: it maps a key back to the batch and to the *full* identities
// that can own a fragment of it.
//
// Why a renderer key rather than a truncated `EntityId`: a `u128` `EntityId`
// does not fit in the 32-bit id target, and packing its low 32 bits would make
// two distinct entities share a fragment value — a silent mis-pick. The table
// keeps the whole `SelectionRef`, so resolution is exact by construction. The
// truncation itself is still *detected and reported* (`PickTable::collisions`),
// because the low-32 form is what a 32-bit-only consumer would see.
//
// A batch may carry **several** sources (the scene builder concatenates adjacent
// line fragments of different entities into one batch, and the merged runs
// concatenate such batches). Fragment geometry does not record which source owns
// which range, so a hit on a multi-source batch resolves to the whole list and is
// reported as ambiguous — never silently attributed to `sources.first()`.

/// The id-pass value for "no geometry": the cleared background.
pub(crate) const PICK_KEY_NONE: u32 = 0;

/// Upper bound on renderer pick keys within one device generation. The key space
/// grows with the batches appended since the last full clear; the bound keeps the
/// flags table (indexed by key) finite and is reported when exhausted rather than
/// silently wrapping.
pub(crate) const MAX_PICK_KEYS: u32 = 1 << 20;

/// Machine key: the batch carries no source at all, so the key cannot name an
/// entity.
pub(crate) const PICK_REASON_NO_SOURCE: &str = "pick-batch-has-no-source";
/// Machine key: the batch carries several sources whose geometry is
/// indistinguishable after representation, so the hit is ambiguous.
pub(crate) const PICK_REASON_MULTI_SOURCE: &str = "pick-multi-source-batch";
/// Machine key: a mesh-face sub-element is not derivable from the id pass (wgpu
/// exposes no primitive index to the fragment stage); CPU refinement is the
/// documented fallback.
pub(crate) const PICK_REASON_SUB_ELEMENT: &str = "pick-sub-element-requires-cpu-refinement";

/// The CPU-side resolution of one pick key.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PickEntry {
    /// Index into the *combined* batch+image list this entry was built from:
    /// batches first (`0..batches.len()`), then images (`batches.len() + i`).
    /// Internal bookkeeping for the table; not part of the public hit surface.
    pub(crate) batch: u32,
    /// Every source that can own a fragment of this key, in the batch's own
    /// order.
    pub(crate) sources: Vec<SelectionRef>,
    /// `true` when one of this entry's sources shares its low-32 `EntityId` with
    /// another distinct identity elsewhere in the table.
    pub(crate) collision: bool,
}

/// A repeated renderer pick key in the resident set.
///
/// Keys are allocated monotonically within a device generation, so a duplicate
/// can only come from the allocator being restarted while keys are still
/// resident — a lifecycle error, never a legitimate state. The table cannot be
/// built unambiguously, so it is reported rather than resolved "first wins".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DuplicatePickKey {
    /// The key carried by two different owners.
    pub(crate) key: u32,
    /// Combined batch/image index of the first owner.
    pub(crate) first: u32,
    /// Combined batch/image index of the second owner.
    pub(crate) second: u32,
}

/// The resolution of one key, owned so a caller can hold it past the table.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResolvedPick {
    pub(crate) batch: u32,
    pub(crate) sources: Vec<SelectionRef>,
    /// The exact identity when the key names exactly one source.
    pub(crate) source: Option<SelectionRef>,
    /// Machine key explaining a non-exact resolution.
    pub(crate) reason: Option<&'static str>,
    pub(crate) collision: bool,
}

/// Key → (batch, full identities) resolution, plus the detected truncation
/// collisions.
///
/// Indexed by key, so `entries[0]` is the background slot and gaps (retired keys)
/// are `None`. Pure data: built from the resident batches and unit-tested without
/// a GPU.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PickTable {
    entries: Vec<Option<PickEntry>>,
    collisions: Vec<PickCollision>,
    /// Highest key present (0 when the table has no entries).
    max_key: u32,
    /// Whole-entity identity → pick keys, so an entity-flag update is
    /// `O(refs)` instead of `O(refs × entries × sources)`. Only sources that are
    /// themselves whole-entity (`sub_element.is_none()`) are indexed: a
    /// sub-element-carrying source is the sub-element overlay and must never be
    /// matched by an entity-level flag (a face highlight would otherwise be
    /// double-tinted, or discarded by `FLAG_HIDDEN`).
    source_index: HashMap<(DocumentId, EntityId, Vec<EntityId>), Vec<u32>>,
}

impl PickTable {
    /// Build the table over `keys[i] -> sources[i]` pairs (the resident batches
    /// and images).
    ///
    /// `key` values must be `1..=MAX_PICK_KEYS`; a value outside that range is
    /// skipped (it can never be written to the id target). A repeated key is a
    /// programming/lifecycle error and is returned as [`DuplicatePickKey`] rather
    /// than silently resolved.
    pub(crate) fn build(
        entries: &[(u32, Vec<SelectionRef>)],
    ) -> Result<PickTable, DuplicatePickKey> {
        let max_key = entries.iter().map(|(key, _)| *key).max().unwrap_or(0);
        let mut slots: Vec<Option<PickEntry>> = vec![None; max_key as usize + 1];
        for (batch, (key, sources)) in entries.iter().enumerate() {
            if *key == PICK_KEY_NONE || *key > MAX_PICK_KEYS {
                continue;
            }
            let slot = &mut slots[*key as usize];
            if let Some(existing) = slot.as_ref() {
                return Err(DuplicatePickKey {
                    key: *key,
                    first: existing.batch,
                    second: batch as u32,
                });
            }
            *slot = Some(PickEntry {
                batch: batch as u32,
                sources: sources.clone(),
                collision: false,
            });
        }
        let collisions = detect_truncation_collisions(&slots);
        let colliding: std::collections::HashSet<u32> = collisions
            .iter()
            .map(|collision| collision.truncated_key)
            .collect();
        for entry in slots.iter_mut().flatten() {
            entry.collision = entry
                .sources
                .iter()
                .any(|source| colliding.contains(&(source.entity.0 as u32)));
        }
        // Index the whole-entity sources so an entity-flag update resolves by
        // identity in `O(refs)` rather than scanning every entry.
        let mut source_index: HashMap<(DocumentId, EntityId, Vec<EntityId>), Vec<u32>> =
            HashMap::new();
        for (key, entry) in slots.iter().enumerate() {
            let Some(entry) = entry else { continue };
            for source in &entry.sources {
                if source.sub_element.is_some() {
                    continue;
                }
                let keys = source_index
                    .entry((source.document, source.entity, source.instance.0.clone()))
                    .or_default();
                let key = key as u32;
                if keys.last() != Some(&key) {
                    keys.push(key);
                }
            }
        }
        Ok(PickTable {
            entries: slots,
            collisions,
            max_key,
            source_index,
        })
    }

    /// Resolve a key read back from the id target. `None` for the background
    /// (`0`) and for a key the current table does not know.
    pub(crate) fn resolve(&self, key: u32) -> Option<ResolvedPick> {
        let entry = self.entries.get(key as usize)?.as_ref()?;
        let source = (entry.sources.len() == 1).then(|| entry.sources[0].clone());
        let reason = if entry.sources.is_empty() {
            Some(PICK_REASON_NO_SOURCE)
        } else if entry.sources.len() > 1 {
            Some(PICK_REASON_MULTI_SOURCE)
        } else {
            None
        };
        Some(ResolvedPick {
            batch: entry.batch,
            sources: entry.sources.clone(),
            source,
            reason,
            collision: entry.collision,
        })
    }

    /// The detected `EntityId` truncation collisions, in key order.
    pub(crate) fn collisions(&self) -> &[PickCollision] {
        &self.collisions
    }

    /// Every pick key whose entry names `source` as one of its **whole-entity**
    /// sources, in key order.
    ///
    /// Identity is `document + entity + instance`; the sub-element is ignored on
    /// the **query** side because the flags path is whole-entity (a mesh face is
    /// a separate overlay). A batch source that itself carries a sub-element is
    /// the sub-element overlay and is **excluded**: an entity-level flag must not
    /// tint (or hide) the face overlay batch. A key is returned once even when
    /// several of its sources match: the caller sets the whole batch's bits,
    /// which is the documented batch-granularity limit for a multi-source run.
    pub(crate) fn keys_for_source(&self, source: &SelectionRef) -> Vec<u32> {
        let identity = (source.document, source.entity, source.instance.0.clone());
        self.source_index
            .get(&identity)
            .cloned()
            .unwrap_or_default()
    }

    /// Highest key present (0 when empty).
    pub(crate) fn max_key(&self) -> u32 {
        self.max_key
    }

    /// Number of live entries.
    pub(crate) fn len(&self) -> usize {
        self.entries.iter().filter(|entry| entry.is_some()).count()
    }
}

/// Group every source's low-32 `EntityId` and report the values shared by two or
/// more *distinct* full identities.
///
/// Distinctness is the full `SelectionRef` (document + entity + instance +
/// sub-element): two sources of the same entity under different insert instances
/// are distinct placements and therefore a real collision for a 32-bit consumer.
fn detect_truncation_collisions(entries: &[Option<PickEntry>]) -> Vec<PickCollision> {
    let mut groups: std::collections::BTreeMap<u32, Vec<SelectionRef>> =
        std::collections::BTreeMap::new();
    for entry in entries.iter().flatten() {
        for source in &entry.sources {
            let truncated = source.entity.0 as u32;
            let group = groups.entry(truncated).or_default();
            if !group.contains(source) {
                group.push(source.clone());
            }
        }
    }
    groups
        .into_iter()
        .filter(|(_, identities)| identities.len() > 1)
        .map(|(truncated_key, identities)| PickCollision {
            truncated_key,
            identities,
        })
        .collect()
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
            conics: Vec::new(),
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

    /// The renderer pick keys these tests use: `1 + batch ordinal`, exactly what
    /// `prepare_upload_batches` assigns (see `PickTable`).
    fn keys(batches: &[cad_scene::RenderBatch]) -> Vec<u32> {
        (0..batches.len() as u32).map(|index| index + 1).collect()
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
        push_line_instances(&mut out, &b, None, origin, 0, 1, 9);
        assert_eq!(out.len(), 1);
        let instance = out[0];
        // (origin-relative) positions
        assert_eq!(instance.p0_width, [-9.0, -18.0, -27.0, 0.35]);
        assert_eq!(instance.p1_pad, [-6.0, -15.0, -24.0, 0.0]);
        // sanitized colour + clamped alpha
        assert_eq!(instance.rgba, [0.25, 0.5, 0.75, 1.0]);
        // the renderer pick key (not a truncated EntityId) + the descriptive flags
        assert_eq!(instance.id_flags[0], 9);
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
        push_line_instances(&mut out, &b, None, origin, 0, 1, 1);
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
            &keys(&batches),
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
        let (chunks, runs) = build_instance_chunks(
            &batches,
            &converted(&batches),
            &keys(&batches),
            2,
            MAX_STREAM_RADIUS,
        );
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
            &keys(&batches),
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
            &keys(&batches),
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
            &keys(&batches),
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

    /// The renderer pick keys these tests use: `1 + batch ordinal`, exactly what
    /// `prepare_upload_batches` assigns (see `PickTable`).
    fn keys(batches: &[cad_scene::RenderBatch]) -> Vec<u32> {
        (0..batches.len() as u32).map(|index| index + 1).collect()
    }

    /// A quad mesh (4 vertices, 2 triangles) at `origin`, tint `color`.
    fn quad(origin: [f64; 3], color: [f32; 3], entity: u128) -> cad_scene::RenderBatch {
        let half = 1.0f32;
        cad_scene::RenderBatch {
            conics: Vec::new(),
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
            &keys(&batches),
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
        let (chunks, runs) =
            build_mesh_chunks(&batches, &[0, 1], &keys(&batches), 4, MAX_STREAM_RADIUS);
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
        let (chunks, runs) =
            build_mesh_chunks(&batches, &[0], &keys(&batches), 4, MAX_STREAM_RADIUS);
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
            &keys(&batches),
            MAX_VERTICES_PER_MESH_STREAM,
            MAX_STREAM_RADIUS,
        );
        assert_eq!(chunks.len(), 2, "a mirrored batch is a pipeline boundary");
        assert!(!chunks[0].mirrored);
        assert!(chunks[1].mirrored);
    }

    #[test]
    fn push_mesh_vertices_bakes_the_tint_and_carries_the_pick_key() {
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
        push_mesh_vertices(&mut out, &batch, piece, origin, 9);
        assert_eq!(out.len(), 4);
        // Baked colour = sanitized tint * sanitized vertex colour.
        assert_eq!(out[0].color, [0.5, 0.0, 0.0]);
        assert_eq!(out[1].color, [0.0, 0.25, 0.0]);
        assert_eq!(out[2].color, [0.0, 0.0, 0.75]);
        assert_eq!(out[3].color, [0.5, 0.25, 0.75]);
        // The renderer pick key/flags ride per vertex (the legacy per-batch draw
        // carried no explicit pick key at all).
        for vertex in &out {
            assert_eq!(vertex.entity_flags[0], 9);
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
            &keys(&batches),
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
                1
            } else {
                2
            };
            assert_eq!(
                vertex.entity_flags[0], expected,
                "vertex {index} must carry its own batch's pick key"
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
        push_mesh_vertices(&mut out, &batch, piece, origin, 1);
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
        push_line_instances(&mut out, &batch, Some(&indices), origin, 0, 5, 7);
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
        // Colour comes from the batch (as the legacy line uniform did); the pick
        // key is new per-instance metadata the legacy draw did not carry.
        assert_eq!(out[0].rgba, [0.25, 0.5, 0.75, 1.0]);
        assert_eq!(out[0].id_flags[0], 7);
    }
}

#[cfg(test)]
mod pick_table_tests {
    use super::*;
    use cad_domain::{EntityId, InstancePath, SelectionRef};

    fn source(entity: u128, instance: &[u128]) -> SelectionRef {
        SelectionRef {
            document: cad_domain::DocumentId(1),
            entity: EntityId(entity),
            instance: InstancePath(instance.iter().copied().map(EntityId).collect()),
            sub_element: None,
        }
    }

    #[test]
    fn pick_key_none_is_the_background_and_never_resolves() {
        let table = PickTable::build(&[(1, vec![source(7, &[])])]).expect("unique keys");
        assert_eq!(PICK_KEY_NONE, 0);
        assert!(table.resolve(PICK_KEY_NONE).is_none());
        assert!(table.resolve(999).is_none(), "an unknown key is not a hit");
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn a_single_source_key_resolves_exactly_and_preserves_the_full_u128() {
        // An EntityId far beyond 32 bits: the table must return the whole value,
        // not its low 32 bits.
        let entity = (1u128 << 100) | 0xDEAD_BEEF;
        let table = PickTable::build(&[(3, vec![source(entity, &[])])]).expect("unique keys");
        let resolved = table.resolve(3).expect("known key");
        assert_eq!(resolved.batch, 0, "batch ordinal is the entry position");
        let hit = resolved.source.expect("exact");
        assert_eq!(hit.entity, EntityId(entity), "full u128 identity preserved");
        assert_eq!(
            hit.entity.0 as u32, 0xDEAD_BEEF,
            "low 32 bits are the truncation"
        );
        assert!(resolved.reason.is_none());
        assert!(!resolved.collision);
        assert!(table.collisions().is_empty());
    }

    #[test]
    fn a_multi_source_batch_is_ambiguous_and_names_every_candidate() {
        // The scene builder concatenates adjacent line fragments of different
        // entities into one batch; the id pass cannot tell them apart, so the
        // resolution must list both and never silently pick the first.
        let a = source(11, &[]);
        let b = source(22, &[]);
        let table = PickTable::build(&[(1, vec![a.clone(), b.clone()])]).expect("unique keys");
        let resolved = table.resolve(1).expect("known key");
        assert!(resolved.source.is_none(), "no exact identity");
        assert_eq!(resolved.sources, vec![a, b]);
        assert_eq!(resolved.reason, Some(PICK_REASON_MULTI_SOURCE));
    }

    #[test]
    fn a_batch_with_no_source_resolves_to_no_identity() {
        let table = PickTable::build(&[(1, Vec::new())]).expect("unique keys");
        let resolved = table.resolve(1).expect("known key");
        assert!(resolved.source.is_none());
        assert!(resolved.sources.is_empty());
        assert_eq!(resolved.reason, Some(PICK_REASON_NO_SOURCE));
    }

    #[test]
    fn keys_for_source_matches_identity_and_ignores_the_sub_element() {
        let instance_a = source(20, &[10]);
        let instance_b = source(20, &[20]);
        let table = PickTable::build(&[
            (1, vec![instance_a.clone()]),
            (2, vec![instance_b.clone()]),
            (3, vec![source(7, &[])]),
        ])
        .expect("unique keys");

        // Exact placement match, in key order.
        assert_eq!(table.keys_for_source(&instance_a), vec![1]);
        assert_eq!(table.keys_for_source(&instance_b), vec![2]);
        // The same entity under a different instance is a different placement.
        assert!(table.keys_for_source(&source(20, &[99])).is_empty());
        // A sub-element ref names the same whole entity: the flags path ignores
        // the sub-element and matches the placement's key.
        let mut face = instance_a.clone();
        face.sub_element = Some(cad_domain::SubElementId {
            source_key: "face-a".into(),
            topology_revision: cad_domain::Revision(0),
        });
        assert_eq!(table.keys_for_source(&face), vec![1]);
    }

    #[test]
    fn keys_for_source_returns_a_multi_source_batch_key_once() {
        // A merged batch whose sources are two entities: a flag on either source
        // must map to the batch's single key (batch-granularity limit).
        let a = source(11, &[]);
        let b = source(22, &[]);
        let table = PickTable::build(&[(5, vec![a.clone(), b.clone()])]).expect("unique keys");
        assert_eq!(table.keys_for_source(&a), vec![5]);
        assert_eq!(table.keys_for_source(&b), vec![5]);
    }

    #[test]
    fn a_sub_element_batch_source_is_not_matched_by_an_entity_flag() {
        // The sub-element overlay batch's source is `E#face-a`. Selecting the
        // whole entity `E` must not resolve to that batch's key, or the face
        // overlay would be double-tinted (and a future `FLAG_HIDDEN` would
        // discard the face highlight). S2.
        let face = |entity: u128| {
            let mut source = source(entity, &[]);
            source.sub_element = Some(cad_domain::SubElementId {
                source_key: "face-a".into(),
                topology_revision: cad_domain::Revision(0),
            });
            source
        };
        let whole = source(40, &[]);
        let table = PickTable::build(&[(1, vec![whole.clone()]), (2, vec![face(40)])])
            .expect("unique keys");
        assert_eq!(
            table.keys_for_source(&whole),
            vec![1],
            "only the whole-entity batch matches"
        );
        // A batch that carries *only* the face source is never matched by an
        // entity-level ref (nor by a different entity).
        let face_only = PickTable::build(&[(2, vec![face(40)])]).expect("unique keys");
        assert!(face_only.keys_for_source(&whole).is_empty());
        assert!(face_only.keys_for_source(&source(41, &[])).is_empty());
    }

    #[test]
    fn distinct_entities_sharing_the_low_32_bits_are_reported_as_a_collision() {
        // Two distinct u128 entities whose low 32 bits agree. The dense key still
        // resolves exactly (no mis-pick), and the truncation is reported.
        let low = 0x0000_1234u128;
        let a = low;
        let b = (1u128 << 64) | low;
        let table = PickTable::build(&[(1, vec![source(a, &[])]), (2, vec![source(b, &[])])])
            .expect("unique keys");
        assert_eq!(table.collisions().len(), 1);
        let collision = &table.collisions()[0];
        assert_eq!(collision.truncated_key, 0x1234);
        assert_eq!(collision.identities.len(), 2);
        // Both keys still resolve to their own full identity...
        assert_eq!(
            table.resolve(1).unwrap().source.unwrap().entity,
            EntityId(a)
        );
        assert_eq!(
            table.resolve(2).unwrap().source.unwrap().entity,
            EntityId(b)
        );
        // ...and both are flagged as colliding.
        assert!(table.resolve(1).unwrap().collision);
        assert!(table.resolve(2).unwrap().collision);
    }

    #[test]
    fn the_same_entity_under_two_instances_is_a_collision_too() {
        // One block placed twice: the same EntityId, two distinct placements. For
        // a 32-bit consumer they are indistinguishable, so this is reported.
        let a = source(5, &[100]);
        let b = source(5, &[200]);
        let table = PickTable::build(&[(1, vec![a]), (2, vec![b])]).expect("unique keys");
        assert_eq!(table.collisions().len(), 1);
        assert_eq!(table.collisions()[0].identities.len(), 2);
    }

    #[test]
    fn distinct_entities_with_distinct_low_bits_are_not_a_collision() {
        let table = PickTable::build(&[(1, vec![source(1, &[])]), (2, vec![source(2, &[])])])
            .expect("unique keys");
        assert!(table.collisions().is_empty());
        assert!(!table.resolve(1).unwrap().collision);
        assert!(!table.resolve(2).unwrap().collision);
    }

    #[test]
    fn a_repeated_key_is_a_hard_error_not_first_wins() {
        // Keys are allocated monotonically, so a duplicate can only come from a
        // lifecycle bug (the allocator restarted while keys were resident).
        // Silently keeping one owner would resolve a fragment to the wrong
        // entity, so the table refuses to build.
        let duplicate = PickTable::build(&[(1, vec![source(1, &[])]), (1, vec![source(2, &[])])])
            .expect_err("a duplicate key must not build a table");
        assert_eq!(duplicate.key, 1);
        assert_eq!(duplicate.first, 0);
        assert_eq!(duplicate.second, 1);
    }

    #[test]
    fn max_key_tracks_the_highest_live_key() {
        let table = PickTable::build(&[(4, vec![source(1, &[])]), (9, vec![source(2, &[])])])
            .expect("unique keys");
        assert_eq!(table.max_key(), 9);
        assert!(table.resolve(5).is_none(), "a gap is not a hit");
    }
}

/// Per-batch world→clip projection for one frame.
///
/// The 2D variant reproduces, bit for bit, the f64 camera-centre subtraction the
/// large-coordinate precision test fences before narrowing to `f32`; the 3D
/// variant multiplies the view-projection by the batch origin (matching the
/// previous `translate_left(&vp, origin)`). Building the matrix inline per
/// accepted batch avoids materialising and cloning a full `Vec<[f32; 16]>` every
/// frame.
#[derive(Clone, Copy)]
pub(crate) enum FrameProjection {
    Ortho {
        sx: f64,
        sy: f64,
        /// Camera centre, kept in f64 so the subtraction stays precise.
        center: [f64; 3],
        z_plane: f32,
        /// World units per pixel, passed to the SDF conic shader so its quad
        /// margin can be floored at ~2 px (see `shaders/arc.wgsl`).
        world_per_px: f64,
    },
    Perspective {
        vp: [[f32; 4]; 4],
        /// Approximate world units per pixel at the camera target; the SDF
        /// conic margin floor uses it so a small on-screen circle's fringe is
        /// not clipped on the 3D path either.
        world_per_px: f64,
    },
}

impl FrameProjection {
    /// World units per pixel, for the SDF conic quad-margin floor.
    pub(crate) fn world_per_px(self) -> f64 {
        match self {
            FrameProjection::Ortho { world_per_px, .. }
            | FrameProjection::Perspective { world_per_px, .. } => world_per_px,
        }
    }

    pub(crate) fn batch_matrix(self, origin: [f32; 3]) -> [f32; 16] {
        self.batch_matrix_f64([
            f64::from(origin[0]),
            f64::from(origin[1]),
            f64::from(origin[2]),
        ])
    }

    /// Per-batch matrix for an `f64` origin.
    ///
    /// The **Ortho** variant keeps the origin and the camera centre in `f64`
    /// through the subtraction before narrowing — the precision contract fenced
    /// by `large_coordinate_batch_keeps_precision`. The instanced line path
    /// passes its stream's exact `f64` fixed-point origin here, so stream
    /// positions stay small even at large drawing coordinates. The **Perspective**
    /// variant narrows the origin to `f32` before `translate_left`, so this `f64`
    /// camera-relative subtraction does not apply on the 3D path.
    pub(crate) fn batch_matrix_f64(self, origin: [f64; 3]) -> [f32; 16] {
        match self {
            FrameProjection::Ortho {
                sx,
                sy,
                center,
                z_plane,
                ..
            } => [
                sx as f32,
                0.0,
                0.0,
                0.0,
                0.0,
                sy as f32,
                0.0,
                0.0,
                0.0,
                0.0,
                1.0,
                0.0,
                (sx * (origin[0] - center[0])) as f32,
                (sy * (origin[1] - center[1])) as f32,
                (origin[2] - center[2]) as f32 + z_plane,
                1.0,
            ],
            // Perspective narrows the origin to `f32` *before* applying the
            // (already `f32`) view-projection, so the `f64` camera-relative
            // subtraction claimed for the Ortho arm above does not apply here.
            FrameProjection::Perspective { vp, .. } => {
                translate_left(&vp, [origin[0] as f32, origin[1] as f32, origin[2] as f32])
            }
        }
    }
}

#[cfg(test)]
mod accelerated_tier_tests {
    use super::*;

    #[test]
    fn pulled_strides_match_their_wgsl_vec4_layouts() {
        // The WGSL structs are four `vec4`s (16 B each); the CPU structs must be
        // exactly that stride or the storage array indexing is wrong.
        assert_eq!(
            std::mem::size_of::<PulledLine>(),
            PULLED_LINE_STRIDE as usize
        );
        assert_eq!(
            std::mem::size_of::<PulledMeshVertex>(),
            PULLED_MESH_VERTEX_STRIDE as usize
        );
        assert_eq!(PULLED_CAMERA_BYTES, 48);
        // The layout is alignment-clean for the storage array stride.
        assert_eq!(std::mem::align_of::<PulledLine>(), 4);
        assert_eq!(std::mem::align_of::<PulledMeshVertex>(), 4);
    }

    #[test]
    fn tier_gate_requires_storage_buffers_and_a_usable_binding() {
        let limits = wgpu::Limits::default();
        // A capable device (storage buffers, defaults) selects accelerated.
        assert_eq!(
            tier_for(true, &limits, None),
            RenderTier::Accelerated,
            "a storage-buffer device must select the accelerated tier"
        );
        // WebGL2 (no storage buffers) downgrades silently.
        assert_eq!(
            tier_for(false, &limits, None),
            RenderTier::Universal,
            "a device without storage buffers must downgrade"
        );
        // The test hook can strip support on a capable device.
        assert_eq!(tier_for(true, &limits, Some(false)), RenderTier::Universal);
        // ...and force it on a device that reports none, so the tier is testable
        // on any adapter.
        assert_eq!(
            tier_for(false, &limits, Some(true)),
            RenderTier::Accelerated
        );
    }

    #[test]
    fn tier_gate_rejects_zero_storage_buffers_and_tiny_bindings() {
        // The gate must require the *true* number of vertex-stage storage
        // bindings (three: `mesh_pulled.wgsl` binds vertices, indices and the
        // chunk table), not one. A device reporting 0, 1 or 2 must downgrade
        // rather than select a tier whose pipelines it cannot create.
        for reported in 0..MIN_PULLED_STORAGE_BINDINGS {
            let limits = wgpu::Limits {
                max_storage_buffers_per_shader_stage: reported,
                ..wgpu::Limits::default()
            };
            assert_eq!(
                tier_for(true, &limits, None),
                RenderTier::Universal,
                "{reported} storage bindings must downgrade"
            );
        }
        let limits = wgpu::Limits {
            max_storage_buffers_per_shader_stage: MIN_PULLED_STORAGE_BINDINGS,
            ..wgpu::Limits::default()
        };
        assert_eq!(
            tier_for(true, &limits, None),
            RenderTier::Accelerated,
            "exactly the required binding count must still select accelerated"
        );
        let limits = wgpu::Limits {
            max_storage_buffer_binding_size: MIN_PULLED_BINDING_BYTES - 1,
            ..wgpu::Limits::default()
        };
        assert_eq!(tier_for(true, &limits, None), RenderTier::Universal);
    }

    #[test]
    fn finalize_tier_downgrades_when_pulled_resources_are_missing() {
        // A device that selected accelerated but could not build its pulled
        // pipelines (a missing flags layout) must downgrade rather than stay
        // labelled Accelerated with a missing pulled path.
        assert_eq!(
            finalize_tier(RenderTier::Accelerated, true),
            RenderTier::Accelerated
        );
        assert_eq!(
            finalize_tier(RenderTier::Accelerated, false),
            RenderTier::Universal
        );
        assert_eq!(
            finalize_tier(RenderTier::Universal, true),
            RenderTier::Universal
        );
        assert_eq!(
            finalize_tier(RenderTier::Universal, false),
            RenderTier::Universal
        );
    }

    fn instance(p0: [f32; 3], p1: [f32; 3], key: u32, flags: u32) -> LineInstance {
        LineInstance {
            p0_width: [p0[0], p0[1], p0[2], 0.0],
            p1_pad: [p1[0], p1[1], p1[2], 0.0],
            rgba: [0.25, 0.5, 0.75, 1.0],
            id_flags: [key, flags],
        }
    }

    #[test]
    fn pulled_lines_concatenate_streams_in_order_and_stamp_the_chunk() {
        let s0 = vec![
            instance([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], 1, 5),
            instance([0.0, 1.0, 0.0], [1.0, 1.0, 0.0], 2, 5),
        ];
        let s1 = vec![instance([2.0, 0.0, 0.0], [3.0, 0.0, 0.0], 3, 1)];
        let streams: Vec<&[LineInstance]> = vec![&s0, &s1];
        let pulled = build_pulled_lines(&streams, 0);
        assert_eq!(pulled.len(), 3);
        // Stream order preserved, each record stamped with its stream index.
        assert_eq!(pulled[0].meta, [1, 5, 0, 0]);
        assert_eq!(pulled[1].meta, [2, 5, 0, 0]);
        assert_eq!(pulled[2].meta, [3, 1, 1, 0]);
        // Geometry fields are carried verbatim.
        assert_eq!(pulled[2].p0_width, [2.0, 0.0, 0.0, 0.0]);
        assert_eq!(pulled[2].p1_pad, [3.0, 0.0, 0.0, 0.0]);
        assert_eq!(pulled[2].rgba, [0.25, 0.5, 0.75, 1.0]);
    }

    fn mesh_vertex(position: [f32; 3], key: u32) -> MeshVertex {
        MeshVertex {
            position,
            normal: [0.0, 0.0, 1.0],
            color: [0.5, 0.5, 0.5],
            entity_flags: [key, 0],
        }
    }

    #[test]
    fn pulled_mesh_vertices_are_stamped_with_their_chunk() {
        let c0 = vec![
            mesh_vertex([0.0, 0.0, 0.0], 1),
            mesh_vertex([1.0, 0.0, 0.0], 1),
        ];
        let c1 = vec![mesh_vertex([2.0, 0.0, 0.0], 2)];
        let chunks: Vec<&[MeshVertex]> = vec![&c0, &c1];
        let pulled = build_pulled_mesh_vertices(&chunks, 0);
        assert_eq!(pulled.len(), 3);
        assert_eq!(pulled[0].meta, [1, 0, 0, 0]);
        assert_eq!(pulled[1].meta, [1, 0, 0, 0]);
        assert_eq!(pulled[2].meta, [2, 0, 1, 0]);
        assert_eq!(pulled[2].position, [2.0, 0.0, 0.0, 0.0]);
        assert_eq!(pulled[2].color, [0.5, 0.5, 0.5, 0.0]);
    }

    #[test]
    fn pulled_mesh_indices_rebase_and_split_mirrored_chunks_last() {
        // Chunk 0: 4 vertices, non-mirrored. Chunk 1: 3 vertices, mirrored.
        let i0: Vec<u32> = vec![0, 2, 1, 0, 3, 2];
        let i1: Vec<u32> = vec![0, 1, 2];
        let chunks: Vec<(&[u32], u32, bool)> =
            vec![(i0.as_slice(), 4, false), (i1.as_slice(), 3, true)];
        let (indices, mirror_split) = build_pulled_mesh_indices_with_bases(&chunks);
        // Non-mirrored first (unchanged, base 0), then mirrored rebased by +4.
        assert_eq!(indices, vec![0, 2, 1, 0, 3, 2, 4, 5, 6]);
        assert_eq!(mirror_split, 6);
    }

    #[test]
    fn pulled_mesh_indices_with_no_mirrored_chunks_have_a_full_split() {
        let i0: Vec<u32> = vec![0, 1, 2];
        let chunks: Vec<(&[u32], u32, bool)> = vec![(i0.as_slice(), 3, false)];
        let (indices, mirror_split) = build_pulled_mesh_indices_with_bases(&chunks);
        assert_eq!(indices, vec![0, 1, 2]);
        assert_eq!(mirror_split, 3);
    }

    #[test]
    fn pulled_camera_columns_are_origin_independent_and_match_the_batch_matrix() {
        let projection = FrameProjection::Ortho {
            sx: 0.03125,
            sy: 0.03125,
            center: [1.0e6, 2.0e6, 0.0],
            z_plane: 0.0,
            world_per_px: 0.0,
        };
        let columns = pulled_camera_columns(projection);
        // The first three columns of a per-chunk matrix must not depend on the
        // chunk origin: one camera uniform serves every chunk.
        let m = projection.batch_matrix_f64([1.0e6, 2.0e6, 0.0]);
        assert_eq!(columns[0], [m[0], m[1], m[2], m[3]]);
        assert_eq!(columns[1], [m[4], m[5], m[6], m[7]]);
        assert_eq!(columns[2], [m[8], m[9], m[10], m[11]]);
        // And the chunk column is exactly the per-chunk matrix's last column, so
        // `mat4x4(c0, c1, c2, t)` reproduces `batch_matrix_f64(origin)`.
        let origin = [1.0e6 + 123.5, 2.0e6 - 77.25, 0.0];
        let t = chunk_transform_column(projection, origin);
        let mo = projection.batch_matrix_f64(origin);
        assert_eq!(t, [mo[12], mo[13], mo[14], mo[15]]);
    }

    #[test]
    fn chunk_transform_column_keeps_the_f64_camera_reduction() {
        // A large drawing coordinate: the translation column must be the f64
        // reduction `sx * (origin - center)`, not a large-magnitude f32 origin.
        let projection = FrameProjection::Ortho {
            sx: 0.001,
            sy: 0.001,
            center: [1.0e7, 0.0, 0.0],
            z_plane: 0.0,
            world_per_px: 0.0,
        };
        let origin = [1.0e7 + 1000.0, 0.0, 0.0];
        let t = chunk_transform_column(projection, origin);
        assert!((t[0] - 1.0).abs() < 1e-4, "expected ~1.0, got {t:?}");
    }

    #[test]
    fn pulled_camera_bytes_are_three_columns_little_endian() {
        let columns = [
            [1.0f32, 2.0, 3.0, 4.0],
            [5.0, 6.0, 7.0, 8.0],
            [9.0, 10.0, 11.0, 12.0],
        ];
        let bytes = pulled_camera_bytes(&columns);
        assert_eq!(bytes.len(), PULLED_CAMERA_BYTES as usize);
        for (i, value) in [
            1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0,
        ]
        .iter()
        .enumerate()
        {
            let at = i * 4;
            assert_eq!(
                &bytes[at..at + 4],
                &value.to_le_bytes(),
                "column word {i} must be little-endian"
            );
        }
    }
}
