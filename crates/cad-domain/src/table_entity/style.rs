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
//! object and read back, so nothing is stored without a round-trip. That now
//! includes the object's **three row value sets** — the data row
//! ([`TableStyleValues::data_row`]), the title row and the header row — because
//! the DXF writer emits one `AcDbTableStyle` row record per row type (groups
//! 7/140/170 and six border records each) and the reader fills all three. The
//! per-cell border colours/lineweights/fills and the annotative flag are *not*
//! part of it (the display has one colour per entity and no filled area); the
//! importer/export report those gaps by name.
//!
//! # Which rows use which value set
//!
//! The `ACAD_TABLE` cell records carry no row-type group, but the style's own
//! title/header suppression flags (groups 280/281) say whether the style has a
//! title row and a header row. The style therefore classifies rows by position —
//! row 0 is the title row when a title row is shown, the next row is the header
//! row when a header row is shown, and every remaining row is a data row — with
//! [`TableStyleValues::row_kind`] as the single rule.
//!
//! A title/header value set is stored only when it **differs** from the data row
//! ([`Self::title_row`]/[`Self::header_row`] are `Option`, and `None` means
//! "same as the data row"). That keeps the pre-existing single-set behaviour
//! byte-identical: a style that never distinguished a row type has one value set
//! and every row inherits it, exactly as before. Writing a row set back to the
//! data row's values re-links it (value identity, no flag).
//!
//! # Cells referencing the style
//!
//! A cell whose border set, text height, text style name and alignment still
//! equal **its row's** values is an **inheriting** cell: it takes them from the
//! style rather than from itself. [`TableGeometry::with_style_resolved`]
//! re-resolves those cells to the new values, so `TABLESTYLE` reaches the cells
//! that never had an explicit override, and leaves an explicitly styled cell
//! alone. Inheritance is therefore value-identity — no extra bit is stored, so
//! nothing can disagree with the values.
//!
//! Text rotation is the one cell field with no style counterpart: a `TABLESTYLE`
//! row record has no rotation group in this DXF writer, so a per-row rotation
//! could never round-trip and is not modelled. A cell's rotation is always its
//! own.

use crate::{CadResult, TableBorder, TableCellBorders};

use super::format::{TableCellAlignment, TableRowKind, TableRowStyle};

/// The default table style name (`TABLESTYLE` dictionary's `Standard`).
pub const DEFAULT_TABLE_STYLE_NAME: &str = "Standard";

/// The default horizontal/vertical cell margin of a table style (DXF groups
/// 40/41 of the `TABLESTYLE` object).
pub const DEFAULT_TABLE_MARGIN: f64 = 0.06;

