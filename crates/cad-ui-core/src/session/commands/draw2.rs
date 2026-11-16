//! Typed-argument grammars for the 2026-11-07 2D draw/modify batch.
//!
//! Every parser is pure and strict: it admits only the documented tokens, never
//! clamps, defaults or repairs. A refused line is answered by the caller with
//! `invalid_<name>_argument_reason` before any IPC. The payload-driven forms are
//! the complete API; these grammars are the command-line convenience.
//!
//! Full contracts: `crate::command_id::draw2` (re-exported from `cad-app`).

use cad_app::{CommandPayload, Draw2Payload, DrawOrderMode, MlStyleCommand, PeditOption, TrexMode};
use cad_domain::{Point3, SelectionRef};

use super::arguments::{parse_positive_decimal, parse_signed_decimal};

pub const DONUT_SELECTOR: &str = "DONUT";
pub const SOLID_SELECTOR: &str = "SOLID";
pub const HELIX_SELECTOR: &str = "HELIX";
pub const MLINE_SELECTOR: &str = "MLINE";
pub const MLSTYLE_SELECTOR: &str = "MLSTYLE";
pub const TREX_SELECTOR: &str = "TREX";
pub const MPEDIT_SELECTOR: &str = "MPEDIT";
pub const DRAWORDER_SELECTOR: &str = "DRAWORDER";
pub const ARRAYPATH_SELECTOR: &str = "ARRAYPATH";

/// `ARRAYPATH <count> [ALIGN]`: the typed 2D path-array form.
///
/// The grammar is strict: one positive whole number (ASCII digits, no sign, no
/// fraction, no exponent) and the optional case-insensitive word `ALIGN`. The
/// position count is re-checked by the command (its total copy count is capped by
/// `MAX_ARRAY_ENTITIES`); this parser only admits the shape, never clamps.
pub fn parse_arraypath_argument(argument: &str) -> Option<(usize, bool)> {
    let tokens = tokens(argument);
    if tokens.is_empty() || tokens.len() > 2 {
        return None;
    }
    let count_text = tokens[0];
    if count_text.is_empty() || !count_text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let count: usize = count_text.parse().ok()?;
    if count == 0 {
        return None;
    }
    let align = match tokens.get(1) {
        None => false,
        Some(word) if word.eq_ignore_ascii_case("ALIGN") => true,
        Some(_) => return None,
    };
    Some((count, align))
}

/// The stable refusal for an `ARRAYPATH` argument outside the grammar.
pub fn invalid_arraypath_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (ARRAYPATH needs a position count and an optional ALIGN, \
         e.g. `ARRAYPATH 4 ALIGN`)",
        text.trim()
    )
}

// ---------------------------------------------------------------------------
// `ARRAY` / `-ARRAY` / `ARRAYCLASSIC` (the typed RECT / POLAR forms)
// ---------------------------------------------------------------------------

/// `ARRAY RECT …` / `ARRAY POLAR …` (AutoCAD's array command, typed).
pub const ARRAY_SELECTOR: &str = "ARRAY";
/// The command-line spelling of the same typed forms.
pub const DASH_ARRAY_SELECTOR: &str = "-ARRAY";
/// `ARRAYCLASSIC …`: the same planner (this build has no legacy dialog).
pub const ARRAYCLASSIC_SELECTOR: &str = "ARRAYCLASSIC";

/// The rectangular grid of `ARRAY RECT <rows> <columns> <row-spacing>
/// <column-spacing>`.
///
/// `rows`/`columns` count the **source object** (the planner skips its own cell,
/// so `rows × columns − 1` copies are inserted). The spacings are the distances
/// between base points; a **negative** spacing arrays in the negative direction,
/// exactly like the session channel the tool reads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArrayRectArgument {
    pub rows: u32,
    pub columns: u32,
    pub row_spacing: f64,
    pub column_spacing: f64,
}

/// The polar array of `ARRAY POLAR <items> <fill-angle> <center-x> <center-y>`.
///
/// `items` counts the source object (the planner inserts `items − 1` copies) and
/// `fill_angle` is in **degrees** with AutoCAD's `0 < |angle| <= 360` rule. The
/// centre is the payload's `center` point (a 2D point on the work plane).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArrayPolarArgument {
    pub items: u32,
    pub fill_angle: f64,
    pub center: [f64; 2],
}

