// Source contracts only: these do not prove Slint compilation or runtime behavior.
const BAR: &str = include_str!("../ui/command-bar.slint");

#[test]
fn completion_interface_is_host_owned_and_reports_every_input_change() {
    assert!(BAR.contains("in property <[string]> completion-items;"));
    assert!(BAR.contains("in property <string> completion-label;"));
    assert!(BAR.contains("callback input-edited(string);"));
    assert!(BAR.contains("changed input => { root.input-edited(root.input); }"));
    assert!(BAR.contains("root.input = root.recall(-1, root.input);"));
    assert!(BAR.contains("root.input = root.recall(1, root.input);"));
    assert!(BAR.contains("accepted => { root.submit(root.input); root.input = \"\"; }"));
    assert_eq!(BAR.matches("root.submit(").count(), 1);
    assert!(!BAR.contains("Key.Escape"));
}

#[test]
fn tab_completion_is_gated_unmodified_and_never_submits() {
    let tab_capture = BAR
        .split("// Tab only fills a selector;")
        .nth(1)
        .expect("completion keyboard capture")
        .split("Button { text: root.expanded")
        .next()
        .unwrap();
    assert!(tab_capture.contains("!root.keyboard-shortcuts-enabled || event.modifiers.control"));
    assert!(tab_capture.contains("event.modifiers.alt || event.modifiers.meta || event.modifiers.shift"));
    assert!(tab_capture.contains("event.text == Key.Tab && root.completion-items.length > 0"));
    assert!(tab_capture.contains("root.input = root.completion-items[0];"));
    assert!(tab_capture.contains("command-input.focus();"));
    assert!(tab_capture.contains("return EventResult.reject;"));
    assert!(!tab_capture.contains("root.submit("));
    assert!(!tab_capture.contains("Key.UpArrow"));
    assert!(!tab_capture.contains("Key.DownArrow"));
    assert!(!tab_capture.contains("Key.Return"));
}

#[test]
fn suggestions_are_bounded_scrollable_catalog_labeled_and_fill_only() {
    let overlay = BAR
        .split("// Overlay above the bar")
        .nth(1)
        .expect("completion overlay");
    assert!(overlay.contains("if root.completion-items.length > 0 : Rectangle"));
    assert!(overlay.contains("y: -self.height;"));
    assert!(overlay.contains("width: root.width;"));
    assert!(overlay.contains("min(3, root.completion-items.length)"));
    assert!(overlay.contains("ScrollView {"));
    assert!(overlay.contains("for selector in root.completion-items : Button"));
    assert!(overlay.contains("accessible-label: root.completion-label + \" \" + selector;"));
    assert!(overlay.contains("min-height: root.phone ? 48px : 36px;"));
    assert!(overlay.contains("root.input = selector;"));
    assert!(overlay.contains("command-input.focus();"));
    assert!(!overlay.contains("root.submit("));
    assert!(BAR.contains("read-only: true;"));
    assert!(BAR.contains("accessible-action-set-value(value) => { }"));
}
