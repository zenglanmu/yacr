//! The typed `HATCHEDIT <option>` line (AutoCAD 2026), the declared v1 subset.
//!
//! `HATCHEDIT` is an argument form of its base selector, not a command of its
//! own: there is no second `CommandId` and no second IPC. The current selection
//! is still the input (exactly one directly addressable model-space hatch), so
//! this module only parses the typed option and dispatches
//! [`cad_app::CommandPayload::HatchEdit`].
//!
//! Grammar (case-insensitive):
//!
//! * `HATCHEDIT ANGLE <degrees>` — one finite signed decimal, in degrees;
//! * `HATCHEDIT SCALE <factor>` — one finite decimal strictly greater than 0;
//! * `HATCHEDIT SOLID`;
//! * `HATCHEDIT GRADIENT`.
//!
//! A bare `HATCHEDIT` never reaches here (it keeps the name-only funnel and the
//! application refuses it with `hatchedit.needs_option`). An argument outside
//! the grammar is refused with the stable
//! [`super::invalid_hatchedit_argument_reason`], recorded for `cad_status.error`,
//! and changes nothing.

use cad_app::{CommandId, CommandPayload, HatchEditOption};

use super::commands::parse_signed_decimal;
use super::diagnostics::cad_error_text;
use super::{CadSession, HATCHEDIT_SELECTOR};

/// Parse the argument of a `HATCHEDIT` selector: one declared v1 option.
///
/// The grammar is `ANGLE <degrees>`, `SCALE <factor>`, `SOLID` and `GRADIENT`
/// (case-insensitive). No other word, partial spelling, option stack or AutoCAD
/// dialog field is admitted; the caller turns `None` into the stable refusal
/// from [`invalid_hatchedit_argument_reason`]. The bare selector never reaches
/// this parser: it keeps the name-only funnel and the application refuses it
/// with `hatchedit.needs_option`.
pub fn parse_hatchedit_argument(argument: &str) -> Option<HatchEditOption> {
    let trimmed = argument.trim();
    let (word, rest) = match trimmed.find([' ', '\t']) {
        Some(index) => (&trimmed[..index], trimmed[index..].trim()),
        None => (trimmed, ""),
    };
    if word.eq_ignore_ascii_case("SOLID") {
        return rest.is_empty().then_some(HatchEditOption::Solid);
    }
    if word.eq_ignore_ascii_case("GRADIENT") {
        return rest.is_empty().then_some(HatchEditOption::Gradient);
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
    None
}

/// The stable rejection reason for a `HATCHEDIT` argument outside the grammar.
///
/// The typed line is echoed and the implemented forms are named, together with
/// the AutoCAD options that are **not** implemented, so nobody reads the v1
/// subset as a promise that the dialog's other fields work.
pub fn invalid_hatchedit_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (HATCHEDIT needs `ANGLE <degrees>`, `SCALE <factor>`, \
         `SOLID` or `GRADIENT`; Pattern, Color, Properties, Draw order, Add/Remove boundaries \
         and associativity are NOT implemented)",
        text.trim()
    )
}

impl CadSession {
    /// Apply one typed `HATCHEDIT <option>` line.
    pub(super) fn hatchedit_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(option) = parse_hatchedit_argument(argument) else {
            return self.refuse_argument(super::invalid_hatchedit_argument_reason(text));
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
}
