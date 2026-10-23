//! DirectComposition geometry: the pure DIP/pixel split of the CAD placement.
//!
//! Deliberately **target-independent** (like [`crate::support`]): it uses only
//! `cad_ui_core::layout` types, so its unit tests run on any host, not just a
//! Windows runner. The Windows surface code applies the result.

use cad_ui_core::layout::{physical_content_size, CadMetrics};

/// The DirectComposition placement of the CAD content visual.
///
/// Units: `offset`/`clip` are **DIPs** (= logical/CSS pixels — DirectComposition
/// composes a visual tree in DIPs), so they are the accepted logical CAD rect
/// **verbatim** and never re-derived from the scale. `physical_size` is the
/// swapchain size in **device pixels** and comes from the one size authority
/// ([`physical_content_size`]), which is what keeps
/// `child window == render target == swapchain` at every scale.
///
/// The visual is clipped to its own content rect (`clip` from `(0, 0)`), so a
/// layout/resize race can only ever show less CAD content, never bleed it over
/// the surrounding chrome.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompositionPlacement {
    /// Visual offset inside the target window's client area (DIPs).
    pub offset: (f32, f32),
    /// Clip size from the visual's origin (DIPs).
    pub clip: (f32, f32),
    /// Swapchain size in device pixels (the size authority).
    pub physical_size: (u32, u32),
}

/// Map an accepted logical CAD rect to the composition placement.
///
/// - offset/clip: the logical rect **unscaled** (DIPs);
/// - physical size: [`physical_content_size`] (device pixels).
pub fn composition_placement(rect: &CadMetrics, scale: f64) -> CompositionPlacement {
    CompositionPlacement {
        // Negative DOM offsets cannot be expressed as a visual offset; the
        // layout validator already rejects a materially out-of-bounds rect.
        offset: (rect.x.max(0.0), rect.y.max(0.0)),
        clip: (rect.width.max(0.0), rect.height.max(0.0)),
        physical_size: physical_content_size(rect, scale),
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

    /// The DIP/pixel split of the composition placement: the visual geometry is
    /// the logical rect **unscaled** (DComp composes in DIPs), while the
    /// swapchain stays physical and must be exactly `physical_content_size` so the
    /// presenter's size guard keeps holding at every DPI.
    #[test]
    fn composition_placement_uses_dip_geometry_and_a_physical_swapchain() {
        // Fractional logical rect (a fractional DPI/layout can produce these).
        let metrics = rect(12.5, 175.25, 333.5, 199.5, 1.5, 1);
        for scale in [1.0_f64, 1.25, 1.5, 2.0] {
            let placement = composition_placement(&metrics, scale);
            assert_eq!(
                placement.offset,
                (12.5, 175.25),
                "the visual offset is the logical position in DIPs, never scaled"
            );
            assert_eq!(
                placement.clip,
                (333.5, 199.5),
                "the clip is the logical content size in DIPs, never scaled"
            );
            assert_eq!(
                placement.physical_size,
                physical_content_size(&metrics, scale),
                "the swapchain size stays the physical size authority at scale {scale}"
            );
        }
        // The DIP geometry is scale-invariant; only the physical size moves.
        let at_1 = composition_placement(&metrics, 1.0);
        let at_2 = composition_placement(&metrics, 2.0);
        assert_eq!(at_1.offset, at_2.offset);
        assert_eq!(at_1.clip, at_2.clip);
        assert_eq!(at_1.physical_size, (334, 200)); // round(333.5), round(199.5)
        assert_eq!(at_2.physical_size, (667, 399));
    }

    /// An integral rect at the scales this product ships for (the values the
    /// evidence runs use) settles the same identity without rounding tricks.
    #[test]
    fn composition_placement_matches_the_authority_for_integral_rects() {
        let metrics = rect(0.0, 175.0, 626.0, 341.0, 1.0, 2);
        for scale in [1.0_f64, 1.25, 1.5, 2.0] {
            let placement = composition_placement(&metrics, scale);
            assert_eq!(placement.offset, (0.0, 175.0));
            assert_eq!(placement.clip, (626.0, 341.0));
            assert_eq!(
                placement.physical_size,
                (
                    (626.0 * scale).round() as u32,
                    (341.0 * scale).round() as u32
                )
            );
        }
    }
}