/// One parsed `ARRAY` argument.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ArrayArgument {
    Rect(ArrayRectArgument),
    Polar(ArrayPolarArgument),
}

/// Why an `ARRAY` argument was refused **before** any session write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrayArgumentError {
    /// The line is outside the declared grammar.
    Malformed,
    /// A non-integer or `< 1` count: `array.count` (naming the field).
    Count(&'static str),
    /// A zero spacing (the cells would coincide): `array.spacing`.
    Spacing(&'static str),
    /// A fill angle outside `0 < |angle| <= 360`: `array.angle`.
    Angle,
    /// The request would exceed the array cap for one source: `array.cap`.
    Cap,
}

/// Parse the argument of an `ARRAY` / `-ARRAY` / `ARRAYCLASSIC` selector.
///
/// Two forms, mirroring the two payloads the dispatcher already routes:
///
/// * `RECT <rows> <columns> <row-spacing> <column-spacing>` — exactly four
///   numbers. The counts are whole numbers `>= 1` (ASCII digits only, so a
///   fraction, a sign or an exponent is a `COUNT` refusal, never a truncation);
///   the spacings are signed plain decimals and must not be `0` (a zero spacing
///   collapses the grid onto the source — a degenerate array, refused instead of
///   stored as coincident copies);
/// * `POLAR <items> <fill-angle> <center-x> <center-y>` — four numbers. `items`
///   is a whole number `>= 1`, the fill angle is a signed decimal with
///   `0 < |angle| <= 360`, and the centre is a 2D point on the work plane.
///
/// The keyword is the **full** word: no abbreviation is admitted (this router
/// never guesses at a partial word). The per-source copy count is checked against
/// [`cad_app::command_id::array::MAX_ARRAY_ENTITIES`] here so an over-bound request
/// is refused before the session channel is written; the planner re-checks the
/// total (multi-source) count against the same constant. Nothing is clamped,
/// defaulted or repaired.
pub fn parse_array_argument(argument: &str) -> Result<ArrayArgument, ArrayArgumentError> {
    let tokens = tokens(argument);
    let Some(word) = tokens.first() else {
        return Err(ArrayArgumentError::Malformed);
    };
    let numbers = &tokens[1..];
    if word.eq_ignore_ascii_case("RECT") {
        if numbers.len() != 4 {
            return Err(ArrayArgumentError::Malformed);
        }
        let rows = parse_array_count(numbers[0], "rows")?;
        let columns = parse_array_count(numbers[1], "columns")?;
        let row_spacing = parse_array_spacing(numbers[2], "row-spacing")?;
        let column_spacing = parse_array_spacing(numbers[3], "column-spacing")?;
        let produced = rows as f64 * columns as f64 - 1.0;
        if produced > cad_app::command_id::array::MAX_ARRAY_ENTITIES as f64 {
            return Err(ArrayArgumentError::Cap);
        }
        return Ok(ArrayArgument::Rect(ArrayRectArgument {
            rows,
            columns,
            row_spacing,
            column_spacing,
        }));
    }
    if word.eq_ignore_ascii_case("POLAR") {
        if numbers.len() != 4 {
            return Err(ArrayArgumentError::Malformed);
        }
        let items = parse_array_count(numbers[0], "items")?;
        let fill_angle = parse_signed_decimal(numbers[1]).ok_or(ArrayArgumentError::Angle)?;
        if fill_angle == 0.0 || fill_angle.abs() > 360.0 {
            return Err(ArrayArgumentError::Angle);
        }
        let center = [
            parse_signed_decimal(numbers[2]).ok_or(ArrayArgumentError::Malformed)?,
            parse_signed_decimal(numbers[3]).ok_or(ArrayArgumentError::Malformed)?,
        ];
        if items as f64 - 1.0 > cad_app::command_id::array::MAX_ARRAY_ENTITIES as f64 {
            return Err(ArrayArgumentError::Cap);
        }
        return Ok(ArrayArgument::Polar(ArrayPolarArgument {
            items,
            fill_angle,
            center,
        }));
    }
    Err(ArrayArgumentError::Malformed)
}

/// One whole array count (rows/columns/items): ASCII digits only, `>= 1`.
///
/// A sign, a fraction and an exponent are all [`ArrayArgumentError::Count`] —
/// never truncated — because a negative or fractional count has no array meaning.
fn parse_array_count(text: &str, field: &'static str) -> Result<u32, ArrayArgumentError> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ArrayArgumentError::Count(field));
    }
    text.parse::<u32>()
        .ok()
        .filter(|count| *count >= 1)
        .ok_or(ArrayArgumentError::Count(field))
}

