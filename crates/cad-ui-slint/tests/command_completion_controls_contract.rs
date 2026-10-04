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
fn completion_capture_is_editor_local_gated_and_unmodified() {
    let capture = BAR
        .split("capture-key-pressed(event) => {")
        .nth(1)
        .expect("completion keyboard capture")
        .split("command-input := LineEdit")
        .next()
        .unwrap();
    assert!(capture.contains("!root.keyboard-shortcuts-enabled || event.modifiers.control"));
    assert!(capture.contains("event.modifiers.alt || event.modifiers.meta || event.modifiers.shift"));
    assert!(capture.contains("return EventResult.reject;"));
    assert!(!capture.contains("root.submit("));
    assert_eq!(BAR.matches("capture-key-pressed(event)").count(), 1);
    assert_eq!(BAR.matches("FocusScope {").count(), 1);
    let archive = BAR.split("for entry in root.history-entries : TextEdit").nth(1).unwrap();
    assert!(!archive.contains("capture-key-pressed"));
}

#[test]
fn tab_fills_highlight_or_first_and_enter_consumes_highlight_without_submission() {
    let tab_capture = BAR
        .split("// Tab fills the highlight or first row, never submits.")
        .nth(1)
        .unwrap()
        .split("// Consume the first Enter")
        .next()
        .unwrap();
    assert!(tab_capture.contains("event.text == Key.Tab && root.completion-items.length > 0"));
    assert!(tab_capture.contains("root.input = root.completion-items[max(0, min(root.selected-completion-index, root.completion-items.length - 1))];"));
    assert!(tab_capture.contains("root.selected-completion-index = -1;"));
    assert!(tab_capture.contains("command-input.focus();"));
    assert!(tab_capture.contains("return EventResult.accept;"));
    assert!(!tab_capture.contains("root.submit("));
    let enter_capture = BAR
        .split("if event.text == Key.Return")
        .nth(1)
        .unwrap()
        .split("command-input := LineEdit")
        .next()
        .unwrap();
    assert!(enter_capture.contains("root.selected-completion-index >= 0"));
    assert!(enter_capture.contains("root.selected-completion-index < root.completion-items.length"));
    assert!(enter_capture.contains("root.input = root.completion-items[root.selected-completion-index];"));
    assert!(enter_capture.contains("root.selected-completion-index = -1;"));
    assert!(enter_capture.contains("return EventResult.accept;"));
    assert!(enter_capture.contains("return EventResult.reject;"));
    assert!(!enter_capture.contains("root.submit("));
}

#[test]
fn suggestion_selection_resets_on_model_changes_and_arrows_stay_bounded() {
    assert!(BAR.contains("private property <int> selected-completion-index: -1;"));
    assert!(BAR.contains("changed completion-items => { root.selected-completion-index = -1; }"));
    let arrows = BAR
        .split("if event.text == Key.UpArrow")
        .nth(1)
        .unwrap()
        .split("// Tab fills")
        .next()
        .unwrap();
    assert_eq!(arrows.matches("if root.completion-items.length > 0 {").count(), 2);
    assert!(arrows.contains("? root.completion-items.length - 1"));
    assert!(arrows.contains("max(0, min(root.selected-completion-index - 1, root.completion-items.length - 1))"));
    assert!(arrows.contains("min(root.selected-completion-index + 1, root.completion-items.length - 1)"));
    assert_eq!(arrows.matches("} else {").count(), 2);
    assert_eq!(arrows.matches("root.input =").count(), 2);
    assert!(!arrows.contains("root.input = root.completion-items"));
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
    assert!(overlay.contains("for selector[index] in root.completion-items : Button"));
    assert!(overlay.contains("accessible-label: root.completion-label + \" \" + selector;"));
    assert!(overlay.contains("checked: index == root.selected-completion-index;"));
    assert!(overlay.contains("accessible-checkable: true;"));
    assert!(overlay.contains("accessible-checked: self.checked;"));
    assert!(overlay.contains("min-height: root.phone ? 48px : 36px;"));
    assert!(overlay.contains("root.input = selector;"));
    assert!(overlay.contains("root.selected-completion-index = -1;"));
    assert!(overlay.contains("command-input.focus();"));
    assert!(!overlay.contains("root.submit("));
    assert!(BAR.contains("read-only: true;"));
    assert!(BAR.contains("accessible-action-set-value(value) => { }"));
}
