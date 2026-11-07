//! Typed-argument grammars of the 2026-11-07 extras batch.
//!
//! These are **argument forms of canonical selectors**, never new IPC:
//! [`PAN_SELECTOR`] (`-PAN`), [`TRACE_SELECTOR`], [`MOCORO_SELECTOR`],
//! [`VIEWRES_SELECTOR`], [`VSCURRENT_SELECTOR`] and [`LINETYPE_SELECTOR`] are
//! admitted by [`super::ARGUMENT_SELECTORS`], parsed here, and dispatched by the
//! session as the matching [`cad_app::CommandId`] with a payload.
//!
//! `SKETCH` and `MSTRETCH` have **no** command-line grammar in this build:
//! their complete input (a sampled point sequence / a rectangle list) is carried
//! by a host/IPC payload, not by one selector argument.
//!
//! Grammar:
//!
//! * `-PAN <dx> <dy>` or `-PAN <x1> <y1> <x2> <y2>` → one `Points([delta])`
//!   payload for the existing `PAN` camera path;
//! * `TRACE <x1> <y1> <x2> <y2> <width>` (five finite numbers, width > 0);
//! * `MOCORO MOVE|COPY <dx> <dy>` / `MOCORO ROTATE <bx> <by> <degrees>` /
//!   `MOCORO SCALE <bx> <by> <factor>`;
//! * `VIEWRES <integer 1..=1000>`;
//! * `VSCURRENT <style>` (the command layer names the unsupported styles);
//! * `LINETYPE LIST` or `LINETYPE SET <name>` (the name takes the whole
//!   remainder, spaces allowed).

use cad_app::{CommandPayload, LinetypeCommand, MocoroCommand};
use cad_domain::Point3;

use super::arguments::parse_signed_decimal;

/// `PAN` / `-PAN`: the typed world-delta form of the existing camera path.
pub const PAN_SELECTOR: &str = "PAN";
/// `TRACE <x1> <y1> <x2> <y2> <width>`.
pub const TRACE_SELECTOR: &str = "TRACE";
/// `MOCORO MOVE|COPY|ROTATE|SCALE`.
pub const MOCORO_SELECTOR: &str = "MOCORO";
/// `VIEWRES <1..=1000>`.
pub const VIEWRES_SELECTOR: &str = "VIEWRES";
/// `VSCURRENT <style>`.
pub const VSCURRENT_SELECTOR: &str = "VSCURRENT";
/// `LINETYPE LIST|SET <name>`.
pub const LINETYPE_SELECTOR: &str = "LINETYPE";

/// `-PAN <dx> <dy>` or `-PAN <x1> <y1> <x2> <y2>`.
pub fn parse_pan_argument(argument: &str) -> Option<CommandPayload> {
    let tokens = tokens(argument);
    let number = |text: &str| parse_signed_decimal(text);
    let delta = match tokens.as_slice() {
        [dx, dy] => Point3 {
            x: number(dx)?,
            y: number(dy)?,
            z: 0.0,
        },
        [x1, y1, x2, y2] => Point3 {
            x: number(x2)? - number(x1)?,
            y: number(y2)? - number(y1)?,
            z: 0.0,
        },
        _ => return None,
    };
    Some(CommandPayload::Points(vec![delta]))
}

/// `TRACE <x1> <y1> <x2> <y2> <width>`.
pub fn parse_trace_argument(argument: &str) -> Option<CommandPayload> {
    let tokens = tokens(argument);
    let [x1, y1, x2, y2, width] = tokens.as_slice() else {
        return None;
    };
    let start = Point3 {
        x: parse_signed_decimal(x1)?,
        y: parse_signed_decimal(y1)?,
        z: 0.0,
    };
    let end = Point3 {
        x: parse_signed_decimal(x2)?,
        y: parse_signed_decimal(y2)?,
        z: 0.0,
    };
    let width = parse_signed_decimal(width)?;
    Some(CommandPayload::Trace { start, end, width })
}

