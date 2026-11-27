//! `MATERIAL <name> ...`: dispatch one material-table edit.

use cad_app::{CommandId, CommandPayload};

use super::commands::{
    invalid_material_argument_reason, parse_material_argument, MATERIAL_SELECTOR,
};

impl super::CadSession {
    /// Apply one typed `MATERIAL` line.
    pub(super) fn material_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(edit) = parse_material_argument(argument) else {
            return self.refuse_argument(invalid_material_argument_reason(text));
        };
        self.dispatch_typed_argument(
            MATERIAL_SELECTOR,
            CommandId::Material,
            CommandPayload::Material(edit),
        )
    }
}
