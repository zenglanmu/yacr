//! The typed argument lines of the 2026-11-07 utility/named-object batch:
//! `PURGE`, `RENAME`, `SETVAR`, `MREDO` and `MULTIPLE`.
//!
//! These are argument forms of their own canonical selectors, not new commands
//! and not a second IPC. `SETVAR` writes are deliberately **not** a second write
//! path: each writable variable is parsed here and then dispatched to the
//! existing `PDMODE`/`PDSIZE`/`LTSCALE`/`SNAPUNIT`/`GRID`/`SNAP`/`ORTHO`
//! `CommandId`, or to the existing session `set_tool_parameter` API for
//! `TEXTSIZE`. `SETVAR <name>` (no value) dispatches the read-only report.
//!
//! `MULTIPLE <command>` resolves the name against the one command vocabulary
//! before dispatching, so an unknown command is a named refusal rather than a
//! stored phantom.

use cad_app::{CommandId, CommandPayload, ToolParameter, ToolValue};

use super::commands::{
    invalid_mredo_argument_reason, invalid_multiple_argument_reason, invalid_purge_argument_reason,
    invalid_rename_argument_reason, invalid_setvar_value_reason, parse_mredo_argument,
    parse_multiple_argument, parse_purge_argument, parse_rename_argument, parse_setvar_argument,
    parse_sysvar_toggle, MREDO_SELECTOR, MULTIPLE_SELECTOR, PURGE_SELECTOR, RENAME_SELECTOR,
    SETVAR_SELECTOR,
};
use super::CadSession;

/// The closed set of variables `SETVAR` reports or writes.
const MODELLED_VARIABLES: &[&str] = &[
    "PDMODE", "PDSIZE", "SNAPUNIT", "LTSCALE", "GRID", "SNAP", "ORTHO", "TEXTSIZE",
];

impl CadSession {
    /// Route one typed utility-batch line to its command path.
    pub(super) fn utility_argument(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        match selector {
            PURGE_SELECTOR => self.purge_argument(argument, text),
            RENAME_SELECTOR => self.rename_argument(argument, text),
            SETVAR_SELECTOR => self.setvar_argument(argument, text),
            MREDO_SELECTOR => self.mredo_argument(argument, text),
            MULTIPLE_SELECTOR => self.multiple_argument(argument, text),
            _ => self.refuse_argument(format!("unsupported utility selector: {selector}")),
        }
    }

