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
    record!(on_cancel_annotation_requested, "cancel-annotation");
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
    control(&ui, "s", false); // No unsupported save action.
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
    ui.set_annotation_tool_active(true);
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
    ui.set_annotation_tool_active(false);
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
            "cancel-annotation",
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
    ui.on_command_submitted(move |text| observed.borrow_mut().push(text.to_string()));
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
