//! The resident CPU base: one shared prefix plus the segments appended since it.
//!
//! A full build produces a prefix and no segments. An accepted incremental
//! append pushes one segment; the prefix is never re-materialised, which is
//! what makes the append O(changed) instead of O(scene).
//!
//! # Per-batch `Arc` payload and release
//!
//! Each prefix/segment owns a [`BatchSet`]: one `Arc<RenderBatch>` per batch
//! behind a shared lock. The granularity is what makes a single-entity in-place
//! update possible ([`ResidentScene::replace_batch`] swaps one `Arc` and never
//! copies the other batches), and the shared lock is what lets the engine
//! release the CPU geometry **once the GPU has committed it**. A released set
//! keeps metadata-only batches (origin, topology, style, texture/PBR keys,
//! draw order) and drops every geometry payload; [`ResidentScene::materialized`]
//! then fails with [`MissingCpuPayload`] instead of returning empty batches.
//! A payload release never changes scene identity: identity is the
//! `BatchSet` allocation plus its content generation, and only
//! [`ResidentScene::replace_batch`] bumps that generation.
//!
//! # Invariants
//!
//! * `appended` segments never carry `images`/`masks`. The append gate
//!   (`super::appendable_delta`) refuses such deltas, because the engine's append
//!   path deliberately keeps the resident textures/masks (it uploads them only
//!   for a non-append base change). [`ResidentScene::images`] /
//!   [`ResidentScene::masks`] therefore always come from the prefix.
//! * a segment is only ever appended; a full build replaces the whole resident
//!   scene (`from_prefix`), so a stale segment can never survive a rebuild.
//! * identity is pointer identity of the prefix, of every segment's
//!   `Arc<SceneDelta>`, and of every segment's `BatchSet`, plus the batch-set
//!   content generation ([`ResidentScene::same_as`]), never content equality.
//!   Releasing the payload does not change any of those; replacing a batch bumps
//!   exactly one generation, which is what the engine's append admission must
//!   observe.
use std::fmt;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use cad_domain::TaskStamp;
use cad_scene::{ImageBatch, MaskBatch, RenderBatch, SceneDelta};

/// Whether a batch set still owns its CPU geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadState {
    Resident,
    /// The GPU owns the committed geometry; only metadata-only batches remain
    /// and a re-materialisation is an explicit [`MissingCpuPayload`].
    GpuOnly,
}

/// A range of batches was requested from a released (GPU-only) payload.
///
/// This is a value, not an empty result: callers must rebuild the CPU base (or
/// surface the failure) instead of staging zero geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissingCpuPayload;

impl fmt::Display for MissingCpuPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the resident CPU batch payload was released after the GPU commit")
    }
}

impl std::error::Error for MissingCpuPayload {}

struct BatchPayload {
    state: PayloadState,
    batches: Vec<Arc<RenderBatch>>,
    vertex_counts: Vec<u32>,
}

/// One immutable batch allocation (prefix or appended segment).
///
/// `len` is immutable and lives outside the lock, so batch counts and offsets
/// never touch the payload. `vertex_counts` is guarded with the payload because
/// an in-place replacement may change a batch's vertex count; reading one count
/// is O(1) under a read lock and never reads geometry.
struct BatchSet {
    len: usize,
    /// Bumped by [`Self::replace`] only. A payload release must not bump it:
    /// identity (and append admission) must survive the release.
    generation: AtomicU64,
    payload: RwLock<BatchPayload>,
}

impl BatchSet {
    fn new(batches: Vec<Arc<RenderBatch>>) -> Self {
        let vertex_counts = batches
            .iter()
            .map(|batch| u32::try_from(batch.vertices.len()).unwrap_or(u32::MAX))
            .collect();
        let len = batches.len();
        Self {
            len,
            generation: AtomicU64::new(0),
            payload: RwLock::new(BatchPayload {
                state: PayloadState::Resident,
                batches,
                vertex_counts,
            }),
        }
    }

