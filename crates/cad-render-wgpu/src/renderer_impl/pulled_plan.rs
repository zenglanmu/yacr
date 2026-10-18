use super::*;

impl Renderer {
    /// Rebuild the accelerated tier's whole-scene pulled buffers from the
    /// resident streams/chunks, or clear them on the universal tier.
    ///
    /// The pulled buffers are rebuilt on **every** commit (not per frame): a
    /// camera move rewrites only the small per-chunk translation table, exactly
    /// like the universal path's per-stream uniform slot, and never re-uploads
    /// the static geometry storage. A commit that keeps a prefix (for example an
    /// annotation append) **does** rebuild and re-upload the whole pulled scene:
    /// the rebuild is `O(total resident geometry)` per commit, not `O(appended)`.
    /// That cost is stated here rather than hidden; an incremental segment update
    /// is a deferred optimisation (see ADR-0007).
    ///
    /// A real scene's whole-scene storage can exceed
    /// `max_storage_buffer_binding_size`, so each class is split into
    /// binding-sized **segments** (one draw per segment — still `O(1)` in the
    /// chunk count, not one per chunk). A class that cannot be segmented — a
    /// single line stream or mesh chunk larger than the binding limit — is left
    /// unbuilt; the frame then reports `accelerated == false` and the class
    /// renders on the universal per-run path rather than being silently dropped.
    /// The universal fallback must then have **real** geometry: the line class's
    /// per-stream buffers are (re)built here so it never draws the prepare-time
    /// placeholder, and the merged mesh class always keeps its own per-chunk
    /// buffers.
    ///
    /// Byte accounting: the pulled geometry bytes are charged per batch at
    /// `prepare_upload_batches` (line 64 B/instance, mesh 64 B/vertex + 4 B/index);
    /// only the small per-class chunk tables are new here, so the lifetime
    /// `uploaded_bytes` counter does not double-count the geometry. (One bounded
    /// exception: an oversized mesh chunk that is left unbuilt was still charged
    /// at prepare, so the counter can over-report that class by its pulled share.)
    ///
    /// Allocation failure is a **verdict**, never a panic: every allocation in
    /// the build is guarded (`allocation_guard`), the attempt aborts at the first
    /// failure before anything uses that handle, and the partial state is dropped
    /// in favour of the universal fallback. The result is
    /// [`PulledState::SkippedOutOfMemory`] when the device reported an
    /// out-of-memory error and [`PulledState::SkippedAllocationFailed`] when it
    /// reported anything else (on this backend a refused allocation surfaces as a
    /// *validation* error on the invalid handle, so that is the common case and is
    /// deliberately not claimed as provable OOM).
    ///
    /// The retained-prefix reuse plan for a commit (renderer-owned proof).
    ///
    /// Never trusts the caller's `base_changed`/`keep_prefix` alone: every
    /// precondition is re-verified against the renderer's own state, and anything
    /// unexpected falls back to a full rebuild (never a skip of cached data).
    pub(super) fn pulled_fastpath_plan(
        &self,
        keep_prefix: usize,
        incoming: &IncomingResources,
    ) -> PulledFastPath {
        let establish = (keep_prefix > 0).then_some(keep_prefix);
        let Some(checkpoint) = self.pulled_prefix_checkpoint.as_ref() else {
            return PulledFastPath::Full { establish };
        };
        let facts = CheckpointFacts {
            device_generation: self.device_generation,
            cache_epoch: self.pulled_cache_epoch,
            accelerated: self.render_tier == RenderTier::Accelerated,
            instance_streams: self.instance_streams.len(),
            mesh_chunks: self.mesh_chunks.len(),
            arc_streams: self.arc_streams.len(),
            line_segments: self
                .pulled_lines
                .as_ref()
                .map_or(0, |pulled| pulled.segments.len()),
            mesh_segments: self
                .pulled_mesh
                .as_ref()
                .map_or(0, |pulled| pulled.segments.len()),
        };
        let admitted = Self::checkpoint_scalar_admits(checkpoint, keep_prefix, &facts)
            // The retained prefix owns exactly the retained resources in identity
            // order, and the **new (incoming)** suffix shares none of them.
            && Self::prefix_identity_count(
                &Self::instanced_sequence(&self.batches[..keep_prefix]),
                checkpoint.instance_streams,
            ) == Some(checkpoint.instance_streams)
            && Self::prefix_identity_count(
                &Self::mesh_sequence(&self.batches[..keep_prefix]),
                checkpoint.mesh_chunks,
            ) == Some(checkpoint.mesh_chunks)
            && Self::prefix_identity_count(
                &Self::arc_sequence(&self.batches[..keep_prefix]),
                checkpoint.arc_streams,
            ) == Some(checkpoint.arc_streams)
            && incoming.disjoint_from(
                checkpoint.instance_streams,
                checkpoint.mesh_chunks,
                checkpoint.arc_streams,
            );
        if admitted {
            PulledFastPath::Reuse(Box::new(checkpoint.clone()))
        } else {
            PulledFastPath::Full { establish }
        }
    }