/// One table style snapshot: the `TABLESTYLE` object's modelled fields.
///
/// `text_height`, `text_style_name`, `borders` and `data_row_alignment` are the
/// **data row** value set; [`Self::title_row`]/[`Self::header_row`] hold the
/// other two sets when they are distinguished from it.
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
    /// Data-row cell text alignment (row style group 170).
    pub data_row_alignment: TableCellAlignment,
    /// The title row's value set; `None` means "the data row's values".
    pub title_row: Option<TableRowStyle>,
    /// The header row's value set; `None` means "the data row's values".
    pub header_row: Option<TableRowStyle>,
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
    /// single visible borders on all four edges, centre/middle cell alignment
    /// and the default margins. All three row value sets are the same, so every
    /// row inherits the data row (the pre-row-style behaviour).
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
            data_row_alignment: TableCellAlignment::DEFAULT,
            title_row: None,
            header_row: None,
            title_suppressed: false,
            header_suppressed: false,
        }
    }

    /// The data row's value set, assembled from the data-row fields.
    pub fn data_row(&self) -> TableRowStyle {
        TableRowStyle {
            text_height: self.text_height,
            text_style_name: self.text_style_name.clone(),
            alignment: self.data_row_alignment,
            borders: self.borders,
        }
    }

    /// Which value set a row uses, from the style's suppression flags.
    ///
    /// Row 0 is the title row when a title row is shown; the next row (if any)
    /// is the header row when a header row is shown; everything else is a data
    /// row. A style whose flags show both row types on a table with fewer rows
    /// simply has no header (or title) row to classify.
    pub fn row_kind(&self, row_index: usize, row_count: usize) -> TableRowKind {
        let title_rows = usize::from(!self.title_suppressed && row_count >= 1);
        let header_rows = usize::from(!self.header_suppressed && row_count > title_rows);
        if row_index < title_rows {
            TableRowKind::Title
        } else if row_index < title_rows + header_rows {
            TableRowKind::Header
        } else {
            TableRowKind::Data
        }
    }

    /// The value set `kind` supplies, with a `None` row falling back to the data
    /// row's values.
    pub fn row_style(&self, kind: TableRowKind) -> TableRowStyle {
        match kind {
            TableRowKind::Data => self.data_row(),
            TableRowKind::Title => self.title_row.clone().unwrap_or_else(|| self.data_row()),
            TableRowKind::Header => self.header_row.clone().unwrap_or_else(|| self.data_row()),
        }
    }

    /// Store one row's value set.
    ///
    /// Writing the data row updates the data-row fields. Writing a title/header
    /// set equal to the data row's values stores `None` instead, so "the same
    /// values" and "no distinction" are one state (value identity, no flag).
    pub fn set_row_style(&mut self, kind: TableRowKind, row: TableRowStyle) {
        match kind {
            TableRowKind::Data => {
                self.text_height = row.text_height;
                self.text_style_name = row.text_style_name;
                self.borders = row.borders;
                self.data_row_alignment = row.alignment;
                // A row that already held exactly the old data-row values was
                // indistinguishable from "follows the data row"; keep it linked
                // when the data row has caught up with it.
                let data = self.data_row();
                self.title_row = self.title_row.take().filter(|title| *title != data);
                self.header_row = self.header_row.take().filter(|header| *header != data);
            }
            TableRowKind::Title => {
                let data = self.data_row();
                self.title_row = (row != data).then_some(row);
            }
            TableRowKind::Header => {
                let data = self.data_row();
                self.header_row = (row != data).then_some(row);
            }
        }
    }

    /// Structural validation of a style snapshot.
    ///
    /// Only **structural** defects are refused: an empty name, a non-finite or
    /// negative margin, and a non-finite or non-positive text height (of any row
    /// value set). A negative margin or a zero height cannot be written to a DXF
    /// `TABLESTYLE` and cannot drive a display, so it is never clamped silently.
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
        for row in [
            Some(self.data_row()),
            self.title_row.clone(),
            self.header_row.clone(),
        ]
        .into_iter()
        .flatten()
        {
            if !row.text_height.is_finite() || row.text_height <= 0.0 {
                return Err(crate::CadError::InvalidInput(
                    "table.style_text_height: the style text height must be finite and positive"
                        .into(),
                ));
            }
        }
        Ok(())
    }

    /// Whether `cell` still inherits every style-provided value from `row`.
    pub(crate) fn supplies_row(
        &self,
        cell: &crate::TableCellGeometry,
        row: &TableRowStyle,
    ) -> CellInheritance {
        CellInheritance {
            borders: cell.borders == row.borders,
            text_height: cell.text_height == row.text_height,
            text_style: cell.style_name == row.text_style_name,
            alignment: cell.alignment == row.alignment,
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
            data_row_alignment: self.data_row_alignment,
            title_row: self.title_row.as_ref().map(|row| scale_row(row, factor)),
            header_row: self.header_row.as_ref().map(|row| scale_row(row, factor)),
            title_suppressed: self.title_suppressed,
            header_suppressed: self.header_suppressed,
        }
    }
}

/// Which of a cell's style-provided values still match its row's value set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CellInheritance {
    pub borders: bool,
    pub text_height: bool,
    pub text_style: bool,
    pub alignment: bool,
}

fn scale_row(row: &TableRowStyle, factor: f64) -> TableRowStyle {
    TableRowStyle {
        text_height: row.text_height * factor,
        text_style_name: row.text_style_name.clone(),
        alignment: row.alignment,
        borders: TableCellBorders {
            top: scale_border(row.borders.top, factor),
            right: scale_border(row.borders.right, factor),
            bottom: scale_border(row.borders.bottom, factor),
            left: scale_border(row.borders.left, factor),
        },
    }
}

fn scale_border(border: TableBorder, factor: f64) -> TableBorder {
    TableBorder {
        visible: border.visible,
        double_spacing: border.double_spacing * factor,
    }
}
