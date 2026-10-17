//! Time-driven adaptive page-budget controller (pure; no wgpu types).
//!
//! The page budget used to be a static [`FrameBudget`] (8M vertices / 2M
//! triangles). Measured on `fixtures/complex-test.dwg` a progressive page spent
//! `cpu_ms + wait_ms = 27.7` per frame, which is not a frame anymore, and the
//! settled frame was reached only after many pages. The product accepted
//! incomplete frames during camera motion, so the page size may adapt; the
//! settled frame must not change (the cursor still walks the whole ordered
//! scene, see [`crate::Renderer`]).
//!
//! This module is the whole decision surface:
//!
//! * the signal is the **renderer-measured** `cpu_ms + wait_ms` of an observed
//!   frame, never tick-to-tick wall intervals (those are vsync-quantised and
//!   drive a limit cycle);
//! * an EWMA (`0.25 * new + 0.75 * old`) smooths the signal; the first
//!   observation seeds it, so there is no shrink before the third observed
//!   frame;
//! * a ±5% deadband ignores noise around the target;
//! * `cost > target * 1.05` for **2** consecutive frames multiplies the scale
//!   by `0.75` (fast down);
//! * `cost < target * 0.95` for **8** consecutive frames multiplies it by
//!   `1.08` (slow up);
//! * a direction change resets the opposite streak;
//! * `scale ∈ [0.05, 4.0]` and applies to `max_vertices` *and* `max_triangles`
//!   (keeping their ratio); `max_bytes` is left untouched.
//!
//! [`step`] is a pure function of `(state, sample)`, so the adaptation is unit
//! tested without a GPU.

use cad_scene::FrameBudget;

/// Un-paced product ceiling on vertices per frame (the static pre-pacing budget).
pub const DEFAULT_MAX_VERTICES: usize = 8_000_000;
/// Un-paced product ceiling on triangles per frame (the static pre-pacing budget).
pub const DEFAULT_MAX_TRIANGLES: usize = 2_000_000;
/// Un-paced product ceiling on upload bytes per frame (not enforced in product).
pub const DEFAULT_MAX_BYTES: usize = usize::MAX;
/// Default target for `cpu_ms + wait_ms`: ~13 ms leaves headroom inside 16.7 ms.
pub const DEFAULT_TARGET_MS: f64 = 13.0;
/// Lower clamp on the adaptive scale.
pub const MIN_SCALE: f64 = 0.05;
/// Upper clamp on the adaptive scale.
pub const MAX_SCALE: f64 = 4.0;
/// Weight of a new observation in the cost EWMA.
pub const EWMA_NEW_WEIGHT: f64 = 0.25;
/// Upper deadband edge, as a factor of the target.
pub const OVER_THRESHOLD: f64 = 1.05;
/// Lower deadband edge, as a factor of the target.
pub const UNDER_THRESHOLD: f64 = 0.95;
/// Consecutive over-target observations before one fast-down step.
pub const OVER_STREAK_LIMIT: u32 = 2;
/// Consecutive under-target observations before one slow-up step.
pub const UNDER_STREAK_LIMIT: u32 = 8;
/// Fast-down gain.
pub const DOWN_GAIN: f64 = 0.75;
/// Slow-up gain.
pub const UP_GAIN: f64 = 1.08;
/// Identifier carried by an over-budget reason produced by the progress floor.
///
/// The diagnostic **code** stays `render.frame_over_budget`; this is the
/// category parameter that says the frame crossed the *paced* page budget only
/// because a floor batch was forced to make progress.
pub const PACING_FLOOR_REASON: &str = "pacing-floor";

/// The observable pacing regime reported on [`crate::FrameStats::pacing`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacingMode {
    /// Progressive rendering with a usable completion signal: the page budget
    /// is the scaled budget and every observed frame feeds the controller.
    Active,
    /// No completion signal exists (the wasm non-blocking poll path), so the
    /// measured wait is not a frame cost. The static ceiling is used unchanged
    /// (bit-identical to the pre-pacing behaviour).
    DisabledNoCompletionSignal,
    /// The renderer is not paging (non-progressive), so the whole scene is
    /// planned against the static ceiling. Nothing to pace.
    AtCeiling,
}

