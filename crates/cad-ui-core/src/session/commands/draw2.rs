//! Typed-argument grammars for the 2026-11-07 2D draw/modify batch.
//!
//! Every parser is pure and strict: it admits only the documented tokens, never
//! clamps, defaults or repairs. A refused line is answered by the caller with
//! `invalid_<name>_argument_reason` before any IPC. The payload-driven forms are
//! the complete API; these grammars are the command-line convenience.
//!
//! Full contracts: `crate::command_id::draw2` (re-exported from `cad-app`).

use cad_app::{Draw2Payload, DrawOrderMode, MlStyleCommand, PeditOption, TrexMode};
use cad_domain::Point3;

use super::arguments::{parse_positive_decimal, parse_signed_decimal};

pub const DONUT_SELECTOR: &str = "DONUT";
pub const SOLID_SELECTOR: &str = "SOLID";
pub const HELIX_SELECTOR: &str = "HELIX";
pub const MLINE_SELECTOR: &str = "MLINE";
pub const MLSTYLE_SELECTOR: &str = "MLSTYLE";
pub const TREX_SELECTOR: &str = "TREX";
pub const MPEDIT_SELECTOR: &str = "MPEDIT";
pub const DRAWORDER_SELECTOR: &str = "DRAWORDER";

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

/// `MLINE <spacing> <offset> <x1,y1> <x2,y2>`: spacing > 0, a finite offset and
/// the two baseline points.
pub fn parse_mline_argument(argument: &str) -> Option<Draw2Payload> {
    let tokens = tokens(argument);
    if tokens.len() != 4 {
        return None;
    }
    Some(Draw2Payload::Mline {
        spacing: Some(parse_positive_decimal(tokens[0])?),
        offset: parse_signed_decimal(tokens[1])?,
        points: vec![parse_point(tokens[2])?, parse_point(tokens[3])?],
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
