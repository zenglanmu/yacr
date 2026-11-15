//! The typed `HATCHEDIT <option>` line (AutoCAD 2026).
//!
//! `HATCHEDIT` is an argument form of its base selector, not a command of its
//! own: there is no second [`cad_app::CommandId`] and no second IPC. The current
//! selection is still the input (one or more directly addressable model-space
//! hatches), so this module only parses the typed option and dispatches
//! [`cad_app::CommandPayload::HatchEdit`].
//!
//! Grammar (case-insensitive):
//!
//! * `ANGLE <degrees>` — one finite signed decimal, in degrees;
//! * `SCALE <factor>` — one finite decimal strictly greater than 0;
//! * `PATTERNTYPE USER|PREDEFINED|CUSTOM` — DXF group 76;
//! * `PATTERN <name>` — replace the fill with the named built-in `.pat` library
//!   pattern (one token, no spaces; matched ASCII case-insensitively against
//!   [`cad_domain::hatch::predefined_pattern`], the one name table), keeping the
//!   hatch's stored angle/scale/`double`. A bare `PATTERN` is
//!   `hatchedit.needs_pattern_name`; an unknown name is
//!   `hatchedit.unknown_pattern` — both are refused **here**, before IPC; a
//!   solid/gradient fill is refused by the command with
//!   `hatchedit.pattern_requires_pattern`;
//! * `PATTERNFILE <path.pat> [<name>]` — the same edit from a **user** `.pat`
//!   file. The path is one whitespace-free token; a line whose second token is
//!   itself a path is a spaced path the grammar cannot split from the name, so it
//!   is refused by name (`hatchedit.pattern_file_path`). The optional name
//!   selects one entry of the file, and without it the file must hold exactly one
//!   pattern. This session reads the file through the host
//!   strategy ([`cad_host::HostResources::read_named_file`]) — the command never
//!   touches the filesystem — so a missing/unreadable file is
//!   `hatchedit.pattern_file_missing`/`_unreadable`, a non-UTF-8 file is
//!   `hatchedit.pattern_file_encoding`, and a host without a filesystem is the
//!   stable `hatchedit.pattern_file_unavailable` (`unsupported-runtime:
//!   pattern-file`). An unparsable file and an unknown name are refused by the
//!   command (`hatchedit.pattern_file_invalid` / `hatchedit.unknown_pattern`);
//! * `STYLE IGNORE|OUTER|NORMAL` — the island-detection style (group 75);
//! * `SOLID`, `GRADIENT` — the fill variant;
//! * `COLOR <aci>` or `COLOR BYLAYER`;
//! * `TRANSPARENCY <percent>` (`0..=90`) or `TRANSPARENCY BYLAYER`;
//! * `LAYER <name>` — the drawing layer name (spaces allowed);
//! * `ORIGIN <x> <y>` — two finite signed decimals: the pattern-origin
//!   displacement in the hatch plane (pattern fill only; a solid/gradient fill is
//!   refused by the command with `hatchedit.not_pattern`);
//! * `DRAWORDER BEHIND|FRONT|BOUNDARY`;
//! * `RECREATEBOUNDARY`, `SEPARATE`;
//! * `ADDBOUNDARIES`, `REMOVEBOUNDARIES` — the pre-selection boundary edit:
//!   exactly one hatch plus the other selected closed entities, rewritten as one
//!   transaction / one undo step;
//! * the named-refusal forms `ASSOCIATE`, `DISASSOCIATE`, `ANNOTATIVE` (they
//!   parse so the application can name the missing model instead of pretending
//!   the option word is invalid).
//!
//! A bare `HATCHEDIT` never reaches here (it keeps the name-only funnel and the
//! application refuses it with `hatchedit.needs_option`). An argument outside
//! the grammar is refused with the stable
//! [`super::invalid_hatchedit_argument_reason`], recorded for `cad_status.error`,
//! and changes nothing.

use std::path::Path;

