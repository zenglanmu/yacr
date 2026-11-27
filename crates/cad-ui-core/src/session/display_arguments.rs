//! `SHADOWS`/`FOG`: the SketchUp display-settings typed lines.
//!
//! The session parses the line and dispatches the corresponding payload; the
//! application owns the document write, the undo step and every semantic
//! refusal, so the typed line and a direct payload caller agree.

use cad_app::{CommandId, CommandPayload};

use super::commands::{
    invalid_fog_argument_reason, invalid_shadows_argument_reason, parse_fog_argument,
    parse_shadows_argument, FOG_SELECTOR, SHADOWS_SELECTOR,
};

impl super::CadSession {
    /// Apply one typed `SHADOWS <option>` line.
    pub(super) fn shadows_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(option) = parse_shadows_argument(argument) else {
            return self.refuse_argument(invalid_shadows_argument_reason(text));
        };
        self.dispatch_typed_argument(
            SHADOWS_SELECTOR,
            CommandId::Shadows,
            CommandPayload::Display(option),
        )
    }

    /// Apply one typed `FOG <option>` line.
    pub(super) fn fog_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(option) = parse_fog_argument(argument) else {
            return self.refuse_argument(invalid_fog_argument_reason(text));
        };
        self.dispatch_typed_argument(
            FOG_SELECTOR,
            CommandId::Fog,
            CommandPayload::Display(option),
        )
    }
}
