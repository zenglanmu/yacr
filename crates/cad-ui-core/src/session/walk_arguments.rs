//! `WALK <distance>` / `LOOKAROUND <yaw> <pitch>`: the typed navigation forms.
//!
//! The bare selectors are one-point captures routed as `BeginDraw`; these typed
//! forms dispatch the same commands with a numeric payload. The application owns
//! the camera rules and every refusal.

use cad_app::{CommandId, CommandPayload};

use super::commands::{
    invalid_lookaround_argument_reason, invalid_walk_argument_reason, parse_lookaround_argument,
    parse_walk_argument, LOOKAROUND_SELECTOR, WALK_SELECTOR,
};

impl super::CadSession {
    /// Apply one typed `WALK <distance>` line.
    pub(super) fn walk_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(action) = parse_walk_argument(argument) else {
            return self.refuse_argument(invalid_walk_argument_reason(text));
        };
        self.dispatch_typed_argument(WALK_SELECTOR, CommandId::Walk, CommandPayload::Walk(action))
    }

    /// Apply one typed `LOOKAROUND <yaw> <pitch>` line.
    pub(super) fn lookaround_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(action) = parse_lookaround_argument(argument) else {
            return self.refuse_argument(invalid_lookaround_argument_reason(text));
        };
        self.dispatch_typed_argument(
            LOOKAROUND_SELECTOR,
            CommandId::LookAround,
            CommandPayload::LookAround(action),
        )
    }
}
