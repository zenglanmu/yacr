//! Pure layout mapping for the CAD partition (host scope).
//!
//! The React frontend reports its CAD viewport rect in **CSS pixels** via the
//! existing `probe_input` `resize` fields (`x`/`y` = viewport CSS offset,
//! `width`/`height` = CAD CSS size, `dpr`). This module validates that rect and
//! maps it to GTK margins for the native `GtkDrawingArea`. It is GPU- and
//! GTK-free so it is fully unit-testable.
//!
//! Rules:
//! - metrics must be finite and strictly positive (zero/invalid => pause);
//! - the rect must lie within the window's CSS bounds (with a small tolerance);
//! - a stale `sequence` (older than the last applied) is rejected, and the
//!   sequence is kept **monotonic across GPU generations**: a new generation does
//!   **not** reset it. When the native surface is rebuilt, the cached CAD rect is
//!   re-validated against the **current** top-level geometry
//!   ([`LayoutTracker::revalidate`]) rather than blindly re-trusted; if it no
//!   longer fits, the host waits for fresh metrics instead of applying stale
//!   margins.

/// A CAD viewport rect in CSS pixels, plus the device pixel ratio and the
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

/// GTK margins for the drawing area (logical px == CSS px).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CadMargins {
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
    /// The rect fell outside the window's CSS bounds.
    OutOfBounds,
    /// The sequence was older than the last applied one.
    StaleSequence,
}

/// Validate a rect against the window's CSS size. `screen_w`/`screen_h` are the
/// window's logical (CSS) size.
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

/// Map a validated rect to clamped GTK margins for a `screen_w` x `screen_h`
/// logical window. A rect that only shifts (same size, new x/y) still produces
/// new margins.
pub fn margins_from(metrics: &CadMetrics, screen_w: f32, screen_h: f32) -> CadMargins {
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
    CadMargins {
        top,
        bottom,
        start,
        end,
    }
}

/// Tracks the accepted layout across GPU generations.
#[derive(Debug, Default, Clone)]
pub struct LayoutTracker {
    applied_sequence: Option<u64>,
    generation: u64,
    confirmed: bool,
    margins: Option<CadMargins>,
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

    /// The last applied margins.
    pub fn margins(&self) -> Option<CadMargins> {
        self.margins
    }

    /// The last accepted CAD rect (CSS px).
    pub fn metrics(&self) -> Option<CadMetrics> {
        self.metrics
    }

    /// The last applied DOM sequence, if any.
    pub fn applied_sequence(&self) -> Option<u64> {
        self.applied_sequence
    }

    /// Validate and accept a **new** rect (strictly newer sequence). Returns the
    /// margins to apply on success.
    pub fn accept(
        &mut self,
        metrics: &CadMetrics,
        screen_w: f32,
        screen_h: f32,
    ) -> Result<CadMargins, LayoutError> {
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
    /// invalid rect waits for fresh metrics instead of applying stale margins. A
    /// strictly newer sequence is also accepted.
    pub fn revalidate(
        &mut self,
        metrics: &CadMetrics,
        screen_w: f32,
        screen_h: f32,
    ) -> Result<CadMargins, LayoutError> {
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
    ) -> Result<CadMargins, LayoutError> {
        validate_metrics(metrics, screen_w, screen_h)?;
        let margins = margins_from(metrics, screen_w, screen_h);
        self.applied_sequence = Some(metrics.sequence);
        self.margins = Some(margins);
        self.metrics = Some(*metrics);
        self.confirmed = true;
        Ok(margins)
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
    fn maps_rect_to_margins() {
        let metrics = m(0.0, 101.0, 960.0, 439.0, 1.0, 1);
        assert_eq!(
            margins_from(&metrics, 960.0, 600.0),
            CadMargins {
                top: 101,
                bottom: 60,
                start: 0,
                end: 0
            }
        );
    }

    #[test]
    fn dpr_does_not_change_css_margins() {
        let a = m(10.0, 20.0, 800.0, 400.0, 1.0, 1);
        let b = m(10.0, 20.0, 800.0, 400.0, 2.0, 1);
        assert_eq!(
            margins_from(&a, 1000.0, 600.0),
            margins_from(&b, 1000.0, 600.0)
        );
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
        // A rect that only shifts (same size) still updates margins.
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
}
