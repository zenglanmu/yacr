use super::*;

impl Renderer {
    /// Poll the device and classify the frame's outcome.
    ///
    /// A `Timeout` means the device made no progress in the bound (1 s): on
    /// native backends that is the observable proxy for a lost/reset device, so
    /// it is reported as [`ScopeOutcome::DeviceLost`]. A wrong submission index
    /// is a frame-level mistake, not a loss. A clean poll means no error the
    /// renderer can observe synchronously; the host's device-lost callback still
    /// remains authoritative.
    ///
    /// On `wasm32` the browser backends (WebGL2/WebGPU) complete asynchronously
    /// and must never block the main thread: `PollType::Wait` is unavailable, so
    /// a non-blocking `Poll` is used and a timeout is **not** treated as device
    /// loss (a lost browser device surfaces through wgpu's device-lost callback).
    /// Read the measurement-only GPU-frame-time bracket, if it is active.
    ///
    /// Measurement-only (ADR 0013 class 1). Returns `None` unless the timestamp
    /// resources exist and the readback mapping completed, so the product
    /// configuration always reports `None`. The blocking readback strategy is
    /// deliberately the **existing** poll: `render_with_projection` requests the
    /// mapping right after `queue.submit` and the device-loss classification poll
    /// (`device_scope_outcome`) completes it — no extra blocking wait is added.
    pub(super) fn read_gpu_time_ms(&self, queue: &wgpu::Queue) -> Option<f64> {
        let resources = self.gpu_time.as_ref()?;
        let slice = resources.readback.slice(0..16);
        let view = match slice.get_mapped_range() {
            Ok(view) => view,
            // The mapping did not complete (for example a poll timeout on the
            // device-lost path). Report no measurement rather than a guess, and
            // clear the pending map so the next frame's copy into this buffer is
            // valid.
            Err(_) => {
                resources.readback.unmap();
                return None;
            }
        };
        let mut ticks = [0u64; 2];
        for (index, slot) in ticks.iter_mut().enumerate() {
            let start = index * 8;
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&view[start..start + 8]);
            *slot = u64::from_le_bytes(bytes);
        }
        drop(view);
        // Always unmap, so the next frame's `copy_buffer_to_buffer` into this
        // `MAP_READ` buffer is valid.
        resources.readback.unmap();
        let period = queue.get_timestamp_period();
        Some(gpu_time::ticks_to_ms(
            ticks[1].saturating_sub(ticks[0]),
            period,
        ))
    }

    pub(super) fn device_scope_outcome(
        &self,
        device: &wgpu::Device,
        submission: wgpu::SubmissionIndex,
    ) -> ScopeOutcome {
        #[cfg(target_arch = "wasm32")]
        let poll_type = {
            let _ = submission;
            wgpu::PollType::Poll
        };
        #[cfg(not(target_arch = "wasm32"))]
        let poll_type = wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(self.poll_timeout),
        };
        match device.poll(poll_type) {
            Ok(_) => ScopeOutcome::Clean,
            Err(wgpu::PollError::Timeout) => {
                #[cfg(target_arch = "wasm32")]
                {
                    ScopeOutcome::Clean
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    ScopeOutcome::DeviceLost
                }
            }
            Err(wgpu::PollError::WrongSubmissionIndex(..)) => ScopeOutcome::FrameError,
        }
    }

    pub fn frame_texture(&self) -> Option<&wgpu::Texture> {
        self.target.as_ref()
    }

    pub fn device_generation(&self) -> u64 {
        self.device_generation
    }

    pub fn batch_count(&self) -> usize {
        self.batches.len()
    }

    /// Enable or disable converting eligible opaque line batches into instanced
    /// streams. Enabled by default; disabling forces the legacy per-batch path
    /// (used to compare the two paths). The two are byte-identical only for an
    /// `f32`-representable shared origin; otherwise the instanced path is bound
    /// to `MAX_STREAM_RADIUS` rather than exact (see the deep-zoom fence).
    ///
    /// The flag takes effect on the **next** upload: the resident scene keeps
    /// rendering with the streams it already holds, so a batch set is never left
    /// with dangling stream references. Re-upload (or `clear_batches`) after
    /// toggling to switch the resident scene onto the other path.
    pub fn set_line_instancing(&mut self, enabled: bool) {
        self.line_instancing = enabled;
    }

    /// Enable or disable flat-merging eligible opaque mesh batches into chunks
    /// (and instancing their edge overlays). Enabled by default; disabling forces
    /// the legacy per-batch mesh path so the two can be compared.
    ///
    /// The flag takes effect on the **next** upload: the resident scene keeps
    /// rendering with the chunks it already holds, so a batch set is never left
    /// with dangling chunk references. Re-upload (or `clear_batches`) after
    /// toggling to switch the resident scene onto the other path.
    pub fn set_mesh_merging(&mut self, enabled: bool) {
        self.mesh_merging = enabled;
    }

    /// The live draw tier (see [`RenderTier`]), chosen once per device in
    /// [`Renderer::initialize_with_device`]. Before a device exists this is
    /// [`RenderTier::Universal`].
    pub fn render_tier(&self) -> RenderTier {
        self.render_tier
    }

    /// Test hook: override the storage-buffer capability the tier decision reads.
    ///
    /// `Some(false)` simulates a device without vertex-stage storage buffers (the
    /// automatic downgrade a WebGL2 device takes); `Some(true)` forces the
    /// accelerated tier so it can be exercised on a device that has storage
    /// buffers; `None` restores the real probe. Takes effect on the next
    /// `initialize_with_device`.
    pub fn set_storage_buffer_support_override(&mut self, support: Option<bool>) {
        self.storage_support_override = support;
    }

    /// Number of instances in the resident accelerated line storage (0 on the
    /// universal tier).
    pub fn pulled_line_instance_count(&self) -> u32 {
        self.pulled_lines.as_ref().map_or(0, |pulled| {
            pulled
                .segments
                .iter()
                .map(|segment| segment.instance_count)
                .sum()
        })
    }

    /// Number of vertices in the resident accelerated merged-mesh storage (0 on
    /// the universal tier).
    pub fn pulled_mesh_vertex_count(&self) -> u32 {
        self.pulled_mesh.as_ref().map_or(0, |pulled| {
            pulled
                .segments
                .iter()
                .map(|segment| segment.vertex_count)
                .sum()
        })
    }

    /// Number of storage segments the accelerated line class is split into (a
    /// real scene can exceed `max_storage_buffer_binding_size`).
    pub fn pulled_line_segment_count(&self) -> usize {
        self.pulled_lines
            .as_ref()
            .map_or(0, |pulled| pulled.segments.len())
    }

    /// Number of storage segments the accelerated merged-mesh class is split
    /// into.
    pub fn pulled_mesh_segment_count(&self) -> usize {
        self.pulled_mesh
            .as_ref()
            .map_or(0, |pulled| pulled.segments.len())
    }

    /// Override the maximum vertices per merged mesh chunk (default
    /// [`plan::MAX_VERTICES_PER_MESH_STREAM`]). A batch with more vertices is
    /// split across several chunks. Test hook. Takes effect on the next upload.
    pub fn set_max_vertices_per_mesh_stream(&mut self, max_vertices: usize) {
        self.max_vertices_per_mesh_stream = max_vertices;
    }

    /// Override the maximum instances per instanced stream buffer (default
    /// [`MAX_INSTANCES_PER_STREAM`]). A batch with more segments is split across
    /// several streams. Test hook: it lets the split path be exercised without
    /// allocating a million-segment batch. Takes effect on the next upload.
    pub fn set_max_instances_per_stream(&mut self, max_instances: usize) {
        self.max_instances_per_stream = max_instances;
    }

    /// Test hook: override the storage-buffer binding limit the accelerated tier
    /// sizes its segments against. Lets the reduced-limit regime (an oversize
    /// stream that must downgrade to the universal per-run path, and the chunk
    /// table limit) be exercised without a device that reports such a limit.
    /// `None` restores the device's real limit. Takes effect on the next commit.
    pub fn set_pulled_binding_limit_override(&mut self, limit: Option<u64>) {
        self.pulled_binding_limit_override = limit;
        // The override changes the segmentation capacity, so a retained base range
        // set is no longer valid.
        self.invalidate_pulled_prefix();
    }

    /// Test hook: override the maximum instances per accelerated line storage
    /// segment, so the multi-segment path is exercisable on a small scene.
    /// `None` restores the binding-derived capacity. Takes effect on the next
    /// commit.
    pub fn set_pulled_line_segment_elems_override(&mut self, elems: Option<usize>) {
        self.pulled_line_segment_elems_override = elems;
        // The override changes the line segmentation, so a retained base range
        // set is no longer valid.
        self.invalidate_pulled_prefix();
    }

    /// Test hook: override the memory budget the pre-flight pulled capacity gate
    /// compares its exact requirement against (see
    /// [`Renderer::pulled_budget_bytes`]). `None` restores the device-limit
    /// derived budget. Takes effect on the next commit; segmentation (and
    /// therefore any retained base range set) is unaffected.
    pub fn set_pulled_memory_budget_override(&mut self, budget: Option<u64>) {
        self.pulled_memory_budget_override = budget;
    }

    /// Test-only hook: treat the pulled allocation error scope as if it had
    /// captured this out-of-memory message, so the capture → discard → universal
    /// fallback path is exercisable without provoking a real driver OOM. Test
    /// builds only; takes effect on the next commit.
    #[cfg(test)]
    pub fn set_pulled_oom_injection(&mut self, message: Option<&'static str>) {
        self.pulled_oom_injection = message;
    }

    /// Observable state of the accelerated pulled organisation (see
    /// [`PulledState`]); also carried on every [`FrameStats`].
    pub fn pulled_state(&self) -> PulledState {
        self.pulled_state.clone()
    }

    /// Test hook: override the anti-aliased line SDF ramp width (world units per
    /// pixel) for the line class only. `None` restores the projection's real
    /// value. A sub-pixel ramp reproduces the pre-unification hard-aliased 1 px
    /// line look, which the visual-acceptance test uses as its "before" frame.
    pub fn set_line_ramp_override(&mut self, world_per_px: Option<f32>) {
        self.line_ramp_override = world_per_px;
    }

    /// Test hook: enable/disable chunk-level view culling. The product always
    /// culls; a test disables it to render the *unculled* reference frame and
    /// fence byte parity against the culled frame.
    pub fn set_view_culling(&mut self, enabled: bool) {
        self.cull_disabled = !enabled;
    }

    /// Number of resident instanced line streams.
    pub fn instance_stream_count(&self) -> usize {
        self.instance_streams.len()
    }

    /// Total number of instanced segments (one per 2-vertex pair) across all
    /// resident streams.
    pub fn instance_segment_count(&self) -> u64 {
        self.instance_streams
            .iter()
            .map(|stream| u64::from(stream.instance_count))
            .sum()
    }

    /// Monotonic count of instanced stream buffers uploaded since the last
    /// device initialization. A camera move (pan/zoom) must never increase it.
    pub fn instance_stream_uploads(&self) -> u64 {
        self.instance_stream_uploads
    }

    /// Number of resident SDF conic streams.
    pub fn arc_stream_count(&self) -> usize {
        self.arc_streams.len()
    }

    /// Total number of SDF conic instances across all resident streams.
    pub fn arc_instance_count(&self) -> u64 {
        self.arc_streams
            .iter()
            .map(|stream| u64::from(stream.instance_count))
            .sum()
    }

    /// Monotonic count of SDF conic stream buffers uploaded since the last
    /// device initialization. A camera move (pan/zoom) must never increase it.
    pub fn arc_stream_uploads(&self) -> u64 {
        self.arc_stream_uploads
    }

    /// Monotonic count of degenerate conics skipped at pack since the last device
    /// initialization (a defensive guard the provider already excludes). Never
    /// silent: a non-zero value means a conic was dropped before it could be
    /// instanced.
    pub fn arc_degenerate_skips(&self) -> u64 {
        self.arc_degenerate_skips
    }

    /// Monotonic count of trailing unpaired vertices dropped from odd-length
    /// `Lines`/`MeshEdges` batches since the last device initialization. A
    /// `LineList` cannot pair the trailing vertex and every path drops it; this
    /// reports the drop so it is never silent.
    pub fn line_odd_vertex_skips(&self) -> u64 {
        self.line_odd_vertex_skips
    }

    /// Number of resident merged mesh chunks.
    pub fn mesh_chunk_count(&self) -> usize {
        self.mesh_chunks.len()
    }

    /// Total number of merged mesh vertices across all resident chunks.
    pub fn mesh_vertex_count(&self) -> u64 {
        self.mesh_chunks
            .iter()
            .map(|chunk| u64::from(chunk.vertex_count))
            .sum()
    }

    /// Monotonic count of merged mesh chunk buffers uploaded since the last
    /// device initialization. A camera move (pan/zoom) must never increase it.
    pub fn mesh_chunk_uploads(&self) -> u64 {
        self.mesh_chunk_uploads
    }

    /// Number of distinct GPU textures resident for the current image set.
    ///
    /// Image quads that share a [`ResourceKey`] share one texture, so this is
    /// the count of unique resources, not of quads.
    pub fn image_texture_count(&self) -> usize {
        self.image_textures.len()
    }

    /// Diagnostic for an over-budget frame, if any is pending.
    pub fn over_budget_reason(report: &OverBudget) -> DiagnosticReason {
        over_budget_reason(report)
    }

    /// Diagnostic describing a device loss.
    pub fn device_lost_reason(detail: &str) -> DiagnosticReason {
        DiagnosticReason::missing(
            codes::RENDER_DEVICE_LOST,
            vec![DiagnosticParameter::Identifier(detail.to_string())],
        )
    }
}
