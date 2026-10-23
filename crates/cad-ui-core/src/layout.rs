//! Pure viewport-layout mapping for a CAD partition (host-agnostic).
//!
//! A frontend reports its CAD viewport rect in **CSS/logical pixels** (`x`/`y` =
//! viewport logical offset, `width`/`height` = CAD content size, `dpr`). This
//! module validates that rect and maps it to the **viewport insets** a native
//! CAD surface must reserve around its content (the space left for surrounding
//! chrome). It is GPU- and toolkit-free so it is fully unit-testable and is
//! reused by every UI host; the concrete host decides how the insets are applied.
//!
//! Rules:
//! - metrics must be finite and strictly positive (zero/invalid => pause);
//! - the rect must lie within the window's logical bounds (with a small
//!   tolerance);
//! - a stale `sequence` (older than the last applied) is rejected, and the
//!   sequence is kept **monotonic across GPU generations**: a new generation does
//!   **not** reset it. When the native surface is rebuilt, the cached CAD rect is
//!   re-validated against the **current** top-level geometry
//!   ([`LayoutTracker::revalidate`]) rather than blindly re-trusted; if it no
//!   longer fits, the host waits for fresh metrics instead of applying stale
//!   insets.

use cad_platform::SurfaceMetrics;

/// A CAD viewport rect in logical pixels, plus the device pixel ratio and the
/// frontend sequence number that produced it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CadMetrics {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub dpr: f32,
    pub sequence: u64,
}

/// The insets a native CAD surface must reserve around its content, in logical
/// pixels (== CSS px at scale 1). This is a toolkit-neutral viewport inset, not
/// a specific widget toolkit's margin type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewportInsets {
    pub top: i32,
    pub bottom: i32,
    pub start: i32,
    pub end: i32,
}

/// Why a rect was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutError {
    /// A value was NaN/infinite.
    NonFinite,
    /// A size/dpr was zero or negative.
    NonPositive,
    /// The rect fell outside the window's logical bounds.
    OutOfBounds,
    /// The sequence was older than the last applied one.
    StaleSequence,
}

/// Validate a rect against the window's logical size. `screen_w`/`screen_h` are
/// the window's logical (CSS) size.
pub fn validate_metrics(
    metrics: &CadMetrics,
    screen_w: f32,
    screen_h: f32,
) -> Result<(), LayoutError> {
    let values = [
        metrics.x,
        metrics.y,
        metrics.width,
        metrics.height,
        metrics.dpr,
    ];
    if values.iter().any(|v| !v.is_finite()) {
        return Err(LayoutError::NonFinite);
    }
    if metrics.width <= 0.0 || metrics.height <= 0.0 || metrics.dpr <= 0.0 {
        return Err(LayoutError::NonPositive);
    }
    if !screen_w.is_finite() || !screen_h.is_finite() || screen_w <= 0.0 || screen_h <= 0.0 {
        return Err(LayoutError::OutOfBounds);
    }
    // Small tolerance for sub-pixel rounding at the edges.
    const TOL: f32 = 2.0;
    if metrics.x < -TOL || metrics.y < -TOL {
        return Err(LayoutError::OutOfBounds);
    }
    if metrics.x + metrics.width > screen_w + TOL || metrics.y + metrics.height > screen_h + TOL {
        return Err(LayoutError::OutOfBounds);
    }
    Ok(())
}

/// Map a validated rect to clamped viewport insets for a `screen_w` x `screen_h`
/// logical window. A rect that only shifts (same size, new x/y) still produces
/// new insets.
pub fn insets_from(metrics: &CadMetrics, screen_w: f32, screen_h: f32) -> ViewportInsets {
    let clamp = |v: f32| -> i32 {
        if v.is_finite() {
            v.round().clamp(0.0, 32767.0) as i32
        } else {
            0
        }
    };
    let top = clamp(metrics.y);
    let start = clamp(metrics.x);
    let bottom = clamp(screen_h - (metrics.y + metrics.height));
    let end = clamp(screen_w - (metrics.x + metrics.width));
    ViewportInsets {
        top,
        bottom,
        start,
        end,
    }
}