impl PacingMode {
    /// Stable lowercase name for `FrameStats::pacing` and host probe lines.
    pub fn as_str(self) -> &'static str {
        match self {
            PacingMode::Active => "active",
            PacingMode::DisabledNoCompletionSignal => "disabled-no-completion-signal",
            PacingMode::AtCeiling => "at-ceiling",
        }
    }
}

/// Adaptive controller state.
///
/// `seeded == false` means no observation has been folded in yet; the next
/// accepted sample seeds [`PacingState::cost_ema_ms`] instead of adapting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PacingState {
    /// Multiplier applied to the un-paced vertex/triangle ceiling.
    pub scale: f64,
    /// EWMA of the observed `cpu_ms + wait_ms`, in milliseconds.
    pub cost_ema_ms: f64,
    /// Target for `cpu_ms + wait_ms`, in milliseconds.
    pub target_ms: f64,
    /// Consecutive over-target frames (reset by the deadband/opposite streak).
    pub over_streak: u32,
    /// Consecutive under-target frames (reset by the deadband/opposite streak).
    pub under_streak: u32,
    /// Whether the EWMA has been seeded from a real observation.
    pub seeded: bool,
}

impl PacingState {
    /// A fresh controller at `scale = 1.0` with no cost knowledge. An invalid
    /// (non-finite or non-positive) target falls back to
    /// [`DEFAULT_TARGET_MS`] rather than producing a nonsense budget.
    pub fn new(target_ms: f64) -> Self {
        PacingState {
            scale: 1.0,
            cost_ema_ms: 0.0,
            target_ms: sanitize_target_ms(target_ms),
            over_streak: 0,
            under_streak: 0,
            seeded: false,
        }
    }
}

impl Default for PacingState {
    fn default() -> Self {
        PacingState::new(DEFAULT_TARGET_MS)
    }
}

/// One observed frame's cost signal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    /// Renderer-measured CPU milliseconds (plan/uniform/encode/submit).
    pub cpu_ms: f64,
    /// Renderer-measured blocking-poll milliseconds (GPU wait).
    pub wait_ms: f64,
    /// Batches accepted by this frame's page plan. A frame that accepted no
    /// batch is not a cost observation (it drew nothing to pay for).
    pub accepted: usize,
    /// The controller's cost knowledge was reset before this frame (a
    /// base-changing scene mutation / `clear_batches` / a new device generation).
    /// This sample seeds the EWMA and does **not** adapt. A camera move is *not*
    /// a restart, and neither is a suffix-only overlay commit: cost knowledge is
    /// scene-specific, not view-specific, and a suffix commit leaves the base
    /// draw-cost profile unchanged.
    pub restart: bool,
}

/// Sanitise a user-supplied target: finite and positive, else the default.
pub fn sanitize_target_ms(target_ms: f64) -> f64 {
    if target_ms.is_finite() && target_ms > 0.0 {
        target_ms
    } else {
        DEFAULT_TARGET_MS
    }
}

fn sanitize_ms(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        0.0
    }
}

fn scaled_limit(limit: usize, scale: f64) -> usize {
    // `usize::MAX` must stay unenforced instead of losing precision through f64.
    if limit == usize::MAX {
        return usize::MAX;
    }
    let scaled = (limit as f64 * scale).round();
    if !scaled.is_finite() || scaled <= 0.0 {
        1
    } else {
        (scaled as usize).max(1)
    }
}

/// Apply `scale` to an un-paced [`FrameBudget`], preserving the vertex/triangle
/// ratio and the byte limit.
///
/// The renderer uses this against its own configurable `Renderer::frame_budget`
/// (tests inject tiny ceilings); [`step`] additionally returns the budget
/// derived from the product defaults so the pure controller is self-contained.
pub fn apply_scale(ceiling: &FrameBudget, scale: f64) -> FrameBudget {
    let scale = if scale.is_finite() {
        scale.clamp(MIN_SCALE, MAX_SCALE)
    } else {
        1.0
    };
    FrameBudget {
        max_vertices: scaled_limit(ceiling.max_vertices, scale),
        max_triangles: scaled_limit(ceiling.max_triangles, scale),
        max_bytes: ceiling.max_bytes,
    }
}

