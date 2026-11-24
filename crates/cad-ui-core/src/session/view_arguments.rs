//! The typed argument lines of the 2026-11-07 view batch:
//! `VIEW <option>`, `VIEWGO <name>` and `VPOINT <direction>`.
//!
//! `-VIEW` is an alias of `VIEW` and reaches the same handler. These are
//! argument forms of their base selectors, not new IPC: the session parses the
//! line, dispatches the corresponding [`cad_app::CommandPayload`] through
//! [`cad_app::CommandId`] and returns the application's real refusal verbatim.
//! A bare `VIEW`/`VIEWGO`/`VPOINT` keeps the name-only funnel and is refused by
//! the application for its missing argument.

use cad_app::{CommandId, CommandPayload, ExtendedCommandId, VPointInput, ViewCommand};
use cad_domain::Point3;

use super::commands::{
    Point3Input, ViewOption, NEWVIEW_SELECTOR, VIEWGO_SELECTOR, VIEW_SELECTOR, VPOINT_SELECTOR,
};
use super::diagnostics::cad_error_text;
use super::{
    invalid_newview_argument_reason, invalid_view_argument_reason, invalid_viewgo_argument_reason,
    invalid_vpoint_argument_reason, parse_newview_argument, parse_view_argument,
    parse_viewgo_argument, parse_vpoint_argument, CadSession, NewViewArgument,
};

impl CadSession {
    /// Apply one typed `VIEW <option>` line (`-VIEW` is the same handler).
    pub(super) fn view_state_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(option) = parse_view_argument(argument) else {
            return self.refuse_argument(invalid_view_argument_reason(text));
        };
        let payload = match option {
            ViewOption::Save(name) => ViewCommand::Save(name),
            ViewOption::Restore(name) => ViewCommand::Restore(name),
            ViewOption::List => ViewCommand::List,
            ViewOption::Delete(name) => ViewCommand::Delete(name),
        };
        self.dispatch_view(
            VIEW_SELECTOR,
            CommandId::View,
            CommandPayload::View(payload),
        )
    }

    /// Apply one typed `NEWVIEW <name>` / `NEWVIEW WINDOW <name> <x1> <y1> <x2> <y2>` line.
    pub(super) fn newview_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(input) = parse_newview_argument(argument) else {
            return self.refuse_argument(invalid_newview_argument_reason(text));
        };
        let payload = match input {
            NewViewArgument::Current(name) => ViewCommand::Save(name),
            NewViewArgument::Window { name, rect } => ViewCommand::Window {
                name,
                first: Point3 {
                    x: rect[0],
                    y: rect[1],
                    z: 0.0,
                },
                second: Point3 {
                    x: rect[2],
                    y: rect[3],
                    z: 0.0,
                },
            },
        };
        self.dispatch_view(
            NEWVIEW_SELECTOR,
            CommandId::Extended(ExtendedCommandId::NewView),
            CommandPayload::View(payload),
        )
    }

    /// Apply one typed `VIEWGO <name>` line.
    pub(super) fn view_go_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(name) = parse_viewgo_argument(argument) else {
            return self.refuse_argument(invalid_viewgo_argument_reason(text));
        };
        self.dispatch_view(
            VIEWGO_SELECTOR,
            CommandId::ViewGo,
            CommandPayload::ViewGo(name),
        )
    }

    /// Apply one typed `VPOINT <x> <y> <z>` / standard-word line.
    pub(super) fn vpoint_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(input) = parse_vpoint_argument(argument) else {
            return self.refuse_argument(invalid_vpoint_argument_reason(text));
        };
        let input = match input {
            Point3Input::Standard(view) => VPointInput::Standard(view),
            Point3Input::Vector([x, y, z]) => VPointInput::Vector(Point3 { x, y, z }),
        };
        self.dispatch_view(
            VPOINT_SELECTOR,
            CommandId::VPoint,
            CommandPayload::VPoint(input),
        )
    }

    fn dispatch_view(
        &self,
        selector: &'static str,
        id: CommandId,
        payload: CommandPayload,
    ) -> Result<(), String> {
        self.core.record_command(selector);
        match self.core.command(id, payload) {
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
