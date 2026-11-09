//! The typed option lines of the interactive modify commands:
//! `JOIN`, `OFFSET`, `MIRROR`, `CHAMFER`, `FILLET`.
//!
//! [`CadSession::modify_argument`] is the one dispatch arm; the grammars live in
//! [`super::commands`]. Every form is an argument of its canonical selector, not
//! a new command and not a second IPC:
//!
//! * `JOIN MULTIPLE|CLOSE` dispatches [`cad_app::CommandPayload::Join`]; a bare
//!   `JOIN` keeps `CommandPayload::None` (source-object, no close).
//! * `OFFSET …` stores the session options read by the next capture
//!   ([`cad_app::session_state::ModifyOptions`]); `OFFSET UNDO` runs the ordinary
//!   `UNDO` command. `OFFSET MULTIPLE` is a **named refusal**: the fixed-step
//!   capture takes exactly one side point, so a multiple-offset loop is only
//!   reachable through an `OffsetOptions` payload with two or more side points.
//! * `MIRROR TEXT 0|1` / `MIRROR ERASE YES|NO` store the same session options.
//!   A capture that runs with a non-default set emits `MirrorOptions`; the bare
//!   `MIRROR` capture keeps readable text and no erase.
//! * `CHAMFER D|A|T|N|M` and `FILLET R|T|N` route to the **existing** typed tool
//!   parameters (`ChamferDistanceA`/`B`, `FilletRadius`) and the stored
//!   `CornerModes` setters (`TRIMMODE`, `CHAMMODE`, the chamfer angle). The
//!   `POLYLINE` forms dispatch the real `ChamferPolyline`/`FilletPolyline`
//!   commands.

use cad_app::{ChamferMethod, CommandId, CommandPayload, ToolParameter, ToolValue};

use super::commands::{
    invalid_chamfer_argument_reason, invalid_fillet_argument_reason, invalid_join_argument_reason,
    invalid_mirror_argument_reason, invalid_offset_argument_reason,
    invalid_pedit_edit_argument_reason, parse_chamfer_argument, parse_fillet_argument,
    parse_join_argument, parse_mirror_argument, parse_offset_argument, parse_pedit_argument,
    parse_pedit_edit_argument, ChamferArgument, FilletArgument, MirrorArgument, OffsetArgument,
    CHAMFER_SELECTOR, FILLET_SELECTOR, JOIN_SELECTOR, MIRROR_SELECTOR, OFFSET_SELECTOR,
};
use super::diagnostics::cad_error_text;
use super::{CadSession, PEDIT_SELECTOR};

impl CadSession {
    /// Apply one typed `PEDIT <option>` line.
    ///
    /// The legacy three (`CLOSE|C`, `OPEN|O`, `REVERSE|R`) keep
    /// `CommandPayload::PeditOption`; the full line and the `PEDITACCEPT` prefix
    /// travel as `CommandPayload::PeditEdit`. A bare `PEDIT` never reaches here
    /// (the application refuses it with `pedit.needs_option`).
    pub(super) fn pedit_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let payload = if let Some(option) = parse_pedit_argument(argument) {
            CommandPayload::PeditOption(option)
        } else if let Some(command) = parse_pedit_edit_argument(argument) {
            CommandPayload::PeditEdit(command)
        } else {
            return self.refuse_argument(invalid_pedit_edit_argument_reason(text));
        };
        self.core.record_command(PEDIT_SELECTOR);
        match self.core.command(CommandId::Pedit, payload) {
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

    /// Route one typed interactive-modify option line.
    pub(super) fn modify_argument(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        match selector {
            JOIN_SELECTOR => self.join_argument(argument, text),
            OFFSET_SELECTOR => self.offset_argument(argument, text),
            MIRROR_SELECTOR => self.mirror_argument(argument, text),
            CHAMFER_SELECTOR => self.chamfer_argument(argument, text),
            FILLET_SELECTOR => self.fillet_argument(argument, text),
            _ => self.refuse_argument(format!("unsupported modify selector: {selector}")),
        }
    }

    fn join_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(options) = parse_join_argument(argument) else {
            return self.refuse_argument(invalid_join_argument_reason(text));
        };
        self.dispatch_typed_argument(
            JOIN_SELECTOR,
            CommandId::Join,
            CommandPayload::Join(options),
        )
    }

