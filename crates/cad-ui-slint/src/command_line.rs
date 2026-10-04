//! Typed shell commands route through the same callbacks as visible controls.
//!
//! Only supported command selectors are interpreted here, not coordinates,
//! paths, CAD scripts or guessed editing payloads. Errors retain the original
//! input, and no callback dispatch is presented as proof of command success.
use crate::{
    command_history::{CommandHistory, RecordOutcome},
    i18n::MessageSource,
    YacrWindow,
};
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc};

/// Resolve a typed token to its canonical command name.
///
/// Exact command names are always returned unchanged, so disabling keyboard
/// shortcuts never removes a real command. A keyboard alias (`L`, `C`, `M`,
/// `TR`, `ESC`) only expands when shortcuts are enabled; otherwise the token is
/// returned as-is and the caller reports it as unknown rather than silently
/// starting a drawing edit.
pub(crate) fn canonical_command(command: &str, shortcuts: bool) -> &str {
    match command {
        "L" if shortcuts => "LINE",
        "C" if shortcuts => "CIRCLE",
        "M" if shortcuts => "MOVE",
        "TR" if shortcuts => "TRIM",
        "ESC" if shortcuts => "CANCEL",
        "DI" | "DIST" if shortcuts => "MEASURE DISTANCE",
        "AA" if shortcuts => "MEASURE AREA",
        "ZE" | "Z E" if shortcuts => "ZOOM EXTENTS",
        "ZI" if shortcuts => "ZOOM IN",
        "ZO" if shortcuts => "ZOOM OUT",
        "U" if shortcuts => "UNDO",
        other => other,
    }
}

/// Normalize command selectors, including tabs and Unicode whitespace.
/// This is deliberately not a payload parser: free text and paths must not be
/// forwarded from this uppercased key to a document command.
fn command_key(text: &str) -> String {
    text.split_whitespace()
        .map(str::to_ascii_uppercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// These callbacks take localized labels, not machine identifiers. Resolve
/// through the current catalog on every submission so language changes work.
fn catalog_command(command: &str) -> Option<(&'static str, &'static str)> {
    match command {
        "DISTANCE" | "MEASURE DISTANCE" => Some(("measure", "measure.kind.distance")),
        "ANGLE" | "MEASURE ANGLE" => Some(("measure", "measure.kind.angle")),
        "AREA" | "MEASURE AREA" => Some(("measure", "measure.kind.area")),
        "MEASURE POLYLINE" => Some(("measure", "measure.kind.polyline")),
        "ANNOTATE TEXT" => Some(("annotation", "annotation.kind.text")),
        "ANNOTATE LEADER" => Some(("annotation", "annotation.kind.leader")),
        "ANNOTATE RECTANGLE" => Some(("annotation", "annotation.kind.rectangle")),
        "ANNOTATE ELLIPSE" => Some(("annotation", "annotation.kind.ellipse")),
        "ANNOTATE FREEHAND" => Some(("annotation", "annotation.kind.freehand")),
        "ANNOTATE CLOUD" => Some(("annotation", "annotation.kind.cloud")),
        "VIEW TOP" => Some(("view", "view.standard.top")),
        "VIEW BOTTOM" => Some(("view", "view.standard.bottom")),
        "VIEW FRONT" => Some(("view", "view.standard.front")),
        "VIEW BACK" => Some(("view", "view.standard.back")),
        "VIEW LEFT" => Some(("view", "view.standard.left")),
        "VIEW RIGHT" => Some(("view", "view.standard.right")),
        "VIEW ISOMETRIC" => Some(("view", "view.standard.isometric")),
        _ => None,
    }
}

fn history_feedback_key(work_mode: bool, available: bool) -> Option<&'static str> {
    if !work_mode {
        Some("draw.error.read_only")
    } else if !available {
        Some("ribbon.command_unsupported")
    } else {
        None
    }
}

pub(crate) fn connect(
    ui: &YacrWindow,
    messages: Rc<RefCell<MessageSource>>,
    config: Rc<RefCell<cad_app::viewer_config::ViewerConfigStore>>,
) {
    let history = Rc::new(RefCell::new(CommandHistory::default()));
    let recall_history = history.clone();
    ui.on_command_history_recalled(move |direction, draft| {
        recall_history
            .borrow_mut()
            .recall(direction, draft.as_str())
            .into()
    });
    let clear_history = history.clone();
    let weak = ui.as_weak();
    ui.on_command_history_cleared(move || {
        clear_history.borrow_mut().clear();
        if let Some(ui) = weak.upgrade() {
            ui.set_command_history_entries(crate::chrome::string_model(&[]));
            ui.set_command_history_storage_limited(false);
        }
    });
    let weak = ui.as_weak();
    ui.on_command_submitted(move |text| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        // This archive records submitted input, not command execution outcomes.
        // Its storage budget is independent from the lifetime of the CAD document.
        let outcome = history.borrow_mut().record(text.as_str());
        let oversized = outcome == RecordOutcome::Oversized;
        ui.set_command_history_storage_limited(oversized);
        if oversized {
            ui.set_command_expanded(true);
        }
        let entries = history
            .borrow()
            .entries()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        ui.set_command_history_entries(crate::chrome::string_model(&entries));
        let shortcuts = config.borrow().effective().interaction.keyboard_shortcuts;
        let command = command_key(text.as_str());
        let command = canonical_command(&command, shortcuts);
        if let Some((family, key)) = catalog_command(command) {
            let label = messages.borrow().text(key, &[]).into();
            match family {
                "measure" => ui.invoke_measure_kind_selected(label),
                "annotation" => ui.invoke_annotation_kind_selected(label),
                "view" => ui.invoke_standard_view_selected(label),
                _ => unreachable!("catalog command families are defined locally"),
            }
            return;
        }
        match command {
            "OPEN" => ui.invoke_open_requested(),
            "FIT" | "ZOOM EXTENTS" => ui.invoke_fit_requested(),
            "PAN" => ui.invoke_pan_requested(),
            "ZOOM IN" => ui.invoke_zoom_requested(1.25),
            "ZOOM OUT" => ui.invoke_zoom_requested(0.8),
            "MEASURE" => ui.invoke_measure_requested(),
            "ANNOTATE" => ui.invoke_annotate_requested(),
            "SAVE MEASUREMENT" => ui.invoke_save_measurement_requested(),
            "CLEAR SELECTION" | "DESELECT" => ui.invoke_clear_selection_requested(),
            "PROJECTION" => ui.invoke_toggle_projection_requested(),
            "VIEW MODE" => ui.invoke_toggle_view_mode_requested(),
            "MODE" => ui.invoke_mode_toggled(),
            "EXPORT" => ui.invoke_export_requested(),
            "IMPORT" => ui.invoke_import_requested(),
            "UNDO" | "REDO" => {
                let available = if command == "UNDO" {
                    ui.get_can_undo()
                } else {
                    ui.get_can_redo()
                };
                if let Some(key) = history_feedback_key(ui.get_work_mode(), available) {
                    ui.set_status_label(
                        messages.borrow().text(key, &[("command", command)]).into(),
                    );
                    ui.set_command_expanded(true);
                } else if command == "UNDO" {
                    ui.invoke_undo_requested();
                } else {
                    ui.invoke_redo_requested();
                }
            }
            // Real draw/edit tools. The machine key is passed straight through;
            // the adapter resolves it (and refuses a Viewer or a MOVE without a
            // selection) rather than fabricating a payload-free command.
            "LINE" => ui.invoke_begin_draw_tool("line".into()),
            "CIRCLE" => ui.invoke_begin_draw_tool("circle".into()),
            "MOVE" => ui.invoke_begin_draw_tool("move".into()),
            "TRIM" => ui.invoke_begin_draw_tool("trim".into()),
            // AutoCAD convention: CONFIRM/ENTER and ESC act on whatever command
            // is currently active, not only a draw/edit capture.
            "CONFIRM" | "ENTER" | "" => {
                if ui.get_measurement_active() {
                    ui.invoke_confirm_measurement_requested();
                } else if ui.get_annotation_tool_active() {
                    ui.invoke_confirm_annotation_requested();
                } else if ui.get_draw_tool_active() {
                    ui.invoke_confirm_draw_requested();
                }
            }
            "CANCEL" => {
                if ui.get_measurement_active() {
                    ui.invoke_cancel_measurement_requested();
                }
                if ui.get_annotation_tool_active() {
                    ui.invoke_cancel_annotation_requested();
                }
                if ui.get_draw_tool_active() {
                    ui.invoke_cancel_draw_requested();
                }
                ui.set_pan_active(false);
            }
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
            _ => {
                // Unknown input is explicit, never an empty successful CAD edit.
                ui.set_status_label(
                    messages
                        .borrow()
                        .text("command.unknown", &[("command", text.trim())])
                        .into(),
                );
                ui.set_command_expanded(true);
            }
        }
    });
}