    fn len(&self) -> usize {
        self.len
    }

    fn vertex_count(&self, index: usize) -> Option<usize> {
        self.payload
            .read()
            .expect("batch payload lock")
            .vertex_counts
            .get(index)
            .map(|count| *count as usize)
    }

    fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    fn has_payload(&self) -> bool {
        self.payload.read().expect("batch payload lock").state == PayloadState::Resident
    }

    fn metadata(&self) -> Vec<Arc<RenderBatch>> {
        self.payload
            .read()
            .expect("batch payload lock")
            .batches
            .clone()
    }

    fn batch_at(&self, index: usize) -> Option<Arc<RenderBatch>> {
        self.payload
            .read()
            .expect("batch payload lock")
            .batches
            .get(index)
            .cloned()
    }

    fn resident(&self) -> Result<Vec<Arc<RenderBatch>>, MissingCpuPayload> {
        let payload = self.payload.read().expect("batch payload lock");
        if payload.state != PayloadState::Resident {
            return Err(MissingCpuPayload);
        }
        Ok(payload.batches.clone())
    }

    fn materialize(&self, start: usize, end: usize) -> Result<Vec<RenderBatch>, MissingCpuPayload> {
        let payload = self.payload.read().expect("batch payload lock");
        if start >= end {
            return Ok(Vec::new());
        }
        if payload.state != PayloadState::Resident {
            return Err(MissingCpuPayload);
        }
        Ok(payload.batches[start..end]
            .iter()
            .map(|batch| (**batch).clone())
            .collect())
    }

    fn replace(&self, index: usize, batch: Arc<RenderBatch>) -> bool {
        let mut payload = self.payload.write().expect("batch payload lock");
        if index >= payload.batches.len() {
            return false;
        }
        payload.vertex_counts[index] = u32::try_from(batch.vertices.len()).unwrap_or(u32::MAX);
        payload.batches[index] = batch;
        drop(payload);
        self.generation.fetch_add(1, Ordering::AcqRel);
        true
    }

    /// Drop every geometry payload, keeping metadata-only batches.
    ///
    /// Idempotent. Returns the approximate bytes released (for diagnostics).
    fn release(&self) -> usize {
        let mut payload = self.payload.write().expect("batch payload lock");
        if payload.state != PayloadState::Resident {
            return 0;
        }
        let mut released = 0usize;
        for batch in payload.batches.iter_mut() {
            released = released.saturating_add(batch.approx_bytes());
            *batch = Arc::new(batch.metadata_only());
        }
        payload.state = PayloadState::GpuOnly;
        drop(payload);
        // Replacing a batch Arc during release changes no identity: the
        // generation stays put, so append admission and flags-only publication
        // still see the same resident scene.
        released
    }
}

/// One segment appended after the prefix: the committed delta it came from
/// (retained for its stamp) plus its batches, one `Arc` per batch.
#[derive(Clone)]
struct Segment {
    delta: Arc<SceneDelta>,
    batches: Arc<BatchSet>,
    /// The set generation this scene saw when the segment was created or
    /// replaced. Shared sets mean a live generation read would be seen by every
    /// clone; the snapshot is what distinguishes a scene from a later mutation.
    generation: u64,
}

/// The committed base: one shared prefix plus the segments appended since it.
///
/// A full build produces a prefix and no segments. An accepted incremental
/// append pushes one segment; the prefix is never re-materialised, which is
/// what makes the append O(changed) instead of O(scene).
#[derive(Clone)]
pub struct ResidentScene {
    prefix: Arc<SceneDelta>,
    /// The prefix's batches, one `Arc` per batch, in resident order.
    prefix_batches: Arc<BatchSet>,
    /// The prefix set generation this scene saw (identity snapshot).
    prefix_generation: u64,
    appended: Vec<Segment>,
}

