use super::*;

/// Bytes of the per-batch uniform struct: `mat4x4<f32>` (64) + `vec4<f32>` tint
/// (16) + `vec4<u32> key_flags` (16) + `vec4<f32>` light (16) in
/// `shaders/{line,mesh,image,line_instanced,mesh_merged}.wgsl`. `key_flags.x` is
/// the batch's renderer pick key and `.y` its [`INSTANCE_FLAG_*`] bitfield;
/// `light.rgb` is the frame's unit vector *toward* the light in world space
/// (`.a` reserved 0). The uniform is 16-byte aligned, so the struct is 112 bytes.
pub(crate) const UNIFORM_STRUCT_BYTES: u32 = 112;

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
    match batch.topology {
        // Line geometry (`Lines` and `MeshEdges`) is uploaded pre-expanded into
        // anti-aliased triangle-list quads (see `expand_line_quads`); a batch with
        // no quads (fewer than two vertices) draws nothing.
        RenderTopology::Lines | RenderTopology::MeshEdges => {
            let Some(quads) = batch.line_quads.as_ref() else {
                return;
            };
            state.set_pipeline(pass, line_pipeline);
            state.set_bind_group0(pass, bind_group, uniform_offset);
            state.set_vertex_buffer(pass, 0, quads);
            pass.draw(0..batch.line_quad_count, 0..1);
        }
        RenderTopology::Mesh => {
            // A mesh converted to a merged chunk owns no per-batch vertex buffer;
            // it is never routed through `draw_batch`. A batch whose edge overlay
            // was instanced keeps its triangle buffer but owns no overlay quads.
            let Some(vertices) = batch.vertices.as_ref() else {
                return;
            };
            let draw_plan = mesh_draw_plan(batch.indices.is_some(), batch.line_quads.is_some());
            if draw_plan.triangles {
                state.set_pipeline(pass, mesh_pipeline);
                state.set_bind_group0(pass, bind_group, uniform_offset);
                state.set_vertex_buffer(pass, 0, vertices);
                if let Some(normals) = &batch.normals {
                    state.set_vertex_buffer(pass, 1, normals);
                }
                if let Some(colors) = &batch.colors {
                    state.set_vertex_buffer(pass, 2, colors);
                }
                let indices = batch
                    .indices
                    .as_ref()
                    .expect("mesh draw plan requires triangle indices");
                state.set_index_buffer(pass, indices);
                pass.draw_indexed(0..batch.index_count, 0, 0..1);
            }
            // Optional anti-aliased wireframe overlay (pre-expanded quads over
            // the shared mesh vertices), drawn after the triangles exactly as the
            // legacy line-list overlay was.
            if draw_plan.edge_overlay {
                let quads = batch
                    .line_quads
                    .as_ref()
                    .expect("draw plan requires overlay quads");
                state.set_pipeline(pass, line_pipeline);
                state.set_vertex_buffer(pass, 0, quads);
                pass.draw(0..batch.line_quad_count, 0..1);
            }
        }
        // A conic batch owns no per-batch buffers; it is drawn through the SDF
        // instance-run path, never `draw_batch`.
        RenderTopology::Conics => {}
    }
}

/// Submit one coalesced instanced line run: `6` unit-quad vertices expanded once
/// per instance in `start..start + count` of `stream`.
///
/// `stream` is the instance buffer (slot 1); `quad` is the shared unit quad
/// `[0,1]^2` (slot 0), the same buffer the SDF conic path binds. Instances are
/// contiguous and in their original upload order, so the run reproduces exactly
/// the draw order the same batches would have had as individual draws.
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
    pass.draw(0..6, start..start + count);
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

pub(crate) fn pack_positions(vectors: &[[f32; 3]]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vectors.len() * 12);
    for v in vectors {
        bytes.extend_from_slice(&v[0].to_le_bytes());
        bytes.extend_from_slice(&v[1].to_le_bytes());
        bytes.extend_from_slice(&v[2].to_le_bytes());
    }
    bytes
}

/// Pack one per-batch uniform: `mat4x4<f32>` transform, `vec4<f32>` tint,
/// `vec4<u32> key_flags` (x = renderer pick key, y = [`INSTANCE_FLAG_*`] bits)
/// and `vec4<f32>` light (`rgb` = unit vector toward the light in world space,
/// `a` reserved 0).
///
/// `light` is a per-frame constant, but it travels in every slot because all
/// shaders share one uniform layout (see [`UNIFORM_STRUCT_BYTES`]).
///
/// Word-oriented so the byte layout is exactly what the shaders declare and does
/// not depend on the host's float representation or `bytemuck` alignment rules.
pub(crate) fn uniform_bytes(
    matrix: &[f32; 16],
    tint: [f32; 4],
    key_flags: [u32; 4],
    light: [f32; 4],
) -> [u8; UNIFORM_STRUCT_BYTES as usize] {
    let mut words = [0u32; 28];
    for (slot, value) in matrix.iter().enumerate() {
        words[slot] = value.to_bits();
    }
    for (slot, value) in tint.iter().enumerate() {
        words[16 + slot] = value.to_bits();
    }
    words[20..24].copy_from_slice(&key_flags);
    for (slot, value) in light.iter().enumerate() {
        words[24 + slot] = value.to_bits();
    }
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
