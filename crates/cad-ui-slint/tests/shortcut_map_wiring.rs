#![cfg(target_os = "linux")]
//! Adapter-level runtime proof for the fixed key map: the Slint-owned key
//! strings reach the real host wiring (config-backed overlay toggle and the
//! localized unsupported message). Runs on the software-Vulkan offscreen
//! platform; one test owns the process-wide install.
use cad_domain::CadResult;
use cad_ui_slint::{offscreen, UiAdapter, UiCommandSink, UiConfiguration, YacrWindow};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, SharedString};
use std::cell::Cell;
use std::rc::Rc;

struct Sink;
impl UiCommandSink for Sink {
    fn send(&mut self, _command: cad_app::Command) -> CadResult<()> {
        Ok(())
    }
}

fn press(ui: &YacrWindow, key: impl Into<SharedString>) {
    let text = key.into();
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn control(ui: &YacrWindow, key: impl Into<SharedString>) {
    ui.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    press(ui, key);
    ui.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
}

#[test]
fn fixed_keymap_reaches_adapter_config_and_localized_feedback() {
    offscreen::install().unwrap();
    let adapter = UiAdapter::new(UiConfiguration::default(), Sink, true).unwrap();
    let ui = adapter.component();
    ui.set_keyboard_shortcuts_enabled(true);
    adapter.handle().set_locale("en").unwrap();
    ui.window().set_size(slint::LogicalSize::new(1280.0, 800.0));
    ui.show().unwrap();
    offscreen::snapshot(ui.window()).unwrap();

    // F7 flips the real, config-backed grid overlay (host applies the patch).
    let grid_before = ui.get_overlay_grid();
    press(ui, Key::F7);
    assert_ne!(
        ui.get_overlay_grid(),
        grid_before,
        "F7 must toggle the grid overlay through the config store"
    );

    // Ctrl+9 expands the command row; Ctrl+0 hides the chrome but not the
    // application-ui gate itself.
    assert!(!ui.get_command_expanded());
    control(ui, "9");
    assert!(ui.get_command_expanded());
    assert!(ui.get_chrome_visible());
    control(ui, "0");
    assert!(ui.get_clean_screen());
    assert!(!ui.get_chrome_visible());
    assert!(ui.get_application_ui());

    // Documented limitation: under clean screen the chrome is hidden, so the
    // unsupported message is set on `status_label` but not visible. Ctrl+S must
    // still report (not silently no-op) and must not restore the chrome.
    assert!(!ui.get_chrome_visible());
    ui.set_command_expanded(false);
    control(ui, "s");
    assert!(
        !ui.get_chrome_visible(),
        "ctrl+s must not restore the chrome"
    );
    assert_eq!(
        ui.get_status_label().to_string(),
        "Shortcut Ctrl+S has no supported action yet."
    );
    assert!(ui.get_command_expanded());

    // Ctrl+N is gated by `can-new`: false → explicit unsupported; true → the
    // host request fires (and does not also report unsupported).
    ui.set_can_new(false);
    control(ui, "n");
    assert_eq!(
        ui.get_status_label().to_string(),
        "Shortcut Ctrl+N has no supported action yet."
    );
    let new_calls = Rc::new(Cell::new(0));
    let observed = new_calls.clone();
    ui.on_new_requested(move || observed.set(observed.get() + 1));
    ui.set_status_label("sentinel".into());
    ui.set_can_new(true);
    control(ui, "n");
    assert_eq!(new_calls.get(), 1, "can-new true fires one new request");
    assert_ne!(
        ui.get_status_label().to_string(),
        "Shortcut Ctrl+N has no supported action yet.",
        "a successful Ctrl+N must not also report unsupported"
    );
}
