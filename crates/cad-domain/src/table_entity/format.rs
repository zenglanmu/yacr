//! Per-cell and per-row formatting: the parts of a table cell's appearance that
//! this build both **stores** and **draws**.
//!
//! Two fields qualify today, and both ride on the DXF records acadrust 0.6.8
//! already models:
//!
//! * **text alignment** — the 3 x 3 cell-alignment code (DXF group 170 of a
//!   `TABLECELL`, group 170 of each `TABLESTYLE` row record). It reaches the
//!   display as the `Text` child's `h_align`/`v_align` plus its anchor corner.
//! * **text rotation** — the per-cell rotation in radians (DXF group 145 of a
//!   `TABLECELL`). It is added to the table frame's own rotation and becomes the
//!   `Text` child's `rotation`.
//!
//! The remaining per-cell appearance fields — content colour (group 64),
//! background fill (groups 63 + 283) and border colour/lineweight (groups
//! 65-69, 275-279) — are modelled as well:
//! [`TableCellGeometry::text_color`](crate::TableCellGeometry::text_color),
//! [`TableCellGeometry::fill_color`](crate::TableCellGeometry::fill_color) and
//! [`TableBorder`]'s own `color`/`lineweight_hundredths`. They ride this table's
//! per-primitive appearance channel, so each one is stored, drawn and written
//! back; a cell that sets none of them derives exactly the pre-appearance draw.
//!
//! What a row value set can supply is the same list: text colour, fill and the
//! four edges' colour/lineweight. A row's text **rotation** stays absent — a
//! `TABLESTYLE` row record has no rotation group in this DXF writer, so a
//! per-row rotation could never come back.
//!
//! A bare cell rotation is *relative* to the table frame (DXF stores it in the
//! table's own coordinate system), which is why it survives a
//! [`TableGeometry::transformed`](crate::TableGeometry::transformed) unchanged
//! while the frame's rotation moves.

use crate::{TableBorder, TableCellBorders, DEFAULT_CELL_TEXT_HEIGHT};

use super::DEFAULT_TABLE_STYLE_NAME;

/// Horizontal placement of a cell's text inside its cell.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TableCellAlignH {
    Left,
    #[default]
    Center,
    Right,
}

/// Vertical placement of a cell's text inside its cell.
///
/// The three values are exactly the vertical half of the DXF cell-alignment
/// code; there is no "baseline" case (a table cell aligns its content box).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TableCellAlignV {
    Top,
    #[default]
    Middle,
    Bottom,
}

/// One cell's text alignment: the 3 x 3 DXF cell-alignment code.
///
/// [`Self::DEFAULT`] (`MiddleCenter`, DXF code 5) is the value every cell had
/// before alignment was modelled, so a table that never sets it derives the same
/// display it always did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TableCellAlignment {
    pub horizontal: TableCellAlignH,
    pub vertical: TableCellAlignV,
}

impl TableCellAlignment {
    /// The DXF default: middle/centre (code 5).
    pub const DEFAULT: TableCellAlignment = TableCellAlignment {
        horizontal: TableCellAlignH::Center,
        vertical: TableCellAlignV::Middle,
    };

    /// The alignment a DXF cell-alignment code names.
    ///
    /// Code 1-9 is the documented 3 x 3 grid (top-left first, reading across and
    /// down). Every other value — including the `0` a cell style carries when
    /// the source wrote no group 170 — resolves to [`Self::DEFAULT`], which is
    /// what acadrust's own `CellAlignment::from` does.
    pub fn from_dxf_code(code: i32) -> Self {
        let horizontal = match (code - 1).rem_euclid(3) {
            0 => TableCellAlignH::Left,
            1 => TableCellAlignH::Center,
            _ => TableCellAlignH::Right,
        };
        let vertical = match (code - 1).div_euclid(3) {
            0 => TableCellAlignV::Top,
            1 => TableCellAlignV::Middle,
            _ => TableCellAlignV::Bottom,
        };
        if (1..=9).contains(&code) {
            TableCellAlignment {
                horizontal,
                vertical,
            }
        } else {
            Self::DEFAULT
        }
    }

    /// The DXF cell-alignment code (1-9) this alignment writes.
    pub fn dxf_code(self) -> i32 {
        let column = match self.horizontal {
            TableCellAlignH::Left => 0,
            TableCellAlignH::Center => 1,
            TableCellAlignH::Right => 2,
        };
        let row = match self.vertical {
            TableCellAlignV::Top => 0,
            TableCellAlignV::Middle => 1,
            TableCellAlignV::Bottom => 2,
        };
        row * 3 + column + 1
    }

