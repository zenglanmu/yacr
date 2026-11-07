//! The typed argument lines of the 2026-11-07 layer batch:
//! `LAYERSTATE <option>`, `LAYWALK <names>`, `LTSCALE <value>` and
//! `LAYERPMODE [ON|OFF]`.
//!
//! These are **argument forms of their own selectors**, not new commands and not
//! a second IPC: the session parses one line, dispatches the corresponding
//! [`cad_app::CommandPayload`] through [`cad_app::CommandId`] and returns the
//! application's real refusal verbatim. A bare selector keeps the name-only
//! funnel: `LAYERPMODE` reports, the other three are refused by the application
//! for their missing argument.
//!
//! Kept in its own module only to keep `session/mod.rs` inside the file-size
//! budget (the `point_style` / `pedit` split pattern).

use cad_app::{CommandId, CommandPayload, LayerStateCommand};

use super::diagnostics::cad_error_text;
use super::{
    invalid_layerpmode_argument_reason, invalid_layerstate_argument_reason,
    invalid_laywalk_argument_reason, invalid_ltscale_argument_reason, parse_layerpmode_argument,
    parse_layerstate_argument, parse_laywalk_argument, parse_ltscale_argument, CadSession,
    LAYERPMODE_SELECTOR, LAYERSTATE_SELECTOR, LAYWALK_SELECTOR, LTSCALE_SELECTOR,
};

impl CadSession {
    /// Apply one typed `LAYERSTATE <option>` line.
    pub(super) fn layer_state_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(option) = parse_layerstate_argument(argument) else {
            return self.refuse_argument(invalid_layerstate_argument_reason(text));
        };
        let payload = match option {
            super::LayerStateOption::Save(name) => LayerStateCommand::Save(name),
            super::LayerStateOption::Restore(name) => LayerStateCommand::Restore(name),
            super::LayerStateOption::List => LayerStateCommand::List,
            super::LayerStateOption::Delete(name) => LayerStateCommand::Delete(name),
        };
        self.dispatch_typed_argument(
            LAYERSTATE_SELECTOR,
            CommandId::LayerState,
            CommandPayload::LayerState(payload),
        )
    }

    /// Apply one typed `LAYWALK <name>[,<name>...]` line.
    pub(super) fn layer_walk_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(names) = parse_laywalk_argument(argument) else {
            return self.refuse_argument(invalid_laywalk_argument_reason(text));
        };
        self.dispatch_typed_argument(
            LAYWALK_SELECTOR,
            CommandId::LayWalk,
            CommandPayload::LayerWalk(names),
        )
    }

    /// Apply one typed `LTSCALE <value>` line.
    pub(super) fn linetype_scale_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(scale) = parse_ltscale_argument(argument) else {
            return self.refuse_argument(invalid_ltscale_argument_reason(text));
        };
        self.dispatch_typed_argument(
            LTSCALE_SELECTOR,
            CommandId::SetLinetypeScale,
            CommandPayload::LinetypeScale(scale),
        )
    }

    /// Apply one typed `LAYERPMODE ON|OFF` line.
    pub(super) fn layer_pmode_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(enabled) = parse_layerpmode_argument(argument) else {
            return self.refuse_argument(invalid_layerpmode_argument_reason(text));
        };
        self.dispatch_typed_argument(
            LAYERPMODE_SELECTOR,
            CommandId::LayerPmode,
            CommandPayload::LayerPmode(enabled),
        )
    }

    /// Echo the admitted selector (never the typed argument) and dispatch one
    /// payload, recording the real refusal for `cad_status.error`.
    fn dispatch_typed_argument(
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
}
