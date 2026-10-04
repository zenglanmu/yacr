//! Source contracts only; assistive technology and keyboard behavior need runtime acceptance.
const BUTTON: &str = include_str!("../ui/button.slint");

#[test]
fn button_exposes_catalog_label_disabled_state_and_guarded_accessible_action() {
    assert!(BUTTON.contains("accessible-label: root.text"));
    assert!(BUTTON.contains("accessible-enabled: root.enabled"));
    assert!(BUTTON.contains("if root.enabled { root.clicked(); }"));
}

#[test]
fn keyboard_activation_and_visible_focus_do_not_consume_modified_shortcuts() {
    assert!(BUTTON.contains("forward-focus: keyboard"));
    assert!(BUTTON.contains("enabled: root.enabled"));
    assert!(BUTTON
        .contains("!event.modifiers.control && !event.modifiers.alt && !event.modifiers.meta"));
    assert!(BUTTON.contains("event.text == Key.Return"));
    assert!(BUTTON.contains("touch.has-hover || keyboard.has-focus"));
}