use cad_app::{CommandId, CommandPayload, HatchDrawOrder, HatchEditOption, PatternFileText};
use cad_domain::hatch::{predefined_pattern, predefined_patterns};
use cad_domain::{HatchPatternType, HatchStyle};
use cad_platform::NamedFileError;

use super::commands::parse_signed_decimal;
use super::diagnostics::cad_error_text;
use super::{CadSession, HATCHEDIT_SELECTOR};

/// Why the typed `HATCHEDIT` argument was refused **before** IPC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HatchEditArgumentError {
    /// The line is outside the declared grammar: the stable
    /// [`invalid_hatchedit_argument_reason`] answer.
    Malformed,
    /// `PATTERN` with no name: `hatchedit.needs_pattern_name`.
    MissingPatternName,
    /// `PATTERN <name>` where the built-in `.pat` library has no such name:
    /// `hatchedit.unknown_pattern`, answered with the accepted name list.
    UnknownPattern(String),
    /// `PATTERNFILE` whose second token is itself a path: the line is a path with
    /// spaces, which this grammar cannot tell from `PATTERNFILE <path> <name>`.
    /// `hatchedit.pattern_file_path`, answered with the rule.
    PatternFilePath,
}

/// Parse the argument of a `HATCHEDIT` selector: one declared option.
///
/// No partial spelling, option stack or AutoCAD dialog field is admitted. A
/// malformed line is [`HatchEditArgumentError::Malformed`] and the caller turns
/// it into the stable [`invalid_hatchedit_argument_reason`]; the two `PATTERN`
/// name states are named refusals of their own
/// ([`HatchEditArgumentError::MissingPatternName`] /
/// [`HatchEditArgumentError::UnknownPattern`]). The bare selector never reaches
/// this parser: it keeps the name-only funnel and the application refuses it
/// with `hatchedit.needs_option`.
pub fn parse_hatchedit_argument(argument: &str) -> Result<HatchEditOption, HatchEditArgumentError> {
    let trimmed = argument.trim();
    let (word, rest) = match trimmed.find([' ', '\t']) {
        Some(index) => (&trimmed[..index], trimmed[index..].trim()),
        None => (trimmed, ""),
    };
    if word.eq_ignore_ascii_case("PATTERN") {
        return parse_pattern_name(rest);
    }
    if word.eq_ignore_ascii_case("PATTERNFILE") {
        return parse_pattern_file(rest);
    }
    parse_declared_option(word, rest).ok_or(HatchEditArgumentError::Malformed)
}

/// Refuse `PATTERN` with no name, and resolve every other token against the one
/// built-in pattern table (never a second name list).
fn parse_pattern_name(text: &str) -> Result<HatchEditOption, HatchEditArgumentError> {
    if text.is_empty() {
        return Err(HatchEditArgumentError::MissingPatternName);
    }
    // A pattern name is one token: embedded whitespace is a malformed line, not
    // an unknown name (the library name set has no spaces).
    if text.contains([' ', '\t']) {
        return Err(HatchEditArgumentError::Malformed);
    }
    if predefined_pattern(text).is_none() {
        return Err(HatchEditArgumentError::UnknownPattern(text.to_string()));
    }
    Ok(HatchEditOption::Pattern(text.to_string()))
}

/// Parse `PATTERNFILE <path.pat> [<name>]`.
///
/// One or two whitespace-separated tokens, never more. The content is **not**
/// read here: this is the pure grammar, and the file is read by the host in
/// [`CadSession::hatchedit_argument`] (the payload carries the text). An empty
/// remainder, or a third token, is a malformed line; a second token carrying a
/// path separator is a spaced path, refused by name
/// ([`HatchEditArgumentError::PatternFilePath`]).
fn parse_pattern_file(text: &str) -> Result<HatchEditOption, HatchEditArgumentError> {
    let mut parts = text.split_whitespace();
    let Some(path) = parts.next() else {
        return Err(HatchEditArgumentError::Malformed);
    };
    let name = parts.next();
    if parts.next().is_some() {
        return Err(HatchEditArgumentError::Malformed);
    }
    // A pattern name never contains a path separator, so a second token that does
    // is a **path with spaces**: refuse it by name instead of reading the first
    // word as a path and the rest as a name.
    if name.is_some_and(|name| name.contains(['/', '\\'])) {
        return Err(HatchEditArgumentError::PatternFilePath);
    }
    let name = name.map(str::to_string);
    Ok(HatchEditOption::PatternFile {
        path: path.to_string(),
        name,
        // Filled by the host read below; a payload that reaches the command
        // without it is refused (`hatchedit.pattern_file_not_read`).
        text: None,
    })
}

