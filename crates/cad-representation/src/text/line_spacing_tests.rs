//! MTEXT line-spacing (DXF MTEXT groups 44 + 73) layout contract.
//!
//! The factor multiplies one single line height; the style chooses which height
//! that is ("at least" = the line's own tallest run, "exactly" = the entity
//! height). The neutral default (`1.0`, "at least") must reproduce the
//! historical `LINE_SPACING * tallest run` advance exactly.

use super::*;
use cad_domain::{Point3, TextAlignH, TextAlignV, TextLineSpacing, TextLineSpacingStyle};

use super::finalize::finalize_lines;
use super::layout::layout_parsed;
use super::tests::mock_measure;

/// The lowest `y` of the finalized geometry: the second baseline, since the
/// mock glyphs start at their baseline and rise.
fn lowest_baseline(text: &str, spacing: TextLineSpacing) -> f64 {
    let parsed = parse_mtext(text, 10.0);
    let lines = layout_parsed(&parsed, 10.0, None, mock_measure);
    let out = finalize_lines(
        lines,
        10.0,
        Point3::default(),
        0.0,
        TextAlignH::Left,
        TextAlignV::Baseline,
        spacing,
    );
    out.iter()
        .flatten()
        .map(|p| p.y)
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn layout_multiplies_the_line_advance_by_the_line_spacing_factor() {
    // One single-line height is `LINE_SPACING * 10 = 12` for the mock font; the
    // factor multiplies exactly that advance and nothing else.
    let neutral = lowest_baseline("AB\\PCD", TextLineSpacing::default());
    let compressed = lowest_baseline(
        "AB\\PCD",
        TextLineSpacing {
            factor: 0.5,
            style: TextLineSpacingStyle::AtLeast,
        },
    );
    let stretched = lowest_baseline(
        "AB\\PCD",
        TextLineSpacing {
            factor: 1.5,
            style: TextLineSpacingStyle::AtLeast,
        },
    );
    assert!((neutral + 12.0).abs() < 1e-9, "neutral {neutral}");
    assert!((compressed + 6.0).abs() < 1e-9, "0.5x {compressed}");
    assert!((stretched + 18.0).abs() < 1e-9, "1.5x {stretched}");
}

#[test]
fn layout_applies_the_factor_to_both_line_spacing_styles() {
    // "Exactly" advances by the entity height (uniform baselines); "at least"
    // advances by the line's own tallest run. Both multiply by the factor.
    let at_least = |factor| TextLineSpacing {
        factor,
        style: TextLineSpacingStyle::AtLeast,
    };
    let exactly = |factor| TextLineSpacing {
        factor,
        style: TextLineSpacingStyle::Exactly,
    };
    // `{\H2x;Big}` makes line 0 twice the base height: "at least" moves line 1
    // by 1.2 * 20 = 24, "exactly" by 1.2 * 10 = 12.
    let tall_at_least = lowest_baseline("{\\H2x;Big}\\Psmall", at_least(1.0));
    let tall_exactly = lowest_baseline("{\\H2x;Big}\\Psmall", exactly(1.0));
    assert!((tall_at_least + 24.0).abs() < 1e-9, "{tall_at_least}");
    assert!((tall_exactly + 12.0).abs() < 1e-9, "{tall_exactly}");
    // The factor scales each style's own single-line height.
    assert!((lowest_baseline("{\\H2x;Big}\\Psmall", exactly(2.0)) + 24.0).abs() < 1e-9);
    assert!((lowest_baseline("{\\H2x;Big}\\Psmall", at_least(0.5)) + 12.0).abs() < 1e-9);
    // Without a taller run the two styles agree, which is why the neutral
    // default renders exactly as it did before the field existed.
    assert!((lowest_baseline("AB\\PCD", exactly(1.0)) + 12.0).abs() < 1e-9);
}
