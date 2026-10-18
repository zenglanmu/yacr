use super::*;

pub(crate) struct GpuPlan {
    pub(crate) accepted: Vec<usize>,
    pub(crate) usage: cad_scene::FrameUsage,
    pub(crate) over_budget: Option<OverBudgetReason>,
}

/// The ordered, budget-accepted batch indices split into the two draw passes.
pub(crate) struct DrawPasses {
    pub(crate) opaque: Vec<usize>,
    pub(crate) transparent: Vec<usize>,
    pub(crate) invisible: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MeshDrawPlan {
    pub(super) triangles: bool,
    pub(super) edge_overlay: bool,
}

pub(super) fn mesh_draw_plan(has_triangles: bool, has_overlay_quads: bool) -> MeshDrawPlan {
    MeshDrawPlan {
        triangles: has_triangles,
        edge_overlay: has_triangles && has_overlay_quads,
    }
}

/// Turn the pure [`DrawOrderPlan`] over the accepted sub-set back into global
/// batch indices, preserving the plan's order.
///
/// `accepted` are the global indices the budget kept (upload order); `entries`
/// are their ordering keys in the same order. Splitting this out keeps the index
/// remapping testable without a GPU.
pub(crate) fn draw_passes(
    accepted: &[usize],
    entries: &[BatchOrderEntry],
    camera_position: Option<[f32; 3]>,
) -> DrawPasses {
    debug_assert_eq!(accepted.len(), entries.len());
    let order = plan_draw_order(entries, camera_position);
    let remap = |positions: &[usize]| -> Vec<usize> {
        positions.iter().map(|&pos| accepted[pos]).collect()
    };
    DrawPasses {
        opaque: remap(&order.opaque),
        transparent: remap(&order.transparent),
        invisible: order.invisible.len(),
    }
}

/// Vertices a resident batch charges the frame budget this frame.
///
/// Every line-like segment rasterizes as an **anti-aliased quad** (six vertices,
/// two triangles) — whether it travels as a per-instance stream
/// ([`GpuBatch::instanced`]) or as pre-expanded per-batch quads
/// ([`GpuBatch::line_quad_count`]). An SDF conic batch draws `6` unit-quad
/// vertices per instance; a mesh charges its packed vertex count. This is the
/// documented unit change from "vertices = stored geometry" to "vertices =
/// rasterized primitives" (see `docs/render-order.md`).
///
/// `cull` charges **zero** for a run whose chunk the frame's view cull removed
/// (the run is not drawn): a culled instanced/conic run contributes no quads, and
/// a converted mesh batch charges its vertex count only while at least one of its
/// mesh runs survives (conservative for a partially culled split batch). Legacy
/// per-batch geometry (per-batch quads, a non-converted mesh) is never culled.
///
/// Byte charging is **unchanged** (batch-granular): `FrameBudget::max_bytes` is
/// `usize::MAX` in the product and models upload residency, not draw work, so
/// culling does not reduce it.
pub(crate) fn batch_budget_vertices(batch: &GpuBatch, cull: &ChunkCull) -> usize {
    let line_quads = batch
        .instanced
        .iter()
        .filter(|run| cull.stream_visible(run.stream as usize))
        .map(|run| run.count as usize)
        .sum::<usize>()
        + batch.line_quad_count as usize;
    let base = match batch.topology {
        RenderTopology::Conics => {
            batch
                .arc_runs
                .iter()
                .filter(|run| cull.arc_visible(run.stream as usize))
                .map(|run| run.count as usize)
                .sum::<usize>()
                * 6
        }
        RenderTopology::Mesh if mesh_batch_visible(batch, cull) => batch.vertex_count as usize,
        // A converted `Lines`/`MeshEdges` batch owns no legacy vertex buffer; its
        // whole charge is the expanded quads above. A fully culled converted mesh
        // charges nothing (falls through to `_`).
        _ => 0,
    };
    base + line_quads * 6
}

/// Triangles a resident batch charges the frame budget this frame: `2` per
/// anti-aliased line quad, `2` per SDF conic instance, the mesh triangle count,
/// else `0`. Culled runs are charged zero (see [`batch_budget_vertices`]).
pub(crate) fn batch_budget_triangles(batch: &GpuBatch, cull: &ChunkCull) -> usize {
    let line_quads = batch
        .instanced
        .iter()
        .filter(|run| cull.stream_visible(run.stream as usize))
        .map(|run| run.count as usize)
        .sum::<usize>()
        + batch.line_quad_count as usize;
    let base = match batch.topology {
        RenderTopology::Conics => {
            batch
                .arc_runs
                .iter()
                .filter(|run| cull.arc_visible(run.stream as usize))
                .map(|run| run.count as usize)
                .sum::<usize>()
                * 2
        }
        RenderTopology::Mesh if mesh_batch_visible(batch, cull) => batch.index_count as usize / 3,
        _ => 0,
    };
    base + line_quads * 2
}

/// Whether a converted mesh batch keeps any of its chunk runs under `cull`. A
/// batch with no mesh runs (a legacy, non-converted mesh) is never culled.
pub(super) fn mesh_batch_visible(batch: &GpuBatch, cull: &ChunkCull) -> bool {
    batch.mesh_runs.is_empty()
        || batch
            .mesh_runs
            .iter()
            .any(|run| cull.mesh_visible(run.chunk as usize))
}

/// Whether an accepted batch actually issues at least one draw under `cull`: a
/// converted batch needs a surviving run; a legacy per-batch draw always runs.
/// Used to decide whether a page is a real pacing cost observation.
pub(crate) fn batch_has_visible_work(batch: &GpuBatch, cull: &ChunkCull) -> bool {
    if batch.instanced.is_empty() && batch.mesh_runs.is_empty() && batch.arc_runs.is_empty() {
        return true;
    }
    batch
        .instanced
        .iter()
        .any(|run| cull.stream_visible(run.stream as usize))
        || batch
            .mesh_runs
            .iter()
            .any(|run| cull.mesh_visible(run.chunk as usize))
        || batch
            .arc_runs
            .iter()
            .any(|run| cull.arc_visible(run.stream as usize))
}

/// Plan one frame from the resident batches in upload order (the legacy planner,
/// also used by the pick path with [`ChunkCull::all_visible`]).
///
/// # Returned `usage` on failure (shared semantics)
///
/// `usage` is charged in place, so a failing batch leaves whatever the byte
/// charge already committed: on a **vertices/triangles** failure the failing
/// batch's bytes are included (its byte charge succeeded before the vertex check
/// ran); on a **bytes** failure nothing of the failing batch is included. This is
/// the single usage contract for both planners — `plan_ordered_page` and its
/// fallback now return the same value, so the two can be differentially compared
/// without a quirk delta.
pub(crate) fn plan_from_gpu(
    batches: &[GpuBatch],
    budget: &FrameBudget,
    initial: cad_scene::FrameUsage,
    cull: &ChunkCull,
) -> GpuPlan {
    let mut usage = initial;
    let mut accepted = Vec::with_capacity(batches.len());
    for (i, batch) in batches.iter().enumerate() {
        let vertices = batch_budget_vertices(batch, cull);
        let triangles = batch_budget_triangles(batch, cull);
        // The GPU bytes actually submitted: position, normal and edge-index
        // buffers plus the triangle index buffer. Derived from the packed GpuBatch
        // so `upload_bytes_per_frame` is charged the same numbers the device sees.
        let bytes = batch.upload_size_bytes();
        let outcome = budget
            .charge_bytes(&mut usage, bytes)
            .and_then(|()| budget.charge(&mut usage, vertices, triangles));
        match outcome {
            Ok(()) => accepted.push(i),
            Err(exceeded) => {
                let report = OverBudget {
                    category: exceeded.category,
                    requested: exceeded.requested,
                    limit: exceeded.limit,
                    skipped_batches: batches.len() - accepted.len(),
                };
                return GpuPlan {
                    accepted,
                    usage,
                    over_budget: Some(OverBudgetReason {
                        reason: over_budget_reason(&report),
                        report,
                    }),
                };
            }
        }
    }
    GpuPlan {
        accepted,
        usage,
        over_budget: None,
    }
}

/// Budget one consecutive page of a globally ordered scene. Unlike the legacy
/// one-frame planner, callers retain the cursor and never discard the suffix.
///
/// `initial` is the usage the page already reserved before geometry (the image
/// reservation on a clear page); the page budget covers geometry *plus* that
/// reservation.
///
/// # Segmented planning
///
/// The linear walk ([`plan_ordered_page_linear`]) is Θ(accepted+1) only while a
/// batch actually fails the budget. A **fully-culled** batch charges `0`
/// vertices and `0` triangles, and the product's `max_bytes == usize::MAX`, so a
/// page whose view cull removes almost everything accepts *every* batch and the
/// walk runs to the end of `ordered` — computing a per-batch vertex/triangle cost
/// for all of them — only to draw one run. This function walks only the **active**
/// batches (those whose charge is non-zero for this cull) and derives the accepted
/// prefix from their gaps. It is exact, not an approximation.
///
/// The active set for this frame is the union of:
/// * every batch that owns a run in a **visible** chunk (`BatchFacts::chunk_batches`,
///   indexed by class and chunk), and
/// * `BatchFacts::always_active` — batches that charge regardless of the cull.
///
/// A gap between two active positions is a batch that charges `0` vertices/triangles
/// and is therefore accepted unconditionally. Gaps still charge **bytes** (byte
/// charging is batch-granular and cull-independent), so the byte boundary is a
/// prefix-sum binary search over `ordered`, not part of the active walk.
///
/// `plan_ordered_page_linear` remains the fallback when nothing is culled (the
/// active set is the whole scene) or the estimated active count is large.
#[allow(clippy::too_many_arguments)]
pub(crate) fn plan_ordered_page(
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
    if cursor >= len {
        return GpuPlan {
            accepted: Vec::new(),
            usage: initial,
            over_budget: None,
        };
    }
    // Fallback (unchanged linear walk): when the cull removed nothing the active
    // set is the whole scene, so the segmented walk would do the same per-batch
    // work plus a sort; and when the estimated active count is large it is cheaper
    // to walk `ordered` linearly.
    if !cull.any_culled() || estimated_active(facts, cull) > SEGMENTED_ACTIVE_THRESHOLD {
        return plan_ordered_page_linear(batches, ordered, cursor, budget, initial, cull);
    }
    plan_ordered_page_segmented(
        batches, ordered, cursor, budget, initial, cull, facts, index,
    )
}

/// The linear page planner: walk `ordered[cursor..]` and accept a contiguous
/// prefix, stopping at the first batch that crosses the budget.
///
/// Kept as the segmented planner's fallback. The returned `usage` follows the
/// shared contract documented on [`plan_from_gpu`]: a vertices/triangles failure
/// keeps the failing batch's bytes, a bytes failure keeps none of it.
pub(crate) fn plan_ordered_page_linear(
    batches: &[GpuBatch],
    ordered: &[usize],
    cursor: usize,
    budget: &FrameBudget,
    initial: cad_scene::FrameUsage,
    cull: &ChunkCull,
) -> GpuPlan {
    let mut usage = initial;
    let mut accepted = Vec::new();
    for &index in &ordered[cursor..] {
        let batch = &batches[index];
        let mut next = usage;
        let triangles = batch_budget_triangles(batch, cull);
        let result = budget
            .charge_bytes(&mut next, batch.upload_size_bytes())
            .and_then(|()| budget.charge(&mut next, batch_budget_vertices(batch, cull), triangles));
        if let Err(exceeded) = result {
            let report = OverBudget {
                category: exceeded.category,
                requested: exceeded.requested,
                limit: exceeded.limit,
                skipped_batches: ordered.len() - cursor - accepted.len(),
            };
            return GpuPlan {
                accepted,
                // `next` carries the byte charge that already committed (none on
                // a byte failure, the failing batch's bytes on a vertex/triangle
                // failure), matching `plan_from_gpu`.
                usage: next,
                over_budget: Some(OverBudgetReason {
                    reason: over_budget_reason(&report),
                    report,
                }),
            };
        }
        usage = next;
        accepted.push(index);
    }
    GpuPlan {
        accepted,
        usage,
        over_budget: None,
    }
}

/// The **pre-amendment** linear page planner, kept only as the differential
/// test's reference for the accepted prefix and the `over_budget` report.
///
/// Its returned `usage` is the pre-batch usage (it excludes the failing batch
/// entirely); the production planner now returns the unified usage
/// ([`plan_from_gpu`] semantics). The test asserts the prefix/report are
/// unchanged and derives the expected unified usage from this one.
#[cfg(test)]
pub(crate) fn plan_ordered_page_reference(
    batches: &[GpuBatch],
    ordered: &[usize],
    cursor: usize,
    budget: &FrameBudget,
    initial: cad_scene::FrameUsage,
    cull: &ChunkCull,
) -> GpuPlan {
    let mut usage = initial;
    let mut accepted = Vec::new();
    for &index in &ordered[cursor..] {
        let batch = &batches[index];
        let mut next = usage;
        let triangles = batch_budget_triangles(batch, cull);
        let result = budget
            .charge_bytes(&mut next, batch.upload_size_bytes())
            .and_then(|()| budget.charge(&mut next, batch_budget_vertices(batch, cull), triangles));
        if let Err(exceeded) = result {
            let report = OverBudget {
                category: exceeded.category,
                requested: exceeded.requested,
                limit: exceeded.limit,
                skipped_batches: ordered.len() - cursor - accepted.len(),
            };
            return GpuPlan {
                accepted,
                usage,
                over_budget: Some(OverBudgetReason {
                    reason: over_budget_reason(&report),
                    report,
                }),
            };
        }
        usage = next;
        accepted.push(index);
    }
    GpuPlan {
        accepted,
        usage,
        over_budget: None,
    }
}
