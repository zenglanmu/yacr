use super::*;

/// Above this many estimated active batches the segmented planner falls back to
/// the linear walk: the per-active work plus the gather/sort would no longer beat
/// walking `ordered`. `4096` keeps the gather allocation modest while covering
/// every page whose cull leaves a small visible set (the settled 1-draw page has
/// `m` in the single digits).
pub(super) const SEGMENTED_ACTIVE_THRESHOLD: usize = 4096;

/// Batch-set facts for the segmented planner, derived once when the resident
/// batch set changes — the same invalidation as [`DrawOrderCache`]
/// (`Renderer::draw_plan = None`). Never rebuilt per frame.
///
/// [`DrawOrderCache`]: crate::DrawOrderCache
#[derive(Default)]
pub(crate) struct BatchFacts {
    /// Per class (`[instance, mesh, arc]`) and chunk index, the batch indices
    /// that own a run in that chunk. A batch listed under a **visible** chunk has
    /// a surviving run, so it charges the frame budget and is a candidate to
    /// terminate the accepted prefix.
    pub(crate) chunk_batches: [Vec<Vec<u32>>; 3],
    /// Batch indices that charge the frame budget **regardless of the cull**:
    /// legacy per-batch geometry with no runs (a per-batch quad line list or a
    /// non-converted mesh) and any batch with `line_quad_count > 0`.
    ///
    /// This must **not** be replaced by [`batch_has_visible_work`]: that predicate
    /// answers "does an accepted batch issue a draw", and a fully-culled converted
    /// batch with legacy per-batch quads is *not* free — its quads are charged
    /// unconditionally (`plan.rs` line-quad term) even though no run survives.
    pub(crate) always_active: Vec<u32>,
    /// Whether any batch has an instanced line run (the "line class" exists).
    pub(crate) has_line_class: bool,
    /// Whether any batch has a merged mesh run (the "mesh class" exists).
    pub(crate) has_mesh_class: bool,
    /// Number of batches converted to an instanced stream or a merged mesh chunk.
    pub(crate) converted_total: usize,
    /// Compact per-batch upload sizes, indexed by batch index. `OrderedPageIndex`
    /// reads this instead of `batches[batch]` so the per-restart byte prefix scan
    /// does not DRAM-miss on the large resident `GpuBatch` struct.
    pub(crate) upload_bytes: Vec<usize>,
    /// Compact per-batch run class, indexed by batch index: a bitfield of
    /// [`CLASS_INSTANCED`]/[`CLASS_MESH`]/[`CLASS_ARC`]. `0` means the batch owns
    /// **no runs** and therefore takes the legacy per-batch encode branch (it
    /// flushes the pending runs and draws its legacy buffers), so it always
    /// participates and always needs a per-batch uniform slot.
    ///
    /// Read instead of `batches[i]` in the uniform and encode loops: the class is
    /// one byte per batch (cache-resident) while the resident `GpuBatch` is ~200
    /// bytes (a DRAM miss when the frame order is not the upload order).
    pub(crate) class: Vec<u8>,
    /// Batch indices whose [`BatchFacts::class`] is `0` (they own no runs). These
    /// are the only batches that need a per-batch uniform slot and the only
    /// run-free participants in the encode loop; kept as a list so a frame can
    /// enumerate them without scanning every resident batch.
    pub(crate) legacy_batches: Vec<u32>,
}

/// Batch owns instanced line runs.
pub(crate) const CLASS_INSTANCED: u8 = 1;

/// Batch owns merged mesh runs.
pub(crate) const CLASS_MESH: u8 = 2;

/// Batch owns SDF conic runs.
pub(crate) const CLASS_ARC: u8 = 4;