impl ResidentScene {
    /// A resident scene made of one prefix and no appended segments (a full
    /// build), **consuming** the delta.
    ///
    /// The batches move into their per-batch `Arc`s (no deep copy), and the
    /// retained prefix `SceneDelta` keeps the whole-segment facts (`stamp`,
    /// `images`, `masks`) with `added` emptied: the batch payload is the
    /// per-batch `Arc` list, so the scene never stores every batch twice.
    pub fn from_owned(delta: SceneDelta) -> Self {
        let SceneDelta {
            stamp,
            added,
            images,
            masks,
        } = delta;
        let prefix_batches = Arc::new(BatchSet::new(added.into_iter().map(Arc::new).collect()));
        Self {
            prefix: Arc::new(SceneDelta {
                stamp,
                added: Vec::new(),
                images,
                masks,
            }),
            prefix_batches,
            prefix_generation: 0,
            appended: Vec::new(),
        }
    }

    /// [`Self::from_owned`] for callers that hold the delta behind an `Arc`.
    ///
    /// The batches are cloned once (the `Arc` cannot be consumed), so the
    /// controller's full build uses [`Self::from_owned`] instead. The retained
    /// prefix delta still has `added` emptied — see [`Self::from_owned`].
    pub fn from_prefix(prefix: Arc<SceneDelta>) -> Self {
        Self::from_owned((*prefix).clone())
    }

    /// Append one committed segment without touching the prefix.
    ///
    /// The segment must not grow the resident raster sets: the engine does not
    /// re-upload images/masks on an append, so an appended `ImageBatch`/
    /// `MaskBatch` would be a silent partial success.
    ///
    /// The per-entity delta is converted to one `Arc` per batch. The retained
    /// segment delta is reduced to its stamp: keeping the original
    /// `Arc<SceneDelta>` would retain a second copy of the same `added`
    /// geometry for the lifetime of the segment.
    pub fn push_appended(&mut self, segment: Arc<SceneDelta>) {
        debug_assert!(
            segment.images.is_empty() && segment.masks.is_empty(),
            "an appended segment must not carry images/masks (the append path never \
             re-uploads them)"
        );
        let batches = Arc::new(BatchSet::new(
            segment.added.iter().cloned().map(Arc::new).collect(),
        ));
        let delta = Arc::new(SceneDelta {
            stamp: segment.stamp.clone(),
            added: Vec::new(),
            images: Vec::new(),
            masks: Vec::new(),
        });
        self.appended.push(Segment {
            delta,
            batches,
            generation: 0,
        });
    }

    /// The shared prefix's whole-segment facts. The engine retains this exact
    /// `Arc` as its applied base identity, and the renderer reads `images`/
    /// `masks` from it.
    ///
    /// Its `added` list is **empty by construction**: the batch payload is the
    /// per-batch `Arc` list (see [`Self::from_owned`]).
    pub fn prefix_delta(&self) -> &Arc<SceneDelta> {
        &self.prefix
    }

    /// Number of segments appended after the prefix.
    pub fn appended_len(&self) -> usize {
        self.appended.len()
    }

    /// Total number of resident base batches across the prefix and every segment.
    pub fn batch_len(&self) -> usize {
        self.prefix_batches.len()
            + self
                .appended
                .iter()
                .map(|segment| segment.batches.len())
                .sum::<usize>()
    }

    /// Vertex count of the batch at a resident index, without touching geometry.
    pub fn vertex_count(&self, index: usize) -> Option<usize> {
        if let Some(count) = self.prefix_batches.vertex_count(index) {
            return Some(count);
        }
        let mut cursor = self.prefix_batches.len();
        for segment in &self.appended {
            if index < cursor + segment.batches.len() {
                return segment.batches.vertex_count(index - cursor);
            }
            cursor += segment.batches.len();
        }
        None
    }