/// The word dispatch of the declared grammar (`PATTERN` is handled before it,
/// because its name states are named refusals rather than a malformed line).
fn parse_declared_option(word: &str, rest: &str) -> Option<HatchEditOption> {
    if word.eq_ignore_ascii_case("SOLID") {
        return rest.is_empty().then_some(HatchEditOption::Solid);
    }
    if word.eq_ignore_ascii_case("GRADIENT") {
        return rest.is_empty().then_some(HatchEditOption::Gradient);
    }
    if word.eq_ignore_ascii_case("RECREATEBOUNDARY") {
        return rest.is_empty().then_some(HatchEditOption::RecreateBoundary);
    }
    if word.eq_ignore_ascii_case("SEPARATE") {
        return rest.is_empty().then_some(HatchEditOption::SeparateHatches);
    }
    if word.eq_ignore_ascii_case("ORIGIN") {
        return parse_origin(rest).map(|(x, y)| HatchEditOption::Origin { x, y });
    }
    if word.eq_ignore_ascii_case("ASSOCIATE") {
        return rest.is_empty().then_some(HatchEditOption::Associate);
    }
    if word.eq_ignore_ascii_case("DISASSOCIATE") {
        return rest.is_empty().then_some(HatchEditOption::Disassociate);
    }
    if word.eq_ignore_ascii_case("ANNOTATIVE") {
        return rest.is_empty().then_some(HatchEditOption::Annotative);
    }
    if word.eq_ignore_ascii_case("ADDBOUNDARIES") {
        return rest.is_empty().then_some(HatchEditOption::AddBoundaries);
    }
    if word.eq_ignore_ascii_case("REMOVEBOUNDARIES") {
        return rest.is_empty().then_some(HatchEditOption::RemoveBoundaries);
    }
    if word.eq_ignore_ascii_case("ANGLE") {
        // `parse_signed_decimal` already refuses an empty value, exponents, a
        // bare sign, embedded spaces and every non-finite magnitude.
        return parse_signed_decimal(rest).map(HatchEditOption::Angle);
    }
    if word.eq_ignore_ascii_case("SCALE") {
        return parse_signed_decimal(rest)
            .filter(|factor| *factor > 0.0)
            .map(HatchEditOption::Scale);
    }
    if word.eq_ignore_ascii_case("PATTERNTYPE") {
        return parse_pattern_type(rest).map(HatchEditOption::PatternType);
    }
    if word.eq_ignore_ascii_case("STYLE") {
        return parse_style(rest).map(HatchEditOption::Style);
    }
    if word.eq_ignore_ascii_case("DRAWORDER") {
        return parse_draw_order(rest).map(HatchEditOption::DrawOrder);
    }
    if word.eq_ignore_ascii_case("COLOR") {
        if rest.eq_ignore_ascii_case("BYLAYER") {
            return Some(HatchEditOption::ColorByLayer);
        }
        return parse_aci(rest).map(HatchEditOption::Color);
    }
    if word.eq_ignore_ascii_case("TRANSPARENCY") {
        if rest.eq_ignore_ascii_case("BYLAYER") {
            return Some(HatchEditOption::TransparencyByLayer);
        }
        return rest
            .parse::<u8>()
            .ok()
            .filter(|percent| *percent <= 90)
            .map(HatchEditOption::Transparency);
    }
    if word.eq_ignore_ascii_case("LAYER") {
        return (!rest.is_empty()).then(|| HatchEditOption::Layer(rest.to_string()));
    }
    None
}

