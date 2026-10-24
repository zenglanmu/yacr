use super::*;

impl Renderer {
    /// Render one frame into the offscreen target using a 2D camera.
    ///
    /// The error is explicit: callers can tell a device loss from a rejected
    /// frame via [`RenderError::is_device_loss`].
    pub fn render(
        &mut self,
        camera: Camera2d,
        target: &RenderTarget,
    ) -> Result<FrameStats, RenderError> {
        let w = target.width.max(1) as f64;
        let h = target.height.max(1) as f64;
        let sx = 2.0 / (w * camera.world_per_px);
        // CAD and clip space are both Y-up. wgpu's viewport maps positive
        // clip Y to the top of the image; negating here mirrors the drawing
        // and disagrees with the host's screen-to-world picking transform.
        let sy = 2.0 / (h * camera.world_per_px);
        // The depth band comes from the resident geometry's world-z extent, so a
        // solid's whole extent lands inside the [0, 1] clip range instead of being
        // clipped by a one-unit slab (see `OrthoDepth`).
        let projection = FrameProjection::Ortho {
            sx,
            sy,
            center: [camera.center.x, camera.center.y, camera.center.z],
            z_plane: camera.z_plane,
            depth: self.ortho_depth(),
            world_per_px: camera.world_per_px,
        };
        // Camera-only world→clip matrix, column-major (WGSL `mat4x4<f32>`). It
        // is the per-batch transform with the batch origin removed, so image
        // vertices (already in world space) can share the same camera uniform;
        // its z row is read back from the projection so the two can never drift.
        let (z_scale, z_offset) = projection
            .ortho_camera_z()
            .expect("an Ortho projection has a depth row");
        let camera_matrix = [
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
            z_scale,
            0.0,
            (sx * -camera.center.x) as f32,
            (sy * -camera.center.y) as f32,
            z_offset,
            1.0,
        ];
        self.render_with_projection(
            projection,
            camera_matrix,
            [
                camera.center.x as f32,
                camera.center.y as f32,
                camera.center.z as f32,
            ],
            // 2D keeps its world-Z headlight, so the diffuse term stays `n.z`.
            [0.0, 0.0, 1.0, 0.0],
            target,
        )
    }

    /// World-z band covering every resident batch, mapped onto the depth range.
    ///
    /// The union is taken over the batch `z_range`s captured at upload, so the
    /// band always contains the geometry the frame can draw (a culled batch only
    /// widens it, which is conservative). An empty scene falls back to a unit
    /// band around the origin ([`OrthoDepth::default`]).
    fn ortho_depth(&self) -> OrthoDepth {
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for batch in &self.batches {
            lo = lo.min(f64::from(batch.z_range[0]));
            hi = hi.max(f64::from(batch.z_range[1]));
        }
        OrthoDepth::covering(lo, hi)
    }

    /// Render one frame into the offscreen target using a 3D camera.
    ///
    /// Returns [`RenderError::Frame`] (no GPU work submitted) when the camera
    /// projection is degenerate.
    pub fn render_3d(
        &mut self,
        camera: Camera3d,
        target: &RenderTarget,
    ) -> Result<FrameStats, RenderError> {
        let aspect = target.width.max(1) as f64 / target.height.max(1) as f64;
        let Some(vp) = camera.view_projection(aspect) else {
            return Err(RenderError::Frame(
                "camera has no usable 3D projection".into(),
            ));
        };
        // Per-batch uniform = VP * T(batch origin).
        // Camera-only VP, column-major flat, for image vertices that are already
        // in world space.
        let mut camera_matrix = [0.0f32; 16];
        for col in 0..4 {
            for row in 0..4 {
                camera_matrix[col * 4 + row] = vp[col][row];
            }
        }
        let eye = [
            camera.eye.x as f32,
            camera.eye.y as f32,
            camera.eye.z as f32,
        ];
        // Camera-relative key light for 3D (see `Camera3d::headlight`): the
        // light follows the orbit but sits a fixed offset above-left of the view
        // axis, so a solid's faces (including a symmetric isometric view's
        // three) no longer all collapse to one shade. The fallback is
        // unreachable after the projection check above; it is world-`+Z` like
        // the 2D light.
        let light = camera
            .headlight()
            .map_or([0.0, 0.0, 1.0, 0.0], |[r, g, b]| [r, g, b, 0.0]);
        self.render_with_projection(
            FrameProjection::Perspective {
                vp,
                world_per_px: camera.world_per_px_at_target(target.height),
            },
            camera_matrix,
            eye,
            light,
            target,
        )
    }

