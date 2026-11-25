//! Chunked scene upload admission, staging, commit and publication.
//!
//! Split out of `session.rs`; behaviour is unchanged.

use super::super::staging::stage_piece;
use super::super::*;
use super::Instant;

fn missing_cpu_payload_error() -> CadError {
    CadError::Invariant(super::super::MISSING_CPU_PAYLOAD.into())
}

impl CadRenderRuntime {
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
        // A full base stage needs the CPU geometry. Check **before** any flags or
        // in-place overwrite is applied: the session rebuilds the base from the
        // live inputs and retries once, so no renderer mutation can be stranded
        // by the retry. An admitted append needs only its fresh tail and is not
        // covered here; `materialized` is the safety net for that range.
        if self.base_stage_needs_full_payload(ready) && !ready.base.has_cpu_payload() {
            return Err(CadError::Invariant(
                super::super::MISSING_CPU_PAYLOAD.into(),
            ));
        }
        // Flags-first delta over the committed geometry. A flags-only preview
        // publication keeps the scene revision, so this must be evaluated
        // BEFORE the `applied_revision == Some(ready.revision)` early return:
        // otherwise the new flags would be silently skipped. Only the entity
        // flags are pushed — no batch, stream or chunk is re-uploaded and the
        // overlay suffix stays resident.
        if flags_delta_applies(
            self.applied_revision,
            self.applied_flags_revision,
            self.pending_upload.is_some(),
            ready,
        ) {
            let report = self
                .renderer
                .as_mut()
                .expect("ready renderer")
                .set_entity_flags(&ready.entity_flags)?;
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
        // P4: a single-entity display update republishes the same base revision
        // with one entity's rebuilt batches. The controller does not own the
        // renderer, so it publishes the replacements as data; apply them here, in
        // place, before the ordinary upload decision. Every batch is validated
        // first, so a multi-batch entity update is all-or-nothing, and a refusal
        // re-stages the whole base instead of keeping the old geometry.
        let mut applied_overwrites = 0usize;
        let mut restage = false;
        match overwrite_admission(
            !ready.overwrites.is_empty(),
            base_changed,
            self.pending_upload.is_some(),
            self.applied_revision,
        ) {
            OverwriteAdmission::NotApplicable => {}
            OverwriteAdmission::FullRestage => {
                self.batch_overwrite_fallbacks += 1;
                self.overwrite_diagnostic = Some(
                    "render.batch_overwrite_refused: no resident base to overwrite in place \
                     (base change, in-flight upload or no applied scene); re-staging the base"
                        .into(),
                );
                restage = true;
            }
            OverwriteAdmission::Apply => {
                let renderer = self.renderer.as_mut().expect("ready renderer");
                let refusal = ready.overwrites.iter().find_map(|(index, batch)| {
                    renderer
                        .check_overwrite_batch(*index, batch)
                        .err()
                        .map(|refusal| (*index, refusal))
                });
                match refusal {
                    Some((index, refusal)) => {
                        self.batch_overwrite_fallbacks += 1;
                        self.overwrite_diagnostic = Some(format!(
                            "{} ({}): batch {index}: {refusal}",
                            refusal.diagnostic_code(),
                            refusal.reason()
                        ));
                        restage = true;
                    }
                    None => {
                        for (index, batch) in &ready.overwrites {
                            renderer.overwrite_batch(*index, batch).expect(
                                "the overwrite gate was validated immediately above and no state \
                                 changed in between",
                            );
                        }
                        applied_overwrites = ready.overwrites.len();
                        self.batch_overwrites += applied_overwrites as u64;
                        self.overwrite_diagnostic = None;
                    }
                }
            }
        }
        let base_changed = base_changed || restage;
        if restage {
            // A re-stage must not be satisfied by an in-flight revision that is
            // still the *same* publication (it may be a suffix-only commit that
            // would keep the base). Reset it so a full base upload starts.
            if self.pending_upload.take().is_some() {
                self.upload_revision_resets += 1;
                self.last_upload_reset = Some("overwrite_restage");
            }
        }
        // An entity-only selection/hover/hidden change republishes the scene with
        // the **same** overlay `Arc` (the controller reuses it) and no base
        // change, so there is no geometry to upload — only flags to push. This is
        // the "selection change uploads only flags data" path: it never re-uploads
        // a batch, stream or chunk.
        let overlay_reused =
            overlay_identity_matches(self.applied_highlight.as_ref(), &ready.highlight);
        if !base_changed && overlay_reused && self.pending_upload.is_none() {
            let report = self
                .renderer
                .as_mut()
                .expect("ready renderer")
                .set_entity_flags(&ready.entity_flags)?;
            self.flags_diagnostic = flags_diagnostic(&report.unmatched);
            if applied_overwrites > 0 {
                // The in-place overwrite replaced one per-batch `Arc` in the exact
                // resident base; refresh the retained identity so a later append's
                // predecessor proof compares against the scene the GPU now holds.
                self.applied_base_scene = Some(ready.base.clone());
            }
            self.applied_revision = Some(ready.revision);
            self.applied_flags_revision = Some(ready.flags_revision);
            self.applied_highlight = Some(ready.highlight.clone());
            // A flags-only update must not wipe a still-relevant image diagnostic:
            // merge rather than clear.
            self.diagnostic = merge_diagnostics(
                self.image_diagnostic.as_deref(),
                self.flags_diagnostic.as_deref(),
            );
            self.merge_overwrite_diagnostic();
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
            let plan = if base_changed {
                admit_base_append(
                    matches!(self.lifecycle, RenderLifecycle::Ready { .. }),
                    self.applied_base,
                    ready.base_revision,
                    self.applied_base_scene.as_ref(),
                    ready.base_append.as_ref(),
                    self.base_batches,
                    ready.base.batch_len(),
                )
                .unwrap_or(BaseUploadPlan::Full)
            } else {
                BaseUploadPlan::Full
            };
            self.pending_upload = Some(PendingUpload {
                scene: ready.clone(),
                group: usize::from(!base_changed),
                offset: if base_changed { plan.keep_prefix() } else { 0 },
                prepared: None,
                base_changed,
                append: base_changed && plan.is_append(),
                base_offset: if base_changed { plan.keep_prefix() } else { 0 },
            });
        }
        let pending = self.pending_upload.as_mut().unwrap();
        #[cfg(not(target_arch = "wasm32"))]
        let started = Instant::now();
        let mut slices = 0;
        let mut prepare_us = 0u64;
        let mut append_us = 0u64;
        while pending.group < 2 && slices < 4 {
            #[cfg(not(target_arch = "wasm32"))]
            if started.elapsed() >= std::time::Duration::from_millis(8) {
                break;
            }
            let batches_len = match pending.group {
                0 => pending.scene.base.batch_len(),
                _ => pending.scene.highlight.added.len(),
            };
            if pending.offset == batches_len {
                pending.group += 1;
                pending.offset = 0;
                continue;
            }
            // One bounded staging slice: at most 256 batches and (after the
            // first batch) at most 262,144 vertices, counted over the **whole**
            // range regardless of which resident segment holds each batch.
            let start = pending.offset;
            let mut end = start;
            let mut vertices = 0usize;
            let mut advance = |count: usize| -> bool {
                if end - start >= 256 {
                    return false;
                }
                let next = vertices.saturating_add(count);
                if end > start && next > 262_144 {
                    return false;
                }
                vertices = next;
                end += 1;
                true
            };
            match pending.group {
                0 => {
                    // Counts come from the payload-free `vertex_count`, so a
                    // released CPU base can still be sliced (and then rejected
                    // explicitly by `materialized` if this is a full re-stage).
                    for index in start..batches_len {
                        let count = pending.scene.base.vertex_count(index).unwrap_or(0);
                        if !advance(count) {
                            break;
                        }
                    }
                }
                _ => {
                    for batch in pending.scene.highlight.added.iter().skip(start) {
                        if !advance(batch.vertices.len()) {
                            break;
                        }
                    }
                }
            }
            let prepare_started = Instant::now();
            // Group 0 is the persistent base: the database scene (entities,
            // raster images, wipeout masks). Group 1 is the transient overlay
            // (reference grid/axes, POINT markers, sub-element highlight, grips,
            // measurement/draw previews, box selection, snap hints). The pointer
            // cursor is a renderer transient (ADR 0028) and never enters either
            // group.
            // Group 0 converts to the stream/chunk/arc classes; group 1 is staged
            // **legacy** so the overlay pass can draw it with per-batch uniforms
            // only, without base stream/chunk/arc slots.
            //
            // The base is staged one bounded materialised piece at a time. The
            // piece is bounded (256 batches / 262,144 vertices), so cloning its
            // batches out of the resident per-batch `Arc`s is O(piece); a full
            // build and every accepted append yield one piece per staging slice.
            let mut piece = None;
            match pending.group {
                0 => {
                    let batches = pending
                        .scene
                        .base
                        .materialized(start..end)
                        .map_err(|_| missing_cpu_payload_error())?;
                    stage_piece(0, renderer, &batches, &mut piece)?;
                }
                _ => stage_piece(
                    1,
                    renderer,
                    &pending.scene.highlight.added[start..end],
                    &mut piece,
                )?,
            }
            let piece = piece.expect("a non-empty staging range yields a piece");
            prepare_us += prepare_started.elapsed().as_micros() as u64;
            let append_started = Instant::now();
            if let Some(prepared) = &mut pending.prepared {
                prepared.append(piece)?;
            } else {
                pending.prepared = Some(piece);
            }
            append_us += append_started.elapsed().as_micros() as u64;
            pending.offset = end;
            slices += 1;
        }
        // wgpu holds every `write_buffer` in its pending-writes staging until a
        // submission. A full-scene upload is staged across many ticks before its
        // single commit, so flush the bounded slices staged in this pass: the
        // host peak stays O(slices), not O(whole scene). The empty submit
        // performs no GPU work (see `Renderer::flush_upload_writes`).
        if slices > 0 {
            renderer.flush_upload_writes();
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
            // The per-pass budget is native-only (`started` is cfg-gated),
            // so this reports an unmeasured 0 on wasm rather than a fabricated
            // duration.
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
        let commit_started = Instant::now();
        // Base/overlay split: `[0, base_end)` is the persistent base, the rest is
        // the transient overlay. On a base change the base is exactly the staged
        // base group; on a suffix-only commit the resident base prefix is
        // unchanged and the newly staged pieces are all overlay.
        let (keep_prefix, base_end) = if pending.base_changed {
            if pending.append {
                // Admitted append: keep the exact resident prefix and stage only
                // the appended base suffix. `base_end` is the complete new base,
                // so the renderer still classifies this as a base change.
                (pending.base_offset, pending.scene.base.batch_len())
            } else {
                (0, pending.scene.base.batch_len())
            }
        } else {
            (self.base_batches, self.base_batches)
        };
        renderer.commit_upload_with_base(prepared, keep_prefix, base_end)?;
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
        let mut image_diagnostic = if pending.append {
            // An admitted append does not change the image/mask arrays: retain
            // the resident textures, masks and their earlier diagnostics (a
            // previous unresolved key must not become a silent success).
            self.image_diagnostic.clone()
        } else {
            None
        };
        if pending.base_changed && !pending.append {
            let empty = super::super::controller::DecodedImageCache::new();
            let cache = ready.images.as_deref().unwrap_or(&empty);
            let report = renderer.upload_images(ready.base.images(), cache)?;
            // Mesh base-colour textures (ADR-0038 D10) are a separate residency
            // set resolved from the **same** decoded-image cache: the scene reports
            // the keys its batches reference, and the renderer uploads exactly
            // those. A failure here must be as observable as a failed image, so
            // both reports fold into the one `image_diagnostic` channel. The code
            // is the shared `image.unresolved` marker; the keys themselves
            // (`<hex>.png|.jpg`) identify the mesh textures, and a textured batch
            // whose key failed is additionally drawn untextured (never with a
            // placeholder) and surfaced by `Renderer::mesh_texture_fallbacks`.
            let mesh_metadata = ready.base.metadata();
            let mesh_report = renderer.upload_mesh_textures(
                &cad_scene::mesh_texture_keys(mesh_metadata.iter().map(|batch| batch.as_ref())),
                cache,
            )?;
            let mut unresolved = report.unresolved.clone();
            unresolved.extend(mesh_report.unresolved.iter().cloned());
            if let Some(reason) = report.diagnostic().or_else(|| mesh_report.diagnostic()) {
                let keys = unresolved
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
        if pending.base_changed && !pending.append {
            renderer.upload_masks(ready.base.masks())?;
        }
        // Push the entity-level flags for the freshly committed scene, **after**
        // any base/image re-upload. A base change (re)allocates pick keys —
        // including the image keys, which `upload_images` assigns fresh per call —
        // so flags pushed before the re-upload would stay on retired keys and the
        // flagged image/geometry would flash back to unflagged after an edit. The
        // renderer also re-applies its retained request on the rebuild; this call
        // surfaces the unmatched refs so the runtime can report them.
        let flag_report = renderer.set_entity_flags(&ready.entity_flags)?;
        // Publication follows successful GPU preparation/commit, never precedes it.
        self.base_batches = ready.base.batch_len();
        self.applied_base = Some(ready.base_revision);
        self.applied_base_scene = Some(ready.base.clone());
        self.applied_revision = Some(ready.revision);
        self.applied_flags_revision = Some(ready.flags_revision);
        self.applied_highlight = Some(ready.highlight.clone());
        self.dirty.invalidate();
        self.image_diagnostic = image_diagnostic;
        self.flags_diagnostic = flags_diagnostic(&flag_report.unmatched);
        self.diagnostic = merge_diagnostics(
            self.image_diagnostic.as_deref(),
            self.flags_diagnostic.as_deref(),
        );
        self.merge_overwrite_diagnostic();
        // The per-pass budget is native-only (`started` is cfg-gated), so this
        // reports an unmeasured 0 on wasm rather than a fabricated duration.
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
            batches: pending.scene.base.batch_len() + pending.scene.highlight.added.len(),
            prepare_us,
            append_us,
            commit_us,
            total_us,
            commit: commit_timing,
        });
        Ok(())
    }

    /// Merge the last overwrite refusal / fallback reason into the published
    /// diagnostic, so a fallback is never silent: the host sees the stable code
    /// and the named reason even though the fallback itself drew the new
    /// geometry.
    fn merge_overwrite_diagnostic(&mut self) {
        let Some(overwrite) = self.overwrite_diagnostic.clone() else {
            return;
        };
        self.diagnostic = Some(match self.diagnostic.take() {
            Some(existing) => format!("{existing}; {overwrite}"),
            None => overwrite,
        });
    }
}