impl BatchFacts {
    /// Derive the batch-set facts from the resident batches. O(total runs); run
    /// only when the batch set changes, never per frame.
    pub(crate) fn build(batches: &[GpuBatch]) -> Self {
        let instance_n = batches
            .iter()
            .flat_map(|batch| batch.instanced.iter())
            .map(|run| run.stream as usize + 1)
            .max()
            .unwrap_or(0);
        let mesh_n = batches
            .iter()
            .flat_map(|batch| batch.mesh_runs.iter())
            .map(|run| run.chunk as usize + 1)
            .max()
            .unwrap_or(0);
        let arc_n = batches
            .iter()
            .flat_map(|batch| batch.arc_runs.iter())
            .map(|run| run.stream as usize + 1)
            .max()
            .unwrap_or(0);
        let mut facts = BatchFacts {
            chunk_batches: [
                vec![Vec::new(); instance_n],
                vec![Vec::new(); mesh_n],
                vec![Vec::new(); arc_n],
            ],
            always_active: Vec::new(),
            has_line_class: false,
            has_mesh_class: false,
            converted_total: 0,
            upload_bytes: Vec::new(),
            class: Vec::new(),
            legacy_batches: Vec::new(),
        };
        facts.upload_bytes = batches
            .iter()
            .map(|batch| batch.upload_size_bytes())
            .collect();
        facts.class = Vec::with_capacity(batches.len());
        for (i, batch) in batches.iter().enumerate() {
            let batch_index = i as u32;
            let mut class = 0u8;
            if !batch.instanced.is_empty() {
                class |= CLASS_INSTANCED;
            }
            if !batch.mesh_runs.is_empty() {
                class |= CLASS_MESH;
            }
            if !batch.arc_runs.is_empty() {
                class |= CLASS_ARC;
            }
            facts.class.push(class);
            if class == 0 {
                facts.legacy_batches.push(batch_index);
            }
            for run in &batch.instanced {
                facts.chunk_batches[0][run.stream as usize].push(batch_index);
            }
            for run in &batch.mesh_runs {
                facts.chunk_batches[1][run.chunk as usize].push(batch_index);
            }
            for run in &batch.arc_runs {
                facts.chunk_batches[2][run.stream as usize].push(batch_index);
            }
            let converted = !batch.instanced.is_empty() || !batch.mesh_runs.is_empty();
            if converted {
                facts.converted_total += 1;
            }
            facts.has_line_class |= !batch.instanced.is_empty();
            facts.has_mesh_class |= !batch.mesh_runs.is_empty();
            // Charge independent of the cull: legacy line quads are always charged
            // (`batch_budget_vertices` line-quad term) and a legacy mesh
            // (`mesh_runs` empty) is never culled (`mesh_batch_visible`).
            if batch.line_quad_count > 0
                || (batch.mesh_runs.is_empty() && batch.topology == RenderTopology::Mesh)
            {
                facts.always_active.push(batch_index);
            }
        }
        facts
    }
}

/// The per-frame set of batches that participate in the encode pass.
///
/// A batch participates iff it owns **no runs** (it takes the legacy branch, which
/// flushes the pending coalesced runs and draws its legacy buffers) or at least
/// one of its runs survives this frame's cull. A batch with runs whose runs are
/// **all** culled is a pure no-op in the encode loop: its per-class branch only
/// bumps the reporting counters, issues no draw and touches no pending run, so
/// skipping it cannot change the emitted work.
///
/// This is exactly [`batch_has_visible_work`]. That predicate is the **right** one
/// here (unlike the segmented planner's gap logic, where a fully-culled converted
/// batch with legacy quads is still charged).
pub(crate) struct EncodeEmitMask {
    /// The cull removed nothing, so every batch keeps its runs and participates.
    /// The bitset stays empty and [`EncodeEmitMask::participates`] returns true.
    all: bool,
    bits: Vec<u64>,
}

impl Default for EncodeEmitMask {
    fn default() -> Self {
        // A never-rebuilt mask must not skip anything (and must not index an empty
        // bitset), so it starts in the `all` state.
        EncodeEmitMask {
            all: true,
            bits: Vec::new(),
        }
    }
}

pub(super) fn set_bit(bits: &mut [u64], index: u32) {
    bits[index as usize / 64] |= 1u64 << (index % 64);
}

impl EncodeEmitMask {
    /// Rebuild the mask for this frame's `cull`, reusing `self.bits`.
    pub(crate) fn rebuild(&mut self, facts: &BatchFacts, cull: &ChunkCull) {
        if !cull.any_culled() {
            // Nothing was removed, so every run survives and every batch
            // participates: skip building the mask entirely.
            self.all = true;
            self.bits.clear();
            return;
        }
        self.all = false;
        self.bits.clear();
        self.bits.resize(facts.class.len().div_ceil(64), 0);
        // Run-free batches take the legacy branch; `always_active` covers the
        // cull-independent charge cases (legacy line quads, legacy mesh) so a batch
        // with `line_quad_count > 0` is never skipped even in the synthetic case
        // where it also owns runs. Both are supersets of "takes the legacy branch".
        for &b in &facts.legacy_batches {
            set_bit(&mut self.bits, b);
        }
        for &b in &facts.always_active {
            set_bit(&mut self.bits, b);
        }
        for c in 0..facts.chunk_batches[0].len() {
            if cull.stream_visible(c) {
                for &b in &facts.chunk_batches[0][c] {
                    set_bit(&mut self.bits, b);
                }
            }
        }
        for c in 0..facts.chunk_batches[1].len() {
            if cull.mesh_visible(c) {
                for &b in &facts.chunk_batches[1][c] {
                    set_bit(&mut self.bits, b);
                }
            }
        }
        for c in 0..facts.chunk_batches[2].len() {
            if cull.arc_visible(c) {
                for &b in &facts.chunk_batches[2][c] {
                    set_bit(&mut self.bits, b);
                }
            }
        }
    }