/// The physical size of the CAD content at `scale`, in **device pixels**.
///
/// This is the single size authority for the CAD partition: a native host must
/// size its CAD child surface with exactly this value, and the render session
/// allocates its render target at the same value. It delegates to the session's
/// own formula ([`cad_engine::render::render_target_size`]) instead of
/// re-deriving it, so the two numbers are identical **by construction** at every
/// scale — not merely equal for integral sizes.
///
/// Why this matters: the presenter refuses to blit a texture whose size differs
/// from the surface (`ProbeError::TextureInvalid` after a bounded wait), so an
/// independently rounded size is a *total* present failure (a transparent CAD
/// region with a sticky error), not a cosmetic one. Deriving the child size from
/// scaled-and-rounded insets was exactly such a second rounding path
/// (`round(container·s) − round(start·s) − round(end·s)`), which can differ from
/// `round((container − start − end)·s)` by a pixel at fractional DPI.
///
/// The **offset** may be rounded independently: `round(x·s)` is fine, because a
/// half-pixel placement is invisible and never affects the size guard.
pub fn physical_content_size(rect: &CadMetrics, scale: f64) -> (u32, u32) {
    cad_engine::render::render_target_size([f64::from(rect.width), f64::from(rect.height)], scale)
}

/// Map an accepted frontend CAD rect (CSS/logical pixels) plus the window scale
/// factor to the [`SurfaceMetrics`] a native host reads each tick.
///
/// The surface reports the CAD rect's **logical** size (not its offset); the
/// native layout reserves the surrounding chrome via the viewport insets, so the
/// surface's own safe insets are zero here. The scale is clamped positive so a
/// degenerate window scale factor can never produce invalid canvas metrics (the
/// host then refuses to advance the viewport instead of fabricating one).
pub fn surface_metrics(rect: &CadMetrics, scale: f64) -> SurfaceMetrics {
    SurfaceMetrics {
        logical: [f64::from(rect.width), f64::from(rect.height)],
        scale: scale.max(0.1),
        safe_insets: [0.0; 4],
    }
}

/// Tracks the accepted layout across GPU generations.
#[derive(Debug, Default, Clone)]
pub struct LayoutTracker {
    applied_sequence: Option<u64>,
    generation: u64,
    confirmed: bool,
    insets: Option<ViewportInsets>,
    metrics: Option<CadMetrics>,
}

impl LayoutTracker {
    /// A fresh tracker (no confirmed rect yet).
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a new GPU generation. The DOM input sequence is deliberately
    /// **not** reset: it stays monotonic across generations so a cached rect can
    /// be re-validated (see [`Self::revalidate`]) instead of blindly re-applied.
    pub fn note_generation(&mut self) {
        self.generation += 1;
    }

    /// Current generation.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Whether a valid rect has been accepted for the current generation.
    pub fn confirmed(&self) -> bool {
        self.confirmed
    }

    /// The last applied viewport insets.
    pub fn insets(&self) -> Option<ViewportInsets> {
        self.insets
    }

    /// The last accepted CAD rect (logical px).
    pub fn metrics(&self) -> Option<CadMetrics> {
        self.metrics
    }

    /// The last applied DOM sequence, if any.
    pub fn applied_sequence(&self) -> Option<u64> {
        self.applied_sequence
    }

    /// Validate and accept a **new** rect (strictly newer sequence). Returns the
    /// insets to apply on success.
    pub fn accept(
        &mut self,
        metrics: &CadMetrics,
        screen_w: f32,
        screen_h: f32,
    ) -> Result<ViewportInsets, LayoutError> {
        if let Some(last) = self.applied_sequence {
            if metrics.sequence <= last {
                return Err(LayoutError::StaleSequence);
            }
        }
        self.commit(metrics, screen_w, screen_h)
    }

