//! Typed-argument grammars for `MLEDIT`, `-MLEDIT`, `MLEADER`, `MLEADERALIGN`,
//! `MLEADEREDIT` and `TOLERANCE`.
//!
//! Every parser is pure and strict: it admits only the documented tokens and
//! never clamps, defaults or repairs. The command layer re-validates the payload
//! (the two selected lines, the selected MULTILEADERs, the picked point, the
//! spacing, the frame text, the creation options) and owns every semantic refusal
//! (`mledit.*` / `mleader.*` / `mleaderalign.*` / `mleaderedit.*` /
//! `tolerance.*`).
//!
//! * `MLEDIT MERGE|CLOSED|CROSS|TEE <x> <y>` — the closed clean-up spellings;
//! * `MLEDIT OPEN <x> <y>` — split both crossing lines at the intersection;
//! * `MLEDIT BREAK <x1> <y1> <x2> <y2>` — remove the picked band from two
//!   parallel lines;
//! * `-MLEDIT …` — the same grammar under the command-line spelling;
//! * `MLEADER [DOGLEG <length>] [DOGLEGANGLE <degrees>] [ATTACH <1-3> <0-10>]
//!   <arrow_x> <arrow_y> <landing_x> <landing_y> <text…>` — create one leader,
//!   optionally overriding the style's dogleg length/direction and text
//!   attachment pair;
//! * `MLEADERALIGN LEFT|RIGHT|CENTER [spacing]` — align and optionally re-space
//!   the selected leaders' text;
//! * `MLEADEREDIT ADD <x> <y>` / `REMOVE <index>`, `PATHTYPE <index>
//!   LINE|SPLINE|INVISIBLE`, `ARROWSIZE <index> <size>`, `BYSTYLE <index>
//!   PATH|ARROW` — add a leader line at the typed arrow, remove one by its 0-based
//!   index, or set/clear one line's own path type and arrowhead size;
//! * `TOLERANCE <x> <y> <text>` — the frame's lower-left corner plus the
//!   `|`-separated compartment text (spaces inside the text are preserved).
//!
//! `MLEADERCOLLECT` takes no argument and is therefore **not** in
//! [`super::ARGUMENT_SELECTORS`]; it is an ordinary selector whose selection is
//! the whole input.

use cad_app::{
    MLeaderAlignCommand, MLeaderAlignMode, MLeaderAttachment, MLeaderEditAction,
    MLeaderEditCommand, MLeaderLineOverrideField, MlEditCommand, MlEditMode, ToleranceCommand,
};
use cad_domain::{MLeaderPathType, Point3};

use super::arguments::{parse_positive_decimal, parse_signed_decimal};

pub const MLEDIT_SELECTOR: &str = "MLEDIT";
pub const DASH_MLEDIT_SELECTOR: &str = "-MLEDIT";
pub const MLEADER_SELECTOR: &str = "MLEADER";
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

