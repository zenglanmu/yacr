use super::*;

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
    ) {
        self.detach();
        let mut renderer = Renderer::new(preference);
        renderer.set_progressive_rendering(true);
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
        self.applied_flags_revision = None;
        self.target_size = None;
        self.base_batches = 0;
        self.lifecycle = RenderLifecycle::Detached;
        self.diagnostic = None;
        self.overlays_suppressed = false;
        self.overlay_suffix_commits = 0;
        self.applied_highlight = None;
        self.applied_entity_flags = Vec::new();
        self.image_diagnostic = None;
        self.flags_diagnostic = None;
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

    pub fn apply_ready_updates(&mut self, ready: Option<&PreparedScene>) -> CadResult<()> {
        if !matches!(self.lifecycle, RenderLifecycle::Ready { .. }) {
            return Ok(());
        }
        let Some(ready) = ready else {
            if self.pending_upload.take().is_some() {
                self.ready_none_clears += 1;
                self.last_upload_reset = Some("ready_none");
            }
            return Ok(());
        };
        // Flags-first delta over the committed geometry. A flags-only preview
        // publication keeps the scene revision, so this must be evaluated
        // BEFORE the `applied_revision == Some(ready.revision)` early return:
        // otherwise the new flags would be silently skipped. Only the entity
        // flags are pushed — no batch, stream or chunk is re-uploaded, and the
        // overlay suffix stays resident (no suppression, no base-only frame).
        if flags_delta_applies(
            self.applied_revision,
            self.applied_flags_revision,
            self.pending_upload.is_some(),
            self.overlays_suppressed,
            ready,
        ) {
            let report = self
                .renderer
                .as_mut()
                .expect("ready renderer")
                .set_entity_flags(&ready.entity_flags)?;
            // Retain the applied flags so the 2D navigation fallback can
            // re-assert them after its truncating commit.
            self.applied_entity_flags = ready.entity_flags.clone();
            self.applied_flags_revision = Some(ready.flags_revision);
            self.flags_diagnostic = flags_diagnostic(&report.unmatched);
            // Never wipe a still-relevant image diagnostic: merge.
            self.diagnostic = merge_diagnostics(
                self.image_diagnostic.as_deref(),
                self.flags_diagnostic.as_deref(),
            );
            self.dirty.invalidate();
            return Ok(());
        }
        if self.applied_revision == Some(ready.revision) {
            return Ok(());
        }
        let base_changed = self.applied_base != Some(ready.base_revision);
        // An entity-only selection/hover/hidden change republishes the scene with
        // the **same** overlay `Arc` (the controller reuses it) and no base
        // change, so there is no geometry to upload — only flags to push. This is
        // the "selection change uploads only flags data" path: it never re-uploads
        // a batch, stream or chunk.
        let overlay_reused =
            overlay_identity_matches(self.applied_highlight.as_ref(), &ready.highlight);
        if !base_changed
            && overlay_reused
            && self.pending_upload.is_none()
            && !self.overlays_suppressed
        {
            let report = self
                .renderer
                .as_mut()
                .expect("ready renderer")
                .set_entity_flags(&ready.entity_flags)?;
            // Retain the applied flags so the 2D navigation fallback can
            // re-assert them after its truncating commit.
            self.applied_entity_flags = ready.entity_flags.clone();
            self.flags_diagnostic = flags_diagnostic(&report.unmatched);
            self.applied_revision = Some(ready.revision);
            self.applied_flags_revision = Some(ready.flags_revision);
            self.applied_highlight = Some(ready.highlight.clone());
            // A flags-only update must not wipe a still-relevant image diagnostic:
            // merge rather than clear.
            self.diagnostic = merge_diagnostics(
                self.image_diagnostic.as_deref(),
                self.flags_diagnostic.as_deref(),
            );
            self.dirty.invalidate();
            return Ok(());
        }
        let renderer = self.renderer.as_mut().expect("ready renderer");
        // Base drawing batches are the prefix; the transient highlight/preview
        // overlay is the replaceable suffix. A highlight-only change still
        // re-uploads the suffix, but never the base.
        if self
            .pending_upload
            .as_ref()
            .is_none_or(|pending| pending.scene.revision != ready.revision)
        {
            match upload_decision(
                true,
                self.pending_upload
                    .as_ref()
                    .map(|pending| pending.scene.revision),
                ready.revision,
            ) {
                UploadDecision::ResetRevision => {
                    self.upload_revision_resets += 1;
                    self.last_upload_reset = Some("revision_replace");
                }
                UploadDecision::Start => self.uploads_started += 1,
                // Unreachable by the guard above; keep the compiler honest.
                UploadDecision::Continue | UploadDecision::ClearPending => {}
            }
            self.pending_upload = Some(PendingUpload {
                scene: ready.clone(),
                group: usize::from(!base_changed),
                offset: 0,
                prepared: None,
                base_changed,
            });
        }
        let pending = self.pending_upload.as_mut().unwrap();
        #[cfg(not(target_arch = "wasm32"))]
        let started = std::time::Instant::now();
        let mut slices = 0;
        let mut prepare_us = 0u64;
        let mut append_us = 0u64;
        while pending.group < 2 && slices < 4 {
            #[cfg(not(target_arch = "wasm32"))]
            if started.elapsed() >= std::time::Duration::from_millis(8) {
                break;
            }
            let batches = match pending.group {
                0 => &pending.scene.base.added,
                _ => &pending.scene.highlight.added,
            };
            if pending.offset == batches.len() {
                pending.group += 1;
                pending.offset = 0;
                continue;
            }
            let mut end = pending.offset;
            let mut vertices = 0usize;
            while end < batches.len() && end - pending.offset < 256 {
                let next = vertices.saturating_add(batches[end].vertices.len());
                if end > pending.offset && next > 262_144 {
                    break;
                }
                vertices = next;
                end += 1;
            }
            let prepare_started = std::time::Instant::now();
            let piece = renderer.prepare_upload_batches(&batches[pending.offset..end])?;
            prepare_us += prepare_started.elapsed().as_micros() as u64;
            let append_started = std::time::Instant::now();
            if let Some(prepared) = &mut pending.prepared {
                prepared.append(piece)?;
            } else {
                pending.prepared = Some(piece);
            }
            append_us += append_started.elapsed().as_micros() as u64;
            pending.offset = end;
            slices += 1;
        }
        if self
            .pending_upload
            .as_ref()
            .is_some_and(|pending| pending.group < 2)
        {
            // Chunked staging only (no commit yet): record the staged progress.
            // Copy the facts out first so the `pending_upload` borrow ends before
            // the whole-`self` timing record.
            let (group, offset, scene_revision, base_changed) = {
                let pending = self.pending_upload.as_ref().expect("checked above");
                (
                    pending.group,
                    pending.offset,
                    pending.scene.revision,
                    pending.base_changed,
                )
            };
            // `started` only exists off wasm (no monotonic clock there); report 0.
            #[cfg(not(target_arch = "wasm32"))]
            let total_us = started.elapsed().as_micros() as u64;
            #[cfg(target_arch = "wasm32")]
            let total_us = 0u64;
            self.record_upload_timing(UploadTimingObservation {
                seq: 0,
                scene_revision,
                base_changed,
                committed: false,
                group,
                batches: offset,
                prepare_us,
                append_us,
                commit_us: 0,
                total_us,
                commit: None,
            });
            return Ok(());
        }
        let pending = self.pending_upload.take().unwrap();
        let prepared = match pending.prepared {
            Some(prepared) => prepared,
            None => renderer.prepare_upload_batches(&[])?,
        };
        let commit_started = std::time::Instant::now();
        renderer.commit_upload(
            prepared,
            if pending.base_changed {
                0
            } else {
                self.base_batches
            },
        )?;
        let commit_us = commit_started.elapsed().as_micros() as u64;
        let commit_timing = renderer.last_commit_timing();
        // A suffix-only commit re-uploads (or truncates) the transient overlay
        // batch range; a base change replaces the whole scene instead.
        if !pending.base_changed {
            self.overlay_suffix_commits += 1;
        }
        // Raster images are a resident set separate from the geometry batches.
        // A base change re-resolves every image quad from the cache the scene
        // was built with; a fresh device resets `applied_base`, so this also
        // re-uploads the textures after a backend rebuild/loss. An absent cache
        // or an empty batch list leaves no resident images and reports nothing:
        // a true no-op. Unresolved keys are surfaced, never silently dropped.
        let mut image_diagnostic = None;
        if pending.base_changed {
            let empty = super::controller::DecodedImageCache::new();
            let cache = ready.images.as_deref().unwrap_or(&empty);
            let report = renderer.upload_images(&ready.base.images, cache)?;
            if let Some(reason) = report.diagnostic() {
                let keys = report
                    .unresolved
                    .iter()
                    .map(|key| key.as_str())
                    .collect::<Vec<_>>()
                    .join(",");
                image_diagnostic = Some(format!("{}: {keys}", reason.code));
            }
        }
        // Wipeout masks are a separate resident set, exactly like images
        // (ADR 0022): a base change re-uploads the whole mask set from the same
        // scene. The renderer's final mask pass paints them last. A boundary it
        // could not fill is observable through `Renderer::mask_skips()`, never
        // silently dropped.
        if pending.base_changed {
            renderer.upload_masks(&ready.base.masks)?;
        }
        // Push the entity-level flags for the freshly committed scene, **after**
        // any base/image re-upload. A base change (re)allocates pick keys —
        // including the image keys, which `upload_images` assigns fresh per call —
        // so flags pushed before the re-upload would stay on retired keys and the
        // flagged image/geometry would flash back to unflagged after an edit. The
        // renderer also re-applies its retained request on the rebuild; this call
        // surfaces the unmatched refs so the runtime can report them.
        let flag_report = renderer.set_entity_flags(&ready.entity_flags)?;
        // Retain the applied flags so the 2D navigation fallback can re-assert
        // them after its truncating commit.
        self.applied_entity_flags = ready.entity_flags.clone();
        // Publication follows successful GPU preparation/commit, never precedes it.
        self.base_batches = ready.base.added.len();
        self.applied_base = Some(ready.base_revision);
        self.applied_revision = Some(ready.revision);
        self.applied_flags_revision = Some(ready.flags_revision);
        self.applied_highlight = Some(ready.highlight.clone());
        // A committed full scene restores the transient overlay suffix, so the
        // base-only fallback is no longer active.
        self.overlays_suppressed = false;
        self.dirty.invalidate();
        self.image_diagnostic = image_diagnostic;
        self.flags_diagnostic = flags_diagnostic(&flag_report.unmatched);
        self.diagnostic = merge_diagnostics(
            self.image_diagnostic.as_deref(),
            self.flags_diagnostic.as_deref(),
        );
        // `started` only exists off wasm (no monotonic clock there); report 0.
        #[cfg(not(target_arch = "wasm32"))]
        let total_us = started.elapsed().as_micros() as u64;
        #[cfg(target_arch = "wasm32")]
        let total_us = 0u64;
        // Record the committed upload's stage timings **after** the last `renderer`
        // borrow (diagnostics only; taken at most once so a host never replays them).
        self.record_upload_timing(UploadTimingObservation {
            seq: 0,
            scene_revision: pending.scene.revision,
            base_changed: pending.base_changed,
            committed: true,
            group: 2,
            batches: pending.scene.base.added.len() + pending.scene.highlight.added.len(),
            prepare_us,
            append_us,
            commit_us,
            total_us,
            commit: commit_timing,
        });
        Ok(())
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

    /// Whether the 2D navigation fallback is currently withholding overlays.
    pub fn overlays_suppressed(&self) -> bool {
        self.overlays_suppressed
    }

    /// Cumulative overlay-suffix commits (suffix-only uploads and fallback
    /// truncations). Camera-only navigation must leave this flat.
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

    /// Record the observation of one actual renderer invocation.
    fn record_render_observation(
        &mut self,
        stats: cad_render_wgpu::FrameStats,
        view_revision: u64,
        progressive: Option<(usize, usize)>,
        progressive_complete: bool,
    ) {
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
    pub(super) fn record_upload_timing(&mut self, mut observation: UploadTimingObservation) {
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
            overlays_suppressed: self.overlays_suppressed,
            overlay_suffix_commits: self.overlay_suffix_commits,
            pulled_prefix_reuses: self
                .renderer
                .as_ref()
                .map_or(0, |renderer| renderer.pulled_prefix_reuses()),
            pulled_prefix_rebuilds: self
                .renderer
                .as_ref()
                .map_or(0, |renderer| renderer.pulled_prefix_rebuilds()),
            frames_rendered: self.frames_rendered,
            uploads_started: self.uploads_started,
            upload_revision_resets: self.upload_revision_resets,
            ready_none_clears: self.ready_none_clears,
            last_upload_reset: self.last_upload_reset,
            diagnostic: self.diagnostic.clone(),
        }
    }

    /// The latest published base is fully resident on the GPU: the device is
    /// ready, the applied base revision matches `base_publication_revision`, and
    /// no pending upload is replacing the base. This is the narrow qualifier that
    /// lets a base-only frame stand in for the full scene during 2D navigation
    /// without ever re-uploading base geometry.
    pub fn base_fully_resident(&self, base_publication_revision: u64) -> bool {
        matches!(self.lifecycle, RenderLifecycle::Ready { .. })
            && self.applied_base == Some(base_publication_revision)
            && self
                .pending_upload
                .as_ref()
                .is_none_or(|pending| !pending.base_changed)
    }

    /// Drop the stale transient overlay suffix while keeping the base drawing
    /// buffers. Runs at most once per fallback entry; a later full-scene commit
    /// restores the suffix from the same base prefix (`base_batches`).
    ///
    /// `prepare_upload_batches(&[])` stages zero bytes, so this re-upload of the
    /// base is impossible by construction: only the replaceable suffix after
    /// `base_batches` is truncated.
    ///
    /// The commit can transiently leave the renderer with an **empty** resident
    /// set (a zero-batch base), which restarts its pick key space. The last
    /// applied entity flags are therefore re-pushed afterwards, exactly as
    /// [`Self::apply_ready_updates`] does on a commit: a camera-only fallback is
    /// not a scene mutation, so the selection tint must survive it, and an
    /// unmatched ref is reported rather than silently dropped. Re-pushing the
    /// unchanged set writes no flags row (the renderer diffs), so a normal
    /// fallback entry costs no GPU upload.
    fn suppress_overlays(&mut self) -> Result<(), RenderError> {
        // Cloned before the renderer borrow (it is the flags the runtime last
        // pushed; the controller remains the sole producer).
        let flags = self.applied_entity_flags.clone();
        let renderer = self
            .renderer
            .as_mut()
            .ok_or_else(|| RenderError::NotInitialized("renderer not initialized".into()))?;
        let empty = renderer
            .prepare_upload_batches(&[])
            .map_err(|error| RenderError::Frame(error.to_string()))?;
        renderer
            .commit_upload(empty, self.base_batches)
            .map_err(|error| RenderError::Frame(error.to_string()))?;
        // Record the truncation-derived state **before** the flags re-push, so a
        // flags failure still leaves the runtime consistent with the dropped
        // suffix (a later full commit restores both).
        self.overlays_suppressed = true;
        self.overlay_suffix_commits += 1;
        // The discarded suffix upload (if any) is overlay-only by the
        // `base_fully_resident` qualifier; the full scene re-commits it.
        self.pending_upload = None;
        // The presented frame is no longer the full current scene.
        self.applied_revision = None;
        self.presented_revision = None;
        let flag_report = renderer
            .set_entity_flags(&flags)
            .map_err(|error| RenderError::Frame(error.to_string()))?;
        self.flags_diagnostic = flags_diagnostic(&flag_report.unmatched);
        self.diagnostic = merge_diagnostics(
            self.image_diagnostic.as_deref(),
            self.flags_diagnostic.as_deref(),
        );
        self.dirty.invalidate();
        Ok(())
    }

    pub fn render_if_dirty(
        &mut self,
        view: &ViewSnapshot,
        logical: [f64; 2],
        scale: f64,
        view_revision: u64,
    ) -> Result<Option<PresentedFrame>, RenderError> {
        let completeness = if self.overlays_suppressed {
            FrameCompleteness::BaseOnlyPendingOverlays
        } else {
            FrameCompleteness::CurrentScene
        };
        self.render_frame(view, logical, scale, view_revision, completeness)
    }

    /// Render the base-only navigation frame over the latest authoritative
    /// camera. The caller has already verified that the published base is fully
    /// resident and that only view/overlay inputs are pending.
    pub fn render_base_only(
        &mut self,
        view: &ViewSnapshot,
        logical: [f64; 2],
        scale: f64,
        view_revision: u64,
    ) -> Result<Option<PresentedFrame>, RenderError> {
        if !matches!(self.lifecycle, RenderLifecycle::Ready { .. }) {
            return Ok(None);
        }
        if !self.overlays_suppressed {
            self.suppress_overlays()?;
        }
        self.render_frame(
            view,
            logical,
            scale,
            view_revision,
            FrameCompleteness::BaseOnlyPendingOverlays,
        )
    }

    fn render_frame(
        &mut self,
        view: &ViewSnapshot,
        logical: [f64; 2],
        scale: f64,
        view_revision: u64,
        completeness: FrameCompleteness,
    ) -> Result<Option<PresentedFrame>, RenderError> {
        if !matches!(self.lifecycle, RenderLifecycle::Ready { .. }) {
            return Ok(None);
        }
        if logical.iter().any(|v| !v.is_finite() || *v <= 0.0) || !scale.is_finite() || scale <= 0.0
        {
            return Err(RenderError::Frame("invalid CAD surface dimensions".into()));
        }
        let size = (
            (logical[0] * scale).round().max(1.0) as u32,
            (logical[1] * scale).round().max(1.0) as u32,
        );
        if self.target_size != Some(size) {
            self.dirty.invalidate();
        }
        let renderer = self.renderer.as_mut().expect("ready renderer");
        // An attachment can change at the same size. Its identity is inspected
        // for presentation even when CAD pixels do not need redrawing.
        if self.dirty.dirty() {
            let target = RenderTarget::new(size.0, size.1);
            let stats = match view.mode {
                ProjectionKind::TwoD => {
                    let params = view
                        .camera
                        .camera2d_params()
                        .map_err(|e| RenderError::Frame(e.to_string()))?;
                    let mut camera = camera2d_from_params(params);
                    camera.world_per_px /= scale;
                    // A changed camera makes `render` rebuild the progressive
                    // frame (transforms/camera differ), so no earlier page
                    // continues under the new camera.
                    renderer.render(camera, &target)?
                }
                ProjectionKind::ThreeD => {
                    let params = view
                        .camera
                        .camera3d_params()
                        .map_err(|e| RenderError::Frame(e.to_string()))?;
                    renderer.render_3d(camera3d_from_params(params), &target)?
                }
            };
            self.target_size = Some(size);
            let progressive_complete = !renderer.frame_pending();
            let progressive = renderer.frame_progress();
            if progressive_complete {
                self.dirty.complete();
            }
            self.frames_rendered += 1;
            self.presented_revision = if progressive_complete {
                self.applied_revision
            } else {
                None
            };
            // Capture the attachment identity/texture, ending the `renderer` field
            // borrow, then record the **actual** renderer invocation (the stats
            // that would otherwise be dropped). Retained/reused frames never reach
            // here.
            let texture_revision = renderer.texture_revision();
            let texture = renderer.frame_texture().cloned();
            self.record_render_observation(stats, view_revision, progressive, progressive_complete);
            return Ok(texture.map(|texture| PresentedFrame {
                binding: FrameBinding {
                    device_epoch: self.epoch,
                    texture_revision,
                    size,
                },
                view_revision,
                completeness,
                texture,
            }));
        }
        Ok(renderer.frame_texture().map(|texture| PresentedFrame {
            binding: FrameBinding {
                device_epoch: self.epoch,
                texture_revision: renderer.texture_revision(),
                size,
            },
            view_revision,
            completeness,
            texture: texture.clone(),
        }))
    }
}
