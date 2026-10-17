//! Chunk-level view culling (backlog §P1).
//!
//! The three chunk builders already accumulate a world-space `f64` AABB while
//! they pack a chunk (used for their split heuristics). This module retains that
//! box on the uploaded chunk and tests it against the frame's own
//! [`FrameProjection`] once per frame, so an off-screen chunk's draws are skipped
//! and its vertex/triangle work is not charged to the page budget.
//!
//! # Unit of culling
//!
//! One **chunk**: an [`InstanceStream`](crate::InstanceStream) (instanced line
//! segments), a [`GpuMeshChunk`](crate::GpuMeshChunk) (merged mesh triangles) or
//! an [`ArcStream`](crate::ArcStream) (SDF conic instances). Every measured draw
//! is a coalesced run confined to one chunk, so culling a chunk removes whole
//! runs (see `docs/render-optimization-backlog.md` §P1).
//!
//! # Conservative by construction
//!
//! The predicate is one-sided: it culls only when the **padded** box is entirely
//! outside the view, so a chunk that could put any pixel on screen is always
//! kept. A non-finite, empty or inverted box is always visible (never culled).
//! An oversized source split across several chunks carries the **whole-batch**
//! AABB in every piece (conservative — every piece is kept while any part of the
//! batch is visible), and a chunk that mixes distant batches has a large box that
//! intersects almost any view, so it never culls. Both are deliberate.

use crate::plan::FrameProjection;

/// A world-space axis-aligned bounding box in the same `f64` coordinates the
/// chunk builders use. Stored, never mutated; the cull predicate pads it at test
/// time (padding is **never** baked into the stored box).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WorldAabb {
    pub(crate) min: [f64; 3],
    pub(crate) max: [f64; 3],
}

impl WorldAabb {
    /// A box that can never be culled: all-NaN fails the finite test below.
    #[cfg(test)]
    pub(crate) const NEVER_CULLED: WorldAabb = WorldAabb {
        min: [f64::NAN; 3],
        max: [f64::NAN; 3],
    };

    /// Every component finite.
    fn is_finite(&self) -> bool {
        self.min.iter().all(|c| c.is_finite()) && self.max.iter().all(|c| c.is_finite())
    }

    /// `min > max` on any axis (an empty/backwards box). A box with equal bounds
    /// is a point and is not inverted.
    fn is_inverted(&self) -> bool {
        (0..3).any(|axis| self.min[axis] > self.max[axis])
    }

    /// Largest half-extent across the three axes. For a full-circle conic AABB
    /// (side `2R` in the drawing plane) this is the conic radius `R`.
    fn max_half_extent(&self) -> f64 {
        (0..3)
            .map(|axis| (self.max[axis] - self.min[axis]) * 0.5)
            .fold(0.0, f64::max)
    }
}

/// Which chunk classes survive this frame's view test.
///
/// The identity mask ([`ChunkCull::all_visible`]) is empty and reports every
/// index visible, so the pick path can share the planners without ever culling.
pub(crate) struct ChunkCull {
    /// Per-class visibility bits (index = the class's chunk index).
    instance: Vec<bool>,
    mesh: Vec<bool>,
    arc: Vec<bool>,
    /// Total chunks culled this frame (across all three classes).
    culled: usize,
}

impl ChunkCull {
    /// The identity mask: nothing is culled. Used by the pick path, which must
    /// keep planning/drawing every run of accepted batches (see `pick.rs`).
    pub(crate) fn all_visible() -> Self {
        ChunkCull {
            instance: Vec::new(),
            mesh: Vec::new(),
            arc: Vec::new(),
            culled: 0,
        }
    }

    /// Whether instance stream `index` survives. An out-of-range index is
    /// reported visible (conservative).
    pub(crate) fn stream_visible(&self, index: usize) -> bool {
        self.instance.get(index).copied().unwrap_or(true)
    }

    /// Whether merged mesh chunk `index` survives (out-of-range = visible).
    pub(crate) fn mesh_visible(&self, index: usize) -> bool {
        self.mesh.get(index).copied().unwrap_or(true)
    }

