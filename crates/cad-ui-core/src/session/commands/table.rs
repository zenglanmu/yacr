//! Typed-argument grammars for the table commands (`TABLE`, `-TABLE`,
//! `TABLEDIT`).
//!
//! Every parser is pure and strict: it admits only the documented tokens and
//! never clamps, defaults or repairs. The command layer re-validates the payload
//! (dimension caps, positive sizes, finite origin, 1-based cell range against
//! the selected table) and is the authority for the named refusals.
//!
//! Full contracts: `crate::app_drawing::table_edit` (re-exported from `cad-app`).
//!
//! * `TABLE <rows> <cols> <col_width> <row_height> <x> <y>` — an empty grid on
//!   the world XY plane. `rows`/`cols` are positive integers (the command caps
//!   them), the two sizes are positive decimals and the origin is a pair of
//!   signed decimals.
//! * `-TABLE …` — the same grammar with the command-line selector spelling.
//! * `TABLEDIT <row> <col> [text...]` — a **1-based** cell address followed by
//!   the text remainder. The remainder is the rest of the line with spaces
//!   preserved (only leading whitespace is dropped) and may be empty, which
//!   clears the cell.

use cad_app::{TableCreateCommand, TableEditCommand};
use cad_domain::Point3;

use super::arguments::{parse_positive_decimal, parse_signed_decimal};

/// `TABLE <rows> <cols> <col_width> <row_height> <x> <y>`.
pub const TABLE_SELECTOR: &str = "TABLE";

/// `-TABLE <rows> <cols> <col_width> <row_height> <x> <y>`.
pub const DASH_TABLE_SELECTOR: &str = "-TABLE";

/// `TABLEDIT <row> <col> [text...]`.
pub const TABLEDIT_SELECTOR: &str = "TABLEDIT";

/// Parse the `TABLE` / `-TABLE` argument: six whitespace-separated tokens.
///
/// The row/column counts are positive `u32` integers (zero, a sign, a fraction
/// and a value beyond `u32` are refused here); the column width and row height
/// are positive decimals; the origin `x`/`y` are signed finite decimals and `z`
/// is fixed at `0` (the world XY work plane). Nothing is clamped: the command
/// owns the `1000 × 1000` / `100 000`-cell caps.
pub fn parse_table_create_argument(argument: &str) -> Option<TableCreateCommand> {
    let tokens: Vec<&str> = argument.split_whitespace().collect();
    if tokens.len() != 6 {
        return None;
    }
    Some(TableCreateCommand {
        rows: parse_positive_integer(tokens[0])?,
        cols: parse_positive_integer(tokens[1])?,
        col_width: parse_positive_decimal(tokens[2])?,
        row_height: parse_positive_decimal(tokens[3])?,
        origin: Point3 {
            x: parse_signed_decimal(tokens[4])?,
            y: parse_signed_decimal(tokens[5])?,
            z: 0.0,
        },
    })
}

/// Parse the `TABLEDIT` argument: a 1-based cell address plus the text
/// remainder.
///
/// The first two tokens are positive integers (1-based, so `0` is refused here
/// as out of the address space). The remainder is the rest of the line after
/// those two tokens: internal and trailing spaces are preserved and only the
/// leading whitespace between the column and the text is dropped. No remainder
/// is an explicit empty string (clear the cell), never a default.
pub fn parse_tabledit_argument(argument: &str) -> Option<TableEditCommand> {
    let text = argument.trim_start();
    let (row, rest) = split_first_token(text)?;
    let (col, rest) = split_first_token(rest)?;
    Some(TableEditCommand {
        row: parse_positive_integer(row)?,
        col: parse_positive_integer(col)?,
        text: rest.trim_start().to_string(),
    })
}

/// Split off the first whitespace-delimited token (the remainder is returned
/// untrimmed).
fn split_first_token(text: &str) -> Option<(&str, &str)> {
    let text = text.trim_start();
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    let token = &text[..end];
    if token.is_empty() {
        return None;
    }
    Some((token, &text[end..]))
}

/// One positive `u32` written with ASCII digits only.
fn parse_positive_integer(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse::<u32>().ok().filter(|value| *value > 0)
}
