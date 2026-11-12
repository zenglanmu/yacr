//! The neutral text width factor and the DXF group-41 sanitizer.
//!
//! `SemanticGeometry::Text` stores a horizontal width factor that layout
//! multiplies into every glyph advance. This pins the one sanitization rule used
//! by import (`text_width_factor_from_dxf`) and the neutral default, so import
//! and layout cannot disagree about what "not stretched" means.

use cad_domain::{text_width_factor_from_dxf, DEFAULT_TEXT_WIDTH_FACTOR};

#[test]
fn neutral_width_factor_is_one() {
    assert_eq!(DEFAULT_TEXT_WIDTH_FACTOR, 1.0);
}

#[test]
fn a_usable_dxf_width_factor_is_kept_verbatim() {
    assert_eq!(text_width_factor_from_dxf(1.0), Some(1.0));
    assert_eq!(text_width_factor_from_dxf(0.5), Some(0.5));
    assert_eq!(text_width_factor_from_dxf(2.75), Some(2.75));
}

#[test]
fn degenerate_dxf_width_factors_are_refused() {
    for bad in [0.0, -0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            text_width_factor_from_dxf(bad),
            None,
            "{bad} cannot mean a positive horizontal stretch"
        );
    }
}