/// The canonical product page budget for `scale` (8M/2M, bytes unenforced).
pub fn canonical_budget(scale: f64) -> FrameBudget {
    apply_scale(
        &FrameBudget {
            max_vertices: DEFAULT_MAX_VERTICES,
            max_triangles: DEFAULT_MAX_TRIANGLES,
            max_bytes: DEFAULT_MAX_BYTES,
        },
        scale,
    )
}

/// Fold one observed frame into the controller and return the next state plus
/// the canonical product budget for its scale.
///
/// A frame that accepted no batch ([`Sample::accepted`] `== 0`) is ignored: it
/// performed no page work, so its cost is not a signal about page size.
pub fn step(mut state: PacingState, sample: Sample) -> (PacingState, FrameBudget) {
    if sample.accepted == 0 {
        return (state, canonical_budget(state.scale));
    }
    let cost = sanitize_ms(sample.cpu_ms) + sanitize_ms(sample.wait_ms);
    // First observation seeds the EWMA; no shrink before the third observed
    // frame (a shrink needs two over-target samples after the seed).
    if !state.seeded || sample.restart {
        state.cost_ema_ms = cost;
        state.seeded = true;
        state.over_streak = 0;
        state.under_streak = 0;
        return (state, canonical_budget(state.scale));
    }
    state.cost_ema_ms = EWMA_NEW_WEIGHT * cost + (1.0 - EWMA_NEW_WEIGHT) * state.cost_ema_ms;
    let high = state.target_ms * OVER_THRESHOLD;
    let low = state.target_ms * UNDER_THRESHOLD;
    if state.cost_ema_ms > high {
        state.over_streak = state.over_streak.saturating_add(1);
        state.under_streak = 0;
        if state.over_streak >= OVER_STREAK_LIMIT {
            state.scale = (state.scale * DOWN_GAIN).clamp(MIN_SCALE, MAX_SCALE);
            state.over_streak = 0;
        }
    } else if state.cost_ema_ms < low {
        state.under_streak = state.under_streak.saturating_add(1);
        state.over_streak = 0;
        if state.under_streak >= UNDER_STREAK_LIMIT {
            state.scale = (state.scale * UP_GAIN).clamp(MIN_SCALE, MAX_SCALE);
            state.under_streak = 0;
        }
    } else {
        // Inside the deadband: neither direction is asserting itself.
        state.over_streak = 0;
        state.under_streak = 0;
    }
    (state, canonical_budget(state.scale))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(cost_ms: f64) -> Sample {
        Sample {
            cpu_ms: cost_ms,
            wait_ms: 0.0,
            accepted: 1,
            restart: false,
        }
    }

    #[test]
    fn canonical_budget_matches_the_product_ceiling() {
        let budget = canonical_budget(1.0);
        assert_eq!(budget.max_vertices, 8_000_000);
        assert_eq!(budget.max_triangles, 2_000_000);
        assert_eq!(budget.max_bytes, usize::MAX);
    }

    #[test]
    fn apply_scale_preserves_the_ratio_and_the_byte_limit() {
        let ceiling = FrameBudget {
            max_vertices: 8_000_000,
            max_triangles: 2_000_000,
            max_bytes: 4_000_000,
        };
        let half = apply_scale(&ceiling, 0.5);
        assert_eq!(half.max_vertices, 4_000_000);
        assert_eq!(half.max_triangles, 1_000_000);
        assert_eq!(
            half.max_bytes, 4_000_000,
            "byte semantics must not be scaled"
        );
        // The 4:1 vertex/triangle ratio is preserved.
        assert_eq!(
            half.max_vertices / half.max_triangles,
            ceiling.max_vertices / ceiling.max_triangles
        );
        // Over-large scale saturates instead of overflowing.
        let huge = apply_scale(
            &FrameBudget {
                max_vertices: usize::MAX,
                max_triangles: usize::MAX,
                max_bytes: usize::MAX,
            },
            4.0,
        );
        assert_eq!(huge.max_vertices, usize::MAX);
        assert_eq!(huge.max_triangles, usize::MAX);
    }

    #[test]
    fn first_observation_seeds_without_shrinking() {
        let (state, budget) = step(PacingState::default(), sample(100.0));
        assert!(state.seeded);
        assert_eq!(state.cost_ema_ms, 100.0);
        assert_eq!(state.scale, 1.0, "seeding must not adapt the scale");
        assert_eq!(budget.max_vertices, 8_000_000);
    }

    #[test]
    fn no_shrink_before_the_third_observed_frame() {
        let mut state = PacingState::default();
        // Observation 1 seeds.
        (state, _) = step(state, sample(50.0));
        // Observation 2 is a single over-target sample: one streak, no shrink.
        (state, _) = step(state, sample(50.0));
        assert_eq!(state.scale, 1.0, "one over-target frame must not shrink");
        assert_eq!(state.over_streak, 1);
        // Observation 3 is the second consecutive over-target sample: shrink.
        (state, _) = step(state, sample(50.0));
        assert_eq!(state.over_streak, 0);
        assert!((state.scale - DOWN_GAIN).abs() < f64::EPSILON);
    }

    /// A seeded state with a chosen cost EMA, so a sample of the same cost holds
    /// the EMA exactly (the EWMA-lag-free way to test the classification).
    fn seeded_with(ema: f64, over: u32, under: u32) -> PacingState {
        PacingState {
            scale: 1.0,
            cost_ema_ms: ema,
            target_ms: 10.0,
            over_streak: over,
            under_streak: under,
            seeded: true,
        }
    }

    #[test]
    fn two_over_frames_shrink_and_eight_under_frames_grow() {
        let mut state = seeded_with(20.0, 0, 0);
        (state, _) = step(state, sample(20.0)); // over streak 1
        assert_eq!(state.over_streak, 1);
        assert_eq!(state.scale, 1.0, "one over frame must not shrink");
        (state, _) = step(state, sample(20.0)); // over streak 2 -> fast down
        assert!((state.scale - 0.75).abs() < 1e-12, "scale {}", state.scale);
        assert_eq!(state.over_streak, 0);
        // Under-target: seven consecutive frames must not grow; the eighth does.
        let mut state = seeded_with(5.0, 0, 0);
        for _ in 0..7 {
            (state, _) = step(state, sample(5.0));
        }
        assert_eq!(state.under_streak, 7);
        assert_eq!(state.scale, 1.0, "seven under frames must not grow");
        (state, _) = step(state, sample(5.0));
        assert!(
            (state.scale - UP_GAIN).abs() < 1e-12,
            "scale {}",
            state.scale
        );
        assert_eq!(state.under_streak, 0);
    }

    #[test]
    fn deadband_neither_shrinks_nor_grows() {
        let mut state = PacingState::new(10.0);
        (state, _) = step(state, sample(10.0)); // seed
        for _ in 0..50 {
            (state, _) = step(state, sample(10.4)); // inside [9.5, 10.5]
        }
        assert_eq!(state.scale, 1.0);
        assert_eq!(state.over_streak, 0);
        assert_eq!(state.under_streak, 0);
    }

    #[test]
    fn direction_change_resets_the_opposite_streak() {
        // An under-target observation after over-streak frames clears the over
        // streak (and starts the under streak).
        let (state, _) = step(seeded_with(5.0, 1, 0), sample(5.0));
        assert_eq!(state.over_streak, 0, "over streak must reset");
        assert_eq!(state.under_streak, 1);
        // An over-target observation after under-streak frames clears the under
        // streak (and starts the over streak).
        let (state, _) = step(seeded_with(20.0, 0, 3), sample(20.0));
        assert_eq!(state.under_streak, 0, "under streak must reset");
        assert_eq!(state.over_streak, 1);
    }

    #[test]
    fn zero_accepted_is_not_an_observation() {
        let mut state = PacingState::default();
        (state, _) = step(state, sample(10.0)); // seed
        let before = state;
        let (state, _) = step(
            state,
            Sample {
                cpu_ms: 1_000.0,
                wait_ms: 1_000.0,
                accepted: 0,
                restart: false,
            },
        );
        assert_eq!(
            state, before,
            "a page that drew nothing is not a cost sample"
        );
    }

    #[test]
    fn restart_seeds_from_the_current_frame_and_does_not_adapt() {
        let mut state = PacingState::default();
        (state, _) = step(state, sample(100.0));
        (state, _) = step(state, sample(100.0));
        assert_eq!(state.scale, 1.0);
        assert_eq!(state.over_streak, 1);
        let (state, _) = step(
            state,
            Sample {
                cpu_ms: 4.0,
                wait_ms: 0.0,
                accepted: 3,
                restart: true,
            },
        );
        assert_eq!(state.cost_ema_ms, 4.0, "a restart reseeds the cost EMA");
        assert_eq!(state.over_streak, 0, "a restart clears stale streaks");
        assert_eq!(state.scale, 1.0);
    }

    #[test]
    fn scale_clamps_to_the_documented_range() {
        let mut state = PacingState::new(1.0);
        (state, _) = step(state, sample(1_000.0)); // seed
        for _ in 0..1_000 {
            let (next, _) = step(state, sample(1_000.0));
            state = next;
        }
        assert!((state.scale - MIN_SCALE).abs() < f64::EPSILON);
        for _ in 0..1_000 {
            let (next, _) = step(state, sample(0.0));
            state = next;
        }
        assert!((state.scale - MAX_SCALE).abs() < f64::EPSILON);
    }

    #[test]
    fn apply_scale_clamps_out_of_range_and_non_finite_scales() {
        let ceiling = FrameBudget {
            max_vertices: 8_000_000,
            max_triangles: 2_000_000,
            max_bytes: usize::MAX,
        };
        assert_eq!(apply_scale(&ceiling, 0.0001).max_vertices, 400_000);
        assert_eq!(apply_scale(&ceiling, 100.0).max_vertices, 32_000_000);
        assert_eq!(apply_scale(&ceiling, f64::NAN), apply_scale(&ceiling, 1.0));
        // A tiny injectable ceiling never collapses to zero (which would make
        // no batch fit even at the floor).
        let tiny = FrameBudget {
            max_vertices: 1,
            max_triangles: 1,
            max_bytes: usize::MAX,
        };
        assert_eq!(apply_scale(&tiny, MIN_SCALE).max_vertices, 1);
    }

    /// Deterministic pseudo-random jitter in `[-3, 3]` (an LCG, no dependency).
    fn jitter(seed: &mut u64) -> f64 {
        *seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((*seed >> 33) % 7) as f64 - 3.0
    }

    /// On a repeated, vsync-quantised cost trace (cost proportional to the
    /// current page scale, ±3 ms jitter folded to 0.5 ms), the controller must
    /// settle instead of oscillating unboundedly.
    #[test]
    fn quantised_cost_trace_has_bounded_scale_oscillation() {
        let mut state = PacingState::default();
        let mut seed = 0x1234_5678_u64;
        let mut previous = state.scale;
        let mut directions: Vec<i32> = Vec::new();
        let mut settled: Vec<f64> = Vec::new();
        for frame in 0..200 {
            // A page under a larger budget costs proportionally more: this closes
            // the loop the way a real renderer's page cost does.
            let raw = (20.0 * state.scale + jitter(&mut seed)).max(1.0);
            let quantised = (raw * 2.0).round() / 2.0;
            let (next, _) = step(state, sample(quantised));
            state = next;
            let direction = if state.scale > previous + 1e-12 {
                1
            } else if state.scale < previous - 1e-12 {
                -1
            } else {
                0
            };
            if direction != 0 {
                directions.push(direction);
            }
            previous = state.scale;
            if frame >= 50 {
                settled.push(state.scale);
            }
        }
        let flips = directions
            .windows(2)
            .filter(|pair| pair[0] != pair[1])
            .count();
        // 100 settled frames (frames 100..200) must not flap more than 8 times.
        let flips_per_100 = flips as f64 / 2.0;
        assert!(
            flips_per_100 <= 8.0,
            "controller oscillates: {flips_per_100} direction changes per 100 frames"
        );
        let min = settled.iter().copied().fold(f64::INFINITY, f64::min);
        let max = settled.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        assert!(
            min >= 0.4 && max <= 0.8,
            "steady-state scale escaped its band: [{min}, {max}]"
        );
        assert!(
            state.seeded && state.cost_ema_ms.is_finite(),
            "controller must stay in a valid state"
        );
    }
}