    /// Every resident batch's metadata, in resident order.
    ///
    /// Works after a payload release (the released sets keep metadata-only
    /// batches). Geometry readers must use [`Self::resident_batches`] or
    /// [`Self::materialized`], which report [`MissingCpuPayload`].
    pub fn metadata(&self) -> Vec<Arc<RenderBatch>> {
        let mut batches = self.prefix_batches.metadata();
        for segment in &self.appended {
            batches.extend(segment.batches.metadata());
        }
        batches
    }

    /// Every resident batch with its CPU geometry, or [`MissingCpuPayload`].
    pub fn resident_batches(&self) -> Result<Vec<Arc<RenderBatch>>, MissingCpuPayload> {
        let mut batches = self.prefix_batches.resident()?;
        for segment in &self.appended {
            batches.extend(segment.batches.resident()?);
        }
        Ok(batches)
    }

    /// Materialise a bounded batch range as owned [`RenderBatch`] values.
    ///
    /// The engine stages at most 256 batches / 262,144 vertices per piece, so
    /// cloning the contents of one bounded piece costs O(piece) and is accepted.
    /// A released range is an explicit [`MissingCpuPayload`], never an empty
    /// piece; a requested range past the end is clipped (never fabricated).
    pub fn materialized(&self, range: Range<usize>) -> Result<Vec<RenderBatch>, MissingCpuPayload> {
        let len = self.batch_len();
        let start = range.start.min(len);
        let end = range.end.min(len);
        if start >= end {
            return Ok(Vec::new());
        }
        let mut out = Vec::with_capacity(end - start);
        let mut cursor = 0usize;
        let prefix_end = self.prefix_batches.len();
        if start < prefix_end {
            let local_end = end.min(prefix_end);
            out.extend(self.prefix_batches.materialize(start, local_end)?);
        }
        cursor += prefix_end;
        for segment in &self.appended {
            if cursor >= end {
                break;
            }
            let segment_end = cursor + segment.batches.len();
            if end > cursor {
                let local_start = start.saturating_sub(cursor);
                let local_end = end.min(segment_end) - cursor;
                if local_start < local_end {
                    out.extend(segment.batches.materialize(local_start, local_end)?);
                }
            }
            cursor = segment_end;
        }
        Ok(out)
    }

    /// The resident batch at `index`, in resident order.
    fn batch_at(&self, index: usize) -> Option<Arc<RenderBatch>> {
        if let Some(batch) = self.prefix_batches.batch_at(index) {
            return Some(batch);
        }
        let mut cursor = self.prefix_batches.len();
        for segment in &self.appended {
            if index < cursor + segment.batches.len() {
                return segment.batches.batch_at(index - cursor);
            }
            cursor += segment.batches.len();
        }
        None
    }

    /// The resident batch at `index`, in resident order.
    ///
    /// Test/bookkeeping helper; production staging uses [`Self::materialized`].
    pub fn batch_metadata_at(&self, index: usize) -> Option<Arc<RenderBatch>> {
        self.batch_at(index)
    }

    /// Replace the resident batch at `index` with `batch`, in O(1) (plus the
    /// O(#segments) segment lookup): no other batch is cloned or moved.
    ///
    /// Returns `false` when `index` is out of range, in which case nothing is
    /// modified. The caller must have validated the whole replacement first: a
    /// batch swap is not reversible from here.
    ///
    /// A replacement changes the resident scene's identity ([`Self::same_as`])
    /// because the owning `BatchSet`'s content generation advances. That is
    /// deliberate: the engine's exact-predecessor append admission must not
    /// treat a scene whose geometry was overwritten as the allocation it last
    /// committed.
    pub fn replace_batch(&mut self, index: usize, batch: Arc<RenderBatch>) -> bool {
        let mut cursor = self.prefix_batches.len();
        if index < cursor {
            let replaced = self.prefix_batches.replace(index, batch);
            if replaced {
                self.prefix_generation = self.prefix_batches.generation();
            }
            return replaced;
        }
        for segment in &mut self.appended {
            if index < cursor + segment.batches.len() {
                let replaced = segment.batches.replace(index - cursor, batch);
                if replaced {
                    segment.generation = segment.batches.generation();
                }
                return replaced;
            }
            cursor += segment.batches.len();
        }
        false
    }