#[cfg(test)]
mod contracts {
    use super::{canonical_command, catalog_command, command_key, history_feedback_key};

    #[test]
    fn selectors_accept_mixed_case_and_repeated_unicode_whitespace() {
        assert_eq!(command_key(" \tzoom\u{2003}  extents\r\n"), "ZOOM EXTENTS");
        assert_eq!(command_key("meAsure\tDistance"), "MEASURE DISTANCE");
        assert_eq!(command_key(" \t\n"), "");
    }

    #[test]
    fn short_aliases_respect_the_shortcuts_setting() {
        for (alias, target) in [
            ("DI", "MEASURE DISTANCE"),
            ("DIST", "MEASURE DISTANCE"),
            ("AA", "MEASURE AREA"),
            ("ZE", "ZOOM EXTENTS"),
            ("Z E", "ZOOM EXTENTS"),
            ("ZI", "ZOOM IN"),
            ("ZO", "ZOOM OUT"),
            ("U", "UNDO"),
        ] {
            assert_eq!(canonical_command(alias, true), target);
            assert_eq!(canonical_command(alias, false), alias);
            assert_eq!(canonical_command(target, false), target);
        }
    }

    #[test]
    fn tool_selectors_use_existing_catalog_labels() {
        assert_eq!(
            catalog_command("MEASURE DISTANCE"),
            Some(("measure", "measure.kind.distance"))
        );
        assert_eq!(
            catalog_command("ANNOTATE CLOUD"),
            Some(("annotation", "annotation.kind.cloud"))
        );
        assert_eq!(
            catalog_command("VIEW ISOMETRIC"),
            Some(("view", "view.standard.isometric"))
        );
    }

    #[test]
    fn unsupported_payloads_are_not_silently_consumed() {
        for input in [
            "MEASURE DISTANCE 0,0 10,10",
            "ANNOTATE TEXT MixedCase content",
            "VIEW TOP extra",
            "POLYLINE",
            "RECTANGLE",
        ] {
            assert_eq!(catalog_command(&command_key(input)), None);
        }
    }

    #[test]
    fn disabled_history_is_unavailable_not_unknown_or_successful() {
        assert_eq!(history_feedback_key(true, true), None);
        assert_eq!(
            history_feedback_key(true, false),
            Some("ribbon.command_unsupported")
        );
        assert_eq!(
            history_feedback_key(false, true),
            Some("draw.error.read_only")
        );
        assert_eq!(
            history_feedback_key(false, false),
            Some("draw.error.read_only")
        );
    }
}
