//! The two typed argument forms of the text commands: `TORIENT <angle-degrees>`
//! / `TORIENT REVERSE` and `TCASE <mode>` (2026-11-07).
//!
//! Both are **argument forms of their base selectors**, not commands of their
//! own: there is no second `CommandId` and no second IPC. The selection is still
//! the input, so this module only parses the typed argument and dispatches the
//! matching [`cad_app::CommandPayload`].
//!
//! * `TORIENT <angle>` carries [`cad_app::CommandPayload::Torient`] with the
//!   absolute angle in **degrees** (finite decimal, sign allowed). The words
//!   `REVERSE`/`R` (case-insensitive) carry
//!   [`cad_app::CommandPayload::TorientReverse`] instead (add 180°). A bare
//!   `TORIENT` never reaches here: it keeps the name-only funnel and the
//!   application applies the Most Readable fold.
//! * `TCASE <mode>` carries [`cad_app::CommandPayload::TCase`] with one of the
//!   five [`cad_app::TextCaseMode`] values. A bare `TCASE` applies Title (the
//!   documented current default) through the name-only funnel.
//!
//! An argument outside its grammar is refused with the stable
//! [`super::invalid_torient_argument_reason`] /
//! [`super::invalid_tcase_argument_reason`], recorded for `cad_status.error`,
//! and changes nothing.

use cad_app::{CommandId, CommandPayload, TextCaseMode};

use super::commands::parse_signed_decimal;
use super::diagnostics::cad_error_text;
use super::CadSession;

/// AutoCAD Express Tools `TORIENT <angle-degrees>`: set one **absolute** text
/// rotation for the current selection.
///
/// The bare `TORIENT` (no argument) is the Most Readable form and keeps the
/// name-only funnel (the application applies the `(-π/2, π/2]` fold). This
/// argument form only carries the absolute angle; the selection is still the
/// input, so there is no second IPC and no interactive capture.
pub const TORIENT_SELECTOR: &str = "TORIENT";

/// AutoCAD Express Tools `TCASE <mode>`: change the case of the current
/// selection.
///
/// The bare `TCASE` applies Title (the documented current default) through the
/// name-only funnel. The argument form accepts `UPPER`, `LOWER`, `TITLE`,
/// `SENTENCE`, `TOGGLE` and the single letters `U`, `L`, `T`, `S`, `G`
/// (case-insensitive), parsed by [`parse_tcase_argument`].
pub const TCASE_SELECTOR: &str = "TCASE";

/// One admitted `TORIENT` argument: an absolute angle or the Reverse option.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TorientArgument {
    /// Absolute rotation in **degrees** (finite decimal, sign allowed).
    Angle(f64),
    /// AutoCAD's `Reverse` option (`REVERSE`/`R`): add 180° to each rotation.
    Reverse,
}

/// Parse the argument of a `TORIENT` selector: one finite decimal angle in
/// **degrees**, or the `REVERSE`/`R` word.
///
/// The numeric shape is the same narrow decimal as `PDSIZE`
/// ([`parse_signed_decimal`]): `45`, `22.5`, `0`, `-90`, `+30`. The sign is
/// meaningful (a clockwise absolute rotation), exponents (`1e3`), a bare
/// `.`/`5.`, embedded spaces (`45 30`) and every non-finite magnitude are
/// refused with `None`. `REVERSE` and `R` are matched case-insensitively and
/// carry [`TorientArgument::Reverse`]; no other word or partial spelling is
/// admitted. The caller turns `None` into the stable refusal from
/// [`invalid_torient_argument_reason`]; nothing is clamped and no default angle
/// is invented. A bare `TORIENT` is the separate Most Readable form.
pub fn parse_torient_argument(argument: &str) -> Option<TorientArgument> {
    let trimmed = argument.trim();
    if trimmed.eq_ignore_ascii_case("REVERSE") || trimmed.eq_ignore_ascii_case("R") {
        return Some(TorientArgument::Reverse);
    }
    parse_signed_decimal(trimmed).map(TorientArgument::Angle)
}