    /// The two-letter code the typed grammar and the `TABLESTYLE` report use
    /// (`TL`, `TC`, `TR`, `ML`, `MC`, `MR`, `BL`, `BC`, `BR`).
    pub fn label(self) -> &'static str {
        match (self.vertical, self.horizontal) {
            (TableCellAlignV::Top, TableCellAlignH::Left) => "TL",
            (TableCellAlignV::Top, TableCellAlignH::Center) => "TC",
            (TableCellAlignV::Top, TableCellAlignH::Right) => "TR",
            (TableCellAlignV::Middle, TableCellAlignH::Left) => "ML",
            (TableCellAlignV::Middle, TableCellAlignH::Center) => "MC",
            (TableCellAlignV::Middle, TableCellAlignH::Right) => "MR",
            (TableCellAlignV::Bottom, TableCellAlignH::Left) => "BL",
            (TableCellAlignV::Bottom, TableCellAlignH::Center) => "BC",
            (TableCellAlignV::Bottom, TableCellAlignH::Right) => "BR",
        }
    }

    /// Parse one [`Self::label`], case-insensitively. `None` for anything else,
    /// so the typed grammar refuses an unknown code instead of defaulting it.
    pub fn from_label(text: &str) -> Option<Self> {
        const LABELS: [&str; 9] = ["TL", "TC", "TR", "ML", "MC", "MR", "BL", "BC", "BR"];
        let index = LABELS
            .iter()
            .position(|label| label.eq_ignore_ascii_case(text.trim()))?;
        Some(Self::from_dxf_code(index as i32 + 1))
    }
}

/// Which of a table style's three row value sets a row uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableRowKind {
    Title,
    Header,
    Data,
}

impl TableRowKind {
    /// The word the typed grammar and the report use.
    pub fn label(self) -> &'static str {
        match self {
            TableRowKind::Title => "TITLE",
            TableRowKind::Header => "HEADER",
            TableRowKind::Data => "DATA",
        }
    }

    /// Parse one [`Self::label`], case-insensitively.
    pub fn from_label(text: &str) -> Option<Self> {
        const KINDS: [TableRowKind; 3] = [
            TableRowKind::Title,
            TableRowKind::Header,
            TableRowKind::Data,
        ];
        KINDS
            .into_iter()
            .find(|kind| kind.label().eq_ignore_ascii_case(text.trim()))
    }
}

/// One row value set of a table style: the fields a row type supplies to the
/// cells that did not override them.
///
/// The fields are exactly the ones the `TABLESTYLE` object writes per row
/// (groups 7/140/170 and the six border records of `AcDbTableStyle`), so the
/// whole set round-trips. A row's text **rotation** is deliberately absent: a
/// `TABLESTYLE` row record has no rotation group in this DXF writer, so a
/// per-row rotation could never come back — the per-cell rotation (group 145)
/// is the only rotation that round-trips, and it is always a cell's own value.
#[derive(Debug, Clone, PartialEq)]
pub struct TableRowStyle {
    /// Text height in drawing units (row group 140).
    pub text_height: f64,
    /// Text style name (row group 7).
    pub text_style_name: String,
    /// Cell text alignment (row group 170).
    pub alignment: TableCellAlignment,
    /// The four edges' visibility, double-line spacing, colour and lineweight
    /// (row groups 64-69 colours, 274-279 lineweights, 284-289 visibility).
    pub borders: TableCellBorders,
    /// The row's cell text colour, sRGB (row group 62). `None` inherits the
    /// entity colour, exactly like a `ByBlock` row record.
    pub text_color: Option<[u8; 3]>,
    /// The row's cell background fill, sRGB (row groups 63 + 283). `None` means
    /// the row enables no fill.
    pub fill_color: Option<[u8; 3]>,
}

impl Default for TableRowStyle {
    fn default() -> Self {
        TableRowStyle {
            text_height: DEFAULT_CELL_TEXT_HEIGHT,
            text_style_name: DEFAULT_TABLE_STYLE_NAME.to_string(),
            alignment: TableCellAlignment::DEFAULT,
            borders: TableCellBorders {
                top: TableBorder::VISIBLE,
                right: TableBorder::VISIBLE,
                bottom: TableBorder::VISIBLE,
                left: TableBorder::VISIBLE,
            },
            text_color: None,
            fill_color: None,
        }
    }
}