    /// Invalidate the retained pulled prefix. Bumped on every event that can
    /// change the meaning of the retained base geometry/indices.
    pub(super) fn invalidate_pulled_prefix(&mut self) {
        self.pulled_cache_epoch = self.pulled_cache_epoch.wrapping_add(1);
        self.pulled_prefix_checkpoint = None;
    }

    /// First-encounter renumbering of a resource-index sequence: `Some(count)`
    /// only when the sequence is identity (`k`-th distinct index == `k`), which is
    /// exactly the condition under which the generic GC is a no-op and the
    /// retained ranges/indices stay valid. `None` for an out-of-range index.
    pub(super) fn prefix_identity_count(sequence: &[u32], len: usize) -> Option<usize> {
        let mut seen = vec![false; len];
        let mut next = 0usize;
        let mut identity = true;
        for &index in sequence {
            let index = index as usize;
            if index >= len {
                return None;
            }
            if !seen[index] {
                seen[index] = true;
                if next != index {
                    identity = false;
                }
                next += 1;
            }
        }
        identity.then_some(next)
    }

    /// Whether the retained prefix owns exactly `[0, n)` resources in identity
    /// order **and** the suffix references none of them (no resource shared across
    /// the boundary). `None` when the prefix is not identity, an index is
    /// out of range, or a suffix reference points into the retained range —
    /// in which case the fast path is declined (full, safe rebuild).
    pub(super) fn owned_prefix_count(
        prefix_seq: &[u32],
        suffix_seq: &[u32],
        len: usize,
    ) -> Option<usize> {
        let boundary = Self::prefix_identity_count(prefix_seq, len)?;
        for &index in suffix_seq {
            let index = index as usize;
            if index >= len || index < boundary {
                return None;
            }
        }
        Some(boundary)
    }

    /// The derived retained resource boundaries for a commit (pure). `establish`
    /// is the surviving prefix length, or `None` for a base replacement.
    ///
    /// For a `Reuse` plan the boundaries come from the checkpoint; for a
    /// `Full { establish: Some(prefix) }` plan they are derived from the validated
    /// prefix ownership (identity order + no suffix sharing), so a full warm-up
    /// still forces the base boundary and can record the base ranges. `None`
    /// boundaries mean "no checkpoint" (the tail invalidates).
    pub(super) fn derived_boundaries(
        &self,
        fast: &PulledFastPath,
    ) -> (Option<usize>, Option<usize>, bool) {
        match fast {
            PulledFastPath::Reuse(checkpoint) => (
                (checkpoint.instance_streams > 0).then_some(checkpoint.instance_streams),
                (checkpoint.mesh_chunks > 0).then_some(checkpoint.mesh_chunks),
                true,
            ),
            PulledFastPath::Full { establish: None } => (None, None, false),
            PulledFastPath::Full {
                establish: Some(prefix),
            } => {
                let prefix = (*prefix).min(self.batches.len());
                let line = Self::owned_prefix_count(
                    &Self::instanced_sequence(&self.batches[..prefix]),
                    &Self::instanced_sequence(&self.batches[prefix..]),
                    self.instance_streams.len(),
                );
                let mesh = Self::owned_prefix_count(
                    &Self::mesh_sequence(&self.batches[..prefix]),
                    &Self::mesh_sequence(&self.batches[prefix..]),
                    self.mesh_chunks.len(),
                );
                let arcs = Self::owned_prefix_count(
                    &Self::arc_sequence(&self.batches[..prefix]),
                    &Self::arc_sequence(&self.batches[prefix..]),
                    self.arc_streams.len(),
                );
                // The ownership proof must hold on every class; otherwise the
                // checkpoint cannot be established safely.
                if arcs.is_none() {
                    return (None, None, false);
                }
                let line = line.filter(|n| *n > 0);
                let mesh = mesh.filter(|n| *n > 0);
                // At least one *pulled* class must own a retained prefix.
                let any = line.is_some() || mesh.is_some();
                (line, mesh, any)
            }
        }
    }

