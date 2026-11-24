//! The typed `UNITS` / `-UNITS` drawing-unit grammar.
//!
//! The bare selectors are the read-only reports; the typed form writes the one
//! representable document setting:
//!
//! ```text
//! UNITS <unit> <decimal-places>
//! -UNITS <unit> <decimal-places>
//! ```
//!
//! `<unit>` is one of `drawing`, `mm`, `cm`, `m`, `in`, `ft` (the persisted
//! schema tokens, case-insensitive). `<decimal-places>` is an integer `0..=8`.
//! The unit becomes both the drawing (source) unit and the display unit, so the
//! stored coordinates are not rescaled — exactly the "declare this drawing in
//! mm" meaning. The AutoCAD length *format* and every angular setting are not
//! modeled; `cad-app` names that gap in its report instead of accepting a value
//! it cannot store.

use cad_app::UnitsCommand;
use cad_domain::Unit;

/// `UNITS <unit> <precision>`: the typed drawing-unit form.
pub const UNITS_SELECTOR: &str = "UNITS";

/// `-UNITS <unit> <precision>`: the command-line spelling of the same write.
pub const DASH_UNITS_SELECTOR: &str = "-UNITS";

/// Parse one typed `UNITS`/`-UNITS` line, or `None` outside the grammar.
pub fn parse_units_argument(argument: &str) -> Option<UnitsCommand> {
    let mut tokens = argument.split_whitespace();
    let unit = parse_unit_token(tokens.next()?)?;
    let decimal_places: u8 = tokens.next()?.parse().ok()?;
    if decimal_places > 8 || tokens.next().is_some() {
        return None;
    }
    Some(UnitsCommand {
        unit,
        decimal_places,
    })
}

/// One closed unit token; anything else is not a default.
fn parse_unit_token(token: &str) -> Option<Unit> {
    match token.to_ascii_lowercase().as_str() {
        "drawing" | "unitless" => Some(Unit::DrawingUnits),
        "mm" | "millimeter" | "millimetre" => Some(Unit::Millimeter),
        "cm" | "centimeter" | "centimetre" => Some(Unit::Centimeter),
        "m" | "meter" | "metre" => Some(Unit::Meter),
        "in" | "inch" => Some(Unit::Inch),
        "ft" | "foot" | "feet" => Some(Unit::Foot),
        _ => None,
    }
}