    /// Whether batch `index` participates in the encode pass this frame.
    pub(crate) fn participates(&self, index: usize) -> bool {
        self.all || (self.bits[index / 64] >> (index % 64)) & 1 == 1
    }
}

/// Per-`ordered` index for the segmented planner, rebuilt whenever `ordered` is
/// rebuilt (the progressive restart / camera move). Derived from the same
/// restart, not a second invalidation truth.
///
/// The buffers are retained across restarts (the previous frame's index is taken
/// and refilled) so a camera move does not allocate ~2 MB of fresh, page-faulting
/// memory every frame.
#[derive(Default)]
pub(crate) struct OrderedPageIndex {
    /// Batch index -> its position in `ordered`, or `u32::MAX` when the batch is
    /// not drawn this frame (invisible, or outside the ordered set).
    pub(crate) batch_pos: Vec<u32>,
    /// Prefix sums of upload bytes over `ordered`: `bytes_prefix[i]` is the total
    /// upload bytes of `ordered[..i]`. Lets a page's byte charge be a range sum.
    pub(crate) bytes_prefix: Vec<usize>,
}

impl OrderedPageIndex {
    /// Build a fresh index (test/standalone use).
    #[cfg(test)]
    pub(crate) fn build(batches: &[GpuBatch], ordered: &[usize]) -> Self {
        let upload_bytes: Vec<usize> = batches
            .iter()
            .map(|batch| batch.upload_size_bytes())
            .collect();
        let mut index = OrderedPageIndex::default();
        index.rebuild(&upload_bytes, ordered);
        index
    }

    /// Refill the index for `ordered`, reusing the existing allocations.
    ///
    /// `upload_bytes` is [`BatchFacts::upload_bytes`] — the compact per-batch
    /// upload sizes, indexed by batch index.
    pub(crate) fn rebuild(&mut self, upload_bytes: &[usize], ordered: &[usize]) {
        self.batch_pos.clear();
        self.batch_pos.resize(upload_bytes.len(), u32::MAX);
        self.bytes_prefix.clear();
        self.bytes_prefix.reserve(ordered.len() + 1);
        self.bytes_prefix.push(0usize);
        for (pos, &batch) in ordered.iter().enumerate() {
            self.batch_pos[batch] = pos as u32;
            // Read from the compact `upload_bytes` table, not `batches[batch]`:
            // the resident `GpuBatch` is ~200 bytes, so a scattered read of it
            // (the frame order is not the upload order) is a DRAM miss per batch,
            // while the table is a few MB and stays in cache.
            self.bytes_prefix
                .push(self.bytes_prefix[pos] + upload_bytes[batch]);
        }
    }
}

/// Upper bound on the active-batch count for this cull: the sum of the
/// per-visible-chunk batch lists plus the always-active set. Counts a batch once
/// per visible chunk it appears in (so it over-estimates), which is exactly what
/// the fallback threshold needs.
pub(super) fn estimated_active(facts: &BatchFacts, cull: &ChunkCull) -> usize {
    let mut count = facts.always_active.len();
    // Index by the chunk lists, not the mask: a chunk index beyond the mask is
    // reported visible by `stream_visible`/`mesh_visible`/`arc_visible`, so it
    // must be counted here too.
    for c in 0..facts.chunk_batches[0].len() {
        if cull.stream_visible(c) {
            count += facts.chunk_batches[0][c].len();
        }
    }
    for c in 0..facts.chunk_batches[1].len() {
        if cull.mesh_visible(c) {
            count += facts.chunk_batches[1][c].len();
        }
    }
    for c in 0..facts.chunk_batches[2].len() {
        if cull.arc_visible(c) {
            count += facts.chunk_batches[2][c].len();
        }
    }
    count
}

