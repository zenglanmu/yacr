//! The typed `REFSET <option>` / `REFCLOSE <option>` lines: the bounded
//! in-place block-edit session's command-line options.
//!
//! Both are **argument forms of canonical selectors**, never new commands:
//! `REFSET ADD|REMOVE` edits the session working set and `REFCLOSE SAVE|DISCARD`
//! closes the edit. The grammar is case-insensitive and deliberately small; an
//! option outside it is the stable `refset.invalid_option` /
//! `refclose.invalid_option` refusal, recorded in the session's error and
//! dispatched nowhere (nothing is written). A bare `REFSET` keeps the
//! application's `ADD` default; a bare `REFCLOSE` is the application's
//! `refclose.needs_option` refusal.
//!
//! The command layer owns every semantic refusal (`refedit.*`); this module only
//! turns the typed word into the typed [`CommandPayload`].

use cad_app::{CommandId, CommandPayload, RefCloseOption, RefSetOption};

use super::commands::{REFCLOSE_SELECTOR, REFSET_SELECTOR};
use super::diagnostics::cad_error_text;
use super::CadSession;

/// Parse one `REFSET` option (`ADD` / `REMOVE`, ASCII case-insensitive).
pub fn parse_refset_option(argument: &str) -> Option<RefSetOption> {
    match argument.trim() {
        value if value.eq_ignore_ascii_case("ADD") => Some(RefSetOption::Add),
        value if value.eq_ignore_ascii_case("REMOVE") => Some(RefSetOption::Remove),
        _ => None,
    }
}

/// Parse one `REFCLOSE` option (`SAVE` / `DISCARD`, ASCII case-insensitive).
pub fn parse_refclose_option(argument: &str) -> Option<RefCloseOption> {
    match argument.trim() {
        value if value.eq_ignore_ascii_case("SAVE") => Some(RefCloseOption::Save),
        value if value.eq_ignore_ascii_case("DISCARD") => Some(RefCloseOption::Discard),
        _ => None,
    }
}

/// The stable refusal for a `REFSET` argument outside the grammar.
pub fn invalid_refset_option_reason(text: &str) -> String {
    format!(
        "refset.invalid_option: REFSET takes ADD or REMOVE; got `{}`. Nothing was changed.",
        text.trim()
    )
}

/// The stable refusal for a `REFCLOSE` argument outside the grammar.
pub fn invalid_refclose_option_reason(text: &str) -> String {
    format!(
        "refclose.invalid_option: REFCLOSE takes SAVE or DISCARD; got `{}`. Nothing was changed.",
        text.trim()
    )
}

impl CadSession {
    /// Apply one typed `REFSET ADD|REMOVE` line.
    pub(super) fn refset_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(option) = parse_refset_option(argument) else {
            return self.refuse_argument(invalid_refset_option_reason(text));
        };
        self.core.record_command(REFSET_SELECTOR);
        match self
            .core
            .command(CommandId::RefSet, CommandPayload::RefSet(option))
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

    /// Apply one typed `REFCLOSE SAVE|DISCARD` line.
    pub(super) fn refclose_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(option) = parse_refclose_option(argument) else {
            return self.refuse_argument(invalid_refclose_option_reason(text));
        };
        self.core.record_command(REFCLOSE_SELECTOR);
        match self
            .core
            .command(CommandId::RefClose, CommandPayload::RefClose(option))
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
