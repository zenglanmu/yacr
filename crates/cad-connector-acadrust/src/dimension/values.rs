//! Resolved DIMSTYLE values used to draw a synthesized dimension, and the
//! acadrust -> database DIMSTYLE table mapping (ADR 0017 B.1/B.2).
//!
//! Submodule of [`crate::dimension`]. The domain owns the entity-level snapshot
//! ([`DimensionStyleValues`]); the database owns the style table
//! ([`cad_db::DimStyle`]). This module reads an acadrust `DimStyle`, maps it to
//! the table entry (full B.2(a) + B.2(b)), and resolves the snapshot the
//! synthesis consumes — so the importer, the database default and the creation
//! path share one resolution.

use super::*;
use acadrust::tables::DimStyle;

/// Map an acadrust `DimStyle` to the database DIMSTYLE table entry, carrying the
/// full B.2(a) display-driving set and the B.2(b) snapshot-only set (ADR 0017
/// B.2). Every field is copied verbatim; the resolution to a display snapshot
/// happens in [`cad_db::DimStyle::resolved`].
pub(crate) fn db_dim_style(style: &DimStyle, id: DimStyleId) -> cad_db::DimStyle {
    cad_db::DimStyle {
        id,
        name: style.name.clone(),
        text_height: style.dimtxt,
        arrow_size: style.dimasz,
        ext_offset: style.dimexo,
        ext_beyond: style.dimexe,
        text_gap: style.dimgap,
        text_vertical: style.dimtad,
        text_vertical_position: style.dimtvp,
        scale: style.dimscale,
        decimals: style.dimdec,
        zero_suppression: style.dimzin,
        linear_factor: style.dimlfac,
        rounding: style.dimrnd,
        postfix: style.dimpost.clone(),
        center_mark: style.dimcen,
        text_style_name: style.dimtxsty.clone(),
        suppress_ext1: style.dimse1,
        suppress_ext2: style.dimse2,
        suppress_dim1: style.dimsd1,
        suppress_dim2: style.dimsd2,
        linear_unit_format: style.dimlunit,
        decimal_separator: style.dimdsep,
        text_justification: style.dimjust,
        text_inside_horizontal: style.dimtih,
        text_outside_horizontal: style.dimtoh,
        text_fit: style.dimatfit,
        text_inside_extensions: style.dimtix,
        text_outside_flow_line: style.dimtofl,
        dim_line_color: style.dimclrd,
        ext_line_color: style.dimclre,
        text_color: style.dimclrt,
        dim_line_extension: style.dimdle,
        dim_line_increment: style.dimdli,
        // DIMCONTINUEMODE is drawing-level and not modelled by acadrust; the
        // table carries the documented On default (declared, ADR 0017 B.2(b)).
        continue_mode: true,
    }
}

impl ImporterBuilder<'_> {
    /// Read the drawing's DIMSTYLE table into the database (ADR 0017 B.1),
    /// carrying the full B.2(a) + B.2(b) variable set. The pinned metric
    /// `Standard` is added by the builder at `finish` when the source carried
    /// none.
    pub(crate) fn read_dim_styles(&mut self) -> CadResult<()> {
        for (index, style) in self.acad.dim_styles.iter().enumerate() {
            let id = DimStyleId(index as u128);
            self.builder.insert_dim_style(db_dim_style(style, id))?;
        }
        Ok(())
    }

    /// Resolve the DIMSTYLE for a dimension, falling back to the pinned metric
    /// `Standard` when the table entry is missing. The text style identity is
    /// resolved here so the domain synthesis never reaches back into the
    /// builder.
    pub(super) fn dim_style_values(&self, name: &str) -> DimensionStyleValues {
        let text_style_name = self.dim_text_style(name);
        let text_style = self.style_id(&text_style_name);
        let text_font = self.style_font(&text_style_name);
        let entry = match self.acad.dim_styles.get(name) {
            Some(style) => db_dim_style(style, DimStyleId(0)),
            None => cad_db::DimStyle::standard_metric(DimStyleId(0)),
        };
        entry.resolved(text_style_name, text_style, text_font)
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