/// The segmented page planner. Preconditions (enforced by [`plan_ordered_page`]):
/// the cull removed at least one chunk and the estimated active count is at most
/// [`SEGMENTED_ACTIVE_THRESHOLD`].
#[allow(clippy::too_many_arguments)]
pub(super) fn plan_ordered_page_segmented(
    batches: &[GpuBatch],
    ordered: &[usize],
    cursor: usize,
    budget: &FrameBudget,
    initial: cad_scene::FrameUsage,
    cull: &ChunkCull,
    facts: &BatchFacts,
    index: &OrderedPageIndex,
) -> GpuPlan {
    let len = ordered.len();
    let bytes_prefix = &index.bytes_prefix;
    let base_bytes = bytes_prefix[cursor];

    // --- byte boundary ------------------------------------------------------
    // The first position `p` in `[cursor, len)` whose cumulative page bytes
    // (including `p`) exceed `max_bytes`. Gaps charge bytes too (byte charging is
    // batch-granular and cull-independent), so this is a prefix-sum search over
    // `ordered`, not part of the active walk. With `max_bytes == usize::MAX` the
    // predicate is never true (the boundary is +∞).
    let byte_exceeds = |p: usize| {
        initial
            .bytes
            .saturating_add(bytes_prefix[p + 1] - base_bytes)
            > budget.max_bytes
    };
    // Binary search for the first `p` where `byte_exceeds(p)` holds. The
    // predicate is monotonic (prefix sums are non-decreasing), so this finds the
    // boundary; `len` means the byte limit is never crossed.
    let mut lo = cursor;
    let mut hi = len;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if byte_exceeds(mid) {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    let byte_fail = lo;

    // --- active positions in `[cursor, len)`, ascending ---------------------
    let batch_pos = &index.batch_pos;
    let mut active: Vec<u32> = Vec::new();
    for c in 0..facts.chunk_batches[0].len() {
        if cull.stream_visible(c) {
            active.extend_from_slice(&facts.chunk_batches[0][c]);
        }
    }
    for c in 0..facts.chunk_batches[1].len() {
        if cull.mesh_visible(c) {
            active.extend_from_slice(&facts.chunk_batches[1][c]);
        }
    }
    for c in 0..facts.chunk_batches[2].len() {
        if cull.arc_visible(c) {
            active.extend_from_slice(&facts.chunk_batches[2][c]);
        }
    }
    active.extend_from_slice(&facts.always_active);
    // Drop batches already drawn on an earlier page (position < cursor) and any
    // batch not in `ordered` this frame, then order by position and de-duplicate.
    active.retain(|&b| {
        let pos = batch_pos[b as usize];
        pos != u32::MAX && pos as usize >= cursor
    });
    active.sort_unstable_by_key(|&b| batch_pos[b as usize]);
    active.dedup_by_key(|b| batch_pos[*b as usize]);
    // Always evaluate the cursor batch: an image reservation may already exceed
    // the budget, in which case even a zero-charge batch fails.
    if active.first().map(|&b| batch_pos[b as usize] as usize) != Some(cursor) {
        active.insert(0, ordered[cursor] as u32);
    }

    // --- vertex/triangle boundary over the active positions -----------------
    let mut usage = initial;
    let mut vt_fail = len;
    let mut vt_category = "vertices";
    let mut vt_requested = 0usize;
    for &b in &active {
        let pos = batch_pos[b as usize] as usize;
        // A byte failure at or before this position means the page never reaches
        // it; the byte category wins (charge_bytes runs first).
        if pos >= byte_fail {
            break;
        }
        let batch = &batches[ordered[pos]];
        let vertices = batch_budget_vertices(batch, cull);
        let triangles = batch_budget_triangles(batch, cull);
        let vertex_total = usage.vertices.checked_add(vertices);
        let next_vertices = vertex_total.unwrap_or(usize::MAX);
        if vertex_total.is_none() || next_vertices > budget.max_vertices {
            vt_fail = pos;
            vt_category = "vertices";
            vt_requested = next_vertices;
            break;
        }
        let triangle_total = usage.triangles.checked_add(triangles);
        let next_triangles = triangle_total.unwrap_or(usize::MAX);
        if triangle_total.is_none() || next_triangles > budget.max_triangles {
            vt_fail = pos;
            vt_category = "triangles";
            vt_requested = next_triangles;
            break;
        }
        usage.vertices = next_vertices;
        usage.triangles = next_triangles;
    }

    let fail_pos = byte_fail.min(vt_fail);
    let accepted = ordered[cursor..fail_pos].to_vec();
    // Usage follows the shared contract (see `plan_from_gpu`): bytes are the page
    // bytes of `cursor..fail_pos` (gaps included, batch-granular), and when the
    // failure is on vertices/triangles the failing batch's byte charge already
    // committed, so its bytes are included too. On a byte failure (or no failure)
    // nothing extra is added. Vertices/triangles are the active charges already
    // accumulated before `fail_pos`.
    let usage_end = if fail_pos < len && byte_fail != fail_pos {
        fail_pos + 1
    } else {
        fail_pos
    };
    usage.bytes = initial
        .bytes
        .saturating_add(bytes_prefix[usage_end] - base_bytes);
    if fail_pos == len {
        return GpuPlan {
            accepted,
            usage,
            over_budget: None,
        };
    }
    // Category precedence at the boundary batch: `charge_bytes` runs first and
    // `and_then` short-circuits, so a batch that crosses bytes *and* vertices is
    // reported as `"bytes"`.
    let (category, requested, limit) = if byte_fail == fail_pos {
        (
            "bytes",
            initial
                .bytes
                .saturating_add(bytes_prefix[fail_pos + 1] - base_bytes),
            budget.max_bytes,
        )
    } else {
        let limit = if vt_category == "vertices" {
            budget.max_vertices
        } else {
            budget.max_triangles
        };
        (vt_category, vt_requested, limit)
    };
    let report = OverBudget {
        category,
        requested,
        limit,
        skipped_batches: len - fail_pos,
    };
    GpuPlan {
        accepted,
        usage,
        over_budget: Some(OverBudgetReason {
            reason: over_budget_reason(&report),
            report,
        }),
    }
}

/// Plan exactly the cursor batch on its own against `budget`.
///
/// The **progress floor** of adaptive pacing: when a paced page would accept no
/// batch while the cursor still has work, the cursor batch alone is planned
/// against the un-paced ceiling, so every page makes progress and only a batch
/// that genuinely exceeds the ceiling is an error. `None` means the cursor is
/// exhausted or the batch does not fit the ceiling.
pub(crate) fn plan_cursor_batch(
    batches: &[GpuBatch],
    ordered: &[usize],
    cursor: usize,
    budget: &FrameBudget,
    cull: &ChunkCull,
) -> Option<GpuPlan> {
    let index = *ordered.get(cursor)?;
    let batch = batches.get(index)?;
    let mut usage = cad_scene::FrameUsage::default();
    budget
        .charge_bytes(&mut usage, batch.upload_size_bytes())
        .ok()?;
    budget
        .charge(
            &mut usage,
            batch_budget_vertices(batch, cull),
            batch_budget_triangles(batch, cull),
        )
        .ok()?;
    Some(GpuPlan {
        accepted: vec![index],
        usage,
        over_budget: None,
    })
}

/// The over-budget reason for a frame that crossed the **active** (paced) page
/// budget only to make progress.
///
/// The diagnostic code stays `render.frame_over_budget`; the category parameter
/// is [`crate::pacing::PACING_FLOOR_REASON`], so a reader can tell a paced floor
/// from a genuine ceiling overflow without a second diagnostic code.
pub(crate) fn pacing_floor_reason(
    plan: &GpuPlan,
    active: &FrameBudget,
    skipped_batches: usize,
) -> OverBudgetReason {
    let report = OverBudget {
        category: crate::pacing::PACING_FLOOR_REASON,
        requested: plan.usage.vertices,
        limit: active.max_vertices,
        skipped_batches,
    };
    OverBudgetReason {
        reason: over_budget_reason(&report),
        report,
    }
}

/// Report usage that crossed `budget`, if any.
///
/// Used to keep the `over_budget` contract intact when a page reserves more than
/// the active budget without going through the geometry planner (an image set
/// that fits the ceiling but not the paced page). Uses the existing
/// vertex/triangle/byte categories, so no second diagnostic code is introduced.
pub(crate) fn usage_over_budget_reason(
    usage: &cad_scene::FrameUsage,
    budget: &FrameBudget,
    skipped_batches: usize,
) -> Option<OverBudgetReason> {
    let (category, requested, limit) = if usage.vertices > budget.max_vertices {
        ("vertices", usage.vertices, budget.max_vertices)
    } else if usage.triangles > budget.max_triangles {
        ("triangles", usage.triangles, budget.max_triangles)
    } else if usage.bytes > budget.max_bytes {
        ("bytes", usage.bytes, budget.max_bytes)
    } else {
        return None;
    };
    let report = OverBudget {
        category,
        requested,
        limit,
        skipped_batches,
    };
    Some(OverBudgetReason {
        reason: over_budget_reason(&report),
        report,
    })
}

pub(crate) fn over_budget_reason(report: &OverBudget) -> DiagnosticReason {
    DiagnosticReason::partial(
        codes::RENDER_FRAME_OVER_BUDGET,
        vec![
            DiagnosticParameter::Identifier(report.category.to_string()),
            DiagnosticParameter::Count(report.requested as u64),
            DiagnosticParameter::Limit(report.limit as u64),
            DiagnosticParameter::Count(report.skipped_batches as u64),
        ],
    )
}
