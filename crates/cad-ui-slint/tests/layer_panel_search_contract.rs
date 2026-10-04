// Source contracts only: these do not prove runtime layout or interaction.
const PANEL: &str = include_str!("../ui/panels.slint");

#[test]
fn layer_search_preserves_authoritative_row_indices_and_visibility() {
    assert!(PANEL.contains("for row[index] in root.layers : VerticalBox"));
    assert!(PANEL.contains("if root.layer-matches(row.name) : CheckBox"));
    assert!(PANEL.contains("checked: row.visible"));
    assert!(PANEL.contains("root.layer(index, self.checked)"));
    assert!(!PANEL.contains("root.layer(row.id"));
}

#[test]
fn layer_search_is_case_insensitive_substring_and_clearable() {
    assert!(PANEL.contains("text <=> root.layer-search-text"));
    assert!(PANEL.contains("root.layer-search-text == \"\" ||"));
    assert!(PANEL.contains(
        "name.to-lowercase().replace-all(root.layer-search-text.to-lowercase(), \"\") != name.to-lowercase()"
    ));
    assert!(PANEL.contains("clicked => { root.layer-search-text = \"\"; }"));
    assert!(PANEL.contains("root.layer-search-edited(root.layer-search-text)"));
}

#[test]
fn layer_search_distinguishes_empty_model_no_matches_and_unknown_count() {
    assert!(PANEL.contains("in property <int> layer-matching-count: -1"));
    assert!(PANEL.contains("root.section == 0 && root.layers.length == 0"));
    assert!(PANEL.contains(
        "root.layers.length > 0 && root.layer-search-text != \"\" && root.layer-matching-count == 0"
    ));
    assert!(PANEL.contains("text: root.layer-search-empty-label"));
    assert!(PANEL.contains("placeholder-text: root.layer-search-placeholder"));
    assert!(PANEL.contains("accessible-label: root.layer-search-placeholder"));
}
