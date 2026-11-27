//! `SOFTENEDGES [<angle>]`: the SketchUp soften-edges typed line.

use cad_app::{CommandId, CommandPayload};

use super::commands::{
    invalid_softenedges_argument_reason, parse_softenedges_argument, SOFTENEDGES_SELECTOR,
};

impl super::CadSession {
    /// Apply one typed `SOFTENEDGES [<angle>]` line.
    pub(super) fn softenedges_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(angle) = parse_softenedges_argument(argument) else {
            return self.refuse_argument(invalid_softenedges_argument_reason(text));
        };
        let payload = match angle {
            Some(angle) => CommandPayload::SoftenEdges(angle),
            None => CommandPayload::None,
        };
        self.dispatch_typed_argument(SOFTENEDGES_SELECTOR, CommandId::SoftenEdges, payload)
    }
}