    /// Whether SDF conic stream `index` survives (out-of-range = visible).
    pub(crate) fn arc_visible(&self, index: usize) -> bool {
        self.arc.get(index).copied().unwrap_or(true)
    }

    /// Whether any chunk was culled this frame. Gates the accelerated pulled
    /// organisation: a pulled draw redraws whole streams/chunks, so it must not
    /// run while culling excluded any of them.
    pub(crate) fn any_culled(&self) -> bool {
        self.culled > 0
    }

    /// Test hook: keep every chunk visible while retaining the chunk counts, so
    /// the reporting still reflects the resident chunk set. Used by
    /// [`Renderer::set_view_culling`](crate::Renderer::set_view_culling) to render
    /// the unculled reference frame.
    pub(crate) fn force_all_visible(mut self) -> Self {
        self.instance.iter_mut().for_each(|visible| *visible = true);
        self.mesh.iter_mut().for_each(|visible| *visible = true);
        self.arc.iter_mut().for_each(|visible| *visible = true);
        self.culled = 0;
        self
    }

    /// Chunks kept (visible) this frame.
    pub(crate) fn kept(&self) -> u64 {
        (self.total() - self.culled) as u64
    }

    /// Chunks tested this frame (`kept() + culled == total()`).
    pub(crate) fn total(&self) -> usize {
        self.instance.len() + self.mesh.len() + self.arc.len()
    }

    /// Chunks culled this frame.
    #[cfg(test)]
    pub(crate) fn culled(&self) -> u64 {
        self.culled as u64
    }
}

/// Build the frame's chunk mask from the retained world AABBs.
///
/// `wpp_line` is the line class's ramp width in world units per pixel (the test
/// hook override when set, else the projection's real value); `wpp_arc` is the
/// projection's real value (the SDF conic class never uses the override). The
/// three slices are indexed by the resident chunk order.
pub(crate) fn build(
    projection: FrameProjection,
    wpp_line: f64,
    wpp_arc: f64,
    instance: impl IntoIterator<Item = WorldAabb>,
    mesh: impl IntoIterator<Item = WorldAabb>,
    arc: impl IntoIterator<Item = WorldAabb>,
) -> ChunkCull {
    let mut culled = 0usize;
    let mut instance_visible = Vec::new();
    for aabb in instance {
        // AA line quads expand by `half = wpp + max(2·wpp, 1e-6)` beyond the
        // segment endpoints (`shaders/line_instanced.wgsl:95-97`,
        // `line.wgsl:105-109`, `line_pulled.wgsl:131-133`), i.e. at most
        // `3·wpp_line`; the `1e-6` covers the tiny-`wpp` floor.
        // `lineweight` is carried but **not rasterized**
        // (`shaders/line_instanced.wgsl:123-124`, `GpuBatch.lineweight`), so it
        // contributes no pad term.
        let visible = class_visible(projection, aabb, 3.0 * wpp_line + 1e-6);
        if !visible {
            culled += 1;
        }
        instance_visible.push(visible);
    }
    let mut mesh_visible = Vec::new();
    for aabb in mesh {
        // Mesh triangles are rasterized at their exact vertices: no stroke/quad
        // expansion, so the epsilon alone.
        let visible = class_visible(projection, aabb, 1e-6);
        if !visible {
            culled += 1;
        }
        mesh_visible.push(visible);
    }
    let mut arc_visible = Vec::new();
    for aabb in arc {
        // SDF conic quads grow by `grow = 1 + max(0.02, 2·wpp_arc/r)` about the
        // center (`shaders/arc.wgsl:122-123`), i.e. the full-circle extent
        // expands by `max(0.02·R, 2·wpp_arc)`. `2·wpp_arc + 0.02·R`
        // upper-bounds that (over-padding is safe).
        let radius = aabb.max_half_extent();
        let visible = class_visible(
            projection,
            aabb,
            2.0 * wpp_arc + (0.02 * radius).max(0.0) + 1e-6,
        );
        if !visible {
            culled += 1;
        }
        arc_visible.push(visible);
    }
    ChunkCull {
        instance: instance_visible,
        mesh: mesh_visible,
        arc: arc_visible,
        culled,
    }
}

