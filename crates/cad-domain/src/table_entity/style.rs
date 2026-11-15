//! The table style snapshot one `ACAD_TABLE` carries (`TABLESTYLE`).
//!
//! # Why a snapshot and not a drawing-level table
//!
//! AutoCAD stores table styles as drawing-global `TABLESTYLE` objects in the
//! `ACAD_TABLESTYLE` dictionary; this build has no drawing-level table-style
//! table (`cad-db` carries text/dim/mleader style tables only), so the style is
//! stored **per entity** as a snapshot — exactly the `MLEADERSTYLE` decision of
//! ADR 0019 C/D. Export writes one `TABLESTYLE` object per distinct name into
//! the document's own dictionary, so the DXF round-trip is still one shared
//! style object per name; the model is what differs.
//!
//! A snapshot is complete: every field here is written to the `TABLESTYLE`
//! object and read back, so nothing is stored without a round-trip. The
//! per-cell border colours/lineweights/fills and the title/header row counts are
//! *not* part of it (the display has one colour per entity and the source does
//! not record the row counts this build could use); the importer/export report
//! those gaps by name.
//!
//! # Cells referencing the style
//!
//! A cell whose border set, text height and text style name still equal the
//! style's values is an **inheriting** cell: it takes them from the style rather
//! than from itself. [`TableGeometry::with_style_resolved`] re-resolves those
//! cells to the new values, so `TABLESTYLE` reaches the cells that never had an
//! explicit override, and leaves an explicitly styled cell alone. Inheritance is
//! therefore value-identity — no extra bit is stored, so nothing can disagree
//! with the values.

use crate::{CadResult, TableBorder, TableCellBorders};

/// The default table style name (`TABLESTYLE` dictionary's `Standard`).
pub const DEFAULT_TABLE_STYLE_NAME: &str = "Standard";

/// The default horizontal/vertical cell margin of a table style (DXF groups
/// 40/41 of the `TABLESTYLE` object).
pub const DEFAULT_TABLE_MARGIN: f64 = 0.06;

/// One table style snapshot: the `TABLESTYLE` object's modelled fields.
///
/// The values are the **data-row** values (the display has no title/header row
/// distinction: the entity does not record the row counts, so every row is a
/// data row — the named degradation the importer and the exporter report).
#[derive(Debug, Clone, PartialEq)]
pub struct TableStyleValues {
    /// The style name (DXF group 3), never empty.
    pub name: String,
    /// The style description (DXF group 4).
    pub description: String,
    /// Horizontal cell margin in drawing units (DXF group 40).
    pub horizontal_margin: f64,
    /// Vertical cell margin in drawing units (DXF group 41).
    pub vertical_margin: f64,
    /// Data-row text height in drawing units (row style group 140).
    pub text_height: f64,
    /// Data-row text style name (row style group 7).
    pub text_style_name: String,
    /// Data-row border visibility and double-line spacing of the four edges.
    pub borders: TableCellBorders,
    /// Whether the title row is suppressed (DXF group 280).
    pub title_suppressed: bool,
    /// Whether the header row is suppressed (DXF group 281).
    pub header_suppressed: bool,
}

impl Default for TableStyleValues {
    fn default() -> Self {
        Self::standard()
    }
}

impl TableStyleValues {
    /// The `Standard` table style: 2.5-unit text, the `Standard` text style,
    /// single visible borders on all four edges and the default margins.
    pub fn standard() -> Self {
        Self::named(DEFAULT_TABLE_STYLE_NAME)
    }

    /// A named style with the standard defaults (used for a source that names a
    /// style this build has no `TABLESTYLE` object for).
    pub fn named(name: &str) -> Self {
        TableStyleValues {
            name: if name.is_empty() {
                DEFAULT_TABLE_STYLE_NAME.to_string()
            } else {
                name.to_string()
            },
            description: String::new(),
            horizontal_margin: DEFAULT_TABLE_MARGIN,
            vertical_margin: DEFAULT_TABLE_MARGIN,
            text_height: crate::table_entity::DEFAULT_CELL_TEXT_HEIGHT,
            text_style_name: DEFAULT_TABLE_STYLE_NAME.to_string(),
            borders: TableCellBorders {
                top: TableBorder::VISIBLE,
                right: TableBorder::VISIBLE,
                bottom: TableBorder::VISIBLE,
                left: TableBorder::VISIBLE,
            },
            title_suppressed: false,
            header_suppressed: false,
        }
    }

    /// Structural validation of a style snapshot.
    ///
    /// Only **structural** defects are refused: an empty name, a non-finite or
    /// negative margin, and a non-finite or non-positive text height. A negative
    /// margin or a zero height cannot be written to a DXF `TABLESTYLE` and
    /// cannot drive a display, so it is never clamped silently.
    pub fn validate(&self) -> CadResult<()> {
        if self.name.trim().is_empty() {
            return Err(crate::CadError::InvalidInput(
                "table.style_name: a table style needs a non-empty name".into(),
            ));
        }
        if !self.horizontal_margin.is_finite()
            || !self.vertical_margin.is_finite()
            || self.horizontal_margin < 0.0
            || self.vertical_margin < 0.0
        {
            return Err(crate::CadError::InvalidInput(
                "table.style_margin: the cell margins must be finite and non-negative".into(),
            ));
        }
        if !self.text_height.is_finite() || self.text_height <= 0.0 {
            return Err(crate::CadError::InvalidInput(
                "table.style_text_height: the style text height must be finite and positive".into(),
            ));
        }
        Ok(())
    }

    /// Whether `cell` still inherits every style-provided value from `self`.
    pub(crate) fn supplies(&self, cell: &crate::TableCellGeometry) -> CellInheritance {
        CellInheritance {
            borders: cell.borders == self.borders,
            text_height: cell.text_height == self.text_height,
            text_style: cell.style_name == self.text_style_name,
        }
    }

    /// Scale every stored length by a uniform factor (the entity transform).
    pub(crate) fn scaled(&self, factor: f64) -> Self {
        TableStyleValues {
            name: self.name.clone(),
            description: self.description.clone(),
            horizontal_margin: self.horizontal_margin * factor,
            vertical_margin: self.vertical_margin * factor,
            text_height: self.text_height * factor,
            text_style_name: self.text_style_name.clone(),
            borders: TableCellBorders {
                top: scale_border(self.borders.top, factor),
                right: scale_border(self.borders.right, factor),
                bottom: scale_border(self.borders.bottom, factor),
                left: scale_border(self.borders.left, factor),
            },
            title_suppressed: self.title_suppressed,
            header_suppressed: self.header_suppressed,
        }
    }
}

/// Which of a cell's style-provided values still match the table style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CellInheritance {
    pub borders: bool,
    pub text_height: bool,
    pub text_style: bool,
}

fn scale_border(border: TableBorder, factor: f64) -> TableBorder {
    TableBorder {
        visible: border.visible,
        double_spacing: border.double_spacing * factor,
    }
}
