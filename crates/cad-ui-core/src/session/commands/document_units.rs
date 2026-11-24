//! The typed `UNITS` / `-UNITS` grammar.
//!
//! The bare selectors are the read-only reports. The typed forms write one or
//! more document-level settings through the same command payload:
//!
//! ```text
//! UNITS <unit> <decimal-places>
//! -UNITS <unit> <decimal-places>
//! UNITS LENGTH <scientific|decimal|engineering|architectural|fractional>
//! UNITS ANGLE <decimal|dms|gradians|radians|surveyors> <0..=8>
//! UNITS ANGLE DIRECTION <east|north|west|south|other> [<degrees>] [CW|CCW]
//! ```
//!
//! `<unit>` is one of `drawing`, `mm`, `cm`, `m`, `in`, `ft` (the persisted
//! schema tokens, case-insensitive). Declaring a unit also declares it as the
//! display unit, so the stored coordinates are not rescaled — exactly the
//! "declare this drawing in mm" meaning.
//!
//! `LENGTH` writes the AutoCAD length *format* table entry; `ANGLE` writes the
//! angular format/precision; `ANGLE DIRECTION` writes `ANGBASE` (the
//! `east`/`north`/`west`/`south` presets or a finite `other` angle) and
//! `ANGDIR` (`CW`/`CCW`, which is left unchanged when omitted).
//!
//! `$INSUNITS` insertion scaling is not modelled and the command's report names
//! it instead of accepting a value it cannot store.

use cad_app::{DeclaredUnits, UnitsCommand};
use cad_db::{AngleFormat, LengthFormat};
use cad_domain::Unit;

/// `UNITS <unit> <precision>`: the typed drawing-unit form.
pub const UNITS_SELECTOR: &str = "UNITS";

/// `-UNITS <unit> <precision>`: the command-line spelling of the same write.
pub const DASH_UNITS_SELECTOR: &str = "-UNITS";

/// Parse one typed `UNITS`/`-UNITS` line, or `None` outside the grammar.
pub fn parse_units_argument(argument: &str) -> Option<UnitsCommand> {
    let tokens: Vec<&str> = argument.split_whitespace().collect();
    let first = tokens.first()?.to_ascii_lowercase();
    match first.as_str() {
        "length" => {
            if tokens.len() != 2 {
                return None;
            }
            Some(UnitsCommand {
                declared: None,
                length_format: Some(LengthFormat::from_token(tokens[1])?),
                angle_format: None,
                angle_precision: None,
                angle_base_degrees: None,
                angle_clockwise: None,
            })
        }
        "angle" => parse_angle(&tokens[1..]),
        _ => {
            let unit = parse_unit_token(tokens[0])?;
            let decimal_places: u8 = tokens.get(1)?.parse().ok()?;
            if decimal_places > 8 || tokens.len() > 2 {
                return None;
            }
            Some(UnitsCommand {
                declared: Some(DeclaredUnits {
                    unit,
                    decimal_places,
                }),
                length_format: None,
                angle_format: None,
                angle_precision: None,
                angle_base_degrees: None,
                angle_clockwise: None,
            })
        }
    }
}

/// Parse the `ANGLE ...` remainder.
fn parse_angle(tokens: &[&str]) -> Option<UnitsCommand> {
    if tokens.first()?.eq_ignore_ascii_case("direction") {
        return parse_angle_direction(&tokens[1..]);
    }
    if tokens.len() != 2 {
        return None;
    }
    let format = AngleFormat::from_token(tokens[0])?;
    let precision: u8 = tokens[1].parse().ok()?;
    if precision > cad_db::UNITS_MAX_PRECISION {
        return None;
    }
    Some(UnitsCommand {
        declared: None,
        length_format: None,
        angle_format: Some(format),
        angle_precision: Some(precision),
        angle_base_degrees: None,
        angle_clockwise: None,
    })
}

/// Parse `DIRECTION <preset|other> [<degrees>] [CW|CCW]`.
fn parse_angle_direction(tokens: &[&str]) -> Option<UnitsCommand> {
    let preset = tokens.first()?.to_ascii_lowercase();
    let preset_degrees = match preset.as_str() {
        "east" => Some(0.0),
        "north" => Some(90.0),
        "west" => Some(180.0),
        "south" => Some(270.0),
        "other" => None,
        _ => return None,
    };
    let mut base = preset_degrees;
    let mut clockwise = None;
    let mut index = 1;
    while index < tokens.len() {
        let token = tokens[index];
        if token.eq_ignore_ascii_case("cw") || token.eq_ignore_ascii_case("clockwise") {
            if clockwise.is_some() {
                return None;
            }
            clockwise = Some(true);
        } else if token.eq_ignore_ascii_case("ccw")
            || token.eq_ignore_ascii_case("counterclockwise")
            || token.eq_ignore_ascii_case("counter-clockwise")
        {
            if clockwise.is_some() {
                return None;
            }
            clockwise = Some(false);
        } else if base.is_none() {
            let degrees: f64 = token.parse().ok()?;
            if !degrees.is_finite() {
                return None;
            }
            base = Some(degrees);
        } else {
            return None;
        }
        index += 1;
    }
    // `other` needs the one angle it cannot derive; a preset already names it.
    let base = base?;
    Some(UnitsCommand {
        declared: None,
        length_format: None,
        angle_format: None,
        angle_precision: None,
        angle_base_degrees: Some(base),
        angle_clockwise: clockwise,
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
