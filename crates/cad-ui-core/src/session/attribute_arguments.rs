//! Dispatch for the typed block-attribute command lines (ADR-0055 M3).
//!
//! `CadSession::attribute_argument` turns one admitted `SELECTOR <argument>`
//! line into the corresponding [`cad_app::CommandPayload`] and runs it through
//! the one command funnel. The parser owns syntax; the application re-validates
//! every semantic rule (unknown block/tag, constants, locked positions, the
//! selection) and either writes one transaction or refuses without writing.

use cad_app::{AttributeCommand, CommandId, CommandPayload, ExtendedCommandId};

use super::commands::{
    invalid_attribute_argument_reason, parse_attribute_definition, parse_attribute_display,
    parse_attribute_edit, parse_attribute_order, parse_attribute_sync, parse_eattedit,
    ATTDEF_SELECTOR, ATTDISP_SELECTOR, ATTEDIT_SELECTOR, ATTIPEDIT_SELECTOR, ATTREDEF_SELECTOR,
    ATTSYNC_SELECTOR, BATTORDER_SELECTOR, DASH_ATTDEF_SELECTOR, DASH_ATTEDIT_SELECTOR,
    EATTEDIT_SELECTOR, GATTE_SELECTOR,
};
use super::diagnostics::cad_error_text;
use super::CadSession;

impl CadSession {
    /// Run one typed attribute line.
    pub(super) fn attribute_argument(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        let refused = || invalid_attribute_argument_reason(selector, text);
        let (ext, payload) = match selector {
            ATTDEF_SELECTOR => (
                ExtendedCommandId::AttDef,
                parse_attribute_definition(argument)
                    .map(|payload| CommandPayload::Attribute(AttributeCommand::Definition(payload)))
                    .ok_or_else(refused)?,
            ),
            DASH_ATTDEF_SELECTOR => (
                ExtendedCommandId::DashAttDef,
                parse_attribute_definition(argument)
                    .map(|payload| CommandPayload::Attribute(AttributeCommand::Definition(payload)))
                    .ok_or_else(refused)?,
            ),
            ATTREDEF_SELECTOR => (
                ExtendedCommandId::AttRedef,
                parse_attribute_definition(argument)
                    .map(|payload| CommandPayload::Attribute(AttributeCommand::Definition(payload)))
                    .ok_or_else(refused)?,
            ),
            ATTEDIT_SELECTOR => (
                ExtendedCommandId::AttEdit,
                parse_attribute_edit(argument, cad_app::AttributeEditScope::Selection)
                    .map(|payload| CommandPayload::Attribute(AttributeCommand::Edit(payload)))
                    .ok_or_else(refused)?,
            ),
            DASH_ATTEDIT_SELECTOR => (
                ExtendedCommandId::DashAttEdit,
                parse_attribute_edit(argument, cad_app::AttributeEditScope::AllInstances)
                    .map(|payload| CommandPayload::Attribute(AttributeCommand::Edit(payload)))
                    .ok_or_else(refused)?,
            ),
            ATTIPEDIT_SELECTOR => (
                ExtendedCommandId::AttIpEdit,
                parse_attribute_edit(argument, cad_app::AttributeEditScope::Selection)
                    .map(|payload| CommandPayload::Attribute(AttributeCommand::Edit(payload)))
                    .ok_or_else(refused)?,
            ),
            GATTE_SELECTOR => (
                ExtendedCommandId::Gatte,
                parse_attribute_edit(argument, cad_app::AttributeEditScope::AllInstances)
                    .map(|payload| CommandPayload::Attribute(AttributeCommand::Edit(payload)))
                    .ok_or_else(refused)?,
            ),
            EATTEDIT_SELECTOR => (
                ExtendedCommandId::EAttEdit,
                parse_eattedit(argument)
                    .map(|payload| CommandPayload::Attribute(AttributeCommand::Edit(payload)))
                    .ok_or_else(refused)?,
            ),
            ATTSYNC_SELECTOR => (
                ExtendedCommandId::AttSync,
                parse_attribute_sync(argument)
                    .map(|payload| CommandPayload::Attribute(AttributeCommand::Sync(payload)))
                    .ok_or_else(refused)?,
            ),
            BATTORDER_SELECTOR => (
                ExtendedCommandId::BattOrder,
                parse_attribute_order(argument)
                    .map(|payload| CommandPayload::Attribute(AttributeCommand::Order(payload)))
                    .ok_or_else(refused)?,
            ),
            ATTDISP_SELECTOR => (
                ExtendedCommandId::AttDisp,
                parse_attribute_display(argument)
                    .map(|payload| CommandPayload::Attribute(AttributeCommand::Display(payload)))
                    .ok_or_else(refused)?,
            ),
            other => return self.refuse_argument(refused().replace(selector, other)),
        };
        // Echo the admitted selector name, never the typed value.
        self.core.record_command(selector);
        match self.core.command(CommandId::Extended(ext), payload) {
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
