//! `SCENE SAVE|RESTORE|LIST|DELETE`: the SketcnUp pages typed line.
//!
//! The session parses the line and dispatches the corresponding payload; the
//! application owns capture/restore, the persisted table and every semantic
//! refusal, so the typed line and a direct payload caller agree.

use cad_app::{CommandId, CommandPayload};

use super::commands::{invalid_scene_argument_reason, parse_scene_argument, SCENE_SELECTOR};

impl super::CadSession {
    /// Apply one typed `SCENE <option>` line.
    pub(super) fn scene_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(option) = parse_scene_argument(argument) else {
            return self.refuse_argument(invalid_scene_argument_reason(text));
        };
        self.dispatch_typed_argument(
            SCENE_SELECTOR,
            CommandId::Scene,
            CommandPayload::Scene(option),
        )
    }
}