/// `MLEADER [DOGLEG <length>] [DOGLEGANGLE <degrees>] [ATTACH <1-3> <0-10>]
/// <arrow_x> <arrow_y> <landing_x> <landing_y> <text…>`.
///
/// The options are optional, order-independent and may each appear once; they
/// must come **before** the four coordinates, and everything after them is the
/// annotation text (internal/trailing spaces preserved, only the leading separator
/// dropped — the same policy as `TOLERANCE`). A duplicated option, a missing
/// value or a non-numeric coordinate is outside the grammar.
///
/// `DOGLEGANGLE` takes **degrees** on the line and converts once to the radian
/// payload field; `ATTACH` takes the raw DXF attachment codes (horizontal, then
/// vertical) and leaves the range check to the command layer, which refuses by
/// name with the accepted codes.
///
/// `TEXTANGLE` (the AutoCAD angle constraint, DXF entity `174` / style `175`) is
/// admitted as a token but answered with [`MLeaderUnmodelledOption::TextAngle`]:
/// this build's MULTILEADER domain has no field for it, so the command layer
/// refuses it by name instead of dropping it.
pub fn parse_mleader_argument(argument: &str) -> Result<MLeaderCreateInput, MLeaderParseError> {
    let mut rest = argument.trim_start();
    let mut dogleg_length: Option<f64> = None;
    let mut dogleg_angle: Option<f64> = None;
    let mut attachment: Option<MLeaderAttachment> = None;
    loop {
        let Some((word, after_word)) = split_first_token(rest) else {
            return Err(MLeaderParseError::Invalid);
        };
        if word.eq_ignore_ascii_case("TEXTANGLE") {
            return Err(MLeaderParseError::Unmodelled(
                MLeaderUnmodelledOption::TextAngle,
            ));
        }
        let is_option = word.eq_ignore_ascii_case("DOGLEG")
            || word.eq_ignore_ascii_case("DOGLEGANGLE")
            || word.eq_ignore_ascii_case("ATTACH");
        if !is_option {
            // The first non-option token is the arrow X: the options are a prefix.
            break;
        }
        let (value, after_value) =
            split_first_token(after_word).ok_or(MLeaderParseError::Invalid)?;
        if word.eq_ignore_ascii_case("DOGLEG") {
            if dogleg_length.is_some() {
                return Err(MLeaderParseError::Invalid);
            }
            dogleg_length = Some(parse_positive_decimal(value).ok_or(MLeaderParseError::Invalid)?);
            rest = after_value;
        } else if word.eq_ignore_ascii_case("DOGLEGANGLE") {
            if dogleg_angle.is_some() {
                return Err(MLeaderParseError::Invalid);
            }
            let degrees = parse_signed_decimal(value).ok_or(MLeaderParseError::Invalid)?;
            dogleg_angle = Some(degrees.to_radians());
            rest = after_value;
        } else {
            if attachment.is_some() {
                return Err(MLeaderParseError::Invalid);
            }
            let point = parse_attachment_code(value).ok_or(MLeaderParseError::Invalid)?;
            let (left_token, after_left) =
                split_first_token(after_value).ok_or(MLeaderParseError::Invalid)?;
            let left = parse_attachment_code(left_token).ok_or(MLeaderParseError::Invalid)?;
            attachment = Some(MLeaderAttachment { point, left });
            rest = after_left;
        }
    }
    let mut coordinates = [0.0_f64; 4];
    for slot in &mut coordinates {
        let (token, after) = split_first_token(rest).ok_or(MLeaderParseError::Invalid)?;
        *slot = parse_signed_decimal(token).ok_or(MLeaderParseError::Invalid)?;
        rest = after;
    }
    Ok(MLeaderCreateInput {
        arrow: Point3 {
            x: coordinates[0],
            y: coordinates[1],
            z: 0.0,
        },
        landing: Point3 {
            x: coordinates[2],
            y: coordinates[3],
            z: 0.0,
        },
        text: rest.trim_start().to_string(),
        dogleg_length,
        dogleg_angle,
        attachment,
    })
}

/// One raw DXF attachment code (a plain non-negative integer).
fn parse_attachment_code(token: &str) -> Option<i16> {
    if token.is_empty() || !token.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    token.parse::<i16>().ok()
}

/// The parsed `MLEADER` creation line (see [`parse_mleader_argument`]).
#[derive(Debug, Clone, PartialEq)]
pub struct MLeaderCreateInput {
    pub arrow: Point3,
    pub landing: Point3,
    /// The annotation text (may be empty, so the command layer answers with its
    /// own `tool.text_empty` refusal instead of a grammar error).
    pub text: String,
    /// The requested dogleg length, or `None` for the resolved style value.
    pub dogleg_length: Option<f64>,
    /// The requested dogleg direction in radians, or `None` for the derived side.
    pub dogleg_angle: Option<f64>,
    /// The requested text attachment pair, or `None` for the style/default pair.
    pub attachment: Option<MLeaderAttachment>,
}

