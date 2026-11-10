//! The typed `TABLE` / `-TABLE` / `TABLEDIT` payload data.
//!
//! Split out of `payload_extra.rs` to keep every code file inside the
//! 1000-line gate. The command enum still owns the variants; this module owns
//! the data, following the existing Boxed payload pattern.
//!
//! The creation grammar is the declared typed form of `TABLE` (AutoCAD opens a
//! dialog, this build has no dialog):
//!
//! `TABLE <rows> <cols> <col_width> <row_height> <x> <y>`
//!
//! `rows`/`cols` are positive integers (capped by the command at 1000 each and
//! 100 000 total cells), `col_width`/`row_height` are finite positive lengths
//! and `(x, y, 0)` is the table frame origin on the world XY plane. There is no
//! rotation or style field in v1: the frame is `+X` horizontal / `+Z` normal and
//! every cell uses the `Standard` style with single visible borders.
//!
//! `TABLEDIT` uses a **pre-selection** model: the selection is the rest of the
//! input, and this payload carries the 1-based cell address plus the text
//! **remainder of the typed line** (spaces preserved, only leading whitespace
//! trimmed; an empty remainder clears the cell).

use cad_domain::Point3;

/// The input of a `TABLE` / `-TABLE` command: one empty table grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TableCreateCommand {
    /// Number of rows (cad_app caps it at 1000).
    pub rows: u32,
    /// Number of columns (cad_app caps it at 1000).
    pub cols: u32,
    /// Width of every column in drawing units (finite, > 0).
    pub col_width: f64,
    /// Height of every row in drawing units (finite, > 0).
    pub row_height: f64,
    /// The table frame origin on the world XY plane (`z` is always 0 here).
    pub origin: Point3,
}

/// The input of a `TABLEDIT` command: one 1-based cell address and its text.
///
/// `row`/`col` are **1-based** (AutoCAD's table cell addressing), so the first
/// cell is `TABLEDIT 1 1 <text>`. `text` is the verbatim remainder of the typed
/// line: internal and trailing spaces are preserved and an empty string clears
/// the cell.
#[derive(Debug, Clone, PartialEq)]
pub struct TableEditCommand {
    pub row: u32,
    pub col: u32,
    pub text: String,
}