fn parse_pattern_type(text: &str) -> Option<HatchPatternType> {
    if text.eq_ignore_ascii_case("USER") {
        Some(HatchPatternType::UserDefined)
    } else if text.eq_ignore_ascii_case("PREDEFINED") {
        Some(HatchPatternType::Predefined)
    } else if text.eq_ignore_ascii_case("CUSTOM") {
        Some(HatchPatternType::Custom)
    } else {
        None
    }
}

fn parse_style(text: &str) -> Option<HatchStyle> {
    if text.eq_ignore_ascii_case("IGNORE") {
        Some(HatchStyle::Ignore)
    } else if text.eq_ignore_ascii_case("OUTER") {
        Some(HatchStyle::Outer)
    } else if text.eq_ignore_ascii_case("NORMAL") {
        Some(HatchStyle::Normal)
    } else {
        None
    }
}

fn parse_draw_order(text: &str) -> Option<HatchDrawOrder> {
    if text.eq_ignore_ascii_case("BEHIND") {
        Some(HatchDrawOrder::BehindAll)
    } else if text.eq_ignore_ascii_case("FRONT") {
        Some(HatchDrawOrder::InFrontOfAll)
    } else if text.eq_ignore_ascii_case("BOUNDARY") {
        Some(HatchDrawOrder::BehindBoundary)
    } else {
        None
    }
}

fn parse_aci(text: &str) -> Option<u8> {
    text.parse::<u8>()
        .ok()
        .filter(|index| (1..=255).contains(index))
}

/// Two finite signed plain decimals: the `ORIGIN <x> <y>` pair.
///
/// Exactly two whitespace-separated tokens; `parse_signed_decimal` already
/// refuses an empty value, exponents, a bare sign and every non-finite
/// magnitude, so nothing about the number syntax is re-implemented here.
fn parse_origin(text: &str) -> Option<(f64, f64)> {
    let mut parts = text.split_whitespace();
    let x = super::commands::parse_signed_decimal(parts.next()?)?;
    let y = super::commands::parse_signed_decimal(parts.next()?)?;
    parts.next().is_none().then_some((x, y))
}

/// The stable rejection reason for a `HATCHEDIT` argument outside the grammar.
///
/// The typed line is echoed and the implemented options are named, together with
/// the documented AutoCAD options that are **not** implemented, so nobody reads
/// the subset as a promise that the whole dialog works.
pub fn invalid_hatchedit_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (HATCHEDIT needs STYLE, ANGLE, SCALE, PATTERN, \
         PATTERNFILE <path.pat> [<name>], PATTERNTYPE, SOLID, GRADIENT, COLOR, TRANSPARENCY, \
         LAYER, ORIGIN <x> <y>, DRAWORDER, RECREATEBOUNDARY, SEPARATE, ADDBOUNDARIES or \
         REMOVEBOUNDARIES; ASSOCIATE/DISASSOCIATE/ANNOTATIVE are named refusals)",
        text.trim()
    )
}

/// The `hatchedit.pattern_file_path` refusal: a `PATTERNFILE` line whose second
/// token is a path, i.e. a spaced path the grammar cannot split from the name.
pub fn pattern_file_path_reason() -> String {
    "hatchedit.pattern_file_path: HATCHEDIT PATTERNFILE takes a whitespace-free `.pat` path and \
     an optional pattern name; a path containing spaces cannot be split from the name — give a \
     path without spaces"
        .to_string()
}

/// The `hatchedit.needs_pattern_name` refusal: `PATTERN` with no name.
///
/// It is answered **before IPC** (the grammar is in this crate), so the line
/// never reaches the command layer and nothing is written.
pub fn needs_pattern_name_reason() -> String {
    "hatchedit.needs_pattern_name: HATCHEDIT PATTERN needs a built-in pattern name, for example \
     ANSI31"
        .to_string()
}

