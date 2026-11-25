//! Render/upload observations, GPU-warning latch and runtime diagnostics.
//!
//! Split out of `session.rs`; behaviour is unchanged.

use super::super::*;

impl CadRenderRuntime {
    /// Update the sticky GPU warning for one pulled-state observation.
    ///
    /// Only a **transition** changes the latch (see [`gpu_warning_transition`]);
    /// the message is never rewritten on every frame, so a lasting fallback keeps
    /// one stable string for the host's own de-duplication.
    fn note_pulled_state(&mut self, state: &cad_render_wgpu::PulledState) {
        match gpu_warning_transition(self.last_pulled_state.as_ref(), state) {
            GpuWarningDecision::Unchanged => {}
            GpuWarningDecision::Warn(message) => self.gpu_warning = Some(message),
            GpuWarningDecision::Clear => self.gpu_warning = None,
        }
        self.last_pulled_state = Some(state.clone());
    }

    /// Record the observation of one actual renderer invocation.
    pub(super) fn record_render_observation(
        &mut self,
        stats: cad_render_wgpu::FrameStats,
        view_revision: u64,
        progressive: Option<(usize, usize)>,
        progressive_complete: bool,
    ) {
        // Latch/clear the one-time GPU warning on a pulled-state transition before
        // moving the state into the observation.
        self.note_pulled_state(&stats.pulled_state);
        self.render_seq += 1;
        let (cursor, total) = match progressive {
            Some((cursor, total)) => (Some(cursor), Some(total)),
            None => (None, None),
        };
        self.pending_render_observation = Some(RenderStatsObservation {
            render_seq: self.render_seq,
            device_epoch: self.epoch,
            view_revision,
            applied_revision: self.applied_revision,
            applied_base: self.applied_base,
            uploads_started: self.uploads_started,
            upload_revision_resets: self.upload_revision_resets,
            ready_none_clears: self.ready_none_clears,
            cpu_ms: stats.cpu_ms,
            gpu_ms: stats.gpu_ms,
            gpu_time: stats.gpu_time,
            plan_ms: stats.plan_ms,
            uniform_ms: stats.uniform_ms,
            encode_ms: stats.encode_ms,
            submit_ms: stats.submit_ms,
            wait_ms: stats.wait_ms,
            draw_calls: stats.draw_calls,
            vertices: stats.vertices,
            triangles: stats.triangles,
            instanced_draws: stats.instanced_draws,
            mesh_draws: stats.mesh_draws,
            legacy_opaque_draws: stats.legacy_opaque_draws,
            arc_draws: stats.arc_draws,
            transparent_batches: stats.transparent_batches,
            image_batches: stats.image_batches,
            cull_chunks_total: stats.cull_chunks_total,
            cull_chunks_kept: stats.cull_chunks_kept,
            accelerated: stats.accelerated,
            pulled_state: stats.pulled_state,
            instance_streams: stats.instance_streams,
            instanced_batches: stats.instanced_batches,
            tier: stats.tier.as_str(),
            over_budget: stats.over_budget.is_some(),
            budget_scale: stats.budget_scale,
            budget_target_ms: stats.budget_target_ms,
            budget_cost_ema_ms: stats.budget_cost_ema_ms,
            budget_vertices: stats.budget_vertices,
            budget_triangles: stats.budget_triangles,
            pacing: stats.pacing,
            progressive_cursor: cursor,
            progressive_total: total,
            progressive_complete,
        });
    }

    /// Take the latest actual-render observation, at most once per render
    /// sequence (so a retained/waiting frame can never replay stale stats).
    pub fn take_render_observation(&mut self) -> Option<RenderStatsObservation> {
        let observation = self.pending_render_observation.as_ref()?;
        if self.last_taken_render_seq == Some(observation.render_seq) {
            return None;
        }
        let taken = self.pending_render_observation.take();
        self.last_taken_render_seq = taken.as_ref().map(|observation| observation.render_seq);
        taken
    }

    /// Record one upload work-unit's stage timings (assigning its sequence).
    pub(in crate::render::runtime) fn record_upload_timing(
        &mut self,
        mut observation: UploadTimingObservation,
    ) {
        self.upload_timing_seq += 1;
        observation.seq = self.upload_timing_seq;
        self.pending_upload_timing = Some(observation);
    }

    /// Take the latest upload stage-timing observation, at most once per upload
    /// work unit (so a host never replays stale timings).
    pub fn take_upload_timing_observation(&mut self) -> Option<UploadTimingObservation> {
        let observation = self.pending_upload_timing.as_ref()?;
        if self.last_taken_upload_timing_seq == Some(observation.seq) {
            return None;
        }
        let taken = self.pending_upload_timing.take();
        self.last_taken_upload_timing_seq = taken.as_ref().map(|observation| observation.seq);
        taken
    }

    /// Read-only runtime bookkeeping for the waiting/upload diagnosis.
    ///
    /// Pure projection: it borrows `&self` and never advances an upload, a
    /// revision or the view, and never forces engine work.
    pub fn diagnostics(&self) -> RuntimeDiagnostics {
        RuntimeDiagnostics {
            lifecycle: match &self.lifecycle {
                RenderLifecycle::Detached => "detached",
                RenderLifecycle::Ready { .. } => "ready",
                RenderLifecycle::Lost { .. } => "lost",
                RenderLifecycle::Failed { .. } => "failed",
            },
            applied_revision: self.applied_revision,
            applied_base: self.applied_base,
            resident_base_batches: self.base_batches,
            uploading: self.pending_upload.is_some(),
            upload: self
                .pending_upload
                .as_ref()
                .map(|pending| UploadDiagnostics {
                    scene_revision: pending.scene.revision,
                    group: pending.group,
                    offset: pending.offset,
                    base_changed: pending.base_changed,
                }),
            overlay_suffix_commits: self.overlay_suffix_commits,
            batch_overwrites: self.batch_overwrites,
            batch_overwrite_fallbacks: self.batch_overwrite_fallbacks,
            pulled_prefix_reuses: self
                .renderer
                .as_ref()
                .map_or(0, |renderer| renderer.pulled_prefix_reuses()),
            pulled_prefix_rebuilds: self
                .renderer
                .as_ref()
                .map_or(0, |renderer| renderer.pulled_prefix_rebuilds()),
            pulled_suffix_skips: self
                .renderer
                .as_ref()
                .map_or(0, |renderer| renderer.pulled_suffix_skips()),
            gc_skipped_commits: self
                .renderer
                .as_ref()
                .map_or(0, |renderer| renderer.gc_skipped_commits()),
            progressive_order_reuses: self
                .renderer
                .as_ref()
                .map_or(0, |renderer| renderer.progressive_order_reuses()),
            progressive_order_rebuilds: self
                .renderer
                .as_ref()
                .map_or(0, |renderer| renderer.progressive_order_rebuilds()),
            draw_plan_rebuilds: self
                .renderer
                .as_ref()
                .map_or(0, |renderer| renderer.draw_plan_rebuilds()),
            navigation_lod: self.renderer.as_ref().is_some_and(Renderer::navigation_lod),
            frames_rendered: self.frames_rendered,
            cpu_payload_releases: self.cpu_payload_releases,
            cpu_payload_bytes_released: self.cpu_payload_bytes_released,
            uploads_started: self.uploads_started,
            upload_revision_resets: self.upload_revision_resets,
            ready_none_clears: self.ready_none_clears,
            last_upload_reset: self.last_upload_reset,
            diagnostic: self.diagnostic.clone(),
            gpu_warning: self.gpu_warning.clone(),
        }
    }
}
