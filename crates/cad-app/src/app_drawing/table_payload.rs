//! The typed payloads of the post-creation table commands (`TABLEDIT`'s merge
//! and format forms, `MATCHCELL`, `TABLESTYLE`).
//!
//! Grouped behind one [`crate::CommandPayload::Table`] variant so the command
//! enum itself stays inside the file-size budget — the same decision
//! `draw2_payload.rs` records for the 2026-11-07 draw/modify batch. Every payload
//! here is plain data: validation, refusals, the drawing read and the single
//! transaction belong to `app_drawing::table_ops`.
//!
//! Addressing is **1-based** on the wire and in these payloads (AutoCAD's table
//! cell addressing), because that is what the typed lines carry; the command
//! converts to the 0-based grid once, with the same `checked_sub` rule `TABLEDIT`
//! already uses (`0` is outside the address space and becomes the ordinary
//! out-of-range refusal).

use cad_domain::{TableCellAlignment, TableRowKind};

/// One 1-based cell address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableCellAddress {
    pub row: u32,
    pub col: u32,
}

impl TableCellAddress {
    /// The 0-based `(row, column)` grid address, or `None` for a zero address.
    pub fn zero_based(self) -> Option<(usize, usize)> {
        let row = self.row.checked_sub(1)?;
        let col = self.col.checked_sub(1)?;
        Some((row as usize, col as usize))
    }
}

/// Which edge of a table-style cell border an assignment addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableStyleEdge {
    Top,
    Right,
    Bottom,
    Left,
    /// All four edges (the shorthand the typed grammar accepts).
    All,
}

/// Which of a style's three row value sets an assignment reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableStyleRowScope {
    /// One row type (`TABLESTYLE ROW TITLE …`).
    One(TableRowKind),
    /// All three value sets: the default, so a plain `TABLESTYLE HEIGHT 3` keeps
    /// every row consistent with the pre-row-style behaviour.
    All,
}

impl TableStyleRowScope {
    /// The row types this scope reaches.
    pub fn kinds(self) -> &'static [TableRowKind] {
        const ALL: [TableRowKind; 3] = [
            TableRowKind::Data,
            TableRowKind::Title,
            TableRowKind::Header,
        ];
        const DATA: [TableRowKind; 1] = [TableRowKind::Data];
        const TITLE: [TableRowKind; 1] = [TableRowKind::Title];
        const HEADER: [TableRowKind; 1] = [TableRowKind::Header];
        match self {
            TableStyleRowScope::All => &ALL,
            TableStyleRowScope::One(TableRowKind::Data) => &DATA,
            TableStyleRowScope::One(TableRowKind::Title) => &TITLE,
            TableStyleRowScope::One(TableRowKind::Header) => &HEADER,
        }
    }
}

/// One row-scoped group of `TABLESTYLE` assignments.
///
/// Every field is `Option` (or an empty list) so "not written" and "written with
/// this value" stay distinguishable: the command starts from the current
/// snapshot and only the present fields are replaced.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TableRowStyleCommand {
    /// Which row value sets this group writes.
    pub scope: Option<TableStyleRowScope>,
    pub text_height: Option<f64>,
    pub text_style_name: Option<String>,
    pub alignment: Option<TableCellAlignment>,
    /// `(edge, visible, double-line spacing)` border assignments.
    pub borders: Vec<(TableStyleEdge, bool, Option<f64>)>,
}

impl TableRowStyleCommand {
    /// Whether this group carries any row-scoped value.
    pub fn is_empty(&self) -> bool {
        self.text_height.is_none()
            && self.text_style_name.is_none()
            && self.alignment.is_none()
            && self.borders.is_empty()
    }
}

/// One typed assignment of a `TABLESTYLE` write.
///
/// The style-level fields (`name`, `description`, margins, row suppression) and
/// the row-scoped groups are separate: the first change the `TABLESTYLE` object
/// itself, the second change one or more of its three row value sets.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TableStyleCommand {
    /// The style name. `None` means "keep the table's current style name".
    pub name: Option<String>,
    pub description: Option<String>,
    pub horizontal_margin: Option<f64>,
    pub vertical_margin: Option<f64>,
    /// Row-scoped field groups, in typed order.
    pub rows: Vec<TableRowStyleCommand>,
    pub title_suppressed: Option<bool>,
    pub header_suppressed: Option<bool>,
}

impl TableStyleCommand {
    /// Whether any field would be written (an empty write is refused by name
    /// instead of bumping a revision for nothing).
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.description.is_none()
            && self.horizontal_margin.is_none()
            && self.vertical_margin.is_none()
            && self.rows.iter().all(TableRowStyleCommand::is_empty)
            && self.title_suppressed.is_none()
            && self.header_suppressed.is_none()
    }
}

/// One post-creation table command's payload.
#[derive(Debug, Clone, PartialEq)]
pub enum TablePayload {
    /// `TABLEDIT MERGE <row> <col> <height> <width>`: merge the range anchored at
    /// the 1-based cell into `height` rows x `width` columns.
    Merge {
        anchor: TableCellAddress,
        height: u32,
        width: u32,
    },
    /// `TABLEDIT UNMERGE <row> <col>`: split the range covering the cell.
    Unmerge { cell: TableCellAddress },
    /// `TABLEDIT FORMAT <row> <col> [ALIGN <code>] [ROTATION <degrees>]`: set the
    /// cell's own alignment and/or text rotation.
    Format {
        cell: TableCellAddress,
        alignment: Option<TableCellAlignment>,
        /// Degrees, converted to the model's radians by the parser.
        rotation: Option<f64>,
    },
    /// `MATCHCELL <src_row> <src_col> <dst_row> <dst_col> [...]`: copy the first
    /// cell's format onto every other cell.
    MatchCell {
        source: TableCellAddress,
        targets: Vec<TableCellAddress>,
    },
    /// `TABLESTYLE [<name>] [<field> <value> ...]`: assign the table's style.
    Style(Box<TableStyleCommand>),
}
