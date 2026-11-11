//! Typed dispatch for the 2026-11-12 MULTILEADER / MLEDIT / TOLERANCE lines.
//!
//! Pure parsing lives in [`super::commands::mleader`]; this module maps one
//! admitted selector to its [`CommandId`] and dispatches the payload through the
//! same `HostCore::command` funnel every other typed form uses. An argument
//! outside a selector's grammar is refused before IPC with a stable reason.
//!
//! `MLEADERCOLLECT` takes no argument, so it never reaches this module: its bare
//! selector dispatches with [`CommandPayload::None`] and the current selection.

use cad_app::{CommandId, CommandPayload, ExtendedCommandId};

use super::commands::{
    parse_mleaderalign_argument, parse_mleaderedit_argument, parse_mledit_argument,
    parse_tolerance_argument, DASH_MLEDIT_SELECTOR, MLEADERALIGN_SELECTOR, MLEADEREDIT_SELECTOR,
    MLEDIT_SELECTOR, TOLERANCE_SELECTOR,
};
use super::diagnostics::cad_error_text;
use super::CadSession;

/// The stable refusal for an `MLEDIT` / `-MLEDIT` argument outside the grammar.
pub(crate) fn invalid_mledit_argument_reason(text: &str) -> String {
    format!(
        "mledit.argument: unsupported or malformed MLEDIT argument in `{}`: the typed forms are \
         `MLEDIT MERGE|CLOSED|CROSS|TEE <x> <y>`, `MLEDIT OPEN <x> <y>` and `MLEDIT BREAK <x1> \
         <y1> <x2> <y2>` (`-MLEDIT` accepts the same grammar); nothing was changed",
        text.trim()
    )
}

/// The stable refusal for a `MLEADERALIGN` / `MLEADEREDIT` argument outside the
/// grammar.
pub(crate) fn invalid_mleader_edit_argument_reason(selector: &str, text: &str) -> String {
    let (code, forms) = if selector == MLEADERALIGN_SELECTOR {
        (
            "mleaderalign.argument",
            "`MLEADERALIGN LEFT|RIGHT|CENTER [positive spacing]`",
        )
    } else {
        (
            "mleaderedit.argument",
            "`MLEADEREDIT ADD <x> <y>` or `MLEADEREDIT REMOVE <0-based line index>`",
        )
    };
    format!(
        "{code}: unsupported or malformed {selector} argument in `{}`: the typed form is {forms}; \
         nothing was changed",
        text.trim()
    )
}

/// The stable refusal for a `TOLERANCE` argument outside the grammar.
pub(crate) fn invalid_tolerance_argument_reason(text: &str) -> String {
    format!(
        "tolerance.argument: unsupported or malformed TOLERANCE argument in `{}`: the typed form \
         is `TOLERANCE <x> <y> <text>` with `|`-separated compartments; nothing was changed",
        text.trim()
    )
}

impl CadSession {
    pub(super) fn mleader_argument(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        match selector {
            MLEDIT_SELECTOR | DASH_MLEDIT_SELECTOR => {
                let Some(payload) = parse_mledit_argument(argument) else {
                    return self.refuse_argument(invalid_mledit_argument_reason(text));
                };
                let id = if selector == DASH_MLEDIT_SELECTOR {
                    CommandId::Extended(ExtendedCommandId::DashMlEdit)
                } else {
                    CommandId::Extended(ExtendedCommandId::MlEdit)
                };
                self.core.record_command(selector);
                self.submit(id, CommandPayload::MlEdit(Box::new(payload)))
            }
            MLEADERALIGN_SELECTOR => {
                let Some(payload) = parse_mleaderalign_argument(argument) else {
                    return self
                        .refuse_argument(invalid_mleader_edit_argument_reason(selector, text));
                };
                self.core.record_command(selector);
                self.submit(
                    CommandId::Extended(ExtendedCommandId::MLeaderAlign),
                    CommandPayload::MLeaderAlign(Box::new(payload)),
                )
            }
            MLEADEREDIT_SELECTOR => {
                let Some(payload) = parse_mleaderedit_argument(argument) else {
                    return self
                        .refuse_argument(invalid_mleader_edit_argument_reason(selector, text));
                };
                self.core.record_command(selector);
                self.submit(
                    CommandId::Extended(ExtendedCommandId::MLeaderEdit),
                    CommandPayload::MLeaderEdit(Box::new(payload)),
                )
            }
            legal => {
                debug_assert_eq!(legal, TOLERANCE_SELECTOR);
                let Some(payload) = parse_tolerance_argument(argument) else {
                    return self.refuse_argument(invalid_tolerance_argument_reason(text));
                };
                self.core.record_command(selector);
                self.submit(
                    CommandId::Extended(ExtendedCommandId::Tolerance),
                    CommandPayload::Tolerance(Box::new(payload)),
                )
            }
        }
    }

    /// Dispatch one payload and fold the result into the session error slot.
    fn submit(&self, id: CommandId, payload: CommandPayload) -> Result<(), String> {
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
}
