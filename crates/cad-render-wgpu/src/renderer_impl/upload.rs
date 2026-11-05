use super::*;

impl Renderer {
    // Upload CPU batches as static GPU buffers (once per change).
    //
    // Mesh batches get an index buffer and a repaired normal buffer; line
    // batches keep the position-only layout.
    pub fn upload(&mut self, delta: &SceneDelta) -> CadResult<()> {
        let prepared = self.prepare_upload(delta)?;
        self.commit_upload(prepared, self.batches.len())?;
        // Wipeout masks are a separate resident set, exactly like images: a full
        // scene upload replaces them wholesale (ADR 0022). Hosts that stage and
        // commit geometry themselves call `upload_masks` on their base change.
        self.upload_masks(&delta.masks)
    }

    /// Atomically replace the suffix after `keep_prefix` (0 replaces the scene).
    /// This supports independently updating an annotation overlay while keeping
    /// base drawing buffers. A stale-device upload cannot be published.
    ///
    /// Compatibility entry point: the **whole resulting resident scene is base**
    /// (no overlay range), which is how every pre-base/overlay direct caller
    /// behaved. Hosts with a real transient overlay use
    /// [`Renderer::commit_upload_with_base`] so the overlay stays out of the base
    /// target and the base walk is not reset.
    pub fn commit_upload(&mut self, prepared: PreparedUpload, keep_prefix: usize) -> CadResult<()> {
        let base_end = keep_prefix.saturating_add(prepared.batches.len());
        self.commit_upload_with_base(prepared, keep_prefix, base_end)
    }

