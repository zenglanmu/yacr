//! `TEXT3D`: the typed extrude-text line.
//!
//! The session parses the line and dispatches the corresponding payload; the
//! application owns the shaping/geometry build, the transaction and every
//! refusal, so a typed line and a direct payload caller agree.

use cad_app::{CommandId, CommandPayload};

use super::commands::{invalid_text3d_argument_reason, parse_text3d_argument, TEXT3D_SELECTOR};

impl super::CadSession {
    /// Apply one typed `TEXT3D <x> <y> <z> <height> <extrusion> <text>` line.
    pub(super) fn text3d_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(input) = parse_text3d_argument(argument) else {
            return self.refuse_argument(invalid_text3d_argument_reason(text));
        };
        self.dispatch_typed_argument(
            TEXT3D_SELECTOR,
            CommandId::Text3d,
            CommandPayload::Text3d(input),
        )
    }
}