    pub(super) fn render_with_projection(
        &mut self,
        projection: FrameProjection,
        camera_matrix: [f32; 16],
        camera_position: [f32; 3],
        light: [f32; 4],
        target: &RenderTarget,
    ) -> Result<FrameStats, RenderError> {
        if self.device_lost {
            return Err(RenderError::DeviceLost(
                self.last_device_lost
                    .clone()
                    .unwrap_or_else(|| "device lost; rebuild required".into()),
            ));
        }
        // Remember the view this frame used: `pick_at` re-runs the same
        // projection for the id pass, so a pick always matches the frame the user
        // is looking at.
        self.last_projection = Some(projection);
        self.last_camera_matrix = camera_matrix;
        self.last_camera_position = Some(camera_position);
        // The id pass re-uses the frame's light (the pick shaders ignore it).
        self.last_light = light;
        // CPU-path timer: plan + uniforms + encode + submit. Stopped right after
        // `queue.submit` so the blocking device poll below is never counted as
        // CPU work.
        let cpu_started = Instant::now();
        self.ensure_target(target)?;
        // The selection-flags table must cover the live key space before the
        // shaders address it by key. It only ever grows, so this is a no-op on a
        // steady scene.
        {
            let device = self.device.as_ref().expect("device").clone();
            let queue = self.queue.as_ref().expect("queue").clone();
            let needed = self.pick_table.max_key() + 1;
            self.ensure_flags(&device, &queue, needed);
        }
        // Inverted WIPEOUT masks fill the complement of their boundary, which
        // needs a ring covering this frame's visible world rect (ADR 0022 C2).
        // The ring is built from the frame's own projection and cached until the
        // view leaves it, so a steady camera or a small pan never re-tessellates.
        self.ensure_inverted_mask_fill(masks::mask_view_coverage(
            projection,
            self.target_size,
            camera_position,
        ));
        // Phase timers below break `cpu_ms` down into plan / uniform / encode /
        // submit; `wait_ms` separately measures the blocking poll (excluded from
        // `cpu_ms`).
        let plan_started = Instant::now();
        // Times the frame-key mismatch reset the progressive cursor this frame.
        let mut progressive_restarts = 0u64;
        // The draw-order partition is camera-independent, so it survives camera
        // moves. Rebuild it only when the batch set changed (the mutation paths
        // that clear or replace `batches` set `draw_plan = None`).
        if self.draw_plan.is_none() {
            let entries = self
                .batches
                .iter()
                .map(|batch| BatchOrderEntry {
                    draw_order: batch.draw_order,
                    alpha: batch.alpha,
                    centroid: batch.centroid,
                })
                .collect();
            self.draw_plan = Some(DrawOrderCache::new(entries));
            // The segmented planner's batch-set facts share this single
            // invalidation (`draw_plan = None`): no second invalidation truth.
            self.batch_facts = plan::BatchFacts::build(&self.batches);
        }
        let frame_key = FrameKey {
            camera_matrix,
            camera_position,
            target_size: self.target_size,
            texture_revision: self.texture_revision,
        };
        if self.progressive
            && self
                .progressive_frame
                .as_ref()
                .is_none_or(|frame| frame.key != frame_key)
        {
            progressive_restarts += 1;
            // Reuse the previous frame's `ordered` and page-index allocations:
            // the restart replaces them, so the old Vecs are cleared and refilled
            // instead of freshly collected per camera move.
            let (mut ordered, mut page_index) = self
                .progressive_frame
                .take()
                .map(|frame| (frame.ordered, frame.page_index))
                .unwrap_or_default();
            let cache = self.draw_plan.as_mut().expect("draw plan cache");
            cache.sort_transparent(Some(camera_position));
            let opaque_count = cache.opaque().len();
            let invisible = cache.invisible().len();
            ordered.clear();
            ordered.extend(cache.opaque().iter().chain(cache.transparent()).copied());
            // Fuse the segmented planner's per-`ordered` index into the same
            // sweep that (re)builds `ordered`, so it is derived from that one
            // truth and rebuilt exactly when the order is.
            page_index.rebuild(&self.batch_facts.upload_bytes, &ordered);
            self.progressive_frame = Some(ProgressiveFrame {
                key: frame_key,
                ordered,
                opaque_count,
                invisible,
                cursor: 0,
                page_index,
            });
        }
        // Cloned so the shared uniform arena can be mutated (grown/filled) while
        // this frame is encoded without fighting the borrow checker over `self`.
        let queue = self
            .queue
            .as_ref()
            .ok_or_else(|| RenderError::NotInitialized("renderer not initialized".into()))?
            .clone();
        let device = self.device.as_ref().unwrap();
        let Some(line_pipeline) = self.line_pipeline.as_ref() else {
            return Err(RenderError::NotInitialized(
                "renderer resources missing".into(),
            ));
        };
        let Some(line_instanced_pipeline) = self.line_instanced_pipeline.as_ref() else {
            return Err(RenderError::NotInitialized(
                "renderer resources missing".into(),
            ));
        };
        let Some(arc_pipeline) = self.arc_pipeline.as_ref() else {
            return Err(RenderError::NotInitialized(
                "renderer resources missing".into(),
            ));
        };
        let Some(unit_quad_buffer) = self.unit_quad_buffer.as_ref() else {
            return Err(RenderError::NotInitialized(
                "renderer resources missing".into(),
            ));
        };
        let Some(mesh_pipeline) = self.mesh_pipeline.as_ref() else {
            return Err(RenderError::NotInitialized(
                "renderer resources missing".into(),
            ));
        };
        let Some(mesh_pipeline_mirrored) = self.mesh_pipeline_mirrored.as_ref() else {
            return Err(RenderError::NotInitialized(
                "renderer resources missing".into(),
            ));
        };
        let Some(mesh_pipeline_transparent) = self.mesh_pipeline_transparent.as_ref() else {
            return Err(RenderError::NotInitialized(
                "renderer resources missing".into(),
            ));
        };
        let Some(mesh_pipeline_mirrored_transparent) =
            self.mesh_pipeline_mirrored_transparent.as_ref()
        else {
            return Err(RenderError::NotInitialized(
                "renderer resources missing".into(),
            ));
        };
        let Some(mesh_pipeline_merged) = self.mesh_pipeline_merged.as_ref() else {
            return Err(RenderError::NotInitialized(
                "renderer resources missing".into(),
            ));
        };
        let Some(mesh_pipeline_merged_mirrored) = self.mesh_pipeline_merged_mirrored.as_ref()
        else {
            return Err(RenderError::NotInitialized(
                "renderer resources missing".into(),
            ));
        };
        let Some(image_pipeline) = self.image_pipeline.as_ref() else {
            return Err(RenderError::NotInitialized(
                "renderer resources missing".into(),
            ));
        };
        let Some(view) = self.target_view.as_ref() else {
            return Err(RenderError::NotInitialized(
                "renderer target missing".into(),
            ));
        };
        let Some(depth_view) = self.depth_view.as_ref() else {
            return Err(RenderError::NotInitialized(
                "renderer depth target missing".into(),
            ));
        };
        let Some(layout) = self.layout.as_ref() else {
            return Err(RenderError::NotInitialized(
                "renderer uniform layout missing".into(),
            ));
        };

        // Scratch index buffers are taken out of `self` and returned before this
        // function ends, so the opaque/transparent split does not allocate every
        // frame.
        let mut opaque = std::mem::take(&mut self.scratch_opaque);
        let mut transparent = std::mem::take(&mut self.scratch_transparent);
        opaque.clear();
        transparent.clear();

        // The active page budget this frame's plan uses. Captured before planning
        // so the reported `FrameStats::budget_*` fields describe exactly what the
        // plan used, and so the controller can sample this frame afterwards.
        let active_budget = self.active_frame_budget();
        let pacing_mode = self.pacing_mode();
        // Chunk-level view culling (backlog §P1): build the frame's mask once,
        // from the per-chunk world AABBs retained at upload (no per-frame AABB
        // computation). The mask is threaded into the planners (so culled runs
        // are charged zero) and the encode sites (so culled runs are skipped). A
        // test disables culling to render the unculled reference frame.
        let cull = {
            let wpp_arc = projection.world_per_px();
            // The line class may use the overridden ramp width (test hook); the
            // SDF conic class always uses the projection's real value.
            let wpp_line = self.line_ramp_override.map(f64::from).unwrap_or(wpp_arc);
            let built = cull::build(
                projection,
                wpp_line,
                wpp_arc,
                self.instance_streams.iter().map(|stream| stream.aabb),
                self.mesh_chunks.iter().map(|chunk| chunk.aabb),
                self.arc_streams.iter().map(|stream| stream.aabb),
            );
            if self.cull_disabled {
                // Test hook: report the resident chunk set but cull nothing.
                built.force_all_visible()
            } else {
                built
            }
        };
        // The frame's encode-participation set, from the batch facts and the cull
        // just built. Derived from the same facts/restart invalidation, not a
        // second truth.
        self.encode_emit_mask.rebuild(&self.batch_facts, &cull);
        let paced_scale = match pacing_mode {
            pacing::PacingMode::Active => self.pacing.scale,
            pacing::PacingMode::DisabledNoCompletionSignal | pacing::PacingMode::AtCeiling => 1.0,
        };
        let pacing_cost_ema_ms = self.pacing.cost_ema_ms;

        // Images are drawn only on the page that clears the target (the first
        // page of a progressive frame, or every frame when not progressive).
        // Continuation pages load the existing colour, so re-drawing an
        // `alpha < 1` image every page would accumulate its opacity and charge it
        // repeatedly; a continuation page must not composite images again.
        //
        // The full image charge is reserved **before** geometry, so geometry
        // cannot saturate the cleared page and leave the images undeclared on the
        // continuation pages (which never composite images again). It is charged
        // against the active page budget, with a fallback to the un-paced ceiling
        // when a paced page cannot hold every image: an image draws exactly once,
        // so a page split must never drop it. Geometry then fills what is left of
        // the active page budget. Visual order is unchanged (images still draw
        // last, in ascending `draw_order` among themselves).
        let clear = self
            .progressive_frame
            .as_ref()
            .is_none_or(|frame| frame.cursor == 0);
        let mut image_usage = cad_scene::FrameUsage::default();
        let mut draw_images: Vec<usize> = Vec::new();
        let mut image_over_budget = None;
        if clear {
            // Reserve the full image charge before geometry, against the active
            // page budget. When the paced page cannot hold every image, fall
            // back to the un-paced ceiling instead of dropping images: they draw
            // exactly once, so a page split must never lose one. Geometry then
            // fills whatever the active budget has left (nothing, in that case),
            // and the page reports crossing the active budget.
            let (accepted_images, over) =
                charge_images(&mut image_usage, &active_budget, &self.images);
            draw_images = accepted_images;
            image_over_budget = over;
            if image_over_budget.is_some() && active_budget != self.frame_budget {
                let mut ceiling_usage = cad_scene::FrameUsage::default();
                let (all_images, ceiling_over) =
                    charge_images(&mut ceiling_usage, &self.frame_budget, &self.images);
                if ceiling_over.is_none() {
                    // The whole image set fits the un-paced ceiling; keep it all
                    // and let the over-budget invariant report the active-page
                    // crossing below.
                    image_usage = ceiling_usage;
                    draw_images = all_images;
                    image_over_budget = None;
                } else {
                    image_usage = ceiling_usage;
                    draw_images = all_images;
                    image_over_budget = ceiling_over;
                }
            }
            draw_images.sort_by_key(|&i| self.images[i].draw_order);
        }

        let (mut budget_plan, invisible_batches) = if let Some(frame) = &self.progressive_frame {
            let mut plan = plan::plan_ordered_page(
                &self.batches,
                &frame.ordered,
                frame.cursor,
                &active_budget,
                image_usage,
                &cull,
                &self.batch_facts,
                &frame.page_index,
            );
            if plan.accepted.is_empty() && frame.cursor < frame.ordered.len() {
                // Progress floor: a cursor batch that fits the un-paced ceiling
                // must be drawn rather than turning the frame into an error, so
                // every page of a paced frame makes progress.
                match plan::plan_cursor_batch(
                    &self.batches,
                    &frame.ordered,
                    frame.cursor,
                    &self.frame_budget,
                    &cull,
                ) {
                    Some(mut floor) => {
                        floor.over_budget = Some(plan::pacing_floor_reason(
                            &floor,
                            &active_budget,
                            frame.ordered.len() - frame.cursor - 1,
                        ));
                        plan = floor;
                    }
                    None => {
                        return Err(RenderError::Frame(
                            "one GPU batch exceeds the progressive frame budget".into(),
                        ))
                    }
                }
            }
            let split = frame
                .opaque_count
                .saturating_sub(frame.cursor)
                .min(plan.accepted.len());
            opaque.extend_from_slice(&plan.accepted[..split]);
            transparent.extend_from_slice(&plan.accepted[split..]);
            (plan, frame.invisible)
        } else {
            let plan = plan_from_gpu(&self.batches, &active_budget, image_usage, &cull);
            let cache = self.draw_plan.as_mut().expect("draw plan cache");
            cache.sort_transparent(Some(camera_position));
            if plan.accepted.len() == self.batches.len() {
                // Full acceptance keeps upload order, so the cached
                // camera-sorted partition over all batches is exact.
                opaque.extend_from_slice(cache.opaque());
                transparent.extend_from_slice(cache.transparent());
                (plan, cache.invisible().len())
            } else {
                // Over budget: `accepted` is an upload-order prefix. Fall
                // back to the fresh plan over that prefix; this path is rare
                // (the frame reported over-budget) and must stay identical to
                // the pre-cache behaviour.
                let accepted: Vec<usize> = plan.accepted.clone();
                let entries: Vec<_> = accepted
                    .iter()
                    .map(|&i| BatchOrderEntry {
                        draw_order: self.batches[i].draw_order,
                        alpha: self.batches[i].alpha,
                        centroid: self.batches[i].centroid,
                    })
                    .collect();
                let passes = draw_passes(&accepted, &entries, Some(camera_position));
                opaque.extend_from_slice(&passes.opaque);
                transparent.extend_from_slice(&passes.transparent);
                (plan, passes.invisible)
            }
        };

        // The image over-budget (images that crossed the un-paced ceiling) is
        // reported unless geometry already reported one.
        if budget_plan.over_budget.is_none() {
            budget_plan.over_budget = image_over_budget;
        }
        // Invariant: a page whose usage (including the image reservation) crossed
        // the active budget must report it. Geometry is charged against the
        // remainder, so this is only reachable when the image reservation alone
        // exceeds a paced page (`pacing-floor` already covers the geometry
        // case).
        if budget_plan.over_budget.is_none() {
            let remaining = match &self.progressive_frame {
                Some(frame) => frame
                    .ordered
                    .len()
                    .saturating_sub(frame.cursor + budget_plan.accepted.len()),
                None => 0,
            };
            budget_plan.over_budget =
                plan::usage_over_budget_reason(&budget_plan.usage, &active_budget, remaining);
        }

        // Lineweight is carried, never drawn: the report is cached at batch
        // mutation (`rebuild_lineweight_cache`), so the frame path only clones
        // the small cached Vec instead of scanning every resident batch.
        let lineweight_not_drawn: Vec<LineweightNotDrawn> = self.lineweight_not_drawn.clone();

        // Decide whether this frame uses the accelerated per-class draw
        // organization. It requires: the accelerated tier is live; the pulled
        // storage buffers were actually built (a class whose pulled buffer
        // exceeded a device limit leaves its geometry on the universal per-run
        // path, never silently dropped); and **every** converted batch is
        // accepted this frame, so the per-class draw cannot pull geometry the
        // frame budget held back. A paginated/over-budget frame therefore falls
        // back to the universal per-run organization (reported as
        // `FrameStats::accelerated == false`).
        // Pure batch-set facts, computed once at the draw-order rebuild
        // (`batch_facts`) instead of scanning every resident batch per frame.
        let has_line_class = self.batch_facts.has_line_class;
        let has_mesh_class = self.batch_facts.has_mesh_class;
        let converted_total = self.batch_facts.converted_total;
        // Everything that decides `use_pulled` except the accepted-set scan. A
        // culled frame can never use the pulled organisation (below), so the
        // O(accepted) converted-batch scan is skipped there instead of running on
        // every settled page whose accepted set is the whole scene.
        let pulled_ready = self.render_tier == RenderTier::Accelerated
            && (has_line_class || has_mesh_class)
            && (!has_line_class || self.pulled_lines.is_some())
            && (!has_mesh_class || self.pulled_mesh.is_some())
            && converted_total > 0
            // A pulled draw redraws whole streams/chunks, so it must not run
            // while culling excluded any chunk: that would draw (uncharged)
            // geometry the cull removed. Pulled-subset draws are not implemented
            // (that is backlog §N1d), so culling forces the universal per-run
            // organization instead.
            && !cull.any_culled();
        let use_pulled = pulled_ready
            && opaque
                .iter()
                .filter(|&&i| {
                    let b = &self.batches[i];
                    !b.instanced.is_empty() || !b.mesh_runs.is_empty()
                })
                .count()
                == converted_total;

        let plan_ms = plan_started.elapsed().as_secs_f64() * 1000.0;

        // Slots are assigned per frame in accepted order; the slot maps are
        // reused across frames to avoid a per-frame allocation.
        let uniform_started = Instant::now();
        let accepted_len = budget_plan.accepted.len();
        if self.frame_slots.len() != self.batches.len() {
            self.frame_slots.resize(self.batches.len(), u32::MAX);
        }
        if self.image_slots.len() != self.images.len() {
            self.image_slots.resize(self.images.len(), u32::MAX);
        }
        // Upper bound for arena growth; the frame flushes only the slots it
        // actually filled (converted batches are compacted out; see below).
        // On the accelerated path the per-stream/per-chunk uniforms are replaced
        // by the pulled pipelines' shared camera uniform + storage chunk table,
        // so no stream/chunk slots are written.
        let stream_slot_count = if use_pulled {
            0
        } else {
            self.instance_streams.len()
        };
        let chunk_slot_count = if use_pulled {
            0
        } else {
            self.mesh_chunks.len()
        };
        // SDF conic streams draw on the universal per-run path on **every** tier,
        // so their uniform slots are always reserved.
        let arc_slot_count = self.arc_streams.len();
        let max_slots = accepted_len
            + draw_images.len()
            + stream_slot_count
            + chunk_slot_count
            + arc_slot_count
            + self.masks.len();
        self.uniforms
            .ensure_slots(max_slots, device, layout, &self.gpu_memory);

        // Write only this page's uniforms. Rewriting every batch for every
        // continuation would turn bounded accumulation into repeated full work.
        // Each uniform is the transform matrix followed by the constant
        // per-batch colour (normalized sRGB) and alpha (both clamped by the same
        // policy used for drawing); they are packed into the shared arena and
        // uploaded with one `write_buffer` per page below.
        //
        // Batches converted to an instanced stream or a merged mesh chunk own no
        // per-batch uniform (the whole stream/chunk shares one), so they are
        // skipped and the legacy slots are compacted: a fully converted scene
        // needs one stream slot and one chunk slot, not one slot per batch.
        // World units per pixel travels in the otherwise-unused `key_flags.z` word
        // (bitcast f32) for every uniform: the SDF conic vertex stage floors its
        // quad margin with it, and the anti-aliased line stages use it as the SDF
        // ramp width (`w = world_per_px` → 1.0 px of linear ink). The mesh/image
        // shaders ignore it.
        let world_per_px_bits = (projection.world_per_px() as f32).to_bits();
        // The line class may use an overridden ramp width (test hook); the SDF
        // conic class always uses the projection's real value.
        let line_wpp_bits = self
            .line_ramp_override
            .map(f32::to_bits)
            .unwrap_or(world_per_px_bits);
        let mut legacy_slots = 0usize;
        for &index in &budget_plan.accepted {
            // Only a batch that owns **no runs** needs a per-batch uniform slot:
            // the legacy `draw_batch` path is the only reader of `frame_slots`, and
            // it is taken only for a run-free batch. A batch with runs draws through
            // its runs (stream/chunk/arc slots) and a fully-culled one emits
            // nothing at all, so both are skipped. `class` is a cache-resident byte
            // instead of the ~200-byte `GpuBatch` the old scan read per batch.
            if self.batch_facts.class[index] != 0 {
                continue;
            }
            let batch = &self.batches[index];
            let (m, [r, g, b], alpha) = (
                projection.batch_matrix(batch.origin),
                renderer_color(batch.color),
                clamp_alpha(batch.alpha),
            );
            let uniform = uniform_bytes(
                &m,
                [r, g, b, alpha],
                [batch.pick_key, batch.instance_flags(), line_wpp_bits, 0],
                light,
            );
            self.uniforms.write_slot(legacy_slots, &uniform);
            self.frame_slots[index] = legacy_slots as u32;
            legacy_slots += 1;
        }

        // Image slots follow the geometry slots; the same page upload covers
        // them. Their transform is the camera-only matrix (image vertices are
        // already world space) and the tint is white with the image's alpha.
        for (j, &i) in draw_images.iter().enumerate() {
            let alpha = self.images[i].alpha;
            let uniform = uniform_bytes(
                &camera_matrix,
                [1.0, 1.0, 1.0, alpha],
                [self.images[i].pick_key, 0, 0, 0],
                light,
            );
            let slot = legacy_slots + j;
            self.uniforms.write_slot(slot, &uniform);
            self.image_slots[i] = slot as u32;
        }

        // Instanced line streams: one uniform slot per **visible** stream,
        // carrying the stream's fixed-point origin through the same `f64` camera
        // subtraction as a legacy batch (see `FrameProjection::batch_matrix_f64`).
        // Every run of a stream shares it, so streams — not batches — bound this
        // cost. A culled stream's runs are never drawn, so its slot is skipped and
        // its `stream_slots` entry stays `u32::MAX` (never bound by a draw).
        // Skipped entirely on the accelerated path, which writes the equivalent
        // per-chunk translation table into its storage buffer instead.
        //
        // Each surviving entry's uniform is byte-identical to the
        // pre-skip layout; only the (purely internal) slot index changes, and the
        // `flush` range shrinks to the compacted count. These classes carry their
        // colour per instance or per vertex, so their uniform tint is opaque white.
        let white = [1.0, 1.0, 1.0, 1.0];
        let mut next_slot = legacy_slots + draw_images.len();
        let mut stream_slots: Vec<u32> = vec![u32::MAX; stream_slot_count];
        for (s, slot) in stream_slots.iter_mut().enumerate() {
            if !cull.stream_visible(s) {
                continue;
            }
            *slot = next_slot as u32;
            let matrix = projection.batch_matrix_f64(self.instance_streams[s].origin);
            // The instanced shader ignores the uniform tint (colour and the pick
            // key are per-instance) but the layout keeps them; write opaque white
            // and a zero key, and the SDF ramp width in `key_flags.z`.
            let uniform = uniform_bytes(&matrix, white, [0, 0, line_wpp_bits, 0], light);
            self.uniforms.write_slot(next_slot, &uniform);
            next_slot += 1;
        }

        // Merged mesh chunks: one uniform slot per **visible** chunk, carrying the
        // chunk's fixed-point origin through the same `f64` camera subtraction. The
        // tint is opaque white because the per-batch colour is baked into each
        // vertex. A culled chunk's runs are never drawn, so its slot is skipped.
        let mut chunk_slots: Vec<u32> = vec![u32::MAX; chunk_slot_count];
        for (c, slot) in chunk_slots.iter_mut().enumerate() {
            if !cull.mesh_visible(c) {
                continue;
            }
            *slot = next_slot as u32;
            let matrix = projection.batch_matrix_f64(self.mesh_chunks[c].origin);
            let uniform = uniform_bytes(&matrix, white, [0, 0, 0, 0], light);
            self.uniforms.write_slot(next_slot, &uniform);
            next_slot += 1;
        }

        // SDF conic streams: one uniform slot per **visible** stream, carrying the
        // stream's fixed-point origin through the same `f64` camera subtraction as a
        // legacy batch. The tint is opaque white and the key 0: colour and the pick
        // key are per-instance. A culled conic stream's runs are never drawn, so its
        // slot is skipped.
        let mut arc_slots: Vec<u32> = vec![u32::MAX; arc_slot_count];
        // World units per pixel travels in the otherwise-unused `key_flags.z` word
        // (bitcast f32) so the SDF conic vertex stage can floor its quad margin
        // at ~2 px for small on-screen circles (see `shaders/arc.wgsl`).
        for (a, slot) in arc_slots.iter_mut().enumerate() {
            if !cull.arc_visible(a) {
                continue;
            }
            *slot = next_slot as u32;
            let matrix = projection.batch_matrix_f64(self.arc_streams[a].origin);
            let uniform = uniform_bytes(&matrix, white, [0, 0, world_per_px_bits, 0], light);
            self.uniforms.write_slot(next_slot, &uniform);
            next_slot += 1;
        }
        // Wipeout mask uniforms: one slot per resident mask, carrying the mask's
        // `f64` origin through the same camera-relative subtraction as a batch.
        // The tint IS this frame's clear colour, so a mask paints the configured
        // background (ADR 0022 D1) and the representation layer never names a
        // colour. `key_flags` is zero: a mask carries no pick key.
        let mut mask_slots: Vec<u32> = Vec::with_capacity(self.masks.len());
        let clear_color = target.clear_color();
        for mask in &self.masks {
            mask_slots.push(next_slot as u32);
            let matrix = projection.batch_matrix_f64(mask.origin);
            let uniform = uniform_bytes(&matrix, clear_color, [0, 0, 0, 0], light);
            self.uniforms.write_slot(next_slot, &uniform);
            next_slot += 1;
        }
        // One `Queue::write_buffer` per page (vs. one per batch), reported as
        // `FrameStats::uniform_writes`. Only the compacted slot count is flushed.
        let total_slots = next_slot;
        let uniform_writes = self.uniforms.flush(&queue, total_slots);
        let uniform_ms = uniform_started.elapsed().as_secs_f64() * 1000.0;

        // Accelerated tier per-frame camera data: the shared three-column camera
        // uniform plus one translation column per resident stream/chunk. These
        // are the ONLY per-frame writes the pulled path issues — the static
        // instance/merged storage buffers are never rewritten on a camera move
        // (the `camera_move_*` fences assert the upload counters stay flat).
        if use_pulled {
            if let Some(camera_buffer) = self.pulled_camera_buffer.as_ref() {
                let mut columns = pulled_camera_columns(projection, light);
                // The line class may use an overridden ramp width (test hook).
                if let Some(w) = self.line_ramp_override {
                    columns[3][0] = w;
                }
                queue.write_buffer(camera_buffer, 0, &pulled_camera_bytes(&columns));
            }
            if let (Some(pulled), false) =
                (self.pulled_lines.as_ref(), self.instance_streams.is_empty())
            {
                let table: Vec<[f32; 4]> = self
                    .instance_streams
                    .iter()
                    .map(|stream| chunk_transform_column(projection, stream.origin))
                    .collect();
                queue.write_buffer(&pulled.chunk_table, 0, &chunk_transform_bytes(&table));
            }
            if let (Some(pulled), false) = (self.pulled_mesh.as_ref(), self.mesh_chunks.is_empty())
            {
                let table: Vec<[f32; 4]> = self
                    .mesh_chunks
                    .iter()
                    .map(|chunk| chunk_transform_column(projection, chunk.origin))
                    .collect();
                queue.write_buffer(&pulled.chunk_table, 0, &chunk_transform_bytes(&table));
            }
        }

        let EncodeOutcome {
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
        } = self.encode_frame(
            device,
            view,
            depth_view,
            clear,
            clear_color,
            use_pulled,
            &opaque,
            &transparent,
            &draw_images,
            &cull,
            &stream_slots,
            &chunk_slots,
            &arc_slots,
            &mask_slots,
            line_pipeline,
            line_instanced_pipeline,
            arc_pipeline,
            mesh_pipeline,
            mesh_pipeline_mirrored,
            mesh_pipeline_transparent,
            mesh_pipeline_mirrored_transparent,
            mesh_pipeline_merged,
            mesh_pipeline_merged_mirrored,
            image_pipeline,
            unit_quad_buffer,
        );
        let mut encoder = encoder;
        // Capture the split sizes before the scratch Vecs are returned to `self`
        // below.
        let opaque_batches = opaque.len();
        let transparent_batches = transparent.len();
        // Converted batches own no per-batch draw; they contribute coalesced
        // instanced draws and/or merged mesh draws instead (a batch over a cap
        // contributes one draw per run). Everything else stays one draw per batch.
        let legacy_opaque = opaque_batches.saturating_sub(converted_batches);
        // Every resident mask is one draw in the frame's final pass, whatever the
        // page, so the count is included in `draw_calls`.
        let mask_draws = self.masks.len() as u64;
        let draw_calls = (legacy_opaque
            + instanced_draws as usize
            + mesh_draws as usize
            + arc_draws as usize
            + transparent_batches
            + draw_images.len()
            + mask_draws as usize) as u64;

        let submit_started = Instant::now();
        // Measurement-only: close the bracket, resolve the two queries and copy
        // the ticks into the reusable `MAP_READ` buffer — all on the **same**
        // encoder/submission (no extra submit). The copy perturbs the frame; it
        // exists only under the explicit opt-in.
        if let Some(gpu_time) = self.gpu_time.as_ref() {
            encoder.write_timestamp(&gpu_time.query_set, 1);
            encoder.resolve_query_set(&gpu_time.query_set, 0..2, &gpu_time.resolve, 0);
            encoder.copy_buffer_to_buffer(&gpu_time.resolve, 0, &gpu_time.readback, 0, 16);
        }
        let submission = queue.submit(Some(encoder.finish()));
        let submit_ms = submit_started.elapsed().as_secs_f64() * 1000.0;
        // Request the timestamp readback mapping before the blocking poll below,
        // so that poll completes it (blocking readback strategy: no second poll).
        if let Some(gpu_time) = self.gpu_time.as_ref() {
            gpu_time
                .readback
                .slice(0..16)
                .map_async(wgpu::MapMode::Read, |_| {});
        }
        // Stop the CPU timer here: everything above is the render thread's CPU
        // work (plan/cache, uniforms, encode, submit). The poll below blocks on
        // the GPU and is not CPU time.
        let cpu_ms = cpu_started.elapsed().as_secs_f64() * 1000.0;
        // Distinguish device loss from a bad frame. A bounded wait classifies
        // the submission; the host's device-lost callback is the authoritative
        // loss signal (see `note_device_lost`).
        let wait_started = Instant::now();
        let outcome = self.device_scope_outcome(device, submission);
        let wait_ms = wait_started.elapsed().as_secs_f64() * 1000.0;
        // Measurement-only readback: the blocking poll above has completed the
        // mapping requested right after submit, so this reads ticks without a
        // second blocking wait. `None` when measurement is off, unsupported, or
        // the mapping did not complete (never an estimate).
        let gpu_ms = self.read_gpu_time_ms(&queue);
        self.draw_calls = draw_calls;
        match outcome {
            ScopeOutcome::Clean => {}
            ScopeOutcome::FrameError => {
                return Err(RenderError::Frame(
                    "frame rejected by GPU validation".into(),
                ));
            }
            ScopeOutcome::DeviceLost => {
                let report = self.note_device_lost("GPU device lost while rendering");
                return Err(report);
            }
        }
        if let Some(frame) = &mut self.progressive_frame {
            frame.cursor += budget_plan.accepted.len();
        }
        // Adaptive page-budget feedback: fold this frame's measured cost in.
        // Only a frame that accepted page work is a signal, and only when pacing
        // is active (progressive + a completion signal); camera moves never reset
        // the controller, so a pan keeps feeding it. `accepted` counts only
        // batches with a **surviving** run (a culled chunk's batch is not work),
        // so a page whose accepted batches were all culled is not a cost
        // observation.
        let accepted_with_work = budget_plan
            .accepted
            .iter()
            .filter(|&&i| batch_has_visible_work(&self.batches[i], &cull))
            .count();
        if pacing_mode == pacing::PacingMode::Active && accepted_with_work > 0 {
            let sample = pacing::Sample {
                cpu_ms,
                wait_ms,
                accepted: accepted_with_work,
                restart: self.pacing_restart_pending,
            };
            let (next, _) = pacing::step(self.pacing, sample);
            self.pacing = next;
            self.pacing_restart_pending = false;
        }
        // Return the scratch index buffers so the next frame reuses them.
        self.scratch_opaque = opaque;
        self.scratch_transparent = transparent;
        Ok(FrameStats {
            cpu_ms,
            gpu_ms,
            gpu_time: self.gpu_time_state,
            draw_calls,
            legacy_opaque_draws: legacy_opaque as u64,
            uniform_writes,
            uploaded_bytes: self.uploaded_bytes,
            vertices: budget_plan.usage.vertices,
            triangles: budget_plan.usage.triangles,
            over_budget: budget_plan.over_budget,
            budget_scale: paced_scale,
            budget_target_ms: self.pacing.target_ms,
            budget_cost_ema_ms: pacing_cost_ema_ms,
            budget_vertices: active_budget.max_vertices,
            budget_triangles: active_budget.max_triangles,
            pacing: pacing_mode.as_str(),
            opaque_batches,
            transparent_batches,
            invisible_batches,
            image_batches: draw_images.len(),
            mask_draws,
            cull_chunks_total: cull.total() as u64,
            cull_chunks_kept: cull.kept(),
            lineweight_not_drawn: lineweight_not_drawn.clone(),
            lineweight_reason: lineweight_reason(&lineweight_not_drawn),
            plan_ms,
            uniform_ms,
            encode_ms,
            submit_ms,
            wait_ms,
            set_pipeline_calls: encode_state.tally.set_pipeline_calls,
            set_bind_group_calls: encode_state.tally.set_bind_group_calls,
            set_vertex_buffer_calls: encode_state.tally.set_vertex_buffer_calls,
            set_index_buffer_calls: encode_state.tally.set_index_buffer_calls,
            uniform_slots: total_slots,
            progressive_restarts,
            instanced_draws,
            instanced_batches,
            instance_streams: self.instance_streams.len(),
            mesh_draws,
            merged_mesh_batches,
            mesh_chunks: self.mesh_chunks.len(),
            arc_draws,
            arc_batches,
            arc_streams: self.arc_streams.len(),
            arc_instances: self
                .arc_streams
                .iter()
                .map(|stream| u64::from(stream.instance_count))
                .sum(),
            tier: self.render_tier,
            accelerated: use_pulled,
            accelerated_draws: if use_pulled {
                instanced_draws + mesh_draws
            } else {
                0
            },
            line_odd_vertex_skips: self.line_odd_vertex_skips,
            pulled_state: self.pulled_state.clone(),
        })
    }
}
