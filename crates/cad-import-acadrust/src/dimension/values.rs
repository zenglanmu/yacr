//! Resolved DIMSTYLE values used to draw a synthesized dimension.
//!
//! Submodule of [`crate::dimension`]. The domain owns the data type
//! ([`DimensionStyleValues`]); this module only reads an acadrust `DimStyle`
//! and resolves the text-style identity the measurement text needs.

use super::*;
use acadrust::tables::DimStyle;

fn positive(value: f64, fallback: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        fallback
    }
}

/// Build the domain style snapshot from an acadrust `DimStyle`, already scaled
/// by `DIMSCALE` where AutoCAD scales them.
pub(crate) fn style_values(
    style: &DimStyle,
    text_style_name: String,
    text_style: StyleId,
    text_font: Option<String>,
) -> DimensionStyleValues {
    let scale = positive(style.dimscale, 1.0);
    DimensionStyleValues {
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
        text_style_name,
        text_style,
        text_font,
    }
}

impl ImporterBuilder<'_> {
    /// Resolve the DIMSTYLE for a dimension, falling back to the acadrust
    /// defaults when the table entry is missing. The text style identity is
    /// resolved here so the domain synthesis never reaches back into the
    /// builder.
    pub(super) fn dim_style_values(&self, name: &str) -> DimensionStyleValues {
        let text_style_name = self.dim_text_style(name);
        let text_style = self.style_id(&text_style_name);
        let text_font = self.style_font(&text_style_name);
        match self.acad.dim_styles.get(name) {
            Some(style) => style_values(style, text_style_name, text_style, text_font),
            None => style_values(
                &DimStyle::standard(),
                text_style_name,
                text_style,
                text_font,
            ),
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
