//! Typed shell commands route through the same callbacks as visible controls.
//!
//! Only supported command selectors are interpreted here, not coordinates,
//! paths, CAD scripts or guessed editing payloads. Errors retain the original
//! input, and no callback dispatch is presented as proof of command success.
use crate::{
    command_completion::{is_keyboard_alias, resolve_exact, resolve_prefix, PrefixResolution},
    command_history::{CommandHistory, RecordOutcome},
    i18n::MessageSource,
    YacrWindow,
};
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc};

/// Canonical AutoCAD command names that the dispatcher recognizes but cannot
/// execute yet. Submitting one is reported through `command.unsupported`, never
/// as a silent no-op or a fake success.
///
/// Remove an entry (and add its executor/acceptance test) as a later phase
/// lands the feature; never route one of these to an existing selector.
const UNSUPPORTED_COMMANDS: &[&str] = &[
    "ARC",
    "ARRAY",
    "BLOCK",
    "BOUNDARY",
    "BREAK",
    "CHAMFER",
    "COPY",
    "DIM",
    "DIMALIGNED",
    "DIMANGULAR",
    "DIMDIAMETER",
    "DIMLINEAR",
    "DIMRADIUS",
    "DIMSTYLE",
    "DIVIDE",
    "ELLIPSE",
    "ERASE",
    "EXPLODE",
    "EXTEND",
    "FILLET",
    "GRADIENT",
    "HATCH",
    "IMPORT",
    "INSERT",
    "JOIN",
    "LAYER",
    "LEADER",
    "LENGTHEN",
    "MEASUREGEOM",
    "MIRROR",
    "MTEXT",
    "OFFSET",
    "PLOT",
    "PLINE",
    "POINT",
    "POLYGON",
    "POLYLINE",
    "PRINT",
    "PUBLISH",
    "PURGE",
    "QSAVE",
    "RECTANGLE",
    "REGION",
    "ROTATE",
    "SAVE",
    "SAVEAS",
    "SCALE",
    "SPLINE",
    "STRETCH",
    "STYLE",
    "TEXT",
    "WBLOCK",
    "WIPEOUT",
    "XLINE",
    "XREF",
];

