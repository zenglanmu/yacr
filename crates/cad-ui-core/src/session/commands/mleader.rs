//! Typed-argument grammars for `MLEDIT`, `-MLEDIT`, `MLEADERALIGN`,
//! `MLEADEREDIT` and `TOLERANCE`.
//!
//! Every parser is pure and strict: it admits only the documented tokens and
//! never clamps, defaults or repairs. The command layer re-validates the payload
//! (the two selected lines, the selected MULTILEADERs, the picked point, the
//! spacing, the frame text) and owns every semantic refusal
//! (`mledit.*` / `mleaderalign.*` / `mleaderedit.*` / `tolerance.*`).
//!
//! * `MLEDIT MERGE|CLOSED|CROSS|TEE <x> <y>` — the closed clean-up spellings;
//! * `MLEDIT OPEN <x> <y>` — split both crossing lines at the intersection;
//! * `MLEDIT BREAK <x1> <y1> <x2> <y2>` — remove the picked band from two
//!   parallel lines;
//! * `-MLEDIT …` — the same grammar under the command-line spelling;
//! * `MLEADERALIGN LEFT|RIGHT|CENTER [spacing]` — align and optionally re-space
//!   the selected leaders' text;
//! * `MLEADEREDIT ADD <x> <y>` / `MLEADEREDIT REMOVE <index>` — add a leader line
//!   at the typed arrow or remove one by its 0-based index;
//! * `TOLERANCE <x> <y> <text>` — the frame's lower-left corner plus the
//!   `|`-separated compartment text (spaces inside the text are preserved).
//!
//! `MLEADERCOLLECT` takes no argument and is therefore **not** in
//! [`super::ARGUMENT_SELECTORS`]; it is an ordinary selector whose selection is
//! the whole input.

use cad_app::{
    MLeaderAlignCommand, MLeaderAlignMode, MLeaderEditAction, MLeaderEditCommand, MlEditCommand,
    MlEditMode, ToleranceCommand,
};
use cad_domain::Point3;

use super::arguments::{parse_positive_decimal, parse_signed_decimal};

pub const MLEDIT_SELECTOR: &str = "MLEDIT";
pub const DASH_MLEDIT_SELECTOR: &str = "-MLEDIT";
pub const MLEADERALIGN_SELECTOR: &str = "MLEADERALIGN";
pub const MLEADEREDIT_SELECTOR: &str = "MLEADEREDIT";
pub const TOLERANCE_SELECTOR: &str = "TOLERANCE";

/// `MLEDIT <option> <x> <y>` / `MLEDIT BREAK <x1> <y1> <x2> <y2>`.
///
/// The four clean-up spellings need the crossing pick; `BREAK` needs the two
/// band corners. A clean-up option with four coordinate tokens or a `BREAK` with
/// two is outside the grammar and is refused by name.
pub fn parse_mledit_argument(argument: &str) -> Option<MlEditCommand> {
    let tokens = tokens(argument);
    let (word, rest) = tokens.split_first()?;
    let mode = if word.eq_ignore_ascii_case("MERGE") {
        MlEditMode::Merge
    } else if word.eq_ignore_ascii_case("CLOSED") {
        MlEditMode::Closed
    } else if word.eq_ignore_ascii_case("CROSS") {
        MlEditMode::Cross
    } else if word.eq_ignore_ascii_case("TEE") {
        MlEditMode::Tee
    } else if word.eq_ignore_ascii_case("OPEN") {
        MlEditMode::Open
    } else if word.eq_ignore_ascii_case("BREAK") {
        MlEditMode::Break
    } else {
        return None;
    };
    let first = Point3 {
        x: parse_signed_decimal(rest.first()?)?,
        y: parse_signed_decimal(rest.get(1)?)?,
        z: 0.0,
    };
    match mode {
        MlEditMode::Break => {
            if rest.len() != 4 {
                return None;
            }
            Some(MlEditCommand {
                mode,
                first,
                second: Some(Point3 {
                    x: parse_signed_decimal(rest[2])?,
                    y: parse_signed_decimal(rest[3])?,
                    z: 0.0,
                }),
            })
        }
        _ => {
            if rest.len() != 2 {
                return None;
            }
            Some(MlEditCommand {
                mode,
                first,
                second: None,
            })
        }
    }
}

/// `MLEADERALIGN <LEFT|RIGHT|CENTER> [positive spacing]`.
pub fn parse_mleaderalign_argument(argument: &str) -> Option<MLeaderAlignCommand> {
    let tokens = tokens(argument);
    if tokens.is_empty() || tokens.len() > 2 {
        return None;
    }
    let mode = if tokens[0].eq_ignore_ascii_case("LEFT") || tokens[0].eq_ignore_ascii_case("L") {
        MLeaderAlignMode::Left
    } else if tokens[0].eq_ignore_ascii_case("RIGHT") || tokens[0].eq_ignore_ascii_case("R") {
        MLeaderAlignMode::Right
    } else if tokens[0].eq_ignore_ascii_case("CENTER") || tokens[0].eq_ignore_ascii_case("C") {
        MLeaderAlignMode::Center
    } else {
        return None;
    };
    let spacing = match tokens.get(1) {
        Some(token) => Some(parse_positive_decimal(token)?),
        None => None,
    };
    Some(MLeaderAlignCommand { mode, spacing })
}

/// `MLEADEREDIT ADD <x> <y>` / `MLEADEREDIT REMOVE <index>`.
///
/// The index is a plain non-negative `usize` (0-based); the command layer checks
/// it against the leader's stored line list.
pub fn parse_mleaderedit_argument(argument: &str) -> Option<MLeaderEditCommand> {
    let tokens = tokens(argument);
    let (word, rest) = tokens.split_first()?;
    let action = if word.eq_ignore_ascii_case("ADD") {
        if rest.len() != 2 {
            return None;
        }
        MLeaderEditAction::Add {
            arrow: Point3 {
                x: parse_signed_decimal(rest[0])?,
                y: parse_signed_decimal(rest[1])?,
                z: 0.0,
            },
        }
    } else if word.eq_ignore_ascii_case("REMOVE") {
        if rest.len() != 1 {
            return None;
        }
        let token = rest[0];
        if token.is_empty() || !token.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        MLeaderEditAction::Remove {
            line_index: token.parse::<usize>().ok()?,
        }
    } else {
        return None;
    };
    Some(MLeaderEditCommand { action })
}

/// `TOLERANCE <x> <y> <text>`.
///
/// The two coordinates are signed plain decimals on the flat work plane
/// (`z = 0`); everything after them is the frame text with internal/trailing
/// spaces preserved (only the leading separator whitespace is dropped). An empty
/// remainder is kept as an empty string so the command layer can answer with its
/// own `tolerance.text` refusal instead of a grammar error.
pub fn parse_tolerance_argument(argument: &str) -> Option<ToleranceCommand> {
    let text = argument.trim_start();
    let (x, rest) = split_first_token(text)?;
    let (y, rest) = split_first_token(rest)?;
    Some(ToleranceCommand {
        origin: Point3 {
            x: parse_signed_decimal(x)?,
            y: parse_signed_decimal(y)?,
            z: 0.0,
        },
        text: rest.trim_start().to_string(),
    })
}

/// Split off the first whitespace-delimited token (the remainder is untrimmed).
fn split_first_token(text: &str) -> Option<(&str, &str)> {
    let text = text.trim_start();
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    let token = &text[..end];
    if token.is_empty() {
        return None;
    }
    Some((token, &text[end..]))
}

fn tokens(argument: &str) -> Vec<&str> {
    argument.split_whitespace().collect()
}