/// One array spacing: a finite signed decimal that is not `0`.
fn parse_array_spacing(text: &str, field: &'static str) -> Result<f64, ArrayArgumentError> {
    let value = parse_signed_decimal(text).ok_or(ArrayArgumentError::Spacing(field))?;
    if value == 0.0 {
        return Err(ArrayArgumentError::Spacing(field));
    }
    Ok(value)
}

/// Build the exact `ARRAY` payload for one parsed argument and selection.
///
/// The rectangular form carries the refs only (the counts/spacings are the
/// session channel), the polar form carries the centre too — exactly the payload
/// fields the dispatcher routes. This is the one conversion the session uses, so
/// the dispatched payload is a pure function of the parsed line and the snapshot.
pub fn array_payload(argument: ArrayArgument, refs: Vec<SelectionRef>) -> CommandPayload {
    match argument {
        ArrayArgument::Rect(_) => CommandPayload::ArrayRect { refs },
        ArrayArgument::Polar(polar) => CommandPayload::ArrayPolar {
            refs,
            center: Point3 {
                x: polar.center[0],
                y: polar.center[1],
                z: 0.0,
            },
        },
    }
}

/// The stable refusal text for one `ARRAY` argument refusal.
///
/// Every branch names the command, the accepted grammar and the offending line,
/// and says that nothing was written, so a refused array is never read as an empty
/// success.
pub fn array_argument_refusal(selector: &str, error: ArrayArgumentError, text: &str) -> String {
    let line = text.trim();
    let cap = cad_app::command_id::array::MAX_ARRAY_ENTITIES;
    match error {
        ArrayArgumentError::Malformed => format!(
            "unsupported CAD command: {line} ({selector} takes RECT <rows> <columns> \
             <row-spacing> <column-spacing> or POLAR <items> <fill-angle> <center-x> \
             <center-y>; a bare {selector} reports the available planners)"
        ),
        ArrayArgumentError::Count(field) => format!(
            "array.count: {selector} {field} must be an integer >= 1 (a zero or negative count \
             has no array), got `{line}`; nothing was written"
        ),
        ArrayArgumentError::Spacing(field) => format!(
            "array.spacing: {selector} {field} must be a non-zero finite distance between base \
             points (a zero spacing would put every cell on the source), got `{line}`; nothing \
             was written"
        ),
        ArrayArgumentError::Angle => format!(
            "array.angle: {selector} fill angle must be finite with 0 < |angle| <= 360 (AutoCAD's \
             single turn), got `{line}`; nothing was written"
        ),
        ArrayArgumentError::Cap => format!(
            "array.cap: {selector} would produce more than {cap} copies from one source; the \
             array cap is {cap}, got `{line}`; nothing was written"
        ),
    }
}

/// `DONUT <cx> <cy> <inner> <outer>`: four finite numbers; radii are checked for
/// sign/order by the command (`donut.*`). The declared AutoCAD form
/// `DONUT <inner> <outer>` has no centre and is not available without a pick
/// prompt, so the centre is typed here.
pub fn parse_donut_argument(argument: &str) -> Option<Draw2Payload> {
    let tokens = tokens(argument);
    if tokens.len() != 4 {
        return None;
    }
    let center = Point3 {
        x: parse_signed_decimal(tokens[0])?,
        y: parse_signed_decimal(tokens[1])?,
        z: 0.0,
    };
    Some(Draw2Payload::Donut {
        center,
        inner: parse_signed_decimal(tokens[2])?,
        outer: parse_signed_decimal(tokens[3])?,
    })
}

/// `SOLID <x,y> <x,y> <x,y> [<x,y>]`: three or four points in ring order.
pub fn parse_solid_argument(argument: &str) -> Option<Draw2Payload> {
    let tokens = tokens(argument);
    if !(3..=4).contains(&tokens.len()) {
        return None;
    }
    Some(Draw2Payload::Solid {
        points: tokens
            .iter()
            .map(|token| parse_point(token))
            .collect::<Option<Vec<_>>>()?,
    })
}

