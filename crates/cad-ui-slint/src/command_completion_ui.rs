//! Slint bridge for pure selector completion. Completion never executes a command.
use crate::{chrome::string_model, command_completion, YacrWindow};
use slint::ComponentHandle;

pub(crate) fn connect(ui: &YacrWindow) {
    let weak = ui.as_weak();
    ui.on_command_input_edited(move |input| {
        if let Some(ui) = weak.upgrade() {
            let items = command_completion::suggestions(input.as_str(), ui.get_work_mode())
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            ui.set_command_completion_items(string_model(&items));
        }
    });
    let weak = ui.as_weak();
    ui.on_command_completion_dismissed(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_command_completion_items(string_model(&[]));
        }
    });
    let weak = ui.as_weak();
    ui.on_command_completion_context_changed(move || {
        // A mode change invalidates the suggestion snapshot immediately; the
        // next editor change recomputes against the actual current mode.
        if let Some(ui) = weak.upgrade() {
            ui.set_command_completion_items(string_model(&[]));
        }
    });
}