/// How a normalized submission key resolves against the command vocabulary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Submission {
    /// Empty input retries the last command that actually executed.
    RepeatLast,
    /// A single canonical selector, exact or unique-prefix.
    Dispatch(&'static str),
    /// Several canonical selectors match; never guess, keep the line open.
    Ambiguous,
    /// A known AutoCAD command without an executor.
    Unsupported,
    /// Nothing in the vocabulary matches.
    Unknown,
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

fn classify_submission(key: &str, shortcuts: bool) -> Submission {
    if key.is_empty() {
        return Submission::RepeatLast;
    }
    if let Some(canonical) = resolve_exact(key, shortcuts) {
        return Submission::Dispatch(canonical);
    }
    // A disabled keyboard alias must stay unknown: never let canonical prefix
    // matching silently reroute it to a different command.
    if !shortcuts && is_keyboard_alias(key) {
        return Submission::Unknown;
    }
    match resolve_prefix(key, shortcuts) {
        PrefixResolution::Unique(canonical) => Submission::Dispatch(canonical),
        PrefixResolution::Multiple => Submission::Ambiguous,
        PrefixResolution::None => {
            if is_unsupported_command(key) {
                Submission::Unsupported
            } else {
                Submission::Unknown
            }
        }
    }
}

/// These callbacks take localized labels, not machine identifiers. Resolve
/// through the current catalog on every submission so language changes work.
fn catalog_command(command: &str) -> Option<(&'static str, &'static str)> {
    match command {
        "DISTANCE" | "MEASURE DISTANCE" => Some(("measure", "measure.kind.distance")),
        "ANGLE" | "MEASURE ANGLE" => Some(("measure", "measure.kind.angle")),
        "AREA" | "MEASURE AREA" => Some(("measure", "measure.kind.area")),
        "MEASURE POLYLINE" => Some(("measure", "measure.kind.polyline")),
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

fn show_unknown(ui: &YacrWindow, messages: &Rc<RefCell<MessageSource>>, raw: &str) {
    ui.set_status_label(
        messages
            .borrow()
            .text("command.unknown", &[("command", raw.trim())])
            .into(),
    );
    ui.set_command_expanded(true);
}

/// Dispatch a resolved canonical selector. Returns whether an executor consumed
/// it; a name without an executor must never be reported as success.
fn dispatch_command(
    ui: &YacrWindow,
    messages: &Rc<RefCell<MessageSource>>,
    canonical: &str,
) -> bool {
    if let Some((family, key)) = catalog_command(canonical) {
        let label = messages.borrow().text(key, &[]).into();
        match family {
            "measure" => ui.invoke_measure_kind_selected(label),
            "view" => ui.invoke_standard_view_selected(label),
            _ => unreachable!("catalog command families are defined locally"),
        }
        return true;
    }
    match canonical {
        "OPEN" => {
            ui.invoke_open_requested();
            true
        }
        "NEW" => {
            // Host-owned like OPEN. A host that has not opted in reports an
            // explicit unsupported, never a silent no-op.
            if ui.get_can_new() {
                ui.invoke_new_requested();
            } else {
                ui.set_status_label(
                    messages
                        .borrow()
                        .text("command.unsupported", &[("command", canonical)])
                        .into(),
                );
                ui.set_command_expanded(true);
            }
            true
        }
        "FIT" | "ZOOM EXTENTS" => {
            ui.invoke_fit_requested();
            true
        }
        "PAN" => {
            ui.invoke_pan_requested();
            true
        }
        "ZOOM IN" => {
            ui.invoke_zoom_requested(1.25);
            true
        }
        "ZOOM OUT" => {
            ui.invoke_zoom_requested(0.8);
            true
        }
        "MEASURE" => {
            ui.invoke_measure_requested();
            true
        }
        "CLEAR SELECTION" | "DESELECT" => {
            ui.invoke_clear_selection_requested();
            true
        }
        "SELECTALL" => {
            ui.invoke_select_all_requested();
            true
        }
        "PROJECTION" => {
            ui.invoke_toggle_projection_requested();
            true
        }
        "VIEW MODE" => {
            ui.invoke_toggle_view_mode_requested();
            true
        }
        "MODE" => {
            ui.invoke_mode_toggled();
            true
        }
        "UNDO" | "REDO" => {
            let available = if canonical == "UNDO" {
                ui.get_can_undo()
            } else {
                ui.get_can_redo()
            };
            if let Some(key) = history_feedback_key(ui.get_work_mode(), available) {
                ui.set_status_label(
                    messages
                        .borrow()
                        .text(key, &[("command", canonical)])
                        .into(),
                );
                ui.set_command_expanded(true);
            } else if canonical == "UNDO" {
                ui.invoke_undo_requested();
            } else {
                ui.invoke_redo_requested();
            }
            true
        }
        // Real draw/edit tools. The machine key is passed straight through;
        // the adapter resolves it (and refuses a Viewer or a MOVE without a
        // selection) rather than fabricating a payload-free command.
        "LINE" => {
            ui.invoke_begin_draw_tool("line".into());
            true
        }
        "CIRCLE" => {
            ui.invoke_begin_draw_tool("circle".into());
            true
        }
        "MOVE" => {
            ui.invoke_begin_draw_tool("move".into());
            true
        }
        "TRIM" => {
            ui.invoke_begin_draw_tool("trim".into());
            true
        }
        // AutoCAD convention: CONFIRM/ENTER acts on whatever command is
        // currently active, not only a draw/edit capture.
        "CONFIRM" | "ENTER" => {
            if ui.get_measurement_active() {
                ui.invoke_confirm_measurement_requested();
            } else if ui.get_draw_tool_active() {
                ui.invoke_confirm_draw_requested();
            }
            true
        }
        "CANCEL" => {
            if ui.get_measurement_active() {
                ui.invoke_cancel_measurement_requested();
            }
            if ui.get_draw_tool_active() {
                ui.invoke_cancel_draw_requested();
            }
            ui.set_pan_active(false);
            true
        }
        "TOOLS" | "RIBBON" => {
            if ui.get_phone_shell() {
                ui.set_tools_open(!ui.get_tools_open());
            } else {
                ui.set_ribbon_expanded(!ui.get_ribbon_expanded());
            }
            true
        }
        "PANELS" => {
            ui.set_side_panel_open(!ui.get_side_panel_open());
            true
        }
        "DIAGNOSTICS" => {
            ui.set_diagnostics_open(!ui.get_diagnostics_open());
            ui.invoke_diagnostics_requested();
            true
        }
        _ => false,
    }
}

/// Whether the normalized key names a known-but-unimplemented AutoCAD command.
/// A trailing payload is tolerated so `ERASE 0,0` still reads as `ERASE`.
fn is_unsupported_command(key: &str) -> bool {
    let head = key.split(' ').next().unwrap_or(key);
    UNSUPPORTED_COMMANDS.contains(&head)
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
    // The last command that actually dispatched, so an empty submit can repeat
    // it. Only canonical selectors are stored, and only after an executor ran.
    let last_command: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let weak = ui.as_weak();
    ui.on_command_submitted(move |text| -> bool {
        let Some(ui) = weak.upgrade() else {
            return false;
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
        let key = command_key(text.as_str());
        match classify_submission(&key, shortcuts) {
            Submission::RepeatLast => {
                if let Some(last) = last_command.borrow().as_deref() {
                    dispatch_command(&ui, &messages, last);
                }
                // An empty Enter is consumed even when nothing has run yet.
                true
            }
            Submission::Dispatch(canonical) => {
                if dispatch_command(&ui, &messages, canonical) {
                    // CONFIRM/ENTER/CANCEL act on whatever is active; they are
                    // permanent no-ops when nothing is, so they must never become
                    // the empty-Enter replay target.
                    if !matches!(canonical, "CONFIRM" | "ENTER" | "CANCEL") {
                        *last_command.borrow_mut() = Some(canonical.to_owned());
                    }
                } else {
                    // A resolved selector without an executor is a bug; report it
                    // explicitly instead of pretending a command ran.
                    show_unknown(&ui, &messages, text.as_str());
                }
                true
            }
            Submission::Ambiguous => {
                // Keep the line open and refresh the disambiguation snapshot; the
                // Slint side resets its highlight when the model changes.
                let items = crate::command_completion::suggestions(&key, ui.get_work_mode())
                    .into_iter()
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                if items.is_empty() {
                    // Ambiguity the current mode cannot offer is explicit, never a
                    // silent swallow with an empty popup.
                    if !ui.get_work_mode() {
                        ui.set_status_label(
                            messages.borrow().text("draw.error.read_only", &[]).into(),
                        );
                        ui.set_command_expanded(true);
                    } else {
                        show_unknown(&ui, &messages, text.as_str());
                    }
                    true
                } else {
                    ui.set_command_completion_items(crate::chrome::string_model(&items));
                    false
                }
            }
            Submission::Unsupported => {
                // Use the input as typed (trimmed) so the placeholder preserves
                // case and is not folded with the payload.
                ui.set_status_label(
                    messages
                        .borrow()
                        .text("command.unsupported", &[("command", text.trim())])
                        .into(),
                );
                ui.set_command_expanded(true);
                true
            }
            Submission::Unknown => {
                show_unknown(&ui, &messages, text.as_str());
                true
            }
        }
    });
}

#[cfg(test)]
mod contracts {
    use super::{
        catalog_command, classify_submission, command_key, history_feedback_key,
        is_unsupported_command, Submission, UNSUPPORTED_COMMANDS,
    };
    use crate::command_completion::{resolve_exact, resolve_prefix, PrefixResolution};

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
            ("L", "LINE"),
            ("C", "CIRCLE"),
            ("M", "MOVE"),
            ("TR", "TRIM"),
            ("P", "PAN"),
            ("U", "UNDO"),
            ("ESC", "CANCEL"),
        ] {
            assert_eq!(resolve_exact(alias, true), Some(target));
            assert_eq!(resolve_exact(alias, false), None);
            assert_eq!(resolve_exact(target, false), Some(target));
        }
        // Full-word synonyms never depend on the shortcuts gate.
        for (synonym, target) in [
            ("FIT", "ZOOM EXTENTS"),
            ("DISTANCE", "MEASURE DISTANCE"),
            ("ANGLE", "MEASURE ANGLE"),
            ("AREA", "MEASURE AREA"),
            ("DESELECT", "CLEAR SELECTION"),
            ("ENTER", "CONFIRM"),
            ("RIBBON", "TOOLS"),
        ] {
            assert_eq!(resolve_exact(synonym, true), Some(target));
            assert_eq!(resolve_exact(synonym, false), Some(target));
        }
    }

    #[test]
    fn submission_classifies_empty_exact_prefix_ambiguous_unsupported_and_unknown() {
        assert_eq!(classify_submission("", true), Submission::RepeatLast);
        assert_eq!(
            classify_submission("LIN", true),
            Submission::Dispatch("LINE")
        );
        assert_eq!(
            classify_submission("LINE", true),
            Submission::Dispatch("LINE")
        );
        assert_eq!(classify_submission("ZOOM", true), Submission::Ambiguous);
        assert_eq!(
            classify_submission("ZO", true),
            Submission::Dispatch("ZOOM OUT")
        );
        // Disabled shortcuts leave a short alias unknown, never a CAD edit.
        assert_eq!(classify_submission("L", false), Submission::Unknown);
        assert_eq!(classify_submission("DI", false), Submission::Unknown);
        // A full-word synonym still resolves when shortcuts are disabled.
        assert_eq!(
            classify_submission("FIT", false),
            Submission::Dispatch("ZOOM EXTENTS")
        );
        assert_eq!(classify_submission("HATCH", true), Submission::Unsupported);
        assert_eq!(
            classify_submission("SAVE plan.dwg", true),
            Submission::Unsupported
        );
        assert_eq!(classify_submission("NOPE", true), Submission::Unknown);
    }

    #[test]
    fn tool_selectors_use_existing_catalog_labels() {
        assert_eq!(
            catalog_command("MEASURE DISTANCE"),
            Some(("measure", "measure.kind.distance"))
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
            "VIEW TOP extra",
            "POLYLINE",
            "RECTANGLE",
        ] {
            assert_eq!(catalog_command(&command_key(input)), None);
        }
    }

    #[test]
    fn unsupported_vocabulary_stays_disjoint_from_supported_commands() {
        for &name in UNSUPPORTED_COMMANDS {
            assert!(
                resolve_exact(name, true).is_none(),
                "{name} must not resolve to a supported command"
            );
            // An unsupported head must not be a prefix of any supported
            // selector/synonym/alias, or a later phase could silently execute a
            // longer word while `{name}` stays listed unsupported.
            assert_eq!(
                resolve_prefix(name, true),
                PrefixResolution::None,
                "{name} prefixes a supported command"
            );
            assert_eq!(
                resolve_prefix(name, false),
                PrefixResolution::None,
                "{name} prefixes a supported command"
            );
        }
        for expected in ["HATCH", "OFFSET", "ARRAY", "ERASE", "ARC", "SAVE"] {
            assert!(
                UNSUPPORTED_COMMANDS.contains(&expected),
                "missing unsupported command {expected}"
            );
        }
        assert!(is_unsupported_command("ERASE 0,0"));
        assert!(!is_unsupported_command("LINE"));
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
