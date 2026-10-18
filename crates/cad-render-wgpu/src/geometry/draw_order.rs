use super::*;

/// An explicit over-budget report produced by [`plan_frame`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverBudget {
    /// `"vertices"`, `"triangles"` or `"bytes"`.
    pub category: &'static str,
    pub requested: usize,
    pub limit: usize,
    /// How many batches were not submitted because the budget ran out.
    pub skipped_batches: usize,
}

// ---------------------------------------------------------------------------
// Draw order + transparency policy (F14)
// ---------------------------------------------------------------------------
//
// These are pure CPU decisions so they can be tested without a GPU. The renderer
// feeds them the same values it uploads, and the two draw passes follow the
// resulting plan. The full policy, including the evidence boundary, is written
// in `docs/render-order.md`.

/// Alpha bucket a batch falls into after clamping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlphaClass {
    /// `alpha >= 1`: drawn in the opaque pass, depth order controlled by the
    /// depth buffer.
    Opaque,
    /// `0 < alpha < 1`: drawn after opaque geometry, back-to-front by centroid
    /// distance so blending composites correctly.
    Transparent,
    /// `alpha <= 0`: contributes nothing; it is skipped and reported, never
    /// silently counted as a normal draw.
    Invisible,
}

/// Clamp a batch alpha into `[0, 1]`.
///
/// Policy for out-of-range and non-finite input, documented in
/// `docs/render-order.md`:
/// * `NaN` (and infinities are clamped by the same rule) is treated as fully
///   **opaque** (`1.0`) — the conservative choice: an unreadable opacity must not
///   make geometry disappear;
/// * `alpha < 0` clamps to `0` (invisible), `alpha > 1` clamps to `1` (opaque).
pub fn clamp_alpha(alpha: f32) -> f32 {
    if alpha.is_nan() {
        return 1.0;
    }
    alpha.clamp(0.0, 1.0)
}

/// Bucket a raw alpha value (it is clamped first).
pub fn classify_alpha(alpha: f32) -> AlphaClass {
    let a = clamp_alpha(alpha);
    if a <= 0.0 {
        AlphaClass::Invisible
    } else if a >= 1.0 {
        AlphaClass::Opaque
    } else {
        AlphaClass::Transparent
    }
}

/// The ordering-relevant view of one batch.
///
/// `centroid` is world-space; `draw_order` is the caller's paint order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BatchOrderEntry {
    pub draw_order: i64,
    pub alpha: f32,
    /// World-space centroid, used only by the transparent back-to-front sort.
    pub centroid: [f32; 3],
}

/// Indices of a batch set partitioned and ordered for submission.
///
/// All three vectors hold indices into the slice passed to [`plan_draw_order`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawOrderPlan {
    /// Opaque batches, ascending `draw_order` (stable: upload order breaks ties).
    pub opaque: Vec<usize>,
    /// Transparent (`0 < alpha < 1`) batches. Back-to-front by centroid distance
    /// from the camera when one is supplied; otherwise ascending `draw_order`.
    pub transparent: Vec<usize>,
    /// `alpha <= 0` batches: not drawn at all.
    pub invisible: Vec<usize>,
}

impl DrawOrderPlan {
    /// Number of batches that will actually be submitted.
    pub fn drawn(&self) -> usize {
        self.opaque.len() + self.transparent.len()
    }
}

/// Stable draw-order plan for a frame.
///
/// Tie-break, documented and tested:
/// 1. opaque batches sort by ascending `draw_order`;
/// 2. transparent batches sort back-to-front by descending squared distance from
///    `camera` (so the farthest batch is drawn first);
/// 3. equal keys keep the input (upload) order, because every sort is stable and
///    the input index is the final comparator term.
///
/// `camera = None` means the caller has no camera position (for example a 2D
/// frame without a depth axis): transparent batches then fall back to ascending
/// `draw_order` only. This is deterministic but **not** depth-correct; the caller
/// can detect the fallback by inspecting the returned plan and the renderer
/// records it as an explicit limitation.
pub fn plan_draw_order(entries: &[BatchOrderEntry], camera: Option<[f32; 3]>) -> DrawOrderPlan {
    let mut opaque = Vec::new();
    let mut transparent = Vec::new();
    let mut invisible = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        match classify_alpha(entry.alpha) {
            AlphaClass::Opaque => opaque.push(index),
            AlphaClass::Transparent => transparent.push(index),
            AlphaClass::Invisible => invisible.push(index),
        }
    }

    // 1. Opaque: ascending draw order. `sort_by_key` is stable, so equal keys
    //    retain upload order (the input index order).
    opaque.sort_by_key(|&index| entries[index].draw_order);

    // 2. Transparent: back-to-front, then draw order, then upload index.
    match camera {
        Some(eye) => transparent.sort_by(|&a, &b| {
            let da = squared_distance(entries[a].centroid, eye);
            let db = squared_distance(entries[b].centroid, eye);
            // Descending distance: farther batches first so nearer ones blend on
            // top. `partial_cmp` falls back to `Equal` on a non-finite distance,
            // and the draw_order/index terms keep the result deterministic.
            db.partial_cmp(&da)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| entries[a].draw_order.cmp(&entries[b].draw_order))
                .then_with(|| a.cmp(&b))
        }),
        None => transparent.sort_by_key(|&index| entries[index].draw_order),
    }

    DrawOrderPlan {
        opaque,
        transparent,
        invisible,
    }
}

