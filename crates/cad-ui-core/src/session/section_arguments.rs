//! `SECTIONPLANE ...`: dispatch the persisted section-plane command.

use cad_app::{CommandId, CommandPayload, SectionCommand};

use super::commands::{
    invalid_section_argument_reason, parse_section_argument, SectionArgument, SECTIONPLANE_SELECTOR,
};

impl super::CadSession {
    /// Apply one typed `SECTIONPLANE <option>` line.
    pub(super) fn section_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(option) = parse_section_argument(argument) else {
            return self.refuse_argument(invalid_section_argument_reason(text));
        };
        let option = match option {
            SectionArgument::Set { origin, normal } => SectionCommand::Set { origin, normal },
            SectionArgument::On => SectionCommand::On,
            SectionArgument::Off => SectionCommand::Off,
            SectionArgument::Toggle => SectionCommand::Toggle,
            SectionArgument::Clear => SectionCommand::Clear,
            SectionArgument::List => SectionCommand::List,
        };
        self.dispatch_typed_argument(
            SECTIONPLANE_SELECTOR,
            CommandId::SectionPlane,
            CommandPayload::Section(option),
        )
    }
}
