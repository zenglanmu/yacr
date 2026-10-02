//! Small shell command vocabulary. No guessed CAD editing implementations.
use crate::{i18n::MessageSource, YacrWindow};
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc};

pub(crate) fn connect(ui: &YacrWindow, messages: Rc<RefCell<MessageSource>>) {
    let weak = ui.as_weak();
    ui.on_command_submitted(move |text| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let command = text.trim().to_ascii_uppercase();
        match command.as_str() {
            "OPEN" => ui.invoke_open_requested(),
            "FIT" | "ZOOM EXTENTS" => ui.invoke_fit_requested(),
            "UNDO" if ui.get_can_undo() => ui.invoke_undo_requested(),
            "REDO" if ui.get_can_redo() => ui.invoke_redo_requested(),
            // Real draw/edit tools. The machine key is passed straight through;
            // the adapter resolves it (and refuses a Viewer or a MOVE without a
            // selection) rather than fabricating a payload-free command.
            "LINE" | "L" => ui.invoke_begin_draw_tool("line".into()),
            "CIRCLE" | "C" => ui.invoke_begin_draw_tool("circle".into()),
            "MOVE" | "M" => ui.invoke_begin_draw_tool("move".into()),
            "TRIM" | "TR" => ui.invoke_begin_draw_tool("trim".into()),
            "CONFIRM" => ui.invoke_confirm_draw_requested(),
            "CANCEL" | "ESC" => ui.invoke_cancel_draw_requested(),
            "TOOLS" | "RIBBON" => {
                if ui.get_phone_shell() {
                    ui.set_tools_open(!ui.get_tools_open());
                } else {
                    ui.set_ribbon_expanded(!ui.get_ribbon_expanded());
                }
            }
            "PANELS" => ui.set_side_panel_open(!ui.get_side_panel_open()),
            "DIAGNOSTICS" => {
                ui.set_diagnostics_open(!ui.get_diagnostics_open());
                ui.invoke_diagnostics_requested();
            }
            "" => {}
            _ => {
                // Unknown input is explicit, never an empty successful CAD edit.
                ui.set_status_label(
                    messages
                        .borrow()
                        .text("command.unknown", &[("command", &command)])
                        .into(),
                );
                ui.set_command_expanded(true);
            }
        }
    });
}
