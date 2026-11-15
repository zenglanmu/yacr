//! The typed payloads of the post-creation table commands (`TABLEDIT`'s merge
//! forms, `MATCHCELL`, `TABLESTYLE`).
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

/// One typed assignment of a `TABLESTYLE` write.
///
/// Every field is `Option` so "not written" and "written with this value" stay
/// distinguishable: the command starts from the current snapshot (or the named
/// default) and only the present fields are replaced.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TableStyleCommand {
    /// The style name. `None` means "keep the table's current style name".
    pub name: Option<String>,
    pub description: Option<String>,
    pub horizontal_margin: Option<f64>,
    pub vertical_margin: Option<f64>,
    pub text_height: Option<f64>,
    pub text_style_name: Option<String>,
    /// `(edge, visible, double-line spacing)` border assignments.
    pub borders: Vec<(TableStyleEdge, bool, Option<f64>)>,
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
            && self.text_height.is_none()
            && self.text_style_name.is_none()
            && self.borders.is_empty()
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
    /// `MATCHCELL <src_row> <src_col> <dst_row> <dst_col> [...]`: copy the first
    /// cell's style onto every other cell.
    MatchCell {
        source: TableCellAddress,
        targets: Vec<TableCellAddress>,
    },
    /// `TABLESTYLE [<name>] [<field> <value> ...]`: assign the table's style.
    Style(Box<TableStyleCommand>),
}
