#![cfg(target_os = "linux")]
//! Synthetic keyboard contracts against the real Slint shell, not CAD/GPU acceptance.
use cad_ui_slint::{offscreen, YacrWindow};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, SharedString};
use std::{cell::RefCell, rc::Rc};

fn press(ui: &YacrWindow, key: impl Into<SharedString>) {
    let text = key.into();
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn control(ui: &YacrWindow, key: impl Into<SharedString>, shift: bool) {
    ui.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    if shift {
        ui.window().dispatch_event(WindowEvent::KeyPressed {
            text: Key::Shift.into(),
        });
    }
    press(ui, key);
    if shift {
        ui.window().dispatch_event(WindowEvent::KeyReleased {
            text: Key::Shift.into(),
        });
    }
    ui.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
}

fn click(ui: &YacrWindow, x: f32, y: f32) {
    let position = slint::LogicalPosition::new(x, y);
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}

#[test]
fn shortcuts_gate_real_callbacks_and_preserve_text_entry() {
    // One platform-owning thread per binary, as in interaction_gating.rs.
    offscreen::install().unwrap();
    let ui = YacrWindow::new().unwrap();
    ui.window().set_size(slint::LogicalSize::new(1280.0, 800.0));
    ui.set_ribbon_visible(false);
    ui.set_panels_visible(false);
    ui.set_navigation_visible(false);
    ui.set_layouts_visible(false);
    ui.set_status_visible(false);
    ui.set_work_mode(true);
    ui.set_can_undo(true);
    ui.set_can_redo(true);
    let calls = Rc::new(RefCell::new(Vec::new()));
    macro_rules! record {
        ($callback:ident, $name:literal) => {{
            let calls = calls.clone();
            ui.$callback(move || calls.borrow_mut().push($name));
        }};
    }
    record!(on_open_requested, "open");
    record!(on_undo_requested, "undo");
    record!(on_redo_requested, "redo");
    record!(on_fit_requested, "fit");
    record!(on_cancel_open_requested, "cancel-open");
    record!(on_cancel_draw_requested, "cancel-draw");
    record!(on_cancel_measurement_requested, "cancel-measurement");
    record!(on_pan_requested, "pan");
    record!(on_diagnostics_closed, "close-diagnostics");
    record!(on_clear_selection_requested, "clear-selection");
    ui.show().unwrap();
    offscreen::snapshot(ui.window()).unwrap();
    click(&ui, ui.get_cad_left() + 100.0, ui.get_cad_top() + 100.0);

    // Fail closed until effective config is supplied, then gate at event time.
    control(&ui, "o", false);
    press(&ui, Key::F2);
    assert!(calls.borrow().is_empty());
    assert!(!ui.get_command_expanded());
    ui.set_keyboard_shortcuts_enabled(true);
    control(&ui, "o", false);
    control(&ui, "z", false);
    control(&ui, "y", false);
    control(&ui, "z", true);
    control(&ui, Key::Home, false);
    assert_eq!(&*calls.borrow(), &["open", "undo", "redo", "redo", "fit"]);
    calls.borrow_mut().clear();
    ui.set_can_open(false);
    ui.set_can_undo(false);
    ui.set_work_mode(false);
    control(&ui, "o", false);
    control(&ui, "z", false);
    control(&ui, "y", false);
    control(&ui, "s", false); // Ctrl+S is an explicit unsupported shortcut now.
    assert!(calls.borrow().is_empty());
    press(&ui, Key::F2);
    assert!(ui.get_command_expanded());
    press(&ui, Key::F2);
    assert!(!ui.get_command_expanded());
    ui.set_command_visible(false);
    press(&ui, Key::F2);
    assert!(!ui.get_command_expanded());

    ui.set_import_active(true);
    ui.set_draw_tool_active(true);
    ui.set_measurement_active(true);
    ui.set_pan_active(true);
    ui.set_diagnostics_open(true);
    ui.set_selection_count(1);
    press(&ui, Key::Escape);
    assert!(
        calls.borrow().is_empty(),
        "noncancellable import blocks lower priorities"
    );
    ui.set_import_cancellable(true);
    press(&ui, Key::Escape);
    ui.set_import_active(false);
    press(&ui, Key::Escape);
    ui.set_draw_tool_active(false);
    press(&ui, Key::Escape);
    ui.set_measurement_active(false);
    press(&ui, Key::Escape);
    ui.set_pan_active(false);
    press(&ui, Key::Escape);
    press(&ui, Key::Escape);
    assert_eq!(
        &*calls.borrow(),
        &[
            "cancel-open",
            "cancel-draw",
            "cancel-measurement",
            "pan",
            "close-diagnostics",
            "clear-selection",
        ]
    );
    calls.borrow_mut().clear();
    ui.set_selection_count(0);
    ui.set_keyboard_shortcuts_enabled(false);
    control(&ui, Key::Home, false);
    assert!(calls.borrow().is_empty());

    // Real command LineEdit owns printable input, spaces, Enter, local undo
    // and Ctrl+Home. Capture handles only the explicitly global actions.
    ui.set_keyboard_shortcuts_enabled(true);
    ui.set_work_mode(true);
    ui.set_can_undo(true);
    ui.set_command_visible(true);
    let submitted = Rc::new(RefCell::new(Vec::new()));
    let observed = submitted.clone();
    ui.on_command_submitted(move |text| {
        observed.borrow_mut().push(text.to_string());
        true
    });

    // --- Phase 2: fixed AutoCAD-default key map ----------------------------
    // The shell owns the key strings; the host only records the emitted
    // callbacks. Overlay flags live in the config store, so the shell emits the
    // flipped value and the host applies it (asserted end-to-end in
    // shortcut_map_wiring.rs).
    let overlays = Rc::new(RefCell::new(Vec::<(String, bool)>::new()));
    let observed_overlays = overlays.clone();
    ui.on_overlay_toggled(move |key, value| {
        observed_overlays
            .borrow_mut()
            .push((key.to_string(), value));
    });
    let unsupported = Rc::new(RefCell::new(Vec::<String>::new()));
    let observed_unsupported = unsupported.clone();
    let status_messages = cad_ui_slint::MessageSource::for_locale(cad_ui_slint::Locale::En);
    let weak = ui.as_weak();
    ui.on_shortcut_unsupported(move |key| {
        observed_unsupported.borrow_mut().push(key.to_string());
        // Mirror the adapter so the localized catalog text is asserted here too;
        // the real adapter wiring is covered by shortcut_map_wiring.rs.
        if let Some(ui) = weak.upgrade() {
            ui.set_status_label(
                status_messages
                    .text("shortcut.unsupported", &[("key", key.as_str())])
                    .into(),
            );
            ui.set_command_expanded(true);
        }
    });

    // F7 grid and F3 running-snap hints emit the flipped overlay value.
    assert!(ui.get_overlay_grid());
    press(&ui, Key::F7);
    assert_eq!(&*overlays.borrow(), &[("grid".to_string(), false)]);
    assert!(ui.get_overlay_snap_hints());
    press(&ui, Key::F3);
    assert_eq!(
        overlays.borrow().last().unwrap(),
        &("snapHints".to_string(), false)
    );
    overlays.borrow_mut().clear();

    // F2 and Ctrl+9 both expand/collapse the command row.
    assert!(!ui.get_command_expanded());
    press(&ui, Key::F2);
    assert!(ui.get_command_expanded());
    control(&ui, "9", false);
    assert!(!ui.get_command_expanded());

    // Ctrl+1 toggles the properties/side panel.
    let side_before = ui.get_side_panel_open();
    control(&ui, "1", false);
    assert_ne!(ui.get_side_panel_open(), side_before);
    control(&ui, "1", false);
    assert_eq!(ui.get_side_panel_open(), side_before);

    // Ctrl+0 clean screen hides every chrome gate but keeps application-ui.
    assert!(!ui.get_clean_screen());
    assert!(ui.get_chrome_visible());
    control(&ui, "0", false);
    assert!(ui.get_clean_screen());
    assert!(!ui.get_chrome_visible());
    assert!(ui.get_application_ui());
    control(&ui, "0", false);
    assert!(!ui.get_clean_screen());
    assert!(ui.get_chrome_visible());

    // Every unsupported key reports its stable label and never fires a CAD
    // callback; the status text is the localized catalog string.
    let cad_calls_before = calls.borrow().len();
    press(&ui, Key::F1);
    control(&ui, "n", false);
    control(&ui, "s", false);
    control(&ui, "s", true);
    control(&ui, "p", false);
    for key in [
        Key::F4,
        Key::F5,
        Key::F6,
        Key::F8,
        Key::F9,
        Key::F10,
        Key::F11,
        Key::F12,
    ] {
        press(&ui, key);
    }
    control(&ui, "c", false);
    control(&ui, "x", false);
    control(&ui, "v", false);
    assert_eq!(
        &*unsupported.borrow(),
        &[
            "F1",
            "Ctrl+N",
            "Ctrl+S",
            "Ctrl+Shift+S",
            "Ctrl+P",
            "F4",
            "F5",
            "F6",
            "F8",
            "F9",
            "F10",
            "F11",
            "F12",
            "Ctrl+C",
            "Ctrl+X",
            "Ctrl+V",
        ]
    );
    assert_eq!(
        ui.get_status_label().to_string(),
        "Shortcut Ctrl+V has no supported action yet."
    );
    assert!(ui.get_command_expanded());
    assert_eq!(
        calls.borrow().len(),
        cad_calls_before,
        "unsupported keys must not fire CAD callbacks"
    );
    // Restore the collapsed command row so the geometry-based click below still
    // lands on the LineEdit.
    ui.set_command_expanded(false);

    offscreen::snapshot(ui.window()).unwrap();
    click(&ui, 600.0, 775.0);
    for character in "hello world".chars() {
        press(&ui, character.to_string());
    }
    press(&ui, Key::Return);
    assert_eq!(&*submitted.borrow(), &["hello world"]);
    press(&ui, "x");
    control(&ui, "z", false);
    control(&ui, Key::Home, false);
    assert!(
        calls.borrow().is_empty(),
        "text editing must not mutate CAD history or fit"
    );
}
