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
                // pending("ui.command.editing"): future command parser/tool wiring.
                // Unsupported input is explicit, never an empty successful CAD edit.
                ui.set_status_label(
                    messages
                        .borrow()
                        .text("command.pending", &[("command", &command)])
                        .into(),
                );
                ui.set_command_expanded(true);
            }
        }
    });
}