    /// Commit with an explicit base/overlay split: base batches are
    /// `[0, base_end)`, the transient overlay is `[base_end, batch_count)`.
    ///
    /// A true suffix commit (`keep_prefix > 0` and an unchanged `base_end`) keeps
    /// `progressive_frame` intact (the base prefix indices are unchanged) and
    /// touches neither the base targets nor pacing: the next render only
    /// recomposites the overlay on top of the resident base. A base change
    /// (`keep_prefix == 0`, or a changed `base_end`) drops the frame so the base
    /// walk restarts.
    ///
    /// Overlay batches must be staged with
    /// [`Renderer::prepare_upload_batches_legacy`]; the overlay pass draws them
    /// with per-batch uniforms only (no stream/chunk/arc slots).
    pub fn commit_upload_with_base(
        &mut self,
        mut prepared: PreparedUpload,
        keep_prefix: usize,
        base_end: usize,
    ) -> CadResult<()> {
        if self.device.as_ref() != Some(&prepared.device)
            || prepared.device_generation != self.device_generation
            || self.device_lost
            || keep_prefix > self.batches.len()
        {
            return Err(CadError::StaleResult);
        }
        let new_total = keep_prefix + prepared.batches.len();
        if base_end > new_total || base_end < keep_prefix {
            return Err(CadError::InvalidInput(format!(
                "base_end {base_end} is outside the committed range \
                 [{keep_prefix}, {new_total})"
            )));
        }
        // The overlay pass binds per-batch uniforms only (no stream/chunk/arc
        // slots), so an overlay batch with converted runs could not be drawn.
        // Reject it explicitly instead of dropping or corrupting geometry.
        let staged_overlay = &prepared.batches[base_end - keep_prefix..];
        if staged_overlay.iter().any(|batch| {
            !batch.instanced.is_empty() || !batch.mesh_runs.is_empty() || !batch.arc_runs.is_empty()
        }) {
            return Err(CadError::InvalidInput(
                "overlay batches must be staged with prepare_upload_batches_legacy \
                 (converted runs have no overlay draw path)"
                    .into(),
            ));
        }
        // `commit_started` covers the incoming reference collection and the
        // admission (prefix scans/allocs) below plus the staging/append/commit, so
        // `CommitTiming::total_us >= the sum of the substages` (it additionally
        // covers those prefix scans/allocs and the append). It begins **after** the
        // stale-validation guard above, which is therefore not attributed to any
        // substage or to `total_us` (negligible, and deliberately unclaimed).
        let commit_started = Instant::now();
        // The **new (incoming)** suffix's rebased resource references (for the
        // ownership proof): the prepared runs are rebased onto the current
        // resource counts.
        let incoming = IncomingResources {
            streams: prepared
                .batches
                .iter()
                .flat_map(|batch| {
                    batch
                        .instanced
                        .iter()
                        .map(|run| run.stream + self.instance_streams.len() as u32)
                })
                .collect(),
            meshes: prepared
                .batches
                .iter()
                .flat_map(|batch| {
                    batch
                        .mesh_runs
                        .iter()
                        .map(|run| run.chunk + self.mesh_chunks.len() as u32)
                })
                .collect(),
            arcs: prepared
                .batches
                .iter()
                .flat_map(|batch| {
                    batch
                        .arc_runs
                        .iter()
                        .map(|run| run.stream + self.arc_streams.len() as u32)
                })
                .collect(),
        };
        // Admission is computed **before** any mutation, from the renderer's own
        // retained state (never from the caller's `base_changed` alone).
        let fast = self.pulled_fastpath_plan(keep_prefix, &incoming);
        // Re-base the prepared streams' run indices onto the existing streams so
        // a kept prefix's converted batches keep pointing at valid buffers.
        let rebase_started = Instant::now();
        let base = self.instance_streams.len() as u32;
        for batch in &mut prepared.batches {
            for run in &mut batch.instanced {
                run.stream += base;
            }
        }
        let mesh_base = self.mesh_chunks.len() as u32;
        for batch in &mut prepared.batches {
            for run in &mut batch.mesh_runs {
                run.chunk += mesh_base;
            }
        }
        // Re-base the prepared conic runs onto the existing conic streams.
        let arc_base = self.arc_streams.len() as u32;
        for batch in &mut prepared.batches {
            for run in &mut batch.arc_runs {
                run.stream += arc_base;
            }
        }
        let rebase_us = rebase_started.elapsed().as_micros() as u64;
        let append_started = Instant::now();
        self.instance_stream_uploads += prepared.instances.len() as u64;
        self.arc_stream_uploads += prepared.arcs.len() as u64;
        self.arc_degenerate_skips += prepared.arc_degenerate_skips;
        self.line_odd_vertex_skips += prepared.line_odd_vertex_skips;
        self.mesh_chunk_uploads += prepared.mesh_chunks.len() as u64;
        self.instance_streams.append(&mut prepared.instances);
        self.arc_streams.append(&mut prepared.arcs);
        self.mesh_chunks.append(&mut prepared.mesh_chunks);
        // The pick keys of the batches this commit drops (the replaced suffix):
        // the pick table retires exactly these entries incrementally instead of
        // rebuilding over the whole resident scene.
        let retired_pick_keys: Vec<u32> = self.batches[keep_prefix..]
            .iter()
            .map(|batch| batch.pick_key)
            .collect();
        self.batches.truncate(keep_prefix);
        self.batches.extend(prepared.batches);
        let append_us = append_started.elapsed().as_micros() as u64;
        let gc_started = Instant::now();
        self.gc_instance_streams();
        self.gc_arc_streams();
        self.gc_mesh_chunks();
        let gc_us = gc_started.elapsed().as_micros() as u64;
        let pulled_started = Instant::now();
        self.rebuild_pulled(fast);
        let pulled_us = pulled_started.elapsed().as_micros() as u64;
        let pick_started = Instant::now();
        self.rebuild_pick_table(Some((keep_prefix, retired_pick_keys)))?;
        let pick_us = pick_started.elapsed().as_micros() as u64;
        // Base/overlay split of the new resident scene. A true suffix commit
        // leaves the base prefix untouched, so the cached progressive walk
        // (`ordered` is base-only, indices are stable) stays valid and is *not*
        // reset; only `draw_plan`/`batch_facts` are rebuilt on the next render
        // because the overlay suffix changed. A base change (a full replacement,
        // or a base count change) invalidates the walk.
        let base_changed = keep_prefix == 0 || base_end != self.base_end;
        self.base_end = base_end;
        if base_changed {
            self.progressive_frame = None;
            // Geometry changed, so any retained-colour exception is void.
            self.preserve_color_on_restart = false;
        }
        self.draw_plan = None;
        // Page-cost knowledge is scene-specific, so a commit that replaces the
        // scene resets the controller. A **suffix-only** commit (a true prefix is
        // kept: `keep_prefix > 0`, the same discriminator the pick table's
        // incremental path uses) only re-uploads the transient overlay range and
        // does not change the base draw-cost profile, so it must NOT reset:
        // otherwise a periodic overlay refresh (e.g. the deferred reference-grid
        // settle, once per camera settle) knocks the learned scale back to 1.0
        // every settle, the controller never converges, and pages stay
        // vertex-heavy. A base change (`keep_prefix == 0`) still resets, as do
        // `clear_batches` and a device generation change / device loss.
        //
        // This is a heuristic cost model, **not** correctness state: it is
        // deliberately not modelled as a dirty/invalidation flag, and nothing
        // else is coupled to the reset (the restart-pending flag is left as-is
        // here — a suffix commit does not reset the knowledge, so a pending
        // restart from an earlier real reset correctly stays pending).
        if keep_prefix == 0 {
            self.reset_pacing();
        }
        let lineweight_started = Instant::now();
        self.rebuild_lineweight_cache();
        let lineweight_us = lineweight_started.elapsed().as_micros() as u64;
        self.uploaded_bytes += prepared.bytes;
        self.last_upload_ms = Some(prepared.elapsed_ms);
        self.last_commit_timing = Some(CommitTiming {
            rebase_us,
            append_us,
            gc_us,
            pulled_us,
            pick_us,
            lineweight_us,
            total_us: commit_started.elapsed().as_micros() as u64,
        });
        Ok(())
    }