/// Whether `aabb` (padded by `pad` on every axis) intersects the view.
fn class_visible(projection: FrameProjection, aabb: WorldAabb, pad: f64) -> bool {
    if !aabb.is_finite() || aabb.is_inverted() {
        // Never cull a box the predicate cannot reason about.
        return true;
    }
    let min = [aabb.min[0] - pad, aabb.min[1] - pad, aabb.min[2] - pad];
    let max = [aabb.max[0] + pad, aabb.max[1] + pad, aabb.max[2] + pad];
    match projection {
        FrameProjection::Ortho { sx, sy, center, .. } => ortho_visible(sx, sy, center, min, max),
        FrameProjection::Perspective { vp, .. } => {
            // Perspective narrows the chunk origin to `f32` before the
            // view-projection (`FrameProjection::batch_matrix_f64`), so the
            // rendered world position can differ from the stored `f64` box by
            // ~ulp(origin). Pad by that relative error before the plane test.
            let margin = perspective_margin(min, max);
            perspective_visible(&vp, min, max, margin)
        }
    }
}

/// Exact `f64` 2-axis test in world space against the NDC cube of the ortho
/// projection.
///
/// `ndc_x = sx·(x − center.x)` (and likewise `y`); a box is visible iff its
/// `x`-range and its `y`-range each overlap `[-1, 1]`. **`z` is not tested**:
/// the ortho clip `z` is `(z − center.z) + z_plane` (`FrameProjection::
/// batch_matrix_f64`) and the depth mapping is not a visibility criterion for the
/// 2D view — testing it could wrongly cull geometry that projects inside.
fn ortho_visible(sx: f64, sy: f64, center: [f64; 3], min: [f64; 3], max: [f64; 3]) -> bool {
    if !axis_overlaps(sx, min[0], max[0], center[0]) {
        return false;
    }
    if !axis_overlaps(sy, min[1], max[1], center[1]) {
        return false;
    }
    true
}

/// Whether `[lo, hi]` maps into an overlap of `[-1, 1]` under `ndc = scale·(x −
/// center)`. Robust to the sign of `scale` (the projection's scales are positive,
/// but the test does not assume it).
fn axis_overlaps(scale: f64, lo: f64, hi: f64, center: f64) -> bool {
    let a = scale * (lo - center);
    let b = scale * (hi - center);
    let (low, high) = if a <= b { (a, b) } else { (b, a) };
    high >= -1.0 && low <= 1.0
}

/// The f32-relative world-space margin for the perspective path: the origin is
/// narrowed to `f32`, so a stored `f64` coordinate can land ~`ulp(origin)` away.
/// `4×` the f32 epsilon times the largest coordinate is a safe upper bound.
fn perspective_margin(min: [f64; 3], max: [f64; 3]) -> f64 {
    let max_abs = min
        .iter()
        .chain(max.iter())
        .map(|c| c.abs())
        .fold(0.0, f64::max);
    4.0 * f64::from(f32::EPSILON) * max_abs
}

/// Six-plane frustum test for the perspective projection.
///
/// Each clip plane is a row combination of the view-projection; the box is culled
/// iff for some plane its **p-vertex** (the corner furthest along the plane
/// normal) is outside it — the exact axis-aligned box vs plane test. `margin`
/// expands the box before the test.
fn perspective_visible(vp: &[[f32; 4]; 4], min: [f64; 3], max: [f64; 3], margin: f64) -> bool {
    for (normal, offset) in clip_planes(vp) {
        // p-vertex: per axis, the bound that maximizes `normal · corner`.
        let mut p = [0.0f64; 3];
        for axis in 0..3 {
            p[axis] = if normal[axis] >= 0.0 {
                max[axis] + margin
            } else {
                min[axis] - margin
            };
        }
        let signed = normal[0] * p[0] + normal[1] * p[1] + normal[2] * p[2] + offset;
        if signed < 0.0 {
            // Entirely outside this plane: cull.
            return false;
        }
    }
    true
}

