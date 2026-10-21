use super::*;

/// The encoder plus every counter the encode pass accumulated. Returned to
/// [`Renderer::render_with_projection`] so the submit/classification tail can
/// finish the frame.
pub(crate) struct EncodeOutcome {
    pub(crate) encoder: wgpu::CommandEncoder,
    pub(crate) encode_state: EncodeState,
    pub(crate) instanced_draws: u64,
    pub(crate) instanced_batches: usize,
    pub(crate) mesh_draws: u64,
    pub(crate) merged_mesh_batches: usize,
    pub(crate) arc_draws: u64,
    pub(crate) arc_batches: usize,
    pub(crate) converted_batches: usize,
    pub(crate) encode_ms: f64,
}

impl Renderer {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_frame(
        &self,
        device: &wgpu::Device,
        view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        clear: bool,
        clear_color: [f32; 4],
        use_pulled: bool,
        opaque: &[usize],
        transparent: &[usize],
        draw_images: &[usize],
        cull: &ChunkCull,
        stream_slots: &[u32],
        chunk_slots: &[u32],
        arc_slots: &[u32],
        mask_slots: &[u32],
        line_pipeline: &wgpu::RenderPipeline,
        line_instanced_pipeline: &wgpu::RenderPipeline,
        arc_pipeline: &wgpu::RenderPipeline,
        mesh_pipeline: &wgpu::RenderPipeline,
        mesh_pipeline_mirrored: &wgpu::RenderPipeline,
        mesh_pipeline_transparent: &wgpu::RenderPipeline,
        mesh_pipeline_mirrored_transparent: &wgpu::RenderPipeline,
        mesh_pipeline_merged: &wgpu::RenderPipeline,
        mesh_pipeline_merged_mirrored: &wgpu::RenderPipeline,
        image_pipeline: &wgpu::RenderPipeline,
        unit_quad_buffer: &wgpu::Buffer,
    ) -> EncodeOutcome {
        let mut encode_state = EncodeState::new();
        // The frame's clear colour, in the attachment's linear channel space. It
        // is the caller's (default [`crate::CAD_CLEAR_COLOR`]); this pass only
        // converts the plain channels once for `wgpu`.
        let clear_value = wgpu::Color {
            r: f64::from(clear_color[0]),
            g: f64::from(clear_color[1]),
            b: f64::from(clear_color[2]),
            a: f64::from(clear_color[3]),
        };
        // Coalesced-instanced bookkeeping (kept outside the render-pass block).
        let mut instanced_draws = 0u64;
        let mut instanced_batches = 0usize;
        let mut mesh_draws = 0u64;
        let mut merged_mesh_batches = 0usize;
        // SDF conic draws this frame (coalesced runs) and member batches.
        let mut arc_draws = 0u64;
        let mut arc_batches = 0usize;
        // Opaque batches that issued no legacy per-batch draw because they were
        // converted to the line path and/or the merged mesh path. A mesh batch
        // with an instanced overlay counts once here.
        let mut converted_batches = 0usize;
        let encode_started = Instant::now();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("cad-encoder"),
        });
        // Measurement-only (ADR 0013 class 1): open the frame bracket on the
        // existing encoder (no extra submission). `gpu_time` is `None` on the
        // product configuration, so this is a no-op there.
        if let Some(gpu_time) = self.gpu_time.as_ref() {
            encoder.write_timestamp(&gpu_time.query_set, 0);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cad-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if clear {
                            wgpu::LoadOp::Clear(clear_value)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: if clear {
                            wgpu::LoadOp::Clear(1.0)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            // Group 1 is the selection-flags table for every pipeline in this
            // crate (the image pipeline's sampled texture moved to group 2). It is
            // frame-constant, so it is bound once here, outside the per-draw
            // state dedup.
            if let Some(flags_bind_group) = self.flags_bind_group.as_ref() {
                pass.set_bind_group(1, flags_bind_group, &[]);
            }

            // Pass 1 — opaque, in ascending draw order. Depth writes are on.
            //
            // Two conversion paths collapse draws here:
            //
            // * instanced line runs — a maximal sequence of adjacent (in draw
            //   order) batches whose instance slices are contiguous in the same
            //   stream; instances are stored in original batch order, so merging
            //   adjacent slices reproduces the exact draw sequence;
            // * merged mesh runs — a maximal sequence of adjacent batches whose
            //   triangle ranges are contiguous in the same chunk; the chunk's
            //   vertices are concatenated in upload order and only *contiguous*
            //   ranges coalesce, so equal-depth ties are broken by in-order
            //   rasterization exactly as the sequential draws were.
            //
            // Frame-discipline invariant: opaque meshes and opaque lines both
            // write depth (`Less` for meshes, `LessEqual` for lines) and blend by
            // replace, so their *cross-class* draw order is not observable — a
            // line wins an equal-depth tie and the nearer fragment wins otherwise.
            // This is what lets a mesh merge across an intervening line batch.
            // Fenced by: `merged_meshes_match_legacy_pixels` (non-overlapping
            // grid), `merged_overlapping_ties_match_legacy_pixels` (mesh-mesh,
            // line-mesh and line-line fragments sharing pixels at equal depth,
            // plus a reversed-order non-vacuous control),
            // `merged_meshes_match_legacy_in_perspective` (Camera3d), and the
            // real-fixture `realscene_render_frame_perf` leg in `cad-cli-tools`
            // (a merged/instanced frame of an opened DWG on the real adapter —
            // non-blank and adapter-labelled, *not* a byte-identity comparison).
            //
            // A mesh run's start flushes the pending line run, so a mesh keeps its
            // interleaved position relative to the lines around it (matching the
            // legacy per-batch draw count in a mixed scene). A line batch does
            // **not** flush a *pending mesh* run while its pending line run can
            // still be extended; when a line run ends (a non-contiguous run
            // follows) the pending mesh run is flushed first, so
            // `mesh A + overlay A`, line, `mesh B` keeps the legacy
            // `A.tri, A.overlay, line, B.tri` order instead of drawing `A.tri`
            // after the line (fenced by
            // `merged_mesh_overlay_line_mesh_keeps_legacy_order`).
            //
            // On the accelerated tier the converted classes collapse to one draw
            // each (see `use_pulled` above); the legacy per-batch branch below
            // still runs for the residual unconverted opaque batches.
            if use_pulled {
                // `use_pulled` requires `!cull.any_culled()`, so every batch keeps
                // its runs and there is nothing to skip; only the reporting
                // counters remain, and they read the cache-resident class byte
                // instead of the ~200-byte `GpuBatch`.
                for &i in opaque {
                    let class = self.batch_facts.class[i];
                    if class & plan::CLASS_INSTANCED != 0 {
                        instanced_batches += 1;
                    }
                    if class & plan::CLASS_MESH != 0 {
                        merged_mesh_batches += 1;
                    }
                    if class & plan::CLASS_ARC != 0 {
                        arc_batches += 1;
                    }
                    if class != 0 {
                        converted_batches += 1;
                    }
                }
                // The merged-mesh class is drawn **before** the line class: with
                // anti-aliased line strokes an overlapping line's sub-unit-coverage
                // fringe composites over whatever is behind it, and the universal
                // per-run path draws the meshes (plan order) before the lines, so
                // drawing the mesh class first keeps the accelerated frame
                // byte-identical to the universal one for a line crossing a mesh at
                // equal depth (fenced by
                // `accelerated_overlapping_ties_match_universal_pixels`). The tie
                // itself is order-independent (the line's `LessEqual` beats the
                // meshes' `Less`); only the fringe blend background depends on it.
                if let (Some(pulled), Some(camera_bg)) = (
                    self.pulled_mesh.as_ref(),
                    self.pulled_camera_bind_group.as_ref(),
                ) {
                    for segment in &pulled.segments {
                        if segment.mirror_split > 0 {
                            if let Some(pipeline) = self.mesh_pulled_pipeline.as_ref() {
                                encode_state.set_pipeline(&mut pass, pipeline);
                                encode_state.set_bind_group0_static(&mut pass, camera_bg);
                                encode_state.set_bind_group2(&mut pass, &segment.bind_group);
                                pass.draw(0..segment.mirror_split, 0..1);
                                mesh_draws += 1;
                            }
                        }
                        if segment.index_count > segment.mirror_split {
                            if let Some(pipeline) = self.mesh_pulled_mirrored_pipeline.as_ref() {
                                encode_state.set_pipeline(&mut pass, pipeline);
                                encode_state.set_bind_group0_static(&mut pass, camera_bg);
                                encode_state.set_bind_group2(&mut pass, &segment.bind_group);
                                pass.draw(segment.mirror_split..segment.index_count, 0..1);
                                mesh_draws += 1;
                            }
                        }
                    }
                }
                if let (Some(pulled), Some(pipeline), Some(camera_bg)) = (
                    self.pulled_lines.as_ref(),
                    self.line_pulled_pipeline.as_ref(),
                    self.pulled_camera_bind_group.as_ref(),
                ) {
                    for segment in &pulled.segments {
                        encode_state.set_pipeline(&mut pass, pipeline);
                        encode_state.set_bind_group0_static(&mut pass, camera_bg);
                        encode_state.set_bind_group2(&mut pass, &segment.bind_group);
                        // Six unit-quad vertices expanded once per pulled instance
                        // (the corners are selected from a constant table); the
                        // instance index addresses this segment's storage buffer.
                        pass.draw(0..6, 0..segment.instance_count);
                        instanced_draws += 1;
                    }
                }
                // The SDF conic class is residual on the accelerated tier: it has
                // no pulled variant this round, so it draws per-run (universal
                // style) after the pulled classes. This is the documented
                // "pulled classes draw before residual" tie exception (ADR-0008).
                let mut pending_arc: Option<(usize, u32, u32)> = None;
                for &i in opaque {
                    let batch = &self.batches[i];
                    if !batch.instanced.is_empty() || !batch.mesh_runs.is_empty() {
                        continue;
                    }
                    if !batch.arc_runs.is_empty() {
                        for run in &batch.arc_runs {
                            let start = self.arc_streams[run.stream as usize].base + run.offset;
                            let end = start + run.count;
                            if let Some((stream, _, pending_end)) = pending_arc {
                                if stream == run.stream as usize && pending_end == start {
                                    let start = pending_arc.map_or(0, |pending| pending.1);
                                    pending_arc = Some((stream, start, end));
                                    continue;
                                }
                            }
                            if flush_pending_arc(
                                &mut pass,
                                &mut encode_state,
                                &mut pending_arc,
                                &self.uniforms,
                                arc_slots,
                                &self.arc_streams,
                                unit_quad_buffer,
                                arc_pipeline,
                            ) {
                                arc_draws += 1;
                            }
                            pending_arc = Some((run.stream as usize, start, end));
                        }
                        continue;
                    }
                    // A pending SDF conic run precedes this legacy batch in plan
                    // order: flush it before the legacy `draw_batch` so the
                    // residual path never reorders the arc behind the batch (and
                    // so a legacy batch between two arc batches cannot let their
                    // runs coalesce across it).
                    if flush_pending_arc(
                        &mut pass,
                        &mut encode_state,
                        &mut pending_arc,
                        &self.uniforms,
                        arc_slots,
                        &self.arc_streams,
                        unit_quad_buffer,
                        arc_pipeline,
                    ) {
                        arc_draws += 1;
                    }
                    let (bind_group, offset) = self.uniforms.binding(self.frame_slots[i] as usize);
                    draw_batch(
                        &mut pass,
                        &mut encode_state,
                        &self.batches[i],
                        bind_group,
                        offset,
                        line_pipeline,
                        if self.batches[i].mirrored {
                            mesh_pipeline_mirrored
                        } else {
                            mesh_pipeline
                        },
                    );
                }
                if flush_pending_arc(
                    &mut pass,
                    &mut encode_state,
                    &mut pending_arc,
                    &self.uniforms,
                    arc_slots,
                    &self.arc_streams,
                    unit_quad_buffer,
                    arc_pipeline,
                ) {
                    arc_draws += 1;
                }
            } else {
                let mut pending: Option<(usize, u32, u32)> = None;
                let mut pending_mesh: Option<(usize, u32, u32)> = None;
                let mut pending_arc: Option<(usize, u32, u32)> = None;
                for &i in opaque {
                    // Skip a batch that provably emits nothing this frame: it owns
                    // runs, none of which survives the cull, so its per-class branch
                    // below is a pure no-op (no draw, no pending-run flush). The
                    // reporting counters still fold it in — they count accepted
                    // converted batches, not emitted draws. `class` is one
                    // cache-resident byte, so the skipped entries cost almost
                    // nothing (the old scan read the ~200-byte `GpuBatch`).
                    let class = self.batch_facts.class[i];
                    if class != 0 {
                        converted_batches += 1;
                        if class & plan::CLASS_MESH != 0 {
                            merged_mesh_batches += 1;
                        }
                        if class & plan::CLASS_INSTANCED != 0 {
                            instanced_batches += 1;
                        }
                        if class & plan::CLASS_ARC != 0 {
                            arc_batches += 1;
                        }
                        if !self.encode_emit_mask.participates(i) {
                            continue;
                        }
                    }
                    let batch = &self.batches[i];
                    let has_mesh = !batch.mesh_runs.is_empty();
                    let has_line = !batch.instanced.is_empty();
                    let has_arc = !batch.arc_runs.is_empty();
                    if !has_mesh && !has_line && !has_arc {
                        // Legacy per-batch draw: flush every pending run.
                        if flush_pending_mesh(
                            &mut pass,
                            &mut encode_state,
                            &mut pending_mesh,
                            &self.uniforms,
                            chunk_slots,
                            &self.mesh_chunks,
                            mesh_pipeline_merged,
                            mesh_pipeline_merged_mirrored,
                        ) {
                            mesh_draws += 1;
                        }
                        if flush_pending_arc(
                            &mut pass,
                            &mut encode_state,
                            &mut pending_arc,
                            &self.uniforms,
                            arc_slots,
                            &self.arc_streams,
                            unit_quad_buffer,
                            arc_pipeline,
                        ) {
                            arc_draws += 1;
                        }
                        if flush_pending_run(
                            &mut pass,
                            &mut encode_state,
                            &mut pending,
                            &self.uniforms,
                            stream_slots,
                            &self.instance_streams,
                            unit_quad_buffer,
                            line_instanced_pipeline,
                        ) {
                            instanced_draws += 1;
                        }
                        let (bind_group, offset) =
                            self.uniforms.binding(self.frame_slots[i] as usize);
                        draw_batch(
                            &mut pass,
                            &mut encode_state,
                            &self.batches[i],
                            bind_group,
                            offset,
                            line_pipeline,
                            if self.batches[i].mirrored {
                                mesh_pipeline_mirrored
                            } else {
                                mesh_pipeline
                            },
                        );
                        continue;
                    }
                    if has_mesh {
                        // A batch over the vertex cap contributes several runs (one
                        // per piece); a normal batch one.
                        for run in &batch.mesh_runs {
                            // Culled chunk: skip this run entirely (never drawn,
                            // never charged). The next visible run's offset is not
                            // contiguous with the pending run across the gap, so a
                            // merge can never span a culled chunk.
                            if !cull.mesh_visible(run.chunk as usize) {
                                continue;
                            }
                            let contiguous = matches!(
                                pending_mesh,
                                Some((chunk, _, end))
                                    if chunk == run.chunk as usize && end == run.index_offset
                            );
                            if contiguous {
                                let (chunk, start, _) = pending_mesh.expect("contiguous mesh run");
                                pending_mesh =
                                    Some((chunk, start, run.index_offset + run.index_count));
                                continue;
                            }
                            if flush_pending_mesh(
                                &mut pass,
                                &mut encode_state,
                                &mut pending_mesh,
                                &self.uniforms,
                                chunk_slots,
                                &self.mesh_chunks,
                                mesh_pipeline_merged,
                                mesh_pipeline_merged_mirrored,
                            ) {
                                mesh_draws += 1;
                            }
                            // A new mesh run starts: flush the pending line run so the
                            // mesh keeps its interleaved position among the lines.
                            if flush_pending_arc(
                                &mut pass,
                                &mut encode_state,
                                &mut pending_arc,
                                &self.uniforms,
                                arc_slots,
                                &self.arc_streams,
                                unit_quad_buffer,
                                arc_pipeline,
                            ) {
                                arc_draws += 1;
                            }
                            if flush_pending_run(
                                &mut pass,
                                &mut encode_state,
                                &mut pending,
                                &self.uniforms,
                                stream_slots,
                                &self.instance_streams,
                                unit_quad_buffer,
                                line_instanced_pipeline,
                            ) {
                                instanced_draws += 1;
                            }
                            pending_mesh = Some((
                                run.chunk as usize,
                                run.index_offset,
                                run.index_offset + run.index_count,
                            ));
                        }
                    }
                    if has_line {
                        // Instanced batch: draw each of its runs in order. A batch
                        // over the stream cap contributes several runs; a normal
                        // batch one.
                        for run in &batch.instanced {
                            // Culled chunk: skip this run entirely. The next
                            // visible run's offset is not contiguous with the
                            // pending run across the gap, so no merge spans a
                            // culled chunk.
                            if !cull.stream_visible(run.stream as usize) {
                                continue;
                            }
                            // Global instance index within the stream's buffer
                            // (`base` is 0 on the universal tier, where each
                            // stream owns its buffer).
                            let base = self.instance_streams[run.stream as usize].base;
                            let start = base + run.offset;
                            let end = start + run.count;
                            if let Some((stream, _, pending_end)) = pending {
                                if stream == run.stream as usize && pending_end == start {
                                    let start = pending.map_or(0, |pending| pending.1);
                                    pending = Some((stream, start, end));
                                    continue;
                                }
                            }
                            // This line run cannot extend the pending run, so the
                            // line-coalescing window ends here. If a line run is
                            // actually pending (a draw is about to be issued), flush
                            // the pending merged-mesh run *first* so a mesh that
                            // would otherwise merge across this line batch keeps its
                            // legacy position relative to it: legacy order for
                            // `mesh A + overlay A`, line, `mesh B` is
                            // `A.tri, A.overlay, line, B.tri`. Without this flush the
                            // merged `A+B.tri` run would draw after the line. Exact
                            // depth ties still resolve line-wins, but non-exact
                            // coplanar fragments could flip on other vendors.
                            // A pending SDF conic run precedes this line in plan
                            // order: flush it before starting the line run.
                            if flush_pending_arc(
                                &mut pass,
                                &mut encode_state,
                                &mut pending_arc,
                                &self.uniforms,
                                arc_slots,
                                &self.arc_streams,
                                unit_quad_buffer,
                                arc_pipeline,
                            ) {
                                arc_draws += 1;
                            }
                            if pending.is_some()
                                && flush_pending_mesh(
                                    &mut pass,
                                    &mut encode_state,
                                    &mut pending_mesh,
                                    &self.uniforms,
                                    chunk_slots,
                                    &self.mesh_chunks,
                                    mesh_pipeline_merged,
                                    mesh_pipeline_merged_mirrored,
                                )
                            {
                                mesh_draws += 1;
                            }
                            if flush_pending_run(
                                &mut pass,
                                &mut encode_state,
                                &mut pending,
                                &self.uniforms,
                                stream_slots,
                                &self.instance_streams,
                                unit_quad_buffer,
                                line_instanced_pipeline,
                            ) {
                                instanced_draws += 1;
                            }
                            pending = Some((run.stream as usize, start, end));
                        }
                    }
                    if has_arc {
                        // SDF conic runs coalesce only with adjacent conic runs in
                        // the same stream. A non-contiguous run flushes the pending
                        // conic run, then any pending mesh/line run, so plan order is
                        // preserved (arcs and lines are both `LessEqual`+depth-write,
                        // so an equal-depth arc-vs-line tie is decided by plan order).
                        for run in &batch.arc_runs {
                            // Culled chunk: skip this conic run entirely.
                            if !cull.arc_visible(run.stream as usize) {
                                continue;
                            }
                            let start = self.arc_streams[run.stream as usize].base + run.offset;
                            let end = start + run.count;
                            if let Some((stream, _, pending_end)) = pending_arc {
                                if stream == run.stream as usize && pending_end == start {
                                    let start = pending_arc.map_or(0, |pending| pending.1);
                                    pending_arc = Some((stream, start, end));
                                    continue;
                                }
                            }
                            if flush_pending_arc(
                                &mut pass,
                                &mut encode_state,
                                &mut pending_arc,
                                &self.uniforms,
                                arc_slots,
                                &self.arc_streams,
                                unit_quad_buffer,
                                arc_pipeline,
                            ) {
                                arc_draws += 1;
                            }
                            if flush_pending_mesh(
                                &mut pass,
                                &mut encode_state,
                                &mut pending_mesh,
                                &self.uniforms,
                                chunk_slots,
                                &self.mesh_chunks,
                                mesh_pipeline_merged,
                                mesh_pipeline_merged_mirrored,
                            ) {
                                mesh_draws += 1;
                            }
                            if flush_pending_run(
                                &mut pass,
                                &mut encode_state,
                                &mut pending,
                                &self.uniforms,
                                stream_slots,
                                &self.instance_streams,
                                unit_quad_buffer,
                                line_instanced_pipeline,
                            ) {
                                instanced_draws += 1;
                            }
                            pending_arc = Some((run.stream as usize, start, end));
                        }
                    }
                }
                if flush_pending_mesh(
                    &mut pass,
                    &mut encode_state,
                    &mut pending_mesh,
                    &self.uniforms,
                    chunk_slots,
                    &self.mesh_chunks,
                    mesh_pipeline_merged,
                    mesh_pipeline_merged_mirrored,
                ) {
                    mesh_draws += 1;
                }
                if flush_pending_arc(
                    &mut pass,
                    &mut encode_state,
                    &mut pending_arc,
                    &self.uniforms,
                    arc_slots,
                    &self.arc_streams,
                    unit_quad_buffer,
                    arc_pipeline,
                ) {
                    arc_draws += 1;
                }
                if flush_pending_run(
                    &mut pass,
                    &mut encode_state,
                    &mut pending,
                    &self.uniforms,
                    stream_slots,
                    &self.instance_streams,
                    unit_quad_buffer,
                    line_instanced_pipeline,
                ) {
                    instanced_draws += 1;
                }
            }
            // Pass 2 — transparent, back-to-front. Depth writes are off so each
            // blended layer composites over the already-resolved opaque pass.
            for &i in transparent {
                let (bind_group, offset) = self.uniforms.binding(self.frame_slots[i] as usize);
                draw_batch(
                    &mut pass,
                    &mut encode_state,
                    &self.batches[i],
                    bind_group,
                    offset,
                    line_pipeline,
                    if self.batches[i].mirrored {
                        mesh_pipeline_mirrored_transparent
                    } else {
                        mesh_pipeline_transparent
                    },
                );
            }
            // Pass 3 — raster images, after opaque/transparent geometry. Full
            // draw-order interleaving with geometry batches is NOT implemented;
            // images are ordered among themselves by ascending draw_order.
            for &i in draw_images {
                let (bind_group, offset) = self.uniforms.binding(self.image_slots[i] as usize);
                draw_image(
                    &mut pass,
                    &mut encode_state,
                    &self.images[i],
                    bind_group,
                    offset,
                    image_pipeline,
                );
            }
            // Pass 4 — wipeout masks, the frame's final pass (ADR 0022 C1).
            //
            // This is why a mask needs its own pass rather than a background
            // coloured mesh: in the shared pass a filled quad drawn later at the
            // same world z is rejected by the depth test (meshes `Less` + write,
            // lines `LessEqual` + write). `depth_compare: Always` with no depth
            // write (pipeline state, see `initialize_with_device`) makes every
            // mask fragment land on top of everything already painted, and the
            // fragment colour is the renderer's clear colour.
            //
            // Masks paint after images and after both geometry passes; masks do
            // not interleave with higher/lower `draw_order` geometry (ADR 0022
            // D2 subset). Under progressive accumulation they paint on every
            // page, so the last page still ends with every mask on top.
            if !self.masks.is_empty() {
                if let Some(mask_pipeline) = self.mask_pipeline.as_ref() {
                    for (index, mask) in self.masks.iter().enumerate() {
                        let (bind_group, offset) =
                            self.uniforms.binding(mask_slots[index] as usize);
                        encode_state.set_pipeline(&mut pass, mask_pipeline);
                        masks::draw_mask(&mut pass, &mut encode_state, mask, bind_group, offset);
                    }
                }
            }
        }
        let encode_ms = encode_started.elapsed().as_secs_f64() * 1000.0;

        EncodeOutcome {
            encoder,
            encode_state,
            instanced_draws,
            instanced_batches,
            mesh_draws,
            merged_mesh_batches,
            arc_draws,
            arc_batches,
            converted_batches,
            encode_ms,
        }
    }
}
