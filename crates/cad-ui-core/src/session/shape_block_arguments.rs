//! Typed-argument dispatch for `SHAPE`, `MINSERT` and `ADDSELECTED`.
//!
//! Pure parsing lives in [`super::commands::shape_block`]; this module maps one
//! admitted selector to its [`CommandId`] and dispatches the payload through the
//! same `HostCore::command` funnel every other typed form uses. An argument
//! outside a selector's grammar is refused before IPC with a stable reason.
//!
//! `SHAPE`'s omitted size/rotation take the session's current text height and
//! rotation (`ToolParameter::TextHeight` / `ToolParameter::TextRotation`, the
//! same typed channel TEXT creation reads), so the bare `SHAPE 65 10 20` form is
//! a real creation and not an invented default.

use cad_app::{CommandId, CommandPayload, ExtendedCommandId, ShapeCommand, ToolParameter};

use super::commands::{
    parse_addselected_argument, parse_minsert_argument, parse_shape_argument, MINSERT_SELECTOR,
    SHAPE_SELECTOR,
};
use super::diagnostics::cad_error_text;
use super::CadSession;

/// The stable refusal for a `SHAPE` / `MINSERT` / `ADDSELECTED` argument outside
/// the documented grammar.
pub(crate) fn invalid_shape_block_argument_reason(selector: &str, text: &str) -> String {
    let (code, forms) = if selector == SHAPE_SELECTOR {
        (
            "shape.argument",
            "`SHAPE <name-or-code> <x> <y> [size] [rotation_deg]`",
        )
    } else if selector == MINSERT_SELECTOR {
        (
            "minsert.argument",
            "`MINSERT <block> <rows> <columns> <row_spacing> <column_spacing> <x> <y>`",
        )
    } else {
        ("addselected.argument", "`ADDSELECTED <x> <y>`")
    };
    format!(
        "{code}: unsupported or malformed {selector} argument in `{}`: the typed form is {forms}; \
         nothing was changed",
        text.trim()
    )
}

impl CadSession {
    /// Apply one typed `SHAPE …`, `MINSERT …` or `ADDSELECTED …` line.
    ///
    /// The parsed payload rides the existing command funnel
    /// (`CommandId::CreateShape` and the `MINSERT`/`ADDSELECTED` extended ids);
    /// the command layer owns every semantic refusal (finite points, the shape
    /// name/code grammar, the missing font, the array cap and the selected
    /// source type). A malformed argument is refused before IPC and changes
    /// nothing.
    pub(super) fn shape_block_argument(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        let (id, payload) = if selector == SHAPE_SELECTOR {
            let Some(parsed) = parse_shape_argument(argument) else {
                return self.refuse_argument(invalid_shape_block_argument_reason(selector, text));
            };
            // The omitted size/rotation are the session's current text height and
            // rotation (degrees) — one channel with TEXT creation, no constant.
            let (height, rotation_degrees) = {
                let controller = self.core.controller.borrow();
                let session = &controller.session;
                (
                    session
                        .tool_parameter(ToolParameter::TextHeight)
                        .number()
                        .unwrap_or(cad_app::draw_tool::TEXT_DEFAULT_HEIGHT),
                    session
                        .tool_parameter(ToolParameter::TextRotation)
                        .number()
                        .unwrap_or(cad_app::draw_tool::TEXT_DEFAULT_ROTATION_DEGREES),
                )
            };
            (
                CommandId::CreateShape,
                CommandPayload::Shape(Box::new(ShapeCommand {
                    name: parsed.name,
                    code: parsed.code,
                    position: parsed.position,
                    size: parsed.size.unwrap_or(height),
                    rotation: parsed
                        .rotation_degrees
                        .unwrap_or(rotation_degrees)
                        .to_radians(),
                    font: None,
                })),
            )
        } else if selector == MINSERT_SELECTOR {
            let Some(parsed) = parse_minsert_argument(argument) else {
                return self.refuse_argument(invalid_shape_block_argument_reason(selector, text));
            };
            (
                CommandId::Extended(ExtendedCommandId::MInsert),
                CommandPayload::MInsert(Box::new(parsed)),
            )
        } else {
            let Some(parsed) = parse_addselected_argument(argument) else {
                return self.refuse_argument(invalid_shape_block_argument_reason(selector, text));
            };
            (
                CommandId::Extended(ExtendedCommandId::AddSelected),
                CommandPayload::AddSelected(Box::new(parsed)),
            )
        };
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
