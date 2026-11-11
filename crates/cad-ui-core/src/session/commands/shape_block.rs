//! Typed-argument grammars for `SHAPE`, `MINSERT` and `ADDSELECTED`.
//!
//! Every parser is pure and strict: it admits only the documented tokens and
//! never clamps, defaults or repairs. The command layer re-validates the payload
//! (finite points, name grammar, positive size, the array cap, the selected
//! source type) and is the authority for the named `shape.*` / `minsert.*` /
//! `addselected.*` refusals.
//!
//! Contracts: `crate::app_drawing::shape` (`SHAPE`, `SHP2BLK`),
//! `crate::app_drawing::minsert` and `crate::app_drawing::addselected`.

use cad_app::{AddSelectedCommand, MInsertCommand};
use cad_domain::Point3;

use super::arguments::{parse_positive_decimal, parse_signed_decimal};

pub const SHAPE_SELECTOR: &str = "SHAPE";
pub const MINSERT_SELECTOR: &str = "MINSERT";
pub const ADDSELECTED_SELECTOR: &str = "ADDSELECTED";

/// The syntax-only result of `SHAPE <name-or-code> <x> <y> [size] [rotation_deg]`.
///
/// `size` and `rotation_degrees` stay optional here because the omitted form
/// takes the session's current text height/rotation (the same typed channel TEXT
/// creation reads); the dispatcher fills them before building the payload.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeArgument {
    /// The shape name as typed (empty for the code form).
    pub name: String,
    /// The shape code; `0` is the name form.
    pub code: u32,
    pub position: Point3,
    pub size: Option<f64>,
    pub rotation_degrees: Option<f64>,
}

/// `SHAPE <name-or-code> <x> <y> [size] [rotation_deg]`.
///
/// The first token is a positive plain integer shape code when it parses as one;
/// any other single token is the shape name (the name form). `x`/`y` are signed
/// plain decimals on the flat work plane (`z = 0`), `size` is a positive plain
/// decimal and `rotation_deg` a signed plain decimal in degrees. Everything else
/// (a zero code, a bare `0`, extra tokens, exponents) is outside the grammar and
/// keeps the stable `shape.argument` refusal.
pub fn parse_shape_argument(argument: &str) -> Option<ShapeArgument> {
    let tokens = tokens(argument);
    if tokens.len() < 3 || tokens.len() > 5 {
        return None;
    }
    // An all-digit token is a **code** and must be a positive `u32`; `0` is
    // reserved for the name form and is refused rather than read as a shape
    // literally named `0`.
    let (name, code) = if tokens[0].bytes().all(|byte| byte.is_ascii_digit()) {
        (
            String::new(),
            tokens[0].parse::<u32>().ok().filter(|code| *code > 0)?,
        )
    } else {
        (tokens[0].to_string(), 0)
    };
    Some(ShapeArgument {
        name,
        code,
        position: Point3 {
            x: parse_signed_decimal(tokens[1])?,
            y: parse_signed_decimal(tokens[2])?,
            z: 0.0,
        },
        size: match tokens.get(3) {
            Some(token) => Some(parse_positive_decimal(token)?),
            None => None,
        },
        rotation_degrees: match tokens.get(4) {
            Some(token) => Some(parse_signed_decimal(token)?),
            None => None,
        },
    })
}

/// `MINSERT <block> <rows> <columns> <row_spacing> <column_spacing> <x> <y>`.
///
/// Rows and columns are positive plain integers (`parse::<u32>`); the two
/// spacings are signed plain decimals (the cell offset *is* the signed spacing,
/// so a negative value arrays down/left); the point is on the flat work plane.
pub fn parse_minsert_argument(argument: &str) -> Option<MInsertCommand> {
    let tokens = tokens(argument);
    if tokens.len() != 7 {
        return None;
    }
    let rows = parse_positive_count(tokens[1])?;
    let columns = parse_positive_count(tokens[2])?;
    Some(MInsertCommand {
        block: tokens[0].to_string(),
        rows,
        columns,
        row_spacing: parse_signed_decimal(tokens[3])?,
        column_spacing: parse_signed_decimal(tokens[4])?,
        insertion: Point3 {
            x: parse_signed_decimal(tokens[5])?,
            y: parse_signed_decimal(tokens[6])?,
            z: 0.0,
        },
    })
}

/// `ADDSELECTED <x> <y>`.
///
/// The two coordinates are signed plain decimals on the flat work plane; the
/// current selection is the source entity.
pub fn parse_addselected_argument(argument: &str) -> Option<AddSelectedCommand> {
    let tokens = tokens(argument);
    if tokens.len() != 2 {
        return None;
    }
    Some(AddSelectedCommand {
        position: Point3 {
            x: parse_signed_decimal(tokens[0])?,
            y: parse_signed_decimal(tokens[1])?,
            z: 0.0,
        },
    })
}

/// One positive plain integer that fits `u32` (a row/column count).
fn parse_positive_count(token: &str) -> Option<u32> {
    if token.is_empty() || !token.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    token.parse::<u32>().ok().filter(|count| *count > 0)
}

fn tokens(argument: &str) -> Vec<&str> {
    argument.split_whitespace().collect()
}