/// Why a `MLEADER` typed line produced no creation payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MLeaderParseError {
    /// Tokens outside the documented grammar (nothing to write).
    Invalid,
    /// An admitted token that names a field this domain does not model.
    Unmodelled(MLeaderUnmodelledOption),
}

/// A `MLEADER` option this build's MULTILEADER domain cannot carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MLeaderUnmodelledOption {
    /// The AutoCAD **angle constraint**: DXF entity `174` (text angle type) and
    /// its `MLEADERSTYLE` mirror `175`, with property-override mask bit `0x2000`.
    TextAngle,
}

impl MLeaderUnmodelledOption {
    /// The stable refusal code of this option.
    pub fn code(self) -> &'static str {
        match self {
            Self::TextAngle => "mleader.option_text_angle",
        }
    }

    /// Why the option cannot be written, naming the missing field and groups.
    pub fn detail(self) -> &'static str {
        match self {
            Self::TextAngle => {
                "the angle constraint (text angle type: entity-tail group 174, MLEADERSTYLE \
                 group 175, override mask bit 0x2000) has no field in this build's MULTILEADER \
                 domain, and the writer declares that group unmodelled — storing a value the \
                 exporter would silently drop is refused instead"
            }
        }
    }
}

/// `MLEADEREDIT ADD <x> <y>` / `REMOVE <index>` / `PATHTYPE <index> <type>` /
/// `ARROWSIZE <index> <size>` / `BYSTYLE <index> <field>`.
///
/// The index is a plain non-negative `usize` (0-based) and the arrowhead size a
/// strictly positive plain decimal; the command layer checks the index against the
/// leader's stored line list and validates the size, so an out-of-range index or an
/// unusable size is its named refusal, not a grammar error.
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
        MLeaderEditAction::Remove {
            line_index: parse_line_index(rest[0])?,
        }
    } else if word.eq_ignore_ascii_case("PATHTYPE") {
        if rest.len() != 2 {
            return None;
        }
        MLeaderEditAction::SetPathType {
            line_index: parse_line_index(rest[0])?,
            path_type: parse_line_path_type(rest[1])?,
        }
    } else if word.eq_ignore_ascii_case("ARROWSIZE") {
        if rest.len() != 2 {
            return None;
        }
        MLeaderEditAction::SetArrowheadSize {
            line_index: parse_line_index(rest[0])?,
            size: parse_positive_decimal(rest[1])?,
        }
    } else if word.eq_ignore_ascii_case("BYSTYLE") {
        if rest.len() != 2 {
            return None;
        }
        MLeaderEditAction::ClearOverride {
            line_index: parse_line_index(rest[0])?,
            field: parse_line_override_field(rest[1])?,
        }
    } else {
        return None;
    };
    Some(MLeaderEditCommand { action })
}

/// A 0-based leader-line index (a plain non-negative `usize`).
fn parse_line_index(token: &str) -> Option<usize> {
    if token.is_empty() || !token.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    token.parse::<usize>().ok()
}

/// `LINE` / `SPLINE` / `INVISIBLE`: the three DXF leader path types (`170`).
fn parse_line_path_type(token: &str) -> Option<MLeaderPathType> {
    if token.eq_ignore_ascii_case("LINE") {
        Some(MLeaderPathType::StraightLineSegments)
    } else if token.eq_ignore_ascii_case("SPLINE") {
        Some(MLeaderPathType::Spline)
    } else if token.eq_ignore_ascii_case("INVISIBLE") {
        Some(MLeaderPathType::Invisible)
    } else {
        None
    }
}

/// `PATH` / `ARROW`: the two per-line overrides this build can set and clear.
fn parse_line_override_field(token: &str) -> Option<MLeaderLineOverrideField> {
    if token.eq_ignore_ascii_case("PATH") {
        Some(MLeaderLineOverrideField::PathType)
    } else if token.eq_ignore_ascii_case("ARROW") {
        Some(MLeaderLineOverrideField::ArrowheadSize)
    } else {
        None
    }
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
