//! `EDGECOLOR ...`: dispatch the session body edge-colour command.

use cad_app::{CommandId, CommandPayload, EdgeColorCommand};

use super::commands::{
    invalid_edge_color_argument_reason, parse_edge_color_argument, EdgeColorArgument,
    EDGECOLOR_SELECTOR,
};

impl super::CadSession {
    /// Apply one typed `EDGECOLOR <option>` line.
    pub(super) fn edge_color_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(option) = parse_edge_color_argument(argument) else {
            return self.refuse_argument(invalid_edge_color_argument_reason(text));
        };
        let option = match option {
            EdgeColorArgument::Set(color) => EdgeColorCommand::Set(color),
            EdgeColorArgument::Default => EdgeColorCommand::Default,
            EdgeColorArgument::Report => EdgeColorCommand::Report,
        };
        self.dispatch_typed_argument(
            EDGECOLOR_SELECTOR,
            CommandId::Extended(cad_app::ExtendedCommandId::EdgeColor),
            CommandPayload::EdgeColor(option),
        )
    }
}