    /// Whether every checkpoint-owned class **actually completed the reuse**: the
    /// pulled class is still present, the recorded base ranges equal the
    /// checkpoint's recorded ranges, and every required retained segment was
    /// reused. If any owned class fell back to the universal path (for example a
    /// segment/table that now exceeds the binding limit) this is false, so the
    /// reuse is declined and the checkpoint invalidated. Pure, so the matrix is
    /// unit-testable without a GPU.
    pub(super) fn reuse_completion(
        checkpoint: &PulledPrefixCheckpoint,
        line_present: bool,
        mesh_present: bool,
        line_ranges: &[(usize, usize)],
        mesh_ranges: &[(usize, usize)],
    ) -> bool {
        let line_ok = checkpoint.instance_streams == 0
            || (line_present
                && line_ranges.len() == checkpoint.line_segments
                && line_ranges == checkpoint.line_base_ranges.as_slice());
        let mesh_ok = checkpoint.mesh_chunks == 0
            || (mesh_present
                && mesh_ranges.len() == checkpoint.mesh_segments
                && mesh_ranges == checkpoint.mesh_base_ranges.as_slice());
        // At least one pulled class must have completed the reuse.
        line_ok && mesh_ok && (checkpoint.instance_streams > 0 || checkpoint.mesh_chunks > 0)
    }

    /// Greedy whole-stream line segmentation that never straddles `boundary`, so
    /// the base ranges are identical across suffix commits (pure).
    pub(super) fn forced_boundary_ranges(
        counts: &[usize],
        segment_elems: usize,
        boundary: Option<usize>,
    ) -> Vec<(usize, usize)> {
        let segment_elems = segment_elems.max(1);
        let mut ranges = Vec::new();
        let mut seg_start = 0usize;
        let mut seg_elems = 0usize;
        for (i, &n) in counts.iter().enumerate() {
            if i > seg_start && (seg_elems + n > segment_elems || Some(i) == boundary) {
                ranges.push((seg_start, i));
                seg_start = i;
                seg_elems = 0;
            }
            seg_elems += n;
        }
        ranges.push((seg_start, counts.len()));
        ranges
    }

    /// Greedy whole-chunk mesh segmentation that never straddles `boundary` (pure).
    pub(super) fn forced_boundary_chunk_ranges(
        vertices: &[usize],
        indices: &[usize],
        max_verts: usize,
        max_indices: usize,
        boundary: Option<usize>,
    ) -> Vec<(usize, usize)> {
        let count = vertices.len();
        let mut ranges = Vec::new();
        let mut start = 0usize;
        let mut verts = 0usize;
        let mut idx = 0usize;
        for i in 0..=count {
            let v = vertices.get(i).copied().unwrap_or(0);
            let n = indices.get(i).copied().unwrap_or(0);
            if i > start
                && (i == count
                    || verts + v > max_verts
                    || idx + n > max_indices
                    || Some(i) == boundary)
            {
                ranges.push((start, i));
                start = i;
                verts = 0;
                idx = 0;
            }
            verts += v;
            idx += n;
        }
        ranges
    }

    /// The number of leading ranges that end at or before `boundary`.
    pub(super) fn base_range_count(ranges: &[(usize, usize)], boundary: Option<usize>) -> usize {
        boundary
            .map(|boundary| ranges.iter().filter(|(_, end)| *end <= boundary).count())
            .unwrap_or(0)
    }

    /// The scalar (non-batch) half of the fast-path admission (pure).
    pub(super) fn checkpoint_scalar_admits(
        checkpoint: &PulledPrefixCheckpoint,
        keep_prefix: usize,
        facts: &CheckpointFacts,
    ) -> bool {
        keep_prefix > 0
            && keep_prefix == checkpoint.retained_batches
            && checkpoint.device_generation == facts.device_generation
            && checkpoint.cache_epoch == facts.cache_epoch
            && facts.accelerated
            && checkpoint.instance_streams <= facts.instance_streams
            && checkpoint.mesh_chunks <= facts.mesh_chunks
            && checkpoint.arc_streams <= facts.arc_streams
            && checkpoint.line_segments <= facts.line_segments
            && checkpoint.mesh_segments <= facts.mesh_segments
            // At least one *pulled* class must be retained (arcs have no pulled
            // variant), and a class that owns resources must retain a base segment
            // — but a class with no resources must NOT require one (line-only and
            // mesh-only scenes are admitted).
            && (checkpoint.instance_streams > 0 || checkpoint.mesh_chunks > 0)
            && (checkpoint.instance_streams == 0 || checkpoint.line_segments > 0)
            && (checkpoint.mesh_chunks == 0 || checkpoint.mesh_segments > 0)
    }

    pub(super) fn instanced_sequence(batches: &[GpuBatch]) -> Vec<u32> {
        batches
            .iter()
            .flat_map(|batch| batch.instanced.iter().map(|run| run.stream))
            .collect()
    }

    pub(super) fn mesh_sequence(batches: &[GpuBatch]) -> Vec<u32> {
        batches
            .iter()
            .flat_map(|batch| batch.mesh_runs.iter().map(|run| run.chunk))
            .collect()
    }

    pub(super) fn arc_sequence(batches: &[GpuBatch]) -> Vec<u32> {
        batches
            .iter()
            .flat_map(|batch| batch.arc_runs.iter().map(|run| run.stream))
            .collect()
    }
}
