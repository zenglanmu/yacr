//! Resolved DIMSTYLE values used to draw a synthesized dimension.
//!
//! Submodule of [`crate::dimension`].

use super::*;
use acadrust::tables::DimStyle;

/// DIMSTYLE values needed to draw a synthesized dimension, already scaled by
/// `DIMSCALE` where AutoCAD scales them.
#[derive(Debug, Clone)]
pub(crate) struct DimStyleValues {
    /// `DIMASZ`: arrowhead length in drawing units.
    pub arrow_size: f64,
    /// `DIMEXO`: gap between the measured point and the extension line.
    pub ext_offset: f64,
    /// `DIMEXE`: how far the extension line runs past the dimension line.
    pub ext_beyond: f64,
    /// `DIMTXT`: measurement text height.
    pub text_height: f64,
    /// `DIMGAP`: gap between the dimension line and the measurement text.
    pub text_gap: f64,
    /// `DIMTAD`: place text above (true) rather than centred on the line.
    pub text_above: bool,
    /// `DIMLFAC`: linear measurement scale factor.
    pub linear_factor: f64,
    /// `DIMDEC`: decimal places for the measurement.
    pub decimals: usize,
    /// `DIMZIN` bit 3: suppress trailing zeros in the measurement.
    pub suppress_trailing_zeros: bool,
    /// Resolved `DIMTXSTY` name, used to pick the text font.
    pub text_style: String,
}

fn positive(value: f64, fallback: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        fallback
    }
}

impl DimStyleValues {
    pub(crate) fn from_style(style: &DimStyle, text_style: String) -> Self {
        let scale = positive(style.dimscale, 1.0);
        Self {
            arrow_size: positive(style.dimasz, 2.5) * scale,
            ext_offset: positive(style.dimexo, 0.625) * scale,
            ext_beyond: positive(style.dimexe, 1.25) * scale,
            text_height: positive(style.dimtxt, 2.5) * scale,
            text_gap: positive(style.dimgap, 0.625) * scale,
            text_above: style.dimtad != 0,
            linear_factor: {
                let factor = style.dimlfac;
                if factor.is_finite() && factor.abs() > 1e-12 {
                    factor.abs()
                } else {
                    1.0
                }
            },
            decimals: style.dimdec.clamp(0, 8) as usize,
            suppress_trailing_zeros: style.dimzin & 8 != 0,
            text_style,
        }
    }
}

impl ImporterBuilder<'_> {
    /// Resolve the DIMSTYLE for a dimension, falling back to the acadrust
    /// defaults when the table entry is missing.
    pub(super) fn dim_style_values(&self, name: &str) -> DimStyleValues {
        let text_style = self.dim_text_style(name);
        match self.acad.dim_styles.get(name) {
            Some(style) => DimStyleValues::from_style(style, text_style),
            None => {
                let standard = DimStyle::standard();
                DimStyleValues::from_style(&standard, text_style)
            }
        }
    }

    /// The TEXTSTYLE table name a handle points at, if any.
    pub(crate) fn text_style_name_for_handle(&self, handle: acadrust::Handle) -> Option<String> {
        if handle.is_null() {
            return None;
        }
        self.acad
            .text_styles
            .iter()
            .find(|style| style.handle.value() == handle.value())
            .map(|style| style.name.clone())
    }

    /// The text-style name a dimension style points at.
    ///
    /// The DXF reader fills `dimtxsty_handle` (group 340) but may leave the name
    /// at its default; resolve the handle against the TEXTSTYLE table so the
    /// measurement text uses the right font instead of the Standard SHX style.
    pub(crate) fn dim_text_style(&self, name: &str) -> String {
        let Some(style) = self.acad.dim_styles.get(name) else {
            return "Standard".to_string();
        };
        if let Some(resolved) = self.text_style_name_for_handle(style.dimtxsty_handle) {
            return resolved;
        }
        if !style.dimtxsty.is_empty() {
            return style.dimtxsty.clone();
        }
        "Standard".to_string()
    }
}
