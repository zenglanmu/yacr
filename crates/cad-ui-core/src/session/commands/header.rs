//! The typed `BASE` / `LIMITS` argument grammars.
//!
//! Split out of `commands::arguments` (which is at the file-size ceiling) so the
//! two vector-header writes keep their parsers beside each other. Both are pure:
//! they decide whether the trimmed argument is inside the selector's grammar and,
//! when it is, return the typed numbers the session dispatches. Nothing is
//! clamped, defaulted or silently repaired.
//!
//! `BASE` and `LIMITS` write `$INSBASE` / `$LIMMIN` / `$LIMMAX` through
//! `cad-db`'s `drawing::header_geometry`; the bare selectors keep their read-only
//! reports (`session::header_arguments` owns the dispatch).

use super::arguments::parse_signed_decimal;

/// `BASE <x> <y> [<z>]`: the model-space insertion base point (`$INSBASE`).
pub const BASE_SELECTOR: &str = "BASE";

/// `LIMITS <min_x> <min_y> <max_x> <max_y>`: the model-space limits
/// (`$LIMMIN` / `$LIMMAX`).
pub const LIMITS_SELECTOR: &str = "LIMITS";

/// Parse `BASE <x> <y> [<z>]`: two or three finite plain decimals.
///
/// Components may be separated by whitespace or commas. A missing `z` is `0.0`
/// (the DXF default for a 2D point); nothing else is defaulted.
pub fn parse_base_argument(argument: &str) -> Option<[f64; 3]> {
    let components = parse_plain_decimals(argument)?;
    match components.as_slice() {
        [x, y] => Some([*x, *y, 0.0]),
        [x, y, z] => Some([*x, *y, *z]),
        _ => None,
    }
}

/// Parse `LIMITS <min_x> <min_y> <max_x> <max_y>`: exactly four finite plain
/// decimals (whitespace or comma separated).
///
/// Both corners travel together because `$LIMMIN`/`$LIMMAX` are one rectangle;
/// `SETVAR LIMMIN`/`SETVAR LIMMAX` write a single corner and read the other from
/// the drawing, so no request is ever ambiguous.
pub fn parse_limits_argument(argument: &str) -> Option<[f64; 4]> {
    let components = parse_plain_decimals(argument)?;
    match components.as_slice() {
        [min_x, min_y, max_x, max_y] => Some([*min_x, *min_y, *max_x, *max_y]),
        _ => None,
    }
}

/// Split one header-vector line into finite plain decimals.
///
/// Separators are whitespace and single commas; every component must be a finite
/// signed plain decimal ([`parse_signed_decimal`]), so an exponent, an **empty**
/// component (`1,,2`, `1,`, `,1`) or a word is refused instead of being
/// repaired — a skipped component would silently shift the corners.
fn parse_plain_decimals(argument: &str) -> Option<Vec<f64>> {
    let text = argument.trim();
    if text.is_empty() {
        return None;
    }
    let mut values = Vec::new();
    for token in text.split_whitespace() {
        for component in token.split(',') {
            // An empty component (`1,,2`, `1,`, `,1`) is a malformed line, not a
            // missing number: nothing is skipped or repaired.
            let value = parse_signed_decimal(component)?;
            if !value.is_finite() {
                return None;
            }
            values.push(value);
        }
    }
    Some(values)
}