/// The `hatchedit.unknown_pattern` refusal: the accepted names come from the one
/// library table ([`predefined_patterns`]), never a second list.
pub fn unknown_pattern_reason(name: &str) -> String {
    let names = predefined_patterns()
        .iter()
        .map(|pattern| pattern.name)
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "hatchedit.unknown_pattern: '{name}' is not a built-in pattern (this build implements \
         {names})"
    )
}

impl CadSession {
    /// Apply one typed `HATCHEDIT <option>` line.
    pub(super) fn hatchedit_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let option = match parse_hatchedit_argument(argument) {
            Ok(option) => option,
            Err(HatchEditArgumentError::Malformed) => {
                return self.refuse_argument(super::invalid_hatchedit_argument_reason(text))
            }
            Err(HatchEditArgumentError::MissingPatternName) => {
                return self.refuse_argument(needs_pattern_name_reason())
            }
            Err(HatchEditArgumentError::UnknownPattern(name)) => {
                return self.refuse_argument(unknown_pattern_reason(&name))
            }
            Err(HatchEditArgumentError::PatternFilePath) => {
                return self.refuse_argument(pattern_file_path_reason())
            }
        };
        // `PATTERNFILE` is the one form whose payload needs a file: the host
        // strategy reads it here (this session has no filesystem of its own, and
        // the command must never read one). Every failure is a named refusal
        // recorded before IPC, and nothing is dispatched.
        let option = match option {
            HatchEditOption::PatternFile {
                path,
                name,
                text: None,
            } => match self.read_pattern_file(&path) {
                Ok(file_text) => HatchEditOption::PatternFile {
                    path,
                    name,
                    text: Some(file_text),
                },
                Err(reason) => return self.refuse_argument(reason),
            },
            other => other,
        };
        // Echo the admitted selector (never the typed argument).
        self.core.record_command(HATCHEDIT_SELECTOR);
        match self
            .core
            .command(CommandId::HatchEdit, CommandPayload::HatchEdit(option))
        {
            Ok(()) => {
                *self.error.borrow_mut() = None;
                Ok(())
            }
            Err(error) => {
                let text = cad_error_text(&error);
                *self.error.borrow_mut() = Some(text.clone());
                Err(text)
            }
        }
    }

    /// Read a `PATTERNFILE` path through the host strategy.
    ///
    /// Viewer is refused **first**, through the same `SessionState::authorize`
    /// gate the command layer uses, so a command that cannot write never performs
    /// host I/O (and the refusal text cannot drift from the command's).
    fn read_pattern_file(&self, path: &str) -> Result<PatternFileText, String> {
        if let Err(error) = self
            .core
            .controller
            .borrow()
            .session
            .authorize(CommandId::HatchEdit)
        {
            return Err(cad_error_text(&error));
        }
        let bytes = self
            .core
            .resources
            .read_named_file(Path::new(path))
            .map_err(|error| pattern_file_read_reason(path, &error))?;
        match std::str::from_utf8(&bytes) {
            // The text is borrowed from `bytes` and copied into the payload: the
            // bytes must live past this call, so the copy is explicit.
            Ok(text) => Ok(PatternFileText::new(text)),
            Err(_) => Err(pattern_file_encoding_reason(path)),
        }
    }
}

/// The named refusal for a host read that did not deliver the pattern file.
fn pattern_file_read_reason(path: &str, error: &NamedFileError) -> String {
    match error {
        NamedFileError::Unavailable => format!(
            "hatchedit.pattern_file_unavailable: unsupported-runtime: pattern-file: this host \
             cannot read the user-named file '{path}' ({})",
            error.reason()
        ),
        NamedFileError::Missing(reason) => {
            format!("hatchedit.pattern_file_missing: {path}: {reason}")
        }
        NamedFileError::Unreadable(reason) => {
            format!("hatchedit.pattern_file_unreadable: {path}: {reason}")
        }
    }
}

/// The named refusal for a file that is not `.pat` text.
fn pattern_file_encoding_reason(path: &str) -> String {
    format!(
        "hatchedit.pattern_file_encoding: {path} is not UTF-8 text; a `.pat` pattern library is \
         text"
    )
}
