use super::*;

// `std::time::Instant::now()` panics on wasm32-unknown-unknown ("time not
// implemented on this platform"); `web-time` re-exports std elsewhere, so this
// is a no-op off wasm. Re-exported for the upload submodule (its timing slices
// are here) so the wasm-safe clock stays the only one named by `session.rs`.
pub(super) use web_time::Instant;

impl CadRenderRuntime {
    /// Attach a host device and initialize the renderer.
    ///
    /// `gpu_time` is the **explicit** measurement-only GPU frame-time opt-in
    /// (ADR 0013 class 1) the host derived from its environment gate; the engine
    /// never reads the environment itself. Callers that do not measure pass
    /// `cad_render_wgpu::gpu_time::GpuTimeConfig::disabled()`.
    pub fn attach_with_gpu_time(
        &mut self,
        device: wgpu::Device,
        queue: wgpu::Queue,
        preference: BackendPreference,
        gpu_time: cad_render_wgpu::gpu_time::GpuTimeConfig,
        msaa_samples: u32,
        msaa_supported_samples: Vec<u32>,
    ) {
        self.detach();
        let mut renderer = Renderer::new(preference);
        renderer.set_progressive_rendering(true);
        // A fresh renderer must be told the host's MSAA request and the
        // adapter's real capability before any pipeline is created.
        renderer.set_requested_msaa_samples(msaa_samples);
        renderer.set_supported_msaa_samples(msaa_supported_samples);
        match renderer.initialize_with_device(device, queue) {
            Ok(caps) => {
                // The production session uses the in-stream flags path for
                // entity-level selection/hover/hidden: the app resolves those
                // refs into `(SelectionRef, bits)` (see `PreparedScene::entity_flags`)
                // and `apply_ready_updates` pushes them here. Enabling the toggle
                // is a 16-byte control-uniform write; with no entity flags set it
                // is pixel-identical to the pre-flags renderer.
                renderer.set_selection_flags_enabled(true);
                // Measurement-only: apply the explicit opt-in against the live
                // device's granted features. Never a runtime dependency; the
                // verdict (enabled/disabled/unsupported) is carried per frame.
                renderer.set_gpu_time_measurement(gpu_time);
                self.epoch += 1;
                self.outcome = Some(BackendOutcome::Initialized {
                    preference: preference_choice(preference),
                    actual: backend_kind(caps.actual),
                });
                self.caps = Some(caps);
                self.renderer = Some(renderer);
                self.lifecycle = RenderLifecycle::Ready {
                    device_epoch: self.epoch,
                };
                self.dirty.invalidate();
            }
            Err(error) => self.fail(preference, error.to_string(), false),
        }
    }

    pub fn detach(&mut self) {
        self.renderer = None;
        self.caps = None;
        self.outcome = None;
        self.applied_revision = None;
        self.pending_upload = None;
        self.presented_revision = None;
        self.applied_base = None;
        self.applied_base_scene = None;
        self.applied_flags_revision = None;
        self.target_size = None;
        self.base_batches = 0;
        self.lifecycle = RenderLifecycle::Detached;
        self.diagnostic = None;
        // A device change starts a fresh pulled organisation: a warning latched
        // against the old device must not outlive it.
        self.gpu_warning = None;
        self.last_pulled_state = None;
        self.overlay_suffix_commits = 0;
        self.applied_highlight = None;
        self.image_diagnostic = None;
        self.flags_diagnostic = None;
        self.overwrite_diagnostic = None;
        self.uploads_started = 0;
        self.upload_revision_resets = 0;
        self.ready_none_clears = 0;
        self.last_upload_reset = None;
        // A device change invalidates any un-taken observation: it must never be
        // replayed against the new device epoch.
        self.pending_render_observation = None;
        self.last_taken_render_seq = None;
        self.pending_upload_timing = None;
        self.last_taken_upload_timing_seq = None;
        self.dirty.invalidate();
    }

    pub fn fail(&mut self, preference: BackendPreference, reason: String, lost: bool) {
        self.detach();
        self.lifecycle = if lost {
            RenderLifecycle::Lost {
                reason: reason.clone(),
            }
        } else {
            RenderLifecycle::Failed {
                reason: reason.clone(),
            }
        };
        self.outcome = Some(BackendOutcome::Failed {
            preference: preference_choice(preference),
            failure: BackendFailure::InitFailed {
                reason: reason.clone(),
            },
        });
        self.diagnostic = Some(reason);
    }

    pub fn uploading(&self) -> bool {
        self.pending_upload.is_some()
    }
    pub fn drawing_pages(&self) -> bool {
        self.renderer.as_ref().is_some_and(Renderer::frame_pending)
    }
    pub fn drawing_progress(&self) -> Option<(usize, usize)> {
        self.renderer.as_ref().and_then(Renderer::frame_progress)
    }

