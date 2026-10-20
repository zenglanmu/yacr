use super::*;

impl Renderer {
    /// Point every resident instance stream at its own real `VERTEX | STORAGE`
    /// buffer (64-byte pulled layout) with `base = 0`.
    ///
    /// Used when the whole-scene pulled line class cannot be built (an oversized
    /// stream, a chunk table past the binding limit, or a failed allocation). The
    /// universal per-run path and the id pass both read `stream.buffer` at
    /// `base + run.offset`, so without this they would draw the prepare-time
    /// placeholder and silently lose the line geometry (or trip validation).
    ///
    /// Fallible: each buffer is allocated through the guarded path, so a device
    /// that cannot satisfy even the per-stream geometry reports an error to the
    /// caller (which turns it into an explicit state) instead of aborting the
    /// process. Streams keep their previous buffer on failure.
    pub(super) fn build_line_fallback(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<(), PulledAllocationError> {
        for stream in &mut self.instance_streams {
            let records = build_pulled_lines(&[stream.pulled_instances.as_slice()], 0);
            let bytes = pulled_line_bytes(&records);
            stream.buffer = Self::try_vertex_storage_buffer(device, Some(queue), &bytes)?;
            stream.base = 0;
        }
        Ok(())
    }

    /// Rebuild the cached "lineweight carried but not drawn" report from the
    /// resident batch set.
    ///
    /// Called only when `batches` changes (upload/clear/device loss), so the
    /// frame path clones a small Vec instead of scanning every batch.
    pub(super) fn rebuild_lineweight_cache(&mut self) {
        self.lineweight_not_drawn.clear();
        self.lineweight_not_drawn.extend(
            self.batches
                .iter()
                .filter(|batch| batch.lineweight > 0.0)
                .map(|batch| LineweightNotDrawn {
                    millimeters: batch.lineweight,
                }),
        );
    }

    pub fn clear_batches(&mut self) {
        self.progressive_frame = None;
        self.draw_plan = None;
        self.reset_pacing();
        self.batches.clear();
        self.instance_streams.clear();
        self.arc_streams.clear();
        self.mesh_chunks.clear();
        self.pulled_lines = None;
        self.pulled_mesh = None;
        self.invalidate_pulled_prefix();
        self.pulled_state = if self.render_tier == RenderTier::Accelerated {
            PulledState::SkippedNoConvertedGeometry
        } else {
            PulledState::Unavailable
        };
        // Resident image quads own pick keys too, and `reset_pick_keys` restarts
        // the key space at 1. Keeping the images would let a re-uploaded batch
        // reuse an image's key, which the pick table could only resolve by
        // silently preferring one owner. Drop them with the batches; the host
        // re-uploads its images after a reset (the texture cache is left to be
        // pruned on the next `upload_images`).
        self.images.clear();
        self.masks.clear();
        self.mask_skips.clear();
        self.rebuild_lineweight_cache();
        // An explicit full teardown is a document switch/close, not a transient
        // empty resident set: the retained entity-flag request belongs to the
        // retired scene and must not survive into the next document.
        self.reset_pick_keys_and_drop_request();
    }

    /// Rebuild the CPU-side pick table from the resident batches, and reset the
    /// key allocator when nothing is resident.
    ///
    /// Called whenever `batches`/`images` change (commit/clear/device loss). An
    /// empty resident set restarts the key space so a re-opened drawing does not
    /// inherit a growing key counter — but it does **not** drop the retained
    /// entity-flag request, because an empty resident set can be a transient
    /// commit state rather than a document switch (see [`Self::reset_pick_keys`]).
    ///
    /// When `incremental` is `Some((keep_prefix, retired_keys))` the update
    /// **replaces the resident suffix**: the kept prefix's entries are unchanged
    /// and only the new suffix entries are appended (keys are never renumbered or
    /// reused within a device generation, so the kept prefix's `key → entry`
    /// mapping cannot change). The incremental path is taken only when it is
    /// provably equivalent to a full rebuild — see
    /// [`Self::pick_table_covers_prefix`] — otherwise a full rebuild runs and
    /// `pick_table_full_rebuilds` is bumped.
    ///
    /// A repeated key is a **hard error**, not "first wins": keys are allocated
    /// monotonically within a device generation, so a duplicate can only come
    /// from the allocator being restarted while keys are still resident (a
    /// lifecycle hole). Silently keeping one owner would resolve a fragment to
    /// the wrong entity.
    pub(super) fn rebuild_pick_table(
        &mut self,
        incremental: Option<(usize, Vec<u32>)>,
    ) -> CadResult<()> {
        if self.batches.is_empty() && self.images.is_empty() {
            // A **transient** empty resident set (a commit between scenes, e.g.
            // the 2D navigation fallback truncating a zero-batch base). Only the
            // key space restarts; the retained entity-flag request is kept so a
            // live selection survives a camera-only operation and re-resolves
            // when the scene returns. See [`Self::reset_pick_keys`].
            self.reset_pick_keys();
            return Ok(());
        }
        // The identity index is only needed by the entity-flag path; skip
        // building it when no request is retained (a later request builds it
        // lazily in `apply_entity_flags`).
        let want_index = !self.entity_flags_request.is_empty();
        let mut used_incremental = false;
        if let Some((keep_prefix, retired_keys)) = incremental.as_ref() {
            // A true prefix must be kept (`keep_prefix > 0`): a base change
            // (`keep_prefix == 0`) retires every key and is a full rebuild.
            //
            // `self.images.is_empty()` keeps the combined batch/image index of
            // every existing entry stable: a resident image entry is indexed as
            // `batches.len() + i`, so appending batches would renumber it. With no
            // images resident, the appended batches take `keep_prefix + i`, exactly
            // what a full rebuild would assign.
            if *keep_prefix > 0
                && self.images.is_empty()
                && self.pick_table_covers_prefix(*keep_prefix)
            {
                let added: Vec<(u32, Vec<SelectionRef>)> = self.batches[*keep_prefix..]
                    .iter()
                    .map(|batch| (batch.pick_key, batch.sources.clone()))
                    .collect();
                self.pick_table
                    .replace_suffix(*keep_prefix as u32, retired_keys, added)
                    .map_err(duplicate_pick_key_error)?;
                used_incremental = true;
            }
        }
        if used_incremental {
            self.pick_table_incremental_extends += 1;
        } else {
            self.pick_table_full_rebuilds += 1;
            let entries: Vec<(u32, Vec<SelectionRef>)> = self
                .batches
                .iter()
                .map(|batch| (batch.pick_key, batch.sources.clone()))
                .chain(
                    self.images
                        .iter()
                        .map(|image| (image.pick_key, image.sources.clone())),
                )
                .collect();
            self.pick_table =
                PickTable::build(entries, want_index).map_err(duplicate_pick_key_error)?;
        }
        // The flags table is indexed by key, so it must cover the live key space.
        // A key is never reused, so `max_key` only grows: this extends the vector
        // rather than resizing it from scratch on the incremental path.
        self.flags_bits
            .resize(self.pick_table.max_key() as usize + 1, 0);
        // A rebuild or suffix replacement retires the dropped keys, so the
        // retained entity-flag request must be re-resolved against the new table:
        // otherwise the bits stay on the retired keys and a flagged image would
        // flash back to its unflagged colour after an edit.
        self.apply_entity_flags()?;
        Ok(())
    }

    /// Whether the pick table still covers the resident batch prefix `prefix`
    /// (the batches `0..prefix`), so an incremental suffix replacement is valid.
    ///
    /// Entries are only ever appended in ascending batch index and a kept prefix's
    /// indices never move, so it suffices to check that the last kept batch's key
    /// is present at combined index `prefix - 1` (O(1)). A table that does not
    /// cover the prefix (a lifecycle hole) falls back to a full rebuild.
    pub(super) fn pick_table_covers_prefix(&self, prefix: usize) -> bool {
        if prefix == 0 {
            return true;
        }
        match self.batches.get(prefix - 1) {
            Some(batch) => {
                self.pick_table.batch_index_of_key(batch.pick_key) == Some(prefix as u32 - 1)
            }
            None => false,
        }
    }

    /// Restart the pick key space for a scene that is no longer resident.
    ///
    /// Called whenever the resident set changes shape (commit/clear/device
    /// loss) and, crucially, on the **transient** empty resident set
    /// [`Self::rebuild_pick_table`] sees while a commit is between scenes (for
    /// example the 2D navigation fallback truncating a zero-batch base).
    ///
    /// The retained [`Self::entity_flags_request`] is deliberately **not**
    /// cleared here: a camera-only operation is not a scene mutation, so a live
    /// selection must survive a transient empty resident set (its keys are
    /// retired, its identity request is not). The request is dropped only where
    /// the key space truly restarts — a new device generation or an explicit
    /// full teardown — via [`Self::reset_pick_keys_and_drop_request`].
    pub(super) fn reset_pick_keys(&mut self) {
        self.pick_key_counter.store(1, Ordering::Relaxed);
        self.pick_table = PickTable::default();
        // A retired scene's keys must not keep their bits: the whole table is
        // re-uploaded (zeroed) before it is used again. The entity-flag diff base
        // is dropped with the scene, so a later update cannot try to clear a key
        // that belonged to the retired scene.
        self.flags_bits.clear();
        self.flags_table_dirty = true;
        self.entity_flag_keys.clear();
    }

    /// The **full** reset: restart the key space **and** drop the retained
    /// entity-flag request.
    ///
    /// Only a new device generation or an explicit full teardown
    /// ([`Self::clear_batches`]) may retire the request: the request names refs
    /// of the retired scene/document, so keeping it could tint an unrelated
    /// entity that reuses an identity in the next document. A transient empty
    /// resident set uses [`Self::reset_pick_keys`] instead and keeps it.
    pub(super) fn reset_pick_keys_and_drop_request(&mut self) {
        self.reset_pick_keys();
        self.entity_flags_request.clear();
    }
}

/// Map a [`DuplicatePickKey`] to the renderer's invalid-input diagnostic. A
/// duplicate key is a lifecycle error (the allocator restarted while keys were
/// resident), never something to paper over: keeping one owner would resolve a
/// fragment to the wrong entity.
pub(super) fn duplicate_pick_key_error(duplicate: DuplicatePickKey) -> CadError {
    CadError::InvalidInput(format!(
        "duplicate renderer pick key {} (owners {} and {}): the key space was \
         restarted while keys were still resident",
        duplicate.key, duplicate.first, duplicate.second
    ))
}
