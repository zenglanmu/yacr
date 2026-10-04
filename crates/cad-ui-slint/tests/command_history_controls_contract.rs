// Source contracts only: these do not prove runtime layout or interaction.
const BAR: &str = include_str!("../ui/command-bar.slint");

#[test]
fn command_history_interface_is_host_owned_and_catalog_labeled() {
    assert!(BAR.contains("in property <[string]> history-entries;"));
    assert!(BAR.contains("in property <string> history-label;"));
    assert!(BAR.contains("in property <string> clear-history-label;"));
    assert!(BAR.contains("callback recall(int, string) -> string;"));
    assert!(BAR.contains("callback clear-history();"));
    assert!(BAR.contains("text: root.history-label;"));
    assert!(BAR.contains("text: root.clear-history-label;"));
}

#[test]
fn command_input_captures_only_gated_unmodified_history_arrows() {
    assert!(BAR.contains("in property <bool> keyboard-shortcuts-enabled: false;"));
    assert!(BAR.contains("forward-focus: command-input;"));
    let capture = BAR
        .split("capture-key-pressed(event) => {")
        .nth(1)
        .expect("command input capture handler")
        .split("command-input := LineEdit")
        .next()
        .unwrap();
    assert!(capture.contains("!root.keyboard-shortcuts-enabled || event.modifiers.control"));
    assert!(
        capture.contains("event.modifiers.alt || event.modifiers.meta || event.modifiers.shift")
    );
    assert!(capture.contains("event.text == Key.UpArrow"));
    assert!(capture.contains("root.input = root.recall(-1, root.input);"));
    assert!(capture.contains("event.text == Key.DownArrow"));
    assert!(capture.contains("root.input = root.recall(1, root.input);"));
    assert_eq!(capture.matches("return EventResult.accept;").count(), 2);
    assert!(capture.contains("return EventResult.reject;"));
    assert!(!capture.contains("Key.Return"));
    assert!(!capture.contains("undo"));
    assert!(BAR.contains("accepted => { root.submit(root.input); root.input = \"\"; }"));
}

#[test]
fn expanded_archive_is_bounded_read_only_and_clearable_when_nonempty() {
    assert!(BAR.contains("if root.expanded : ScrollView"));
    assert!(BAR.contains("height: 104px;"));
    assert!(BAR.contains("for entry in root.history-entries : TextEdit"));
    assert!(BAR.contains("text: entry;"));
    assert!(BAR.contains("read-only: true;"));
    assert!(BAR.contains("if root.history-storage-limited : Text"));
    assert!(BAR.contains("text: root.history-limit-label;"));
    assert!(BAR.contains("accessible-action-set-value(value) => { }"));
    assert!(BAR.contains("wrap: word-wrap;"));
    assert!(BAR.contains("enabled: root.history-entries.length > 0;"));
    assert!(BAR.contains("clicked => { root.clear-history(); }"));
    assert!(BAR.contains(
        "height: root.expanded ? (root.phone ? 304px : 280px) : (root.phone ? 64px : 58px);"
    ));
}