/// The six clip planes in world space as `(normal, offset)`, each satisfied when
/// `normal · point + offset >= 0`. Column-major `vp` (`vp[col][row]`).
///
/// wgpu clip space: `-w <= x <= w`, `-w <= y <= w`, `0 <= z <= w`.
fn clip_planes(vp: &[[f32; 4]; 4]) -> [([f64; 3], f64); 6] {
    let c = |col: usize, row: usize| f64::from(vp[col][row]);
    // The clip components are the matrix **rows** (`row(r) = [vp[0][r],
    // vp[1][r], vp[2][r], vp[3][r]]`); `vp` is column-major.
    let row = |i: usize| [c(0, i), c(1, i), c(2, i), c(3, i)];
    let (x, y, z, w) = (row(0), row(1), row(2), row(3));
    let add = |a: [f64; 4], b: [f64; 4]| [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]];
    let sub = |a: [f64; 4], b: [f64; 4]| [a[0] - b[0], a[1] - b[1], a[2] - b[2], a[3] - b[3]];
    let plane = |row: [f64; 4]| ([row[0], row[1], row[2]], row[3]);
    [
        plane(add(x, w)), // left:   x + w >= 0
        plane(sub(w, x)), // right:  w - x >= 0
        plane(add(y, w)), // bottom: y + w >= 0
        plane(sub(w, y)), // top:    w - y >= 0
        plane(z),         // near:   z >= 0
        plane(sub(w, z)), // far:    w - z >= 0
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ortho(sx: f64, sy: f64, center: [f64; 3]) -> FrameProjection {
        FrameProjection::Ortho {
            sx,
            sy,
            center,
            z_plane: 0.0,
            world_per_px: 1.0 / sx.max(sy),
        }
    }

    /// A right-handed perspective looking down `-z` from `(0, 0, eye)` at a target
    /// `(0, 0, 0)`, `fov` radians, aspect `a`, near `n`, far `f`, in wgpu clip
    /// space (`z` in `[0, w]`). Column-major.
    fn perspective(eye: f32, fov: f32, aspect: f32, n: f32, f: f32) -> FrameProjection {
        let t = (fov * 0.5).tan();
        let a = f / (n - f);
        let b = (f * n) / (n - f);
        let vp = [
            [1.0 / (aspect * t), 0.0, 0.0, 0.0],
            [0.0, 1.0 / t, 0.0, 0.0],
            [0.0, 0.0, a, -1.0],
            [0.0, 0.0, b - a * eye, eye],
        ];
        FrameProjection::Perspective {
            vp,
            world_per_px: 1.0 / 64.0,
        }
    }

    fn box_at(cx: f64, cy: f64, half: f64) -> WorldAabb {
        WorldAabb {
            min: [cx - half, cy - half, -half],
            max: [cx + half, cy + half, half],
        }
    }

    #[test]
    fn ortho_culls_a_chunk_entirely_outside_and_keeps_one_inside() {
        // sx = sy = 1 -> visible world x,y in [-1, 1].
        let p = ortho(1.0, 1.0, [0.0, 0.0, 0.0]);
        let inside = build(p, 0.0, 0.0, [box_at(0.0, 0.0, 0.5)], [], []);
        assert_eq!((inside.kept(), inside.culled(), inside.total()), (1, 0, 1));

        let outside = build(p, 0.0, 0.0, [box_at(5.0, 5.0, 0.1)], [], []);
        assert_eq!(
            (outside.kept(), outside.culled(), outside.total()),
            (0, 1, 1)
        );
        assert!(outside.any_culled());
    }

    /// (a) The line pad: a chunk whose raw box misses the viewport but whose
    /// `3·wpp` band reaches into it must be kept.
    #[test]
    fn line_pad_keeps_a_chunk_whose_stroke_band_touches_the_view() {
        let p = ortho(1.0, 1.0, [0.0, 0.0, 0.0]);
        // Right edge at x = 1.0 (visible). A chunk whose raw box starts just past
        // it (min.x = 1.05) is culled with no pad...
        let raw = WorldAabb {
            min: [1.05, -0.1, -0.1],
            max: [1.5, 0.1, 0.1],
        };
        let without = build(p, 0.0, 0.0, [raw], [], []);
        assert_eq!(without.culled(), 1, "raw box is fully off-screen");
        // ...but kept once the 3·wpp stroke band reaches back across the edge.
        let wpp = 0.02; // 3·wpp = 0.06 >= 0.05.
        let with = build(p, wpp, wpp, [raw], [], []);
        assert_eq!(with.kept(), 1, "the stroke band touches the viewport");
    }

    /// (a) The arc pad: `2·wpp + 0.02·R` (upper-bounds `max(0.02·R, 2·wpp)`).
    #[test]
    fn arc_pad_uses_the_conic_radius_and_wpp() {
        let p = ortho(1.0, 1.0, [0.0, 0.0, 0.0]);
        // A full circle of radius R = 1 centered at x = 2.0: extent [1.0, 3.0] in
        // x, so the raw box already touches the view at x = 1.0. Move it out so
        // only the pad can save it.
        let r = 1.0f64;
        let center_x = 2.05;
        let aabb = WorldAabb {
            min: [center_x - r, -r, -r],
            max: [center_x + r, r, r],
        };
        // Raw box: [1.05, 3.05] -> off-screen (culled).
        assert_eq!(build(p, 0.0, 0.0, [], [], [aabb]).culled(), 1);
        // 0.02·R = 0.02 < 0.05 gap, so the radius term alone is not enough...
        // ...but the SDF fringe floor `2·wpp` (with wpp = 0.05) covers it.
        let arc = build(p, 0.0, 0.05, [], [], [aabb]);
        assert_eq!(arc.kept(), 1, "2·wpp fringe reaches the viewport");
        // And a radius large enough that 0.02·R alone closes the gap also keeps.
        let big = WorldAabb {
            min: [center_x - 10.0, -10.0, -10.0],
            max: [center_x + 10.0, 10.0, 10.0],
        };
        let arc_big = build(p, 0.0, 0.0, [], [], [big]);
        assert_eq!(arc_big.kept(), 1, "0.02·R closes the gap");
    }

    /// (b) Soundness, one-sided: for randomised boxes, a padded box that
    /// intersects the view is never culled.
    #[test]
    fn soundness_intersecting_chunks_are_never_culled() {
        let p = ortho(2.0, 2.0, [0.25, -0.5, 0.0]);
        // Deterministic pseudo-random sweep (no external RNG).
        let mut state = 0x1234_5678_9abc_def0u64;
        let mut next = || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((state >> 11) as f64) / ((1u64 << 53) as f64)
        };
        for _ in 0..2000 {
            let cx = (next() * 40.0) - 20.0;
            let cy = (next() * 40.0) - 20.0;
            let hx = next() * 2.0;
            let hy = next() * 2.0;
            let aabb = WorldAabb {
                min: [cx - hx, cy - hy, -1.0],
                max: [cx + hx, cy + hy, 1.0],
            };
            // Visible world x = center.x ± 1/sx = ±0.5; y = center.y ± 0.5.
            let vis = build(p, 0.0, 0.0, [aabb], [], []);
            let intersects = (0..2).all(|axis| {
                let (lo, hi) = (aabb.min[axis], aabb.max[axis]);
                let (c, half) = if axis == 0 { (0.25, 0.5) } else { (-0.5, 0.5) };
                hi >= c - half && lo <= c + half
            });
            if intersects {
                assert_eq!(
                    vis.kept(),
                    1,
                    "an intersecting chunk must never be culled: {aabb:?}"
                );
            }
        }
    }

    /// (c) Non-finite / inverted boxes are always visible.
    #[test]
    fn non_finite_and_inverted_boxes_are_never_culled() {
        let p = ortho(1.0, 1.0, [0.0, 0.0, 0.0]);
        let nan = WorldAabb::NEVER_CULLED;
        let inverted = WorldAabb {
            min: [10.0, 10.0, 10.0],
            max: [-10.0, -10.0, -10.0],
        };
        let inf = WorldAabb {
            min: [f64::NEG_INFINITY; 3],
            max: [f64::INFINITY; 3],
        };
        let cull = build(p, 0.0, 0.0, [nan, inverted, inf], [], []);
        assert_eq!(cull.kept(), 3);
        assert!(!cull.any_culled());
    }

    /// (c) An oversized chunk (a box spanning far past the view) is always kept.
    #[test]
    fn oversized_chunk_is_kept() {
        let p = ortho(1.0, 1.0, [0.0, 0.0, 0.0]);
        let huge = WorldAabb {
            min: [-1.0e9, -1.0e9, -1.0e9],
            max: [1.0e9, 1.0e9, 1.0e9],
        };
        let cull = build(p, 0.0, 0.0, [huge], [], []);
        assert_eq!(cull.kept(), 1);
    }

    /// (d) Counter contract: `kept + culled == total`.
    #[test]
    fn counters_partition_the_chunk_set() {
        let p = ortho(1.0, 1.0, [0.0, 0.0, 0.0]);
        let instance = [box_at(0.0, 0.0, 0.2), box_at(9.0, 0.0, 0.2)];
        let mesh = [
            box_at(0.0, 0.0, 0.2),
            box_at(0.0, 9.0, 0.2),
            box_at(0.0, 0.0, 0.2),
        ];
        let arc = [box_at(9.0, 9.0, 0.2)];
        let cull = build(p, 0.0, 0.0, instance, mesh, arc);
        assert_eq!(cull.total(), 6);
        assert_eq!(cull.kept() + cull.culled(), cull.total() as u64);
        assert_eq!(cull.culled(), 3);
        assert_eq!(cull.kept(), 3);
    }

    /// The identity mask never culls and reports every index visible.
    #[test]
    fn all_visible_mask_never_culls() {
        let cull = ChunkCull::all_visible();
        assert!(!cull.any_culled());
        assert_eq!(cull.total(), 0);
        assert!(cull.stream_visible(0));
        assert!(cull.stream_visible(999));
        assert!(cull.mesh_visible(5));
        assert!(cull.arc_visible(5));
    }

    /// Perspective: a box in front of the camera is kept; one behind it or far to
    /// the side is culled.
    #[test]
    fn perspective_culls_behind_and_off_axis_boxes() {
        // Eye at z = 5 looking toward -z; near 0.1, far 100, fov 90°.
        let p = perspective(5.0, std::f32::consts::FRAC_PI_2, 1.0, 0.1, 100.0);
        let front = WorldAabb {
            min: [-0.5, -0.5, -0.5],
            max: [0.5, 0.5, 0.5],
        };
        assert_eq!(build(p, 0.0, 0.0, [front], [], []).kept(), 1);
        // Behind the camera (z > eye): culled.
        let behind = WorldAabb {
            min: [-0.5, -0.5, 6.0],
            max: [0.5, 0.5, 7.0],
        };
        assert_eq!(build(p, 0.0, 0.0, [behind], [], []).culled(), 1);
        // Far off to the side at the target depth: culled.
        let aside = WorldAabb {
            min: [50.0, -0.5, -0.5],
            max: [51.0, 0.5, 0.5],
        };
        assert_eq!(build(p, 0.0, 0.0, [aside], [], []).culled(), 1);
    }

    /// (a) Perspective f32-margin: a box a hair past a clip plane would be culled
    /// without the margin but is kept once the f32-relative margin (covering the
    /// origin narrowing) is applied.
    #[test]
    fn perspective_f32_margin_keeps_a_grazing_box() {
        let p = perspective(5.0, std::f32::consts::FRAC_PI_2, 1.0, 0.1, 100.0);
        // At the target plane (z = 0, distance 5) the visible half-width is 5.
        // A flat box a hair past the right plane:
        let graze = WorldAabb {
            min: [5.0 + 5.0e-7, -0.01, 0.0],
            max: [5.0 + 6.0e-7, 0.01, 0.0],
        };
        let FrameProjection::Perspective { vp, .. } = p else {
            unreachable!("perspective helper")
        };
        assert!(
            !perspective_visible(&vp, graze.min, graze.max, 0.0),
            "without a margin the grazing box is culled"
        );
        let m = perspective_margin(graze.min, graze.max);
        assert!(
            perspective_visible(&vp, graze.min, graze.max, m),
            "the f32 margin covers the narrowing error"
        );
        // The margin grows with the coordinate magnitude.
        let big = perspective_margin([1.0e7; 3], [1.0e7 + 1.0; 3]);
        assert!(big > 1.0, "margin scales with magnitude: {big}");
    }
}