/// Parse the argument of a `TCASE` selector: one of the five case modes.
///
/// Accepted (case-insensitive): `UPPER`/`U`, `LOWER`/`L`, `TITLE`/`T`,
/// `SENTENCE`/`S`, `TOGGLE`/`G`. No other word, partial spelling or empty
/// argument is admitted; the caller turns `None` into the stable refusal from
/// [`invalid_tcase_argument_reason`]. A bare `TCASE` is Title (the documented
/// current default) and never reaches this parser.
pub fn parse_tcase_argument(argument: &str) -> Option<TextCaseMode> {
    let key = argument.trim();
    if key.eq_ignore_ascii_case("UPPER") || key.eq_ignore_ascii_case("U") {
        return Some(TextCaseMode::Upper);
    }
    if key.eq_ignore_ascii_case("LOWER") || key.eq_ignore_ascii_case("L") {
        return Some(TextCaseMode::Lower);
    }
    if key.eq_ignore_ascii_case("TITLE") || key.eq_ignore_ascii_case("T") {
        return Some(TextCaseMode::Title);
    }
    if key.eq_ignore_ascii_case("SENTENCE") || key.eq_ignore_ascii_case("S") {
        return Some(TextCaseMode::Sentence);
    }
    if key.eq_ignore_ascii_case("TOGGLE") || key.eq_ignore_ascii_case("G") {
        return Some(TextCaseMode::Toggle);
    }
    None
}

/// The stable rejection reason for a `TORIENT` argument outside the grammar.
///
/// The typed line is echoed and both accepted forms are named: one finite
/// decimal angle in degrees (sign allowed) or the `REVERSE`/`R` word (add 180°).
/// The bare command is a different form (Most Readable), so an empty argument
/// never reaches this refusal.
pub fn invalid_torient_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (TORIENT needs one finite decimal angle in degrees, e.g. \
         `TORIENT 45`, REVERSE or R to add 180°, or no argument for Most Readable)",
        text.trim()
    )
}

/// The stable rejection reason for a `TCASE` argument outside the grammar.
///
/// The typed line is echoed and the five accepted modes (with their single
/// letters) are named, so a typo is refused by name rather than silently
/// applied as Title.
pub fn invalid_tcase_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (TCASE needs UPPER, LOWER, TITLE, SENTENCE or TOGGLE, or \
         U, L, T, S, G)",
        text.trim()
    )
}

impl CadSession {
    /// Apply one typed `TORIENT <angle-degrees>` / `TORIENT REVERSE` line.
    fn torient_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(argument) = super::parse_torient_argument(argument) else {
            return self.refuse_argument(super::invalid_torient_argument_reason(text));
        };
        let payload = match argument {
            TorientArgument::Angle(angle_degrees) => CommandPayload::Torient {
                angle_degrees: Some(angle_degrees),
            },
            TorientArgument::Reverse => CommandPayload::TorientReverse,
        };
        self.dispatch_argument(TORIENT_SELECTOR, CommandId::Torient, payload)
    }

    /// Apply one typed `TCASE <mode>` line.
    fn tcase_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(mode) = super::parse_tcase_argument(argument) else {
            return self.refuse_argument(super::invalid_tcase_argument_reason(text));
        };
        self.dispatch_argument(
            TCASE_SELECTOR,
            CommandId::TCase,
            CommandPayload::TCase { mode },
        )
    }

    /// Echo the admitted selector (never the typed value) and dispatch.
    fn dispatch_argument(
        &self,
        selector: &'static str,
        id: CommandId,
        payload: CommandPayload,
    ) -> Result<(), String> {
        self.core.record_command(selector);
        match self.core.command(id, payload) {
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

    /// The `argument_command` arms for the two text selectors. Kept here so the
    /// point-style module stays the dispatch table only.
    pub(super) fn text_command_argument(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        match selector {
            TORIENT_SELECTOR => self.torient_argument(argument, text),
            _ => {
                debug_assert_eq!(selector, TCASE_SELECTOR);
                self.tcase_argument(argument, text)
            }
        }
    }
}
