//! Typed-argument forms of the dimension commands: `DIMSPACE`, `DIMSTYLE`,
//! `-DIMSTYLE`, `DIMTEDIT`, `DIMEDIT`, `DIMOVERRIDE` and `DIMJOGLINE`.
//!
//! Each is an **argument form of its base selector**, not a second command:
//! `-DIMSTYLE` normalizes onto `DIMSTYLE` before this module sees it, and every
//! other selector keeps its one [`CommandId`]. The selection (where relevant) is
//! still the input; this module only parses the typed argument and dispatches the
//! matching payload (`CommandPayload::DimSpace` for the spacing value,
//! [`CommandPayload::Dimension`] for the editing batch).
//!
//! A bare selector keeps the name-only funnel (Auto for `DIMSPACE`; the
//! editing batch is refused by the application with `*.needs_option`), never
//! defaulted. An argument outside a grammar is refused before IPC with
//! [`invalid_dim_argument_reason`] and changes nothing.

use cad_app::{
    CommandId, CommandPayload, DimEditCommand, DimJogLineCommand, DimOverrideCommand,
    DimStyleCommand, DimTextEditCommand, DimensionBatchPayload,
};

use super::diagnostics::cad_error_text;
use super::CadSession;

/// `DIMSPACE <spacing>`: one explicit positive row pitch; a bare selector is
/// Auto (the base dimension's measurement).
pub const DIMSPACE_SELECTOR: &str = "DIMSPACE";

/// `DIMSTYLE LIST|CURRENT|SET <name>`: the stored DIMSTYLE table.
pub const DIMSTYLE_SELECTOR: &str = "DIMSTYLE";

/// `DIMTEDIT MOVE <x> <y>|ROTATE <degrees>|RESET`: text placement/rotation.
pub const DIMTEDIT_SELECTOR: &str = "DIMTEDIT";

/// `DIMEDIT TEXT <override>|RESET`: the dimension text override.
pub const DIMEDIT_SELECTOR: &str = "DIMEDIT";

/// `DIMOVERRIDE <variable> <value>`: one entity style-snapshot override.
pub const DIMOVERRIDE_SELECTOR: &str = "DIMOVERRIDE";

/// `DIMJOGLINE ADD <x> <y>|REMOVE`: modelled so the refusal can name the gap.
pub const DIMJOGLINE_SELECTOR: &str = "DIMJOGLINE";

/// Parse `DIMSPACE <spacing>`: exactly one **non-negative** finite decimal.
///
/// The numeric shape is the repository's plain-decimal grammar
/// ([`super::commands::parse_signed_decimal`]) with an unsigned, non-negative
/// value: `5`, `2.5`, `0.5` and `0`. Zero is meaningful — AutoCAD's
/// `DIMSPACE 0` **aligns** every target on the base dimension line — so it is
/// admitted here and handed to the command. Negatives, a leading sign,
/// exponents (`1e3`), a bare `.`/`5.`, embedded spaces (`5 6`) and trailing
/// junk are refused with `None`. The caller turns `None` into
/// [`invalid_dimspace_argument_reason`]; nothing is clamped and no spacing is
/// invented. A bare `DIMSPACE` is the separate **Auto** form and never reaches
/// this parser.
pub fn parse_dimspace_argument(argument: &str) -> Option<f64> {
    let text = argument.trim();
    if text.starts_with(['+', '-']) {
        return None;
    }
    super::commands::parse_signed_decimal(text).filter(|value| *value >= 0.0)
}

/// Parse `DIMSTYLE LIST|CURRENT|SET <name>`.
///
/// The three words are case-insensitive. `SET` requires a non-empty name
/// (spaces allowed, so a style name is carried verbatim); `LIST`/`CURRENT` admit
/// no extra tokens. No abbreviation and no default option are admitted.
pub fn parse_dimstyle_argument(argument: &str) -> Option<DimStyleCommand> {
    let trimmed = argument.trim();
    let (word, rest) = match trimmed.split_once(char::is_whitespace) {
        Some((word, rest)) => (word, rest.trim()),
        None => (trimmed, ""),
    };
    if word.eq_ignore_ascii_case("LIST") && rest.is_empty() {
        return Some(DimStyleCommand::List);
    }
    if word.eq_ignore_ascii_case("CURRENT") && rest.is_empty() {
        return Some(DimStyleCommand::Current);
    }
    if word.eq_ignore_ascii_case("SET") && !rest.is_empty() {
        return Some(DimStyleCommand::Set(rest.to_string()));
    }
    None
}

/// Parse `DIMTEDIT MOVE <x> <y>|ROTATE <degrees>|RESET`.
///
/// The coordinates/angle use the repository's one signed plain-decimal grammar
/// (no exponent, no bare `.`/`5.`). No other word, partial spelling or trailing
/// token is admitted.
pub fn parse_dimtedit_argument(argument: &str) -> Option<DimTextEditCommand> {
    let tokens: Vec<&str> = argument.split_whitespace().collect();
    match tokens.as_slice() {
        [word, x, y] if word.eq_ignore_ascii_case("MOVE") => Some(DimTextEditCommand::Move {
            x: super::commands::parse_signed_decimal(x)?,
            y: super::commands::parse_signed_decimal(y)?,
        }),
        [word, degrees] if word.eq_ignore_ascii_case("ROTATE") => {
            Some(DimTextEditCommand::Rotate {
                degrees: super::commands::parse_signed_decimal(degrees)?,
            })
        }
        [word] if word.eq_ignore_ascii_case("RESET") => Some(DimTextEditCommand::Reset),
        _ => None,
    }
}