/// `HELIX <base x> <base y> <radius> <turns> <height>`: five finite numbers.
pub fn parse_helix_argument(argument: &str) -> Option<Draw2Payload> {
    let tokens = tokens(argument);
    if tokens.len() != 5 {
        return None;
    }
    Some(Draw2Payload::Helix {
        base: Point3 {
            x: parse_signed_decimal(tokens[0])?,
            y: parse_signed_decimal(tokens[1])?,
            z: 0.0,
        },
        radius: parse_signed_decimal(tokens[2])?,
        turns: parse_signed_decimal(tokens[3])?,
        height: parse_signed_decimal(tokens[4])?,
    })
}

/// `MLINE <spacing> <offset> <x1,y1> <x2,y2> [<x3,y3> …]`: spacing > 0, a finite
/// offset and at least two baseline vertices.
///
/// The vertex list is unbounded here; the command layer owns the declared
/// 2..=1024 bound (`mline.point_count`) because it is the entity-count rule, not
/// a grammar rule.
pub fn parse_mline_argument(argument: &str) -> Option<Draw2Payload> {
    let tokens = tokens(argument);
    if tokens.len() < 4 {
        return None;
    }
    Some(Draw2Payload::Mline {
        spacing: Some(parse_positive_decimal(tokens[0])?),
        offset: parse_signed_decimal(tokens[1])?,
        points: tokens[2..]
            .iter()
            .map(|token| parse_point(token))
            .collect::<Option<Vec<_>>>()?,
    })
}

/// `MLSTYLE LIST` or `MLSTYLE SET <name> <spacing>`.
pub fn parse_mlstyle_argument(argument: &str) -> Option<Draw2Payload> {
    let tokens = tokens(argument);
    let (word, rest) = tokens.split_first()?;
    if word.eq_ignore_ascii_case("LIST") && rest.is_empty() {
        return Some(Draw2Payload::MlStyle(MlStyleCommand::List));
    }
    if word.eq_ignore_ascii_case("SET") && rest.len() == 2 {
        return Some(Draw2Payload::MlStyle(MlStyleCommand::Set {
            name: rest[0].to_string(),
            spacing: parse_positive_decimal(rest[1])?,
        }));
    }
    None
}

/// `TREX TRIM|EXTEND`.
pub fn parse_trex_argument(argument: &str) -> Option<Draw2Payload> {
    let word = argument.trim();
    if word.eq_ignore_ascii_case("TRIM") {
        return Some(Draw2Payload::Trex(TrexMode::Trim));
    }
    if word.eq_ignore_ascii_case("EXTEND") {
        return Some(Draw2Payload::Trex(TrexMode::Extend));
    }
    None
}

/// `MPEDIT CLOSE|OPEN|REVERSE` (the same option subset as `PEDIT`).
pub fn parse_mpedit_argument(argument: &str) -> Option<Draw2Payload> {
    let key = argument.trim();
    let option = if key.eq_ignore_ascii_case("CLOSE") || key.eq_ignore_ascii_case("C") {
        PeditOption::Close
    } else if key.eq_ignore_ascii_case("OPEN") || key.eq_ignore_ascii_case("O") {
        PeditOption::Open
    } else if key.eq_ignore_ascii_case("REVERSE") || key.eq_ignore_ascii_case("R") {
        PeditOption::Reverse
    } else {
        return None;
    };
    Some(Draw2Payload::Mpedit(option))
}

/// `DRAWORDER FRONT|BACK|ABOVE|BELOW`.
pub fn parse_draworder_argument(argument: &str) -> Option<Draw2Payload> {
    let word = argument.trim();
    let mode = if word.eq_ignore_ascii_case("FRONT") {
        DrawOrderMode::Front
    } else if word.eq_ignore_ascii_case("BACK") {
        DrawOrderMode::Back
    } else if word.eq_ignore_ascii_case("ABOVE") {
        DrawOrderMode::Above
    } else if word.eq_ignore_ascii_case("BELOW") {
        DrawOrderMode::Below
    } else {
        return None;
    };
    Some(Draw2Payload::DrawOrder(mode))
}

fn tokens(argument: &str) -> Vec<&str> {
    argument.split_whitespace().collect()
}

/// `x,y` with two finite signed plain decimals. `z` is the flat work plane's 0.
fn parse_point(token: &str) -> Option<Point3> {
    let (x, y) = token.split_once(',')?;
    if y.contains(',') {
        return None;
    }
    Some(Point3 {
        x: parse_signed_decimal(x)?,
        y: parse_signed_decimal(y)?,
        z: 0.0,
    })
}