    /// `PURGE <kind>`: dispatch the one named kind (a bare `PURGE` goes through
    /// the name-only path and purges every kind it can).
    fn purge_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(target) = parse_purge_argument(argument) else {
            return self.refuse_argument(invalid_purge_argument_reason(text));
        };
        self.dispatch_typed_argument(
            PURGE_SELECTOR,
            CommandId::Purge,
            CommandPayload::Purge(target),
        )
    }

    /// `RENAME <kind> <old> <new>`.
    fn rename_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(rename) = parse_rename_argument(argument) else {
            return self.refuse_argument(invalid_rename_argument_reason(text));
        };
        self.dispatch_typed_argument(
            RENAME_SELECTOR,
            CommandId::Rename,
            CommandPayload::Rename(rename),
        )
    }

    /// `MREDO <n>`.
    fn mredo_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(count) = parse_mredo_argument(argument) else {
            return self.refuse_argument(invalid_mredo_argument_reason(text));
        };
        self.dispatch_typed_argument(
            MREDO_SELECTOR,
            CommandId::MRedo,
            CommandPayload::MRedo(count),
        )
    }

    /// `MULTIPLE <command>`: resolve the target against the command vocabulary.
    fn multiple_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(name) = parse_multiple_argument(argument) else {
            return self.refuse_argument(invalid_multiple_argument_reason(text));
        };
        let Some(canonical) = super::canonical_command_name(&name) else {
            return self.refuse_argument(format!(
                "multiple.unknown_command: '{name}' maps to no admitted CAD command"
            ));
        };
        self.dispatch_typed_argument(
            MULTIPLE_SELECTOR,
            CommandId::Multiple,
            CommandPayload::Multiple(Some(canonical.to_string())),
        )
    }

    /// `SETVAR <name>` (report) or `SETVAR <name> <value>` (write through the
    /// existing variable handler).
    fn setvar_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some((name, value)) = parse_setvar_argument(argument) else {
            return self.refuse_argument(format!(
                "setvar.needs_name: '{text}' needs `<name>` or `<name> <value>`"
            ));
        };
        let name = name.to_ascii_uppercase();
        if !MODELLED_VARIABLES.contains(&name.as_str()) {
            return self.refuse_argument(format!(
                "setvar.unknown_variable: SETVAR does not model '{name}'; the closed set is {}",
                MODELLED_VARIABLES.join(", ")
            ));
        }
        match value {
            None => self.dispatch_typed_argument(
                SETVAR_SELECTOR,
                CommandId::SetVar,
                CommandPayload::SetVar(name),
            ),
            Some(value) => self.setvar_write(&name, &value),
        }
    }

    /// Write one modelled variable through the **existing** handler.
    fn setvar_write(&self, name: &str, value: &str) -> Result<(), String> {
        let invalid = |expected: &str| {
            self.refuse_argument(invalid_setvar_value_reason(name, value, expected))
        };
        match name {
            "PDMODE" => {
                let Some(mode) = super::parse_pdmode_argument(value) else {
                    return invalid("one integer that fits i16");
                };
                self.dispatch_typed_argument(
                    SETVAR_SELECTOR,
                    CommandId::SetPointDisplayMode,
                    CommandPayload::PointDisplayMode(mode),
                )
            }
            "PDSIZE" => {
                let Some(size) = super::parse_pdsize_argument(value) else {
                    return invalid("one signed finite decimal");
                };
                self.dispatch_typed_argument(
                    SETVAR_SELECTOR,
                    CommandId::SetPointDisplaySize,
                    CommandPayload::PointDisplaySize(size),
                )
            }
            "SNAPUNIT" => {
                let Some(spacing) = super::parse_snap_unit_argument(value) else {
                    return invalid("one positive decimal spacing");
                };
                self.dispatch_typed_argument(
                    SETVAR_SELECTOR,
                    CommandId::SetSnapSpacing,
                    CommandPayload::SnapSpacing(spacing),
                )
            }
            "LTSCALE" => {
                let Some(scale) = super::parse_ltscale_argument(value) else {
                    return invalid("one positive finite decimal");
                };
                self.dispatch_typed_argument(
                    SETVAR_SELECTOR,
                    CommandId::SetLinetypeScale,
                    CommandPayload::LinetypeScale(scale),
                )
            }
            // The three switches only have toggle commands. `SETVAR` needs
            // set-to-value semantics, so compare the current session state and
            // dispatch the ordinary toggle only when the requested value differs;
            // an already-matching value is reported, never flipped away.
            "GRID" | "SNAP" | "ORTHO" => {
                let Some(desired) = parse_sysvar_toggle(value) else {
                    return invalid("ON, OFF, 1 or 0");
                };
                let controller = self.core.controller.borrow();
                let current = match name {
                    "GRID" => controller.grid_enabled(),
                    "SNAP" => controller.snap_grid_enabled(),
                    _ => controller.ortho_enabled(),
                };
                drop(controller);
                if current == desired {
                    return self.dispatch_typed_argument(
                        SETVAR_SELECTOR,
                        CommandId::SetVar,
                        CommandPayload::SetVar(name.to_string()),
                    );
                }
                let id = match name {
                    "GRID" => CommandId::ToggleGrid,
                    "SNAP" => CommandId::ToggleSnapGrid,
                    _ => CommandId::ToggleOrtho,
                };
                self.dispatch_typed_argument(SETVAR_SELECTOR, id, CommandPayload::None)
            }
            "TEXTSIZE" => {
                let Some(height) = super::parse_snap_unit_argument(value) else {
                    return invalid("one positive finite text height");
                };
                self.core.record_command(SETVAR_SELECTOR);
                match self.set_tool_parameter(ToolParameter::TextHeight, ToolValue::Number(height))
                {
                    Ok(()) => {
                        *self.error.borrow_mut() = None;
                        Ok(())
                    }
                    Err(text) => {
                        *self.error.borrow_mut() = Some(text.clone());
                        Err(text)
                    }
                }
            }
            _ => self.refuse_argument(format!("setvar.unknown_variable: '{name}'")),
        }
    }
}
