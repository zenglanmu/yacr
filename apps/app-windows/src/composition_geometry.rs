//! DirectComposition geometry: the CAD content visual in **physical pixels**.
//!
//! DirectComposition visual geometry (`SetOffsetX`/`SetOffsetY`,
//! `IDCompositionRectangleClip::SetLeft/Top/Right/Bottom`) is expressed in
//! **pixels**, not device-independent pixels: the reference pages say "in pixels",
//! the DirectComposition API has no DPI property (no `SetDpi` on any device
//! interface), and a visual has no size of its own — `SetContent` draws the
//! swapchain 1:1 at its buffer's pixel size, and `SetClip` only *crops* it.
//!
//! Therefore the visual rect must be the accepted logical CAD rect **scaled by the
//! window scale factor** — the same numbers the swapchain is configured with.
//! Passing the unscaled CSS rect instead leaves the content covering only
//! `1/scale` of the partition at any non-100% display scaling; because the
//! top-level window is transparent, the uncovered part shows the desktop through
//! the WebView.
//!
//! Deliberately **target-independent** (like [`crate::support`]): it uses only
//! `cad_ui_core::layout` types, so its unit tests run on any host.

use cad_ui_core::layout::{physical_content_size, CadMetrics};

/// The DirectComposition placement of the CAD content visual, in **device pixels**.
///
/// `offset` and `clip` are the physical CAD rect (so `clip` equals the swapchain
/// size exactly and the content is composited 1:1, never resampled); the swapchain
/// itself is sized from the one size authority ([`physical_content_size`]), which
/// keeps `child window == render target == swapchain == clip` at every scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompositionPlacement {
    /// Visual offset inside the target window's client area (pixels).
    pub offset: (f32, f32),
    /// Clip size from the visual's origin (pixels) — exactly the swapchain size.
    pub clip: (f32, f32),
    /// Swapchain size in device pixels (the size authority).
    pub physical_size: (u32, u32),
}

/// One axis of the visual origin: `round(value · scale)`, independently rounded
/// and clamped at zero (a negative DOM offset cannot be expressed as a visual
/// offset; the layout validator already rejects a materially out-of-bounds rect).
///
/// This is **the same formula** `Win32CadSurface::apply_layout` uses for the child
/// window position, so the child and the composed visual cannot drift apart.
fn pixel_origin(value: f32, scale: f64) -> f32 {
    (f64::from(value) * scale).round().max(0.0) as f32
}

/// Map an accepted logical CAD rect to the physical composition placement.
pub fn composition_placement(rect: &CadMetrics, scale: f64) -> CompositionPlacement {
    let scale = scale.max(0.1);
    let physical_size = physical_content_size(rect, scale);
    CompositionPlacement {
        offset: (pixel_origin(rect.x, scale), pixel_origin(rect.y, scale)),
        // Clip to the content itself: a layout/size race can only ever show less
        // CAD content, never bleed pixels over the surrounding chrome.
        clip: (physical_size.0 as f32, physical_size.1 as f32),
        physical_size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32, dpr: f32, sequence: u64) -> CadMetrics {
        CadMetrics {
            x,
            y,
            width: w,
            height: h,
            dpr,
            sequence,
        }
    }

    /// The visual rect is the **physical** rect: DirectComposition has no DIP
    /// conversion, so the visual geometry must carry the window scale itself.
    /// Regression for the observed Windows defect: a CSS-space placement covered
    /// only `1/scale` of the partition at fractional display scaling and the
    /// transparent window showed the desktop through the rest.
    #[test]
    fn composition_placement_scales_with_the_window_scale() {
        // Fractional logical rect (a fractional DPI/layout can produce these).
        let metrics = rect(12.5, 175.25, 333.5, 199.5, 1.5, 1);
        for scale in [1.0_f64, 1.25, 1.5, 2.0] {
            let placement = composition_placement(&metrics, scale);
            let expected_size = physical_content_size(&metrics, scale);
            assert_eq!(
                placement.offset,
                (
                    (12.5 * scale).round() as f32,
                    (175.25 * scale).round() as f32
                ),
                "the visual offset is the physical pixel position at scale {scale}"
            );
            assert_eq!(
                placement.clip,
                (expected_size.0 as f32, expected_size.1 as f32),
                "the clip is exactly the swapchain size at scale {scale}"
            );
            assert_eq!(
                placement.physical_size, expected_size,
                "the swapchain size stays the physical size authority at scale {scale}"
            );
        }
    }

    /// At 100% scaling the placement is the logical rect: the fix must not move
    /// the proven scale-1 geometry.
    #[test]
    fn composition_placement_matches_the_logical_rect_at_scale_one() {
        let metrics = rect(0.0, 175.0, 626.0, 341.0, 1.0, 2);
        let placement = composition_placement(&metrics, 1.0);
        assert_eq!(placement.offset, (0.0, 175.0));
        assert_eq!(placement.clip, (626.0, 341.0));
        assert_eq!(placement.physical_size, (626, 341));
    }

    /// The visual clip and the swapchain are the same rect, so the content is
    /// composited 1:1: a clip smaller than the buffer would silently crop the
    /// drawing, and a larger one would expose an unpainted region.
    #[test]
    fn the_clip_is_never_a_crop_of_the_rendered_frame() {
        for (w, h, scale) in [(1176.0_f32, 350.0_f32, 1.5_f64), (800.0, 600.0, 1.25)] {
            let placement = composition_placement(&rect(0.0, 262.0, w, h, scale as f32, 1), scale);
            assert_eq!(
                placement.clip,
                (
                    placement.physical_size.0 as f32,
                    placement.physical_size.1 as f32
                )
            );
        }
    }

    /// A negative DOM offset cannot be expressed as a visual offset; it clamps to
    /// zero exactly like the child window position does.
    #[test]
    fn a_negative_origin_clamps_to_zero() {
        let placement = composition_placement(&rect(-3.0, -1.5, 100.0, 50.0, 1.0, 1), 1.0);
        assert_eq!(placement.offset, (0.0, 0.0));
    }
}