    /// Re-validate the **cached** rect against the current top-level geometry
    /// when the native surface is rebuilt. The sequence may equal the last applied
    /// one (the DOM rect did not change); only the geometry is re-checked, so an
    /// invalid rect waits for fresh metrics instead of applying stale insets. A
    /// strictly newer sequence is also accepted.
    pub fn revalidate(
        &mut self,
        metrics: &CadMetrics,
        screen_w: f32,
        screen_h: f32,
    ) -> Result<ViewportInsets, LayoutError> {
        if let Some(last) = self.applied_sequence {
            if metrics.sequence < last {
                return Err(LayoutError::StaleSequence);
            }
        }
        self.commit(metrics, screen_w, screen_h)
    }

    fn commit(
        &mut self,
        metrics: &CadMetrics,
        screen_w: f32,
        screen_h: f32,
    ) -> Result<ViewportInsets, LayoutError> {
        validate_metrics(metrics, screen_w, screen_h)?;
        let insets = insets_from(metrics, screen_w, screen_h);
        self.applied_sequence = Some(metrics.sequence);
        self.insets = Some(insets);
        self.metrics = Some(*metrics);
        self.confirmed = true;
        Ok(insets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(x: f32, y: f32, w: f32, h: f32, dpr: f32, seq: u64) -> CadMetrics {
        CadMetrics {
            x,
            y,
            width: w,
            height: h,
            dpr,
            sequence: seq,
        }
    }

    #[test]
    fn maps_rect_to_insets() {
        let metrics = m(0.0, 101.0, 960.0, 439.0, 1.0, 1);
        assert_eq!(
            insets_from(&metrics, 960.0, 600.0),
            ViewportInsets {
                top: 101,
                bottom: 60,
                start: 0,
                end: 0
            }
        );
    }

    #[test]
    fn dpr_does_not_change_logical_insets() {
        let a = m(10.0, 20.0, 800.0, 400.0, 1.0, 1);
        let b = m(10.0, 20.0, 800.0, 400.0, 2.0, 1);
        assert_eq!(
            insets_from(&a, 1000.0, 600.0),
            insets_from(&b, 1000.0, 600.0)
        );
    }

    #[test]
    fn accepted_rect_maps_to_canvas_surface_metrics() {
        let rect = m(12.0, 40.0, 800.0, 600.0, 2.0, 7);
        let metrics = surface_metrics(&rect, 2.0);
        // The surface reports the CAD rect (CSS/logical px), not the offset.
        assert_eq!(metrics.logical, [800.0, 600.0]);
        assert_eq!(metrics.scale, 2.0);
        assert_eq!(metrics.safe_insets, [0.0; 4]);
        // A degenerate scale factor is clamped, never accepted as-is.
        assert_eq!(surface_metrics(&rect, 0.0).scale, 0.1);
    }

    #[test]
    fn zero_and_negative_sizes_are_rejected() {
        assert_eq!(
            validate_metrics(&m(0.0, 0.0, 0.0, 100.0, 1.0, 1), 960.0, 600.0),
            Err(LayoutError::NonPositive)
        );
        assert_eq!(
            validate_metrics(&m(0.0, 0.0, 100.0, -1.0, 1.0, 1), 960.0, 600.0),
            Err(LayoutError::NonPositive)
        );
    }

    #[test]
    fn non_finite_is_rejected() {
        assert_eq!(
            validate_metrics(&m(f32::NAN, 0.0, 100.0, 100.0, 1.0, 1), 960.0, 600.0),
            Err(LayoutError::NonFinite)
        );
    }

    #[test]
    fn out_of_bounds_is_rejected() {
        assert_eq!(
            validate_metrics(&m(0.0, 0.0, 2000.0, 100.0, 1.0, 1), 960.0, 600.0),
            Err(LayoutError::OutOfBounds)
        );
        assert_eq!(
            validate_metrics(&m(-50.0, 0.0, 100.0, 100.0, 1.0, 1), 960.0, 600.0),
            Err(LayoutError::OutOfBounds)
        );
    }

    #[test]
    fn stale_sequence_is_rejected_and_shift_is_applied() {
        let mut tracker = LayoutTracker::new();
        assert!(tracker
            .accept(&m(0.0, 101.0, 960.0, 439.0, 1.0, 5), 960.0, 600.0)
            .is_ok());
        // A rect that only shifts (same size) still updates the insets.
        let shifted = tracker
            .accept(&m(20.0, 120.0, 960.0, 439.0, 1.0, 6), 1000.0, 600.0)
            .unwrap();
        assert_eq!(shifted.start, 20);
        assert_eq!(shifted.top, 120);
        // An older sequence must not overwrite.
        assert_eq!(
            tracker.accept(&m(0.0, 101.0, 960.0, 439.0, 1.0, 5), 960.0, 600.0),
            Err(LayoutError::StaleSequence)
        );
    }

    #[test]
    fn generation_keeps_dom_sequence_monotonic() {
        let mut tracker = LayoutTracker::new();
        tracker
            .accept(&m(0.0, 101.0, 960.0, 439.0, 1.0, 5), 960.0, 600.0)
            .unwrap();
        assert!(tracker.confirmed());
        tracker.note_generation();
        assert_eq!(tracker.generation(), 1);
        // The sequence is NOT reset: the same (old) sequence stays stale.
        assert_eq!(
            tracker.accept(&m(0.0, 101.0, 960.0, 439.0, 1.0, 5), 960.0, 600.0),
            Err(LayoutError::StaleSequence)
        );
        // A strictly newer sequence is still accepted.
        assert!(tracker
            .accept(&m(0.0, 101.0, 960.0, 439.0, 1.0, 6), 960.0, 600.0)
            .is_ok());
        assert_eq!(tracker.applied_sequence(), Some(6));
    }

    #[test]
    fn cached_valid_rect_revalidates_against_current_geometry() {
        let mut tracker = LayoutTracker::new();
        let rect = m(0.0, 101.0, 960.0, 439.0, 1.0, 7);
        let first = tracker.accept(&rect, 960.0, 600.0).unwrap();
        tracker.note_generation();
        // Same sequence + same geometry is allowed through revalidation.
        let again = tracker.revalidate(&rect, 960.0, 600.0).unwrap();
        assert_eq!(first, again);
        assert_eq!(tracker.applied_sequence(), Some(7));
        assert!(tracker.confirmed());
        assert_eq!(tracker.metrics().map(|m| m.sequence), Some(7));
    }

    #[test]
    fn revalidate_waits_for_new_metrics_when_top_shrinks() {
        let mut tracker = LayoutTracker::new();
        let rect = m(0.0, 101.0, 960.0, 439.0, 1.0, 7);
        tracker.accept(&rect, 960.0, 600.0).unwrap();
        tracker.note_generation();
        // The top-level shrank; the cached rect no longer fits -> not applied.
        assert_eq!(
            tracker.revalidate(&rect, 400.0, 300.0),
            Err(LayoutError::OutOfBounds)
        );
        // Fresh metrics that fit are accepted on the next resize.
        let smaller = m(0.0, 40.0, 400.0, 220.0, 1.0, 8);
        assert!(tracker.accept(&smaller, 400.0, 300.0).is_ok());
    }

    #[test]
    fn margins_reference_the_container_bounds_not_the_window_inner_size() {
        // Native compositor capture A: the DOM CAD rect is reported in the
        // WebView viewport (the GTK container that receives the margins), NOT in
        // the Tauri window `inner_size()` (which includes CSD and is larger).
        let rect_a = m(300.0, 203.0, 626.0, 266.0, 1.0, 1);
        // Container = the real viewport.
        validate_metrics(&rect_a, 960.0, 553.0).expect("A rect fits the container");
        assert_eq!(
            insets_from(&rect_a, 960.0, 553.0),
            ViewportInsets {
                top: 203,
                bottom: 84,
                start: 300,
                end: 34,
            }
        );
        // The (wrong) Tauri inner_size bounds produced the observed inflated
        // insets: end=86 / bottom=183. This must not be what the host uses.
        assert_eq!(
            insets_from(&rect_a, 1012.0, 652.0),
            ViewportInsets {
                top: 203,
                bottom: 183,
                start: 300,
                end: 86,
            }
        );

        // Capture B: resized container 1200x800, rect 1166x513 at (0,203).
        let rect_b = m(0.0, 203.0, 1166.0, 513.0, 1.0, 2);
        validate_metrics(&rect_b, 1200.0, 800.0).expect("B rect fits the container");
        assert_eq!(
            insets_from(&rect_b, 1200.0, 800.0),
            ViewportInsets {
                top: 203,
                bottom: 84,
                start: 0,
                end: 34,
            }
        );

        // The accepted DOM rect and the reserved native extent agree within
        // rounding: host margins + rect == container bounds.
        let a = insets_from(&rect_a, 960.0, 553.0);
        assert_eq!(a.start as f32 + rect_a.width + a.end as f32, 960.0);
        assert_eq!(a.top as f32 + rect_a.height + a.bottom as f32, 553.0);
    }
    // --- the single CAD-content-size authority ------------------------------

    /// The one rounding is `round(width · scale)` per axis: never a scaled
    /// rounding of already-rounded insets. Grid over fractional content rects and
    /// every scale this product ships for (100/125/150/175/200/250 %).
    #[test]
    fn physical_content_size_is_exactly_round_width_times_scale() {
        let scales = [1.0_f64, 1.25, 1.5, 1.75, 2.0, 2.5];
        let sizes = [
            (626.0_f32, 266.0_f32),
            (333.5, 199.5),
            (1166.0, 513.0),
            (1023.4, 768.6),
            (0.4, 0.4),
        ];
        for (w, h) in sizes {
            let rect = m(3.7, 11.3, w, h, 1.5, 1);
            for scale in scales {
                let expected = (
                    (f64::from(w) * scale).round().max(1.0) as u32,
                    (f64::from(h) * scale).round().max(1.0) as u32,
                );
                assert_eq!(
                    physical_content_size(&rect, scale),
                    expected,
                    "rounding the product once for {w}x{h} at scale {scale}"
                );
                // The session's own authority is the same function, so the child
                // surface and the render target cannot drift apart.
                assert_eq!(
                    physical_content_size(&rect, scale),
                    cad_engine::render::render_target_size([f64::from(w), f64::from(h)], scale),
                    "the host and the render session share one formula"
                );
            }
        }
    }

    /// Regression for the H1 defect: the previous insets-derived child size
    /// (`round(container·s) − round(start·s) − round(end·s)`) disagrees with the
    /// render target for fractional content rects, and the presenter refuses every
    /// such frame. The size authority must never be re-derived that way.
    #[test]
    fn insets_derived_child_size_can_disagree_with_the_render_target() {
        // A fractional DOM rect inside an integral logical container: 0 + 333.5 +
        // 666.5 = 1000. `insets_from` rounds the *logical* offsets first, so the
        // insets-derived physical width loses the fraction's carry.
        let rect = m(0.0, 0.0, 333.5, 200.0, 1.25, 1);
        let insets = insets_from(&rect, 1000.0, 800.0);
        assert_eq!((insets.start, insets.end), (0, 667));
        let scale = 1.5_f64;
        let container_px = (1000.0_f64 * scale).round() as i32;
        let insets_width = container_px
            - (f64::from(insets.start) * scale).round() as i32
            - (f64::from(insets.end) * scale).round() as i32;
        let target = physical_content_size(&rect, scale);
        assert_eq!(target.0, 500, "round(333.5 · 1.5)");
        assert_eq!(
            insets_width,
            i32::try_from(target.0).unwrap() - 1,
            "the insets-derived path is a pixel narrower: that disagreement is a \
             total present failure, so the child size must come from the authority"
        );
    }
}