/// Parse `DIMEDIT TEXT <override>|RESET`.
///
/// The override is the whole trimmed remainder (spaces allowed, `<>` is the
/// measurement placeholder); `RESET` admits no extra token. An empty override is
/// not expressible on the command line: `TEXT` without text is refused by the
/// grammar, and clearing the override is `RESET`.
pub fn parse_dimedit_argument(argument: &str) -> Option<DimEditCommand> {
    let trimmed = argument.trim();
    let (word, rest) = match trimmed.split_once(char::is_whitespace) {
        Some((word, rest)) => (word, rest.trim()),
        None => (trimmed, ""),
    };
    if word.eq_ignore_ascii_case("RESET") && rest.is_empty() {
        return Some(DimEditCommand::Reset);
    }
    if word.eq_ignore_ascii_case("TEXT") && !rest.is_empty() {
        return Some(DimEditCommand::Text(rest.to_string()));
    }
    None
}

/// Parse `DIMOVERRIDE <variable> <value>`.
///
/// The variable is one token and the value is the whole trimmed remainder
/// (verbatim; no quote processing). The closed variable set and every value
/// grammar belong to the command layer, which refuses an unknown variable or a
/// bad value by name.
pub fn parse_dimoverride_argument(argument: &str) -> Option<DimOverrideCommand> {
    let trimmed = argument.trim();
    let (variable, value) = trimmed.split_once(char::is_whitespace)?;
    let value = value.trim();
    if variable.is_empty() || value.is_empty() {
        return None;
    }
    Some(DimOverrideCommand {
        variable: variable.to_string(),
        value: value.to_string(),
    })
}

/// Parse `DIMJOGLINE ADD <x> <y>|REMOVE`.
///
/// The command refuses by name (the domain cannot store a jog), but the typed
/// grammar is still admitted so the refusal names the *real* capability gap
/// instead of pretending the option word is invalid.
pub fn parse_dimjogline_argument(argument: &str) -> Option<DimJogLineCommand> {
    let tokens: Vec<&str> = argument.split_whitespace().collect();
    match tokens.as_slice() {
        [word, x, y] if word.eq_ignore_ascii_case("ADD") => Some(DimJogLineCommand::Add {
            x: super::commands::parse_signed_decimal(x)?,
            y: super::commands::parse_signed_decimal(y)?,
        }),
        [word] if word.eq_ignore_ascii_case("REMOVE") => Some(DimJogLineCommand::Remove),
        _ => None,
    }
}

/// The stable refusal for a `DIMSPACE` argument outside the grammar.
///
/// The typed line is echoed and both accepted forms are named: one positive
/// finite spacing value, or no argument for **Auto**.
pub fn invalid_dimspace_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (DIMSPACE needs one non-negative finite spacing — `0` \
         aligns — e.g. `DIMSPACE 5`, or no argument for Auto)",
        text.trim()
    )
}

/// The stable refusal for an argument outside a selector's grammar.
pub fn invalid_dim_argument_reason(selector: &str, text: &str) -> String {
    format!(
        "unsupported or malformed {selector} argument in `{}`: the typed form is the documented \
         {selector} option (see the command contract); nothing was changed",
        text.trim()
    )
}

impl CadSession {
    /// Apply one typed `DIMSPACE <spacing>` line: the explicit positive row
    /// pitch travels as [`CommandPayload::DimSpace`]`(Some(value))`.
    ///
    /// The current selection is still the whole object input. A bare `DIMSPACE`
    /// never reaches here: it keeps the name-only funnel and dispatches
    /// `CommandPayload::None` (Auto, the base dimension's measurement).
    pub(super) fn dimspace_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(spacing) = parse_dimspace_argument(argument) else {
            return self.refuse_argument(invalid_dimspace_argument_reason(text));
        };
        self.core.record_command(DIMSPACE_SELECTOR);
        match self
            .core
            .command(CommandId::DimSpace, CommandPayload::DimSpace(Some(spacing)))
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

    /// Dispatch one admitted dimension-batch argument line.
    pub(super) fn dim_argument(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        let (id, payload) = match selector {
            DIMSTYLE_SELECTOR => (
                CommandId::DimStyle,
                parse_dimstyle_argument(argument).map(DimensionBatchPayload::Style),
            ),
            DIMTEDIT_SELECTOR => (
                CommandId::DimTextEdit,
                parse_dimtedit_argument(argument).map(DimensionBatchPayload::TextEdit),
            ),
            DIMEDIT_SELECTOR => (
                CommandId::DimEdit,
                parse_dimedit_argument(argument).map(DimensionBatchPayload::Edit),
            ),
            DIMOVERRIDE_SELECTOR => (
                CommandId::DimOverride,
                parse_dimoverride_argument(argument).map(DimensionBatchPayload::Override),
            ),
            _ => (
                CommandId::DimJogLine,
                parse_dimjogline_argument(argument).map(DimensionBatchPayload::JogLine),
            ),
        };
        let Some(payload) = payload else {
            return self.refuse_argument(invalid_dim_argument_reason(selector, text));
        };
        self.core.record_command(selector);
        match self
            .core
            .command(id, CommandPayload::Dimension(Box::new(payload)))
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
}
