//! `GUIDE LIST|CLEAR`: dispatch the guide-table management command.

use cad_app::{CommandId, CommandPayload, GuideCommand};

use super::commands::{
    invalid_guide_argument_reason, parse_guide_argument, GuideArgument, GUIDE_SELECTOR,
};

impl super::CadSession {
    /// Apply one typed `GUIDE <option>` line.
    pub(super) fn guide_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(option) = parse_guide_argument(argument) else {
            return self.refuse_argument(invalid_guide_argument_reason(text));
        };
        let option = match option {
            GuideArgument::List => GuideCommand::List,
            GuideArgument::Clear => GuideCommand::Clear,
            GuideArgument::Delete(index) => GuideCommand::Delete(index),
            GuideArgument::Point(point) => GuideCommand::Point(point),
            GuideArgument::StyleColor(color) => GuideCommand::StyleColor(color),
            GuideArgument::StyleDash(dashed) => GuideCommand::StyleDash(dashed),
        };
        self.dispatch_typed_argument(
            GUIDE_SELECTOR,
            CommandId::Guide,
            CommandPayload::Guide(option),
        )
    }
}