/// Camera-independent prefix of a frame's draw-order plan.
///
/// The opaque order and the opaque/transparent/invisible partition depend only
/// on the batch set (which changes on upload), while only the transparent order
/// depends on the camera. Caching the partition lets a camera move skip the
/// O(n log n) opaque sort and the partition scan, and re-sort only the (usually
/// small) transparent slice.
///
/// Equivalence to [`plan_draw_order`] is the contract and is fuzzed by
/// `cached_partition_matches_fresh_plan_across_cameras`: the transparent
/// comparator is a total order (distance, then draw order, then upload index),
/// so re-sorting a cached subset yields the same sequence as sorting the
/// freshly partitioned subset.
#[derive(Debug, Clone)]
pub struct DrawOrderCache {
    entries: Vec<BatchOrderEntry>,
    opaque: Vec<usize>,
    transparent: Vec<usize>,
    invisible: Vec<usize>,
}

impl DrawOrderCache {
    /// Build the camera-independent partition over `entries` in upload order.
    ///
    /// Both sorted groups start in ascending `draw_order` order (stable, upload
    /// index breaks ties), which is exactly what [`plan_draw_order`] produces
    /// with `camera = None`.
    pub fn new(entries: Vec<BatchOrderEntry>) -> Self {
        let mut cache = Self {
            entries,
            opaque: Vec::new(),
            transparent: Vec::new(),
            invisible: Vec::new(),
        };
        for (index, entry) in cache.entries.iter().enumerate() {
            match classify_alpha(entry.alpha) {
                AlphaClass::Opaque => cache.opaque.push(index),
                AlphaClass::Transparent => cache.transparent.push(index),
                AlphaClass::Invisible => cache.invisible.push(index),
            }
        }
        let entries = &cache.entries;
        cache.opaque.sort_by_key(|&index| entries[index].draw_order);
        cache
            .transparent
            .sort_by_key(|&index| entries[index].draw_order);
        cache
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Opaque batches in final order (ascending `draw_order`).
    pub fn opaque(&self) -> &[usize] {
        &self.opaque
    }

    /// Transparent batches in their current order. Call
    /// [`DrawOrderCache::sort_transparent`] after a camera change.
    pub fn transparent(&self) -> &[usize] {
        &self.transparent
    }

    /// `alpha <= 0` batch indices, never drawn.
    pub fn invisible(&self) -> &[usize] {
        &self.invisible
    }

    /// Re-sort only the transparent slice for `camera`. Opaque and invisible are
    /// already final; this is the only camera-dependent step of the plan.
    pub fn sort_transparent(&mut self, camera: Option<[f32; 3]>) {
        let entries = &self.entries;
        match camera {
            Some(eye) => self.transparent.sort_by(|&a, &b| {
                let da = squared_distance(entries[a].centroid, eye);
                let db = squared_distance(entries[b].centroid, eye);
                db.partial_cmp(&da)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| entries[a].draw_order.cmp(&entries[b].draw_order))
                    .then_with(|| a.cmp(&b))
            }),
            None => self.transparent.sort_by(|&a, &b| {
                // Ascending `draw_order`, then upload index. The explicit index
                // term makes this a total order, so re-sorting a slice that is
                // no longer in upload order (for example after a camera sort)
                // still matches `plan_draw_order(.., None)`.
                entries[a]
                    .draw_order
                    .cmp(&entries[b].draw_order)
                    .then_with(|| a.cmp(&b))
            }),
        }
    }

    /// Owned snapshot of the current plan.
    pub fn to_plan(&self) -> DrawOrderPlan {
        DrawOrderPlan {
            opaque: self.opaque.clone(),
            transparent: self.transparent.clone(),
            invisible: self.invisible.clone(),
        }
    }
}
