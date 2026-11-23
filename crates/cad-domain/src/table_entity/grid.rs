//! A cell's `GRIDFORMAT` margins: the one `GRIDFORMAT` sub-field this build both
//! **stores**, **draws** and **round-trips**.
//!
//! # What a `GRIDFORMAT` record carries, and what reaches the file
//!
//! A DXF `GRIDFORMAT` block (group `302` + group `95` edge mask) carries the
//! edge's border type (group `91`, single/double), colour (`62`), lineweight
//! (`92`), line type (`340`, an `LTYPE` **handle**), visibility (`93`) and
//! double-line spacing (`40`). The `ACAD_TABLE` **entity** record this build
//! writes carries none of those blocks — acadrust 0.6.8's entity writer emits
//! only the legacy groups (`65`-`69`, `275`-`279`, `285`-`289`) — so the values
//! ride the **`TABLECONTENT` object** the exporter writes beside the entity and
//! the importer reads back (see `cad_connector_acadrust::export::table`). The
//! per-cell margin block (`CELLMARGIN`, four group `40` values plus the two
//! spacings) is written and read the same way.
//!
//! # Why margins and not the other `GRIDFORMAT` sub-fields
//!
//! * **border type / double-line spacing** — already modelled per edge as
//!   [`TableBorder::double_spacing`](crate::TableBorder::double_spacing): a
//!   spacing above zero *is* the `Double` border type (the DXF record's group
//!   `91` is derived from it on export), and the display already draws the
//!   second line. Nothing new is stored here.
//! * **border line type** — deliberately **not** modelled. The only DXF group is
//!   an `LTYPE` *handle* (`340`), this build's fragment expansion overwrites a
//!   fragment's own linetype with the entity-resolved one, and the domain has no
//!   drawing-level `LTYPE` table to turn a border's name into a dash pattern. A
//!   stored value could neither draw nor round-trip, so the commands refuse it
//!   by name (`tabledit.format_unsupported`) instead of accepting it.
//! * **margins** — stored here, drawn as the text inset (see
//!   [`super::display`]) and written into the `CELLMARGIN` block.
//!
//! # Unset means inherit
//!
//! Every margin field is an `Option`: `None` is "no override", so the cell text
//! keeps the anchor it had before margins existed and the exporter writes no
//! `CELLMARGIN` block for that cell. A table that never sets a margin therefore
//! derives exactly the pre-margin display **and** the pre-margin file.

use super::TableFieldEdit;

/// One cell's four margin overrides, in drawing units.
///
/// Each field is independent: `TABLEdit FORMAT 1 1 MARGIN 0.2 INHERIT 0.2
/// INHERIT` (a partial form is not in the grammar, but the model accepts it)
/// stores the two values it names and leaves the other two inheriting.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TableCellMargins {
    /// Inset of the cell's text from its left edge.
    pub left: Option<f64>,
    /// Inset of the cell's text from its top edge.
    pub top: Option<f64>,
    /// Inset of the cell's text from its right edge.
    pub right: Option<f64>,
    /// Inset of the cell's text from its bottom edge.
    pub bottom: Option<f64>,
}

impl TableCellMargins {
    /// No override at all: the cell inherits its placement.
    pub const INHERIT: TableCellMargins = TableCellMargins {
        left: None,
        top: None,
        right: None,
        bottom: None,
    };

    /// A cell inset by the same value on all four edges.
    pub const fn uniform(value: f64) -> Self {
        TableCellMargins {
            left: Some(value),
            top: Some(value),
            right: Some(value),
            bottom: Some(value),
        }
    }

    /// A cell inset by its own four values.
    pub const fn new(left: f64, top: f64, right: f64, bottom: f64) -> Self {
        TableCellMargins {
            left: Some(left),
            top: Some(top),
            right: Some(right),
            bottom: Some(bottom),
        }
    }

    /// Whether the cell inherits its placement (no field is set).
    pub fn is_inherit(&self) -> bool {
        *self == Self::INHERIT
    }

    /// Whether every stored value is a finite non-negative length.
    ///
    /// A negative or non-finite margin cannot be written to a `CELLMARGIN`
    /// block and cannot drive a text anchor, so it is refused rather than
    /// clamped.
    pub fn is_valid(&self) -> bool {
        [self.left, self.top, self.right, self.bottom]
            .into_iter()
            .flatten()
            .all(|value| value.is_finite() && value >= 0.0)
    }

    /// The stored values in `[top, left, bottom, right]` order — the order a
    /// `CELLMARGIN` block writes its four group `40` values in.
    pub fn dxf_values(&self) -> [Option<f64>; 4] {
        [self.top, self.left, self.bottom, self.right]
    }

    /// Scale every stored value by a uniform factor (the entity transform).
    pub(crate) fn scaled(&self, factor: f64) -> Self {
        let scale = |value: Option<f64>| value.map(|value| value * factor);
        TableCellMargins {
            left: scale(self.left),
            top: scale(self.top),
            right: scale(self.right),
            bottom: scale(self.bottom),
        }
    }

    /// The short typed/report label (`INHERIT`, `0.20`, or
    /// `0.20/0.10/0.20/0.10` in left/top/right/bottom order).
    pub fn label(&self) -> String {
        if self.is_inherit() {
            return "INHERIT".to_string();
        }
        if let (Some(left), Some(top), Some(right), Some(bottom)) =
            (self.left, self.top, self.right, self.bottom)
        {
            if left == top && top == right && right == bottom {
                return format!("{left}");
            }
        }
        let part = |value: Option<f64>| match value {
            Some(value) => format!("{value}"),
            None => "INHERIT".to_string(),
        };
        format!(
            "{}/{}/{}/{}",
            part(self.left),
            part(self.top),
            part(self.right),
            part(self.bottom)
        )
    }
}

/// The margin override one cell stores, or [`TableCellMargins::INHERIT`].
///
/// The overrides are **sparse**: a table keeps one row only for a cell that
/// actually sets a margin, so a table that never sets one stores no table at all
/// and derives the pre-margin display and file exactly.
pub(crate) type CellMarginTable = std::collections::BTreeMap<(usize, usize), TableCellMargins>;

/// One cell's `CELLMARGIN` write (`TABLEDIT FORMAT MARGIN …`).
///
/// Four independent [`TableFieldEdit`]s, so a write can set one edge and leave
/// the others as they are — the same three-state rule as every other optional
/// cell field.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TableCellMarginsEdit {
    pub left: TableFieldEdit<f64>,
    pub top: TableFieldEdit<f64>,
    pub right: TableFieldEdit<f64>,
    pub bottom: TableFieldEdit<f64>,
}

impl TableCellMarginsEdit {
    /// A write that sets all four edges to the same value.
    pub const fn uniform(value: f64) -> Self {
        TableCellMarginsEdit {
            left: TableFieldEdit::Set(value),
            top: TableFieldEdit::Set(value),
            right: TableFieldEdit::Set(value),
            bottom: TableFieldEdit::Set(value),
        }
    }

    /// A write that clears all four edges (back to inherit).
    pub const CLEAR: TableCellMarginsEdit = TableCellMarginsEdit {
        left: TableFieldEdit::Clear,
        top: TableFieldEdit::Clear,
        right: TableFieldEdit::Clear,
        bottom: TableFieldEdit::Clear,
    };

    /// Whether the write leaves every edge alone.
    pub fn is_keep(&self) -> bool {
        self.left.is_keep() && self.top.is_keep() && self.right.is_keep() && self.bottom.is_keep()
    }
}
