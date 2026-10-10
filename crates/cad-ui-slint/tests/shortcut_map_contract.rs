//! Source contracts for the fixed AutoCAD-default key map.
//!
//! These inspect the Slint chrome and the adapter wiring directly; they prove
//! neither Slint compilation nor runtime behavior. The runtime behavior lives in
//! `keyboard_shortcuts.rs` (shell dispatch) and `shortcut_map_wiring.rs`
//! (adapter + config propagation).
const APP: &str = include_str!("../ui/app.slint");
const ADAPTER: &str = include_str!("../src/adapter.rs");

// Both declarations are indented 8 spaces inside `shortcut-scope := FocusScope`.
// The leading indentation makes the bubble header unique: the capture header
// contains the bubble text only as a `capture-` suffixed substring, so a plain
// `"key-pressed(event) => {"` would first match inside the capture handler.
const CAPTURE_HEADER: &str = "        capture-key-pressed(event) => {";
const BUBBLE_HEADER: &str = "        key-pressed(event) => {";
const BUBBLE_END: &str = "\n    VerticalBox {";

/// Slice one handler body out of the Slint source. Panics if the header is
/// absent so a renamed/moved handler fails loudly instead of matching nothing.
fn handler_body<'a>(source: &'a str, header: &str, end: Option<&str>) -> &'a str {
    assert_eq!(
        source.matches(header).count(),
        1,
        "handler header must appear exactly once: {header:?}"
    );
    let body = source
        .split(header)
        .nth(1)
        .unwrap_or_else(|| panic!("missing handler header: {header:?}"));
    match end {
        Some(end) => body.split(end).next().unwrap(),
        None => body,
    }
}

#[test]
fn the_fixed_key_map_declares_its_callback_and_clean_screen_gate() {
    for clause in [
        "callback shortcut-unsupported(string);",
        "in-out property <bool> clean-screen: false;",
        "out property <bool> chrome-visible: root.application-ui && !root.clean-screen;",
        // New blank drawing: host capability + request callback.
        "in property <bool> can-new: false;",
        "callback new-requested();",
        // Vector plot/export: host capability + request callback.
        "in property <bool> can-plot: false;",
        "callback plot-requested();",
        // Lossy Save As: host capability + request callback.
        "in property <bool> can-save: false;",
        "callback save-requested();",
    ] {
        assert!(APP.contains(clause), "key map is missing: {clause}");
    }
}

#[test]
fn capture_handler_binds_only_the_global_modifier_and_function_keys() {
    let capture = handler_body(APP, CAPTURE_HEADER, Some(BUBBLE_HEADER));
    for clause in [
        // Gating + modifier hygiene.
        "if !root.application-ui || !root.keyboard-shortcuts-enabled {",
        "if event.modifiers.alt || event.modifiers.meta { return EventResult.reject; }",
        // Ctrl+O gated by can-open, and the view/UI toggles.
        "(event.text == \"o\" || event.text == \"O\") && root.can-open",
        "if !event.modifiers.shift && event.text == \"1\" {",
        "root.side-panel-open = !root.side-panel-open;",
        "if !event.modifiers.shift && event.text == \"9\" && root.command-visible {",
        "root.command-expanded = !root.command-expanded;",
        "if !event.modifiers.shift && event.text == \"0\" {",
        "root.clean-screen = !root.clean-screen;",
        "if event.text == Key.F7 {",
        "root.overlay-toggled(\"grid\", !root.overlay-grid);",
        "if event.text == Key.F3 {",
        "root.overlay-toggled(\"snapHints\", !root.overlay-snap-hints);",
        "if event.text == Key.F2 && root.command-visible {",
        // Unsupported Ctrl combos.
        "if !event.modifiers.shift && (event.text == \"n\" || event.text == \"N\") {",
        "if root.can-new {",
        "root.new-requested();",
        "if event.modifiers.shift && (event.text == \"s\" || event.text == \"S\") {",
        "if !event.modifiers.shift && (event.text == \"s\" || event.text == \"S\") {",
        "if root.can-save {",
        "root.save-requested();",
        "if !event.modifiers.shift && (event.text == \"p\" || event.text == \"P\") {",
        "if root.can-plot {",
        "root.plot-requested();",
        "root.shortcut-unsupported(\"Ctrl+N\")",
        "root.shortcut-unsupported(\"Ctrl+S\")",
        "root.shortcut-unsupported(\"Ctrl+Shift+S\")",
        "root.shortcut-unsupported(\"Ctrl+P\")",
        // Unsupported function keys (labels + dispatch lines).
        "if event.text == Key.F1 {",
        "if event.text == Key.F4 {",
        "if event.text == Key.F5 {",
        "if event.text == Key.F6 {",
        "if event.text == Key.F8 {",
        "if event.text == Key.F9 {",
        "if event.text == Key.F10 {",
        "if event.text == Key.F11 {",
        "if event.text == Key.F12 {",
        "root.shortcut-unsupported(\"F1\")",
        "root.shortcut-unsupported(\"F4\")",
        "root.shortcut-unsupported(\"F5\")",
        "root.shortcut-unsupported(\"F6\")",
        "root.shortcut-unsupported(\"F8\")",
        "root.shortcut-unsupported(\"F9\")",
        "root.shortcut-unsupported(\"F10\")",
        "root.shortcut-unsupported(\"F11\")",
        "root.shortcut-unsupported(\"F12\")",
    ] {
        assert!(
            capture.contains(clause),
            "capture handler is missing: {clause}"
        );
    }
    // Editor-first-refusal / clipboard actions must live in the bubbling handler.
    for absent in [
        "Key.Home",
        "root.select-all-requested()",
        "root.fit-requested()",
        "root.shortcut-unsupported(\"Ctrl+C\")",
        "root.shortcut-unsupported(\"Ctrl+X\")",
        "root.shortcut-unsupported(\"Ctrl+V\")",
        "event.text == \"a\"",
        "event.text == \"z\"",
        "event.text == \"y\"",
    ] {
        assert!(
            !capture.contains(absent),
            "capture handler must not bind the bubbling action: {absent}"
        );
    }
}