    /// Raise the page budget to the ceiling for the settling walk; the cost model
    /// is retained. No-op without a renderer.
    pub fn boost_page_budget(&mut self) {
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.boost_pacing_scale();
        }
    }

    /// Cumulative overlay-suffix commits (suffix-only uploads). Camera-only
    /// navigation must leave this flat.
    pub fn overlay_suffix_commits(&self) -> u64 {
        self.overlay_suffix_commits
    }

    /// The GPU base revision the runtime considers resident. Test/diagnostic only.
    #[cfg(test)]
    pub(crate) fn applied_base_for_test(&self) -> Option<u64> {
        self.applied_base
    }

    /// The base batch prefix preserved across the overlay suffix. Test only.
    #[cfg(test)]
    pub(crate) fn base_batches_for_test(&self) -> usize {
        self.base_batches
    }

    /// The retained applied-overlay `Arc`, so a hardware test can assert it is the
    /// same allocation as the published scene's overlay (the ABA guard). Test only.
    #[cfg(test)]
    pub(crate) fn applied_highlight_for_test(&self) -> Option<Arc<SceneDelta>> {
        self.applied_highlight.clone()
    }

    /// Model a resident base without a GPU so the pure-state qualifier can be
    /// exercised honestly (the GPU path is covered by the ignored hardware test).
    #[cfg(test)]
    pub(crate) fn force_resident_base_for_test(&mut self, base_revision: u64, base_batches: usize) {
        self.lifecycle = RenderLifecycle::Ready { device_epoch: 1 };
        self.applied_base = Some(base_revision);
        self.applied_revision = Some(base_revision);
        self.base_batches = base_batches;
        self.pending_upload = None;
    }

    /// Whether the GPU has committed exactly `scene_revision` and its base
    /// revision. A frame may only be labeled [`FrameCompleteness::CurrentScene`]
    /// when this holds: a CPU-ready scene whose chunked upload is still pending
    /// must never be presented under that label (it would show the resident old
    /// scene while claiming it is current).
    /// The scene revision the GPU has committed (the runtime's own bookkeeping,
    /// exposed for host evidence; it is `applied_revision`).
    pub fn committed_revision(&self) -> Option<u64> {
        self.applied_revision
    }

    pub fn scene_committed(&self, scene_revision: u64, base_revision: u64) -> bool {
        self.applied_revision == Some(scene_revision) && self.applied_base == Some(base_revision)
    }

    /// The in-flight upload's `base_changed` kind, if any (`diagnostics` only).
    pub(crate) fn pending_upload_kind(&self) -> Option<bool> {
        self.pending_upload
            .as_ref()
            .map(|pending| pending.base_changed)
    }

    /// The current device epoch (incremented on every successful attach).
    pub(crate) fn device_epoch(&self) -> u64 {
        self.epoch
    }

    /// The latest published base is fully resident on the GPU: the device is
    /// ready, the applied base revision matches `base_publication_revision`, and
    /// no pending upload is replacing the base. This is the narrow qualifier that
    /// lets the resident scene be presented while newer overlay inputs are still
    /// preparing, and that lets the flags catch-up resolve refs against resident
    /// pick keys — without ever re-uploading or re-committing base geometry.
    pub fn base_fully_resident(&self, base_publication_revision: u64) -> bool {
        matches!(self.lifecycle, RenderLifecycle::Ready { .. })
            && self.applied_base == Some(base_publication_revision)
            && self
                .pending_upload
                .as_ref()
                .is_none_or(|pending| !pending.base_changed)
    }

    /// Push **only** the entity-level flags of a published scene whose geometry
    /// is gate-stale (newer overlay inputs are still preparing), so the hover /
    /// selection tint never waits for the next publication and can never revert
    /// to an older set while the cursor moves.
    ///
    /// Safety: it never touches geometry. It applies only when the device is
    /// ready, no upload is in flight, and the published scene's base is exactly
    /// the resident base (`base_fully_resident`) — so the pick keys the refs
    /// resolve against are the resident ones. `flags_revision` is monotonic in
    /// the controller, and an already-applied revision is a no-op.
    pub fn apply_flags_catch_up(
        &mut self,
        base_revision: u64,
        flags: &[(SelectionRef, u32)],
        flags_revision: u64,
    ) -> CadResult<()> {
        if !matches!(self.lifecycle, RenderLifecycle::Ready { .. })
            || self.pending_upload.is_some()
            || !self.base_fully_resident(base_revision)
            || self.applied_flags_revision == Some(flags_revision)
        {
            return Ok(());
        }
        let report = self
            .renderer
            .as_mut()
            .expect("ready renderer")
            .set_entity_flags(flags)?;
        self.applied_flags_revision = Some(flags_revision);
        self.flags_diagnostic = flags_diagnostic(&report.unmatched);
        // Never wipe a still-relevant image diagnostic: merge.
        self.diagnostic = merge_diagnostics(
            self.image_diagnostic.as_deref(),
            self.flags_diagnostic.as_deref(),
        );
        self.dirty.invalidate();
        Ok(())
    }
}

mod frame;
mod observation;
mod upload;
