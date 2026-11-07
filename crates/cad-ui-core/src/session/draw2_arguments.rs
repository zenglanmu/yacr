//! Typed-argument dispatch for the 2026-11-07 draw/modify batch.
//!
//! Pure parsing lives in [`super::commands::draw2`]; this module maps one
//! admitted selector to its [`CommandId`] and dispatches the payload through the
//! same `HostCore::command` funnel every other typed form uses. An argument
//! outside a selector's grammar is refused before IPC with a stable reason.

use cad_app::{CommandId, CommandPayload};

use super::commands::{
    parse_donut_argument, parse_draworder_argument, parse_helix_argument, parse_mline_argument,
    parse_mlstyle_argument, parse_mpedit_argument, parse_solid_argument, parse_trex_argument,
    DONUT_SELECTOR, HELIX_SELECTOR, MLINE_SELECTOR, MLSTYLE_SELECTOR, MPEDIT_SELECTOR,
    SOLID_SELECTOR, TREX_SELECTOR,
};
use super::diagnostics::cad_error_text;
use super::CadSession;

/// The stable refusal for an argument outside a selector's grammar.
pub fn invalid_draw2_argument_reason(selector: &str, text: &str) -> String {
    format!(
        "unsupported or malformed {selector} argument in `{}`: the typed form is {selector} \
         <argument> (see the command contract); nothing was changed",
        text.trim()
    )
}

impl CadSession {
    pub(super) fn draw2_argument(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        let (id, payload) = match selector {
            DONUT_SELECTOR => (CommandId::Donut, parse_donut_argument(argument)),
            SOLID_SELECTOR => (CommandId::Solid, parse_solid_argument(argument)),
            HELIX_SELECTOR => (CommandId::Helix, parse_helix_argument(argument)),
            MLINE_SELECTOR => (CommandId::Mline, parse_mline_argument(argument)),
            MLSTYLE_SELECTOR => (CommandId::MlStyle, parse_mlstyle_argument(argument)),
            TREX_SELECTOR => (CommandId::Trex, parse_trex_argument(argument)),
            MPEDIT_SELECTOR => (CommandId::Mpedit, parse_mpedit_argument(argument)),
            _ => (CommandId::DrawOrder, parse_draworder_argument(argument)),
        };
        let Some(payload) = payload else {
            return self.refuse_argument(invalid_draw2_argument_reason(selector, text));
        };
        self.core.record_command(selector);
        match self
            .core
            .command(id, CommandPayload::Draw2(Box::new(payload)))
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