#[test]
fn bubble_handler_keeps_editor_first_refusal_and_preserved_shortcuts() {
    let bubble = handler_body(APP, BUBBLE_HEADER, Some(BUBBLE_END));
    for clause in [
        "if !root.application-ui || !root.keyboard-shortcuts-enabled",
        "event.modifiers.alt || event.modifiers.meta",
        // Ctrl+A.
        "if !event.modifiers.shift && (event.text == \"a\" || event.text == \"A\") {",
        "root.select-all-requested();",
        // Ctrl+C/X/V keep editor first refusal, then report explicitly.
        "if !event.modifiers.shift && (event.text == \"c\" || event.text == \"C\") {",
        "if !event.modifiers.shift && (event.text == \"x\" || event.text == \"X\") {",
        "if !event.modifiers.shift && (event.text == \"v\" || event.text == \"V\") {",
        "root.shortcut-unsupported(\"Ctrl+C\")",
        "root.shortcut-unsupported(\"Ctrl+X\")",
        "root.shortcut-unsupported(\"Ctrl+V\")",
        // Ctrl+Z / Ctrl+Shift+Z.
        "if (event.text == \"z\" || event.text == \"Z\") && root.work-mode {",
        "if event.modifiers.shift && root.can-redo {",
        "root.redo-requested(); return EventResult.accept;",
        "if !event.modifiers.shift && root.can-undo {",
        "root.undo-requested(); return EventResult.accept;",
        // Ctrl+Y.
        "if !event.modifiers.shift && (event.text == \"y\" || event.text == \"Y\")",
        // Ctrl+Home.
        "if !event.modifiers.shift && event.text == Key.Home {",
        "root.fit-requested(); return EventResult.accept;",
    ] {
        assert!(
            bubble.contains(clause),
            "bubble handler is missing: {clause}"
        );
    }
    // The capture-only keymap must not leak into the bubbling handler.
    for absent in [
        "Key.F1",
        "Key.F2",
        "Key.F3",
        "Key.F7",
        "root.clean-screen",
        "root.side-panel-open = !root.side-panel-open;",
        "root.shortcut-unsupported(\"Ctrl+N\")",
        "root.shortcut-unsupported(\"Ctrl+S\")",
        "root.shortcut-unsupported(\"Ctrl+P\")",
    ] {
        assert!(
            !bubble.contains(absent),
            "bubble handler must not bind the capture-only action: {absent}"
        );
    }
}

#[test]
fn clean_screen_gates_the_chrome_without_touching_application_ui() {
    // Every layout section moved to the new gate; the two key handlers keep the
    // raw application-ui check.
    assert_eq!(APP.matches("if root.chrome-visible").count(), 19);
    assert_eq!(APP.matches("if root.application-ui").count(), 0);
    assert_eq!(APP.matches("if !root.application-ui").count(), 2);
}

#[test]
fn the_adapter_reports_unsupported_shortcuts_from_the_catalog() {
    assert!(ADAPTER.contains("on_shortcut_unsupported"));
    assert!(ADAPTER.contains(".text(\"shortcut.unsupported\", &[(\"key\", key.as_str())])"));
    assert!(ADAPTER.contains("ui.set_command_expanded(true);"));
    // New blank drawing is host-owned: the shell request maps to the command.
    assert!(ADAPTER.contains("on_new_requested"));
    assert!(ADAPTER.contains("CommandId::NewDrawing"));
    // Vector plot/export is host-owned: the shell request maps to the command.
    assert!(ADAPTER.contains("on_plot_requested"));
    assert!(ADAPTER.contains("CommandId::PlotDrawing"));
    // Lossy Save As is host-owned: the shell request maps to the command.
    assert!(ADAPTER.contains("on_save_requested"));
    assert!(ADAPTER.contains("CommandId::SaveDrawingAs"));
}