    /// Drop instance streams no surviving batch references and re-number the
    /// rest, so a truncated scene does not leak GPU buffers across commits.
    pub(super) fn gc_instance_streams(&mut self) {
        if self.instance_streams.is_empty() {
            return;
        }
        let mut map: Vec<Option<usize>> = vec![None; self.instance_streams.len()];
        let mut next = 0usize;
        for batch in &mut self.batches {
            for run in &mut batch.instanced {
                let slot = &mut map[run.stream as usize];
                let new_index = match *slot {
                    Some(index) => index,
                    None => {
                        *slot = Some(next);
                        next += 1;
                        next - 1
                    }
                };
                run.stream = new_index as u32;
            }
        }
        if next == self.instance_streams.len()
            && map
                .iter()
                .enumerate()
                .all(|(index, slot)| *slot == Some(index))
        {
            // Identity mapping: nothing dropped and no renumbering happened.
            return;
        }
        let mut result: Vec<Option<InstanceStream>> = (0..next).map(|_| None).collect();
        for (old_index, stream) in std::mem::take(&mut self.instance_streams)
            .into_iter()
            .enumerate()
        {
            if let Some(new_index) = map[old_index] {
                result[new_index] = Some(stream);
            }
        }
        self.instance_streams = result
            .into_iter()
            .map(|stream| stream.expect("referenced by a surviving batch"))
            .collect();
    }

    /// Drop SDF conic streams no surviving batch references and re-number the
    /// rest, so a truncated scene does not leak GPU buffers across commits.
    pub(super) fn gc_arc_streams(&mut self) {
        if self.arc_streams.is_empty() {
            return;
        }
        let mut map: Vec<Option<usize>> = vec![None; self.arc_streams.len()];
        let mut next = 0usize;
        for batch in &mut self.batches {
            for run in &mut batch.arc_runs {
                let slot = &mut map[run.stream as usize];
                let new_index = match *slot {
                    Some(index) => index,
                    None => {
                        *slot = Some(next);
                        next += 1;
                        next - 1
                    }
                };
                run.stream = new_index as u32;
            }
        }
        if next == self.arc_streams.len()
            && map
                .iter()
                .enumerate()
                .all(|(index, slot)| *slot == Some(index))
        {
            return;
        }
        let mut result: Vec<Option<ArcStream>> = (0..next).map(|_| None).collect();
        for (old_index, stream) in std::mem::take(&mut self.arc_streams)
            .into_iter()
            .enumerate()
        {
            if let Some(new_index) = map[old_index] {
                result[new_index] = Some(stream);
            }
        }
        self.arc_streams = result
            .into_iter()
            .map(|stream| stream.expect("referenced by a surviving batch"))
            .collect();
    }

    /// Drop merged mesh chunks no surviving batch references and re-number the
    /// rest, so a truncated scene does not leak GPU buffers across commits.
    pub(super) fn gc_mesh_chunks(&mut self) {
        if self.mesh_chunks.is_empty() {
            return;
        }
        let mut map: Vec<Option<usize>> = vec![None; self.mesh_chunks.len()];
        let mut next = 0usize;
        for batch in &mut self.batches {
            for run in &mut batch.mesh_runs {
                let slot = &mut map[run.chunk as usize];
                let new_index = match *slot {
                    Some(index) => index,
                    None => {
                        *slot = Some(next);
                        next += 1;
                        next - 1
                    }
                };
                run.chunk = new_index as u32;
            }
        }
        if next == self.mesh_chunks.len()
            && map
                .iter()
                .enumerate()
                .all(|(index, slot)| *slot == Some(index))
        {
            // Identity mapping: nothing dropped and no renumbering happened.
            return;
        }
        let mut result: Vec<Option<GpuMeshChunk>> = (0..next).map(|_| None).collect();
        for (old_index, chunk) in std::mem::take(&mut self.mesh_chunks)
            .into_iter()
            .enumerate()
        {
            if let Some(new_index) = map[old_index] {
                result[new_index] = Some(chunk);
            }
        }
        self.mesh_chunks = result
            .into_iter()
            .map(|chunk| chunk.expect("referenced by a surviving batch"))
            .collect();
    }
}

/// World-space z range `[min, max]` of a scene batch's geometry.
///
/// Used only to derive an orthographic frame's depth band ([`plan::OrthoDepth`]):
/// a batch whose triangles span `[-1, 1]` in z needs a depth range that contains
/// that span, or its faces are clipped by (or, at the far plane, rejected by) the
/// depth mapping. Positions are `local_origin + stored`, narrowed to `f32` after
/// the `f64` addition exactly like the packed vertices. A batch with no
/// positions (an empty batch) reports its origin.
pub(crate) fn batch_world_z_range(batch: &cad_scene::RenderBatch) -> [f32; 2] {
    let origin_z = batch.local_origin.z;
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    let mut sample = |z: f64| {
        let world = origin_z + z;
        if world.is_finite() {
            lo = lo.min(world);
            hi = hi.max(world);
        }
    };
    for vertex in &batch.vertices {
        sample(f64::from(vertex[2]));
    }
    for conic in &batch.conics {
        sample(conic.center.z);
    }
    if lo > hi {
        let z = origin_z as f32;
        return [z, z];
    }
    [lo as f32, hi as f32]
}