    fn offset_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(parsed) = parse_offset_argument(argument) else {
            return self.refuse_argument(invalid_offset_argument_reason(text));
        };
        match parsed {
            OffsetArgument::Undo => {
                self.dispatch_typed_argument(OFFSET_SELECTOR, CommandId::Undo, CommandPayload::None)
            }
            OffsetArgument::Multiple => self.refuse_argument(format!(
                "offset.multiple_not_interactive: `{}` requests the multiple-offset loop, but the \
                 fixed-step OFFSET capture takes exactly one side point; use an OffsetOptions \
                 payload with two or more side points instead. Nothing was changed",
                text.trim()
            )),
            OffsetArgument::Distance(distance) => {
                self.core.record_command(OFFSET_SELECTOR);
                self.core
                    .controller
                    .borrow_mut()
                    .session
                    .modify_options
                    .set_offset_distance(Some(distance));
                self.accept_session_mode()
            }
            OffsetArgument::Through => {
                self.core.record_command(OFFSET_SELECTOR);
                self.core
                    .controller
                    .borrow_mut()
                    .session
                    .modify_options
                    .set_offset_distance(None);
                self.accept_session_mode()
            }
            OffsetArgument::ToggleErase => {
                self.core.record_command(OFFSET_SELECTOR);
                let mut controller = self.core.controller.borrow_mut();
                let options = &mut controller.session.modify_options;
                let erase = !options.offset_erase();
                options.set_offset_erase(erase);
                drop(controller);
                self.accept_session_mode()
            }
            OffsetArgument::Layer { current } => {
                self.core.record_command(OFFSET_SELECTOR);
                self.core
                    .controller
                    .borrow_mut()
                    .session
                    .modify_options
                    .set_offset_current_layer(current);
                self.accept_session_mode()
            }
        }
    }

    fn mirror_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(parsed) = parse_mirror_argument(argument) else {
            return self.refuse_argument(invalid_mirror_argument_reason(text));
        };
        self.core.record_command(MIRROR_SELECTOR);
        let mut controller = self.core.controller.borrow_mut();
        let options = &mut controller.session.modify_options;
        match parsed {
            MirrorArgument::MirrorText(value) => options.set_mirrortext(value),
            MirrorArgument::Erase(value) => options.set_mirror_erase(value),
        }
        drop(controller);
        self.accept_session_mode()
    }

    fn chamfer_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(parsed) = parse_chamfer_argument(argument) else {
            return self.refuse_argument(invalid_chamfer_argument_reason(text));
        };
        match parsed {
            ChamferArgument::Polyline => self.dispatch_typed_argument(
                CHAMFER_SELECTOR,
                CommandId::ChamferPolyline,
                CommandPayload::None,
            ),
            ChamferArgument::Trim(trim) => {
                self.core.record_command(CHAMFER_SELECTOR);
                self.core
                    .controller
                    .borrow_mut()
                    .session
                    .drafting_extras
                    .corner_modes_mut()
                    .set_trim(trim);
                self.accept_session_mode()
            }
            ChamferArgument::Method(method) => {
                self.core.record_command(CHAMFER_SELECTOR);
                self.core
                    .controller
                    .borrow_mut()
                    .session
                    .drafting_extras
                    .corner_modes_mut()
                    .set_chamfer_method(method);
                self.accept_session_mode()
            }
            ChamferArgument::Distances { first, second } => {
                let second = second.unwrap_or(first);
                if let Err(error) = validate_number(ToolParameter::ChamferDistanceA, first) {
                    return self.refuse_argument(error);
                }
                if let Err(error) = validate_number(ToolParameter::ChamferDistanceB, second) {
                    return self.refuse_argument(error);
                }
                self.core.record_command(CHAMFER_SELECTOR);
                self.set_parameter(ToolParameter::ChamferDistanceA, first)?;
                self.set_parameter(ToolParameter::ChamferDistanceB, second)?;
                Ok(())
            }
            ChamferArgument::Angle { distance, degrees } => {
                if let Err(error) = validate_number(ToolParameter::ChamferDistanceA, distance) {
                    return self.refuse_argument(error);
                }
                // Build the whole candidate mode first so a refused angle changes
                // neither the angle nor the method (no half-applied state).
                let mut modes = self
                    .core
                    .controller
                    .borrow()
                    .session
                    .drafting_extras
                    .corner_modes();
                modes.set_chamfer_method(ChamferMethod::Angle);
                if let Err(error) = modes.set_chamfer_angle_degrees(degrees) {
                    return self.refuse_argument(describe(&error));
                }
                self.core.record_command(CHAMFER_SELECTOR);
                self.set_parameter(ToolParameter::ChamferDistanceA, distance)?;
                *self
                    .core
                    .controller
                    .borrow_mut()
                    .session
                    .drafting_extras
                    .corner_modes_mut() = modes;
                self.accept_session_mode()
            }
        }
    }

    fn fillet_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(parsed) = parse_fillet_argument(argument) else {
            return self.refuse_argument(invalid_fillet_argument_reason(text));
        };
        match parsed {
            FilletArgument::Polyline => self.dispatch_typed_argument(
                FILLET_SELECTOR,
                CommandId::FilletPolyline,
                CommandPayload::None,
            ),
            FilletArgument::Trim(trim) => {
                self.core.record_command(FILLET_SELECTOR);
                self.core
                    .controller
                    .borrow_mut()
                    .session
                    .drafting_extras
                    .corner_modes_mut()
                    .set_trim(trim);
                self.accept_session_mode()
            }
            FilletArgument::Radius(radius) => {
                if let Err(error) = validate_number(ToolParameter::FilletRadius, radius) {
                    return self.refuse_argument(error);
                }
                self.core.record_command(FILLET_SELECTOR);
                self.set_parameter(ToolParameter::FilletRadius, radius)?;
                Ok(())
            }
        }
    }

    /// Dispatch one **validated** numeric tool parameter and record its error.
    fn set_parameter(&self, parameter: ToolParameter, value: f64) -> Result<(), String> {
        match self.set_tool_parameter(parameter, ToolValue::Number(value)) {
            Ok(()) => {
                *self.error.borrow_mut() = None;
                Ok(())
            }
            Err(text) => self.refuse_argument(text),
        }
    }

    /// Clear the error for a session-mode change that has no command payload.
    fn accept_session_mode(&self) -> Result<(), String> {
        *self.error.borrow_mut() = None;
        Ok(())
    }
}

/// Validate a numeric tool parameter through the parameter's own range/kind rule.
fn validate_number(parameter: ToolParameter, value: f64) -> Result<(), String> {
    parameter
        .validate(ToolValue::Number(value))
        .map_err(|error| describe(&error))
}

fn describe(error: &cad_domain::CadError) -> String {
    super::diagnostics::cad_error_text(error)
}