/// `MOCORO MOVE|COPY <dx> <dy>` / `ROTATE <bx> <by> <degrees>` /
/// `SCALE <bx> <by> <factor>`.
pub fn parse_mocoro_argument(argument: &str) -> Result<CommandPayload, String> {
    let tokens = tokens(argument);
    let Some((word, rest)) = tokens.split_first() else {
        return Err(invalid(
            "MOCORO",
            "MOCORO MOVE|COPY <dx> <dy>, ROTATE <bx> <by> <degrees> or SCALE <bx> <by> <factor>",
        ));
    };
    let operation = match word.to_ascii_uppercase().as_str() {
        "MOVE" | "COPY" if rest.len() == 2 => {
            let delta = Point3 {
                x: decimal(rest[0], "MOCORO")?,
                y: decimal(rest[1], "MOCORO")?,
                z: 0.0,
            };
            if word.eq_ignore_ascii_case("MOVE") {
                MocoroCommand::Move { delta }
            } else {
                MocoroCommand::Copy { delta }
            }
        }
        "ROTATE" if rest.len() == 3 => MocoroCommand::Rotate {
            base: point(rest[0], rest[1], "MOCORO")?,
            degrees: decimal(rest[2], "MOCORO")?,
        },
        "SCALE" if rest.len() == 3 => {
            let factor = decimal(rest[2], "MOCORO")?;
            if factor <= 0.0 {
                return Err("mocoro.invalid_factor: MOCORO SCALE needs a finite factor > 0".into());
            }
            MocoroCommand::Scale {
                base: point(rest[0], rest[1], "MOCORO")?,
                factor,
            }
        }
        _ => {
            return Err(invalid(
                "MOCORO",
                "MOCORO MOVE|COPY <dx> <dy>, ROTATE <bx> <by> <degrees> or SCALE <bx> <by> \
                 <factor>",
            ))
        }
    };
    Ok(CommandPayload::Mocoro(operation))
}

/// `VIEWRES <integer percentage 1..=1000>`.
pub fn parse_viewres_argument(argument: &str) -> Result<CommandPayload, String> {
    let token = argument.trim();
    if token.is_empty() || token.split_whitespace().count() != 1 {
        return Err(invalid(
            "VIEWRES",
            "VIEWRES <integer percentage in 1..=1000>",
        ));
    }
    let percent = token.parse::<u32>().map_err(|_| {
        invalid(
            "VIEWRES",
            "VIEWRES needs one integer percentage in 1..=1000",
        )
    })?;
    if !(1..=1000).contains(&percent) {
        return Err(format!(
            "viewres.out_of_range: VIEWRES needs an integer percentage in 1..=1000, got {percent}; \
             nothing was changed"
        ));
    }
    Ok(CommandPayload::ViewRes(percent))
}

/// `VSCURRENT <style>`: the parser admits one word; the command refuses every
/// style except `WIREFRAME` by name.
pub fn parse_vscurrent_argument(argument: &str) -> Result<CommandPayload, String> {
    let style = argument.trim();
    if style.is_empty() || style.split_whitespace().count() != 1 {
        return Err(invalid("VSCURRENT", "VSCURRENT <style> (only WIREFRAME)"));
    }
    Ok(CommandPayload::VisualStyle(style.to_string()))
}

/// `LINETYPE LIST` or `LINETYPE SET <name>` (the name may contain spaces).
pub fn parse_linetype_argument(argument: &str) -> Result<CommandPayload, String> {
    let expected = "LINETYPE LIST or LINETYPE SET <name>";
    let trimmed = argument.trim();
    let (word, rest) = match trimmed.find(char::is_whitespace) {
        Some(index) => (
            &trimmed[..index],
            trimmed[index..].trim_start_matches(char::is_whitespace),
        ),
        None => (trimmed, ""),
    };
    if word.eq_ignore_ascii_case("LIST") && rest.is_empty() {
        return Ok(CommandPayload::Linetype(LinetypeCommand::List));
    }
    if word.eq_ignore_ascii_case("SET") && !rest.is_empty() {
        return Ok(CommandPayload::Linetype(LinetypeCommand::Set(
            rest.to_string(),
        )));
    }
    Err(invalid("LINETYPE", expected))
}

/// Split one argument into non-empty whitespace-delimited tokens.
fn tokens(argument: &str) -> Vec<&str> {
    argument.split_whitespace().collect()
}

/// Parse one finite decimal or produce a stable selector refusal.
fn decimal(text: &str, selector: &str) -> Result<f64, String> {
    parse_signed_decimal(text).ok_or_else(|| {
        invalid(
            selector,
            "every numeric component must be one finite decimal number",
        )
    })
}

/// Parse one finite `x y` point or produce a stable selector refusal.
fn point(x: &str, y: &str, selector: &str) -> Result<Point3, String> {
    Ok(Point3 {
        x: decimal(x, selector)?,
        y: decimal(y, selector)?,
        z: 0.0,
    })
}

/// The stable refusal sentence for an argument outside a selector's grammar.
fn invalid(selector: &str, expected: &str) -> String {
    format!("unsupported CAD command: {selector} argument (expected {expected})")
}