    /// Release every CPU geometry payload (prefix and appended segments).
    ///
    /// Idempotent. The scene's identity and batch counts are unchanged; only
    /// [`Self::materialized`]/[`Self::resident_batches`] start reporting
    /// [`MissingCpuPayload`]. Returns the approximate bytes released.
    pub fn release_payload(&self) -> usize {
        let mut released = self.prefix_batches.release();
        for segment in &self.appended {
            released = released.saturating_add(segment.batches.release());
        }
        released
    }

    /// Whether every resident batch still owns its CPU geometry.
    pub fn has_cpu_payload(&self) -> bool {
        self.prefix_has_cpu_payload()
            && self
                .appended
                .iter()
                .all(|segment| segment.batches.has_payload())
    }

    /// Whether the prefix (the part a full re-stage must materialise first)
    /// still owns its CPU geometry. An append only stages its tail, so a
    /// released prefix is still a valid append predecessor.
    pub fn prefix_has_cpu_payload(&self) -> bool {
        self.prefix_batches.has_payload()
    }

    /// Current payload state (mixed scenes report `GpuOnly`).
    pub fn payload_state(&self) -> PayloadState {
        if self.has_cpu_payload() {
            PayloadState::Resident
        } else {
            PayloadState::GpuOnly
        }
    }

    /// The resident image draw items. Always the prefix's: an appended segment
    /// never carries any.
    pub fn images(&self) -> &[ImageBatch] {
        &self.prefix.images
    }

    /// The resident wipeout masks. Always the prefix's: an appended segment never
    /// carries any.
    pub fn masks(&self) -> &[MaskBatch] {
        &self.prefix.masks
    }

    /// The stamp of the newest committed content: the last appended segment's
    /// stamp, or the prefix's when nothing was appended.
    pub fn stamp(&self) -> &TaskStamp {
        self.appended
            .last()
            .map_or(&self.prefix.stamp, |segment| &segment.delta.stamp)
    }

    /// Whether `other` is the **same** resident scene: the same prefix
    /// allocation, the same segment allocations in the same order, the same
    /// `BatchSet` allocations, and the same per-set content generation.
    ///
    /// A payload release changes none of those. A replaced batch advances its
    /// set's generation, so the scene is a different base; an accepted append
    /// keeps the predecessor's prefix and adds one segment.
    pub fn same_as(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.prefix, &other.prefix)
            && Arc::ptr_eq(&self.prefix_batches, &other.prefix_batches)
            && self.prefix_generation == other.prefix_generation
            && self.appended.len() == other.appended.len()
            && self
                .appended
                .iter()
                .zip(&other.appended)
                .all(|(left, right)| {
                    Arc::ptr_eq(&left.delta, &right.delta)
                        && Arc::ptr_eq(&left.batches, &right.batches)
                        && left.generation == right.generation
                })
    }

    /// Materialise the whole resident scene as one [`SceneDelta`].
    ///
    /// O(scene) by construction: for byte-identity checks against a full build
    /// (tests, exporters), never on the append path. A released payload is an
    /// explicit error rather than an empty delta.
    pub fn to_delta(&self) -> Result<SceneDelta, MissingCpuPayload> {
        let added = self
            .resident_batches()?
            .into_iter()
            .map(|batch| (*batch).clone())
            .collect();
        Ok(SceneDelta {
            stamp: self.stamp().clone(),
            added,
            images: self.prefix.images.clone(),
            masks: self.prefix.masks.clone(),
        })
    }
}

#[cfg(test)]
#[path = "resident_tests.rs"]
mod tests;
