// Source contracts only: runtime filtering, layout and clipboard are not verified.
const PANEL: &str = include_str!("../ui/panels.slint");

fn annotation_section() -> &'static str {
    PANEL
        .split("if root.section == 2 : VerticalBox {")
        .nth(1)
        .expect("annotation section")
        .split("if root.diagnostics-open : Text")
        .next()
        .expect("annotation section body")
}

#[test]
fn annotation_search_exposes_host_query_count_and_catalog_interface() {
    for declaration in [
        "in-out property <string> annotation-search-text;",
        "in property <string> annotation-search-placeholder;",
        "in property <string> annotation-search-clear-label;",
        "in property <string> annotation-search-empty-label;",
        "in property <int> annotation-matching-count: -1;",
        "callback annotation-search-edited(string);",
        "changed annotation-search-text => { root.annotation-search-edited(root.annotation-search-text); }",
    ] {
        assert!(PANEL.contains(declaration), "missing {declaration}");
    }
    let section = annotation_section();
    assert!(section.contains("text <=> root.annotation-search-text;"));
    assert!(section.contains("placeholder-text: root.annotation-search-placeholder;"));
    assert!(section.contains("accessible-label: root.annotation-search-placeholder;"));
    assert!(section.contains("text: root.annotation-search-clear-label;"));
    assert!(section.contains("clicked => { root.annotation-search-text = \"\"; }"));
    assert!(section.contains("enabled: root.annotations.length > 0 || root.annotation-search-text != \"\";"));
    assert!(section.contains("enabled: root.annotation-search-text != \"\";"));
    assert!(!section.contains("min-height: root.phone ?"));
}

#[test]
fn annotation_search_matches_kind_or_full_text_case_insensitively() {
    let matcher = PANEL
        .split("pure function annotation-matches(kind: string, text: string) -> bool {")
        .nth(1)
        .expect("annotation matcher")
        .split('}')
        .next()
        .expect("matcher body");
    assert!(matcher.contains("root.annotation-search-text == \"\" ||"));
    for field in ["kind", "text"] {
        assert!(matcher.contains(&format!(
            "{field}.to-lowercase().replace-all(root.annotation-search-text.to-lowercase(), \"\") != {field}.to-lowercase()"
        )));
    }
    assert!(matcher.contains("!= kind.to-lowercase() ||"));
    assert!(!matcher.contains("kind +"));
}

#[test]
fn annotation_filter_collapses_rows_and_preserves_original_action_indices() {
    let section = annotation_section();
    assert!(section.contains(
        "VerticalBox {\n                padding: 0px; spacing: 0px;"
    ));
    assert!(section.contains(
        "for row[index] in root.annotations : VerticalBox {\n                    padding: 0px; spacing: 0px;\n                    if root.annotation-matches(row.kind, row.text) : VerticalBox {"
    ));
    for action in [
        "clicked => { root.annotation-select(index); }",
        "toggled => { root.annotation-visible(index, self.checked); }",
        "clicked => { root.annotation-delete(index); }",
    ] {
        assert!(section.contains(action));
    }
    assert!(section.contains("checked: row.visible; enabled: root.work;"));
    assert!(section.contains("min-height: 48px; enabled: root.work;"));
    assert!(section.contains("row.selected ? \" *\" : \"\""));
    assert!(!section.contains("root.annotations ="));
    assert!(!section.contains("root.annotation-select(row.id"));
    assert!(!section.contains("root.annotation-delete(row.id"));
    assert!(!section.contains("root.annotation-visible(row.id"));
}

#[test]
fn annotation_search_distinguishes_empty_database_no_matches_and_unknown_count() {
    let section = annotation_section();
    assert!(section.contains(
        "if root.annotations.length == 0 : Text { text: root.labels[10];"
    ));
    assert!(section.contains(
        "if root.annotations.length > 0 && root.annotation-search-text != \"\" && root.annotation-matching-count == 0 && root.annotation-search-empty-label != \"\" : Text {"
    ));
    assert!(section.contains("text: root.annotation-search-empty-label;"));
    assert!(!section.contains("annotation-matching-count <= 0"));
}

#[test]
fn annotation_text_remains_complete_copyable_and_read_only() {
    let section = annotation_section();
    assert!(section.contains("TextEdit {"));
    assert!(section.contains("text: row.text;"));
    assert!(section.contains("read-only: true; enabled: true; wrap: word-wrap;"));
    assert!(section.contains("accessible-label: row.kind;"));
    assert!(section.contains("accessible-action-set-value(value) => { }"));
    assert!(section.contains("min-height: 48px; preferred-height: root.phone ? 96px : 72px;"));
    assert!(!section.contains("overflow: elide"));
    assert!(!section.contains("text <=> row.text"));
    assert!(!section.contains("self.text ="));
    assert!(!section.contains("row.text ="));
}

#[test]
fn annotation_search_stays_inside_normal_panels_preserving_layer_controls() {
    let normal = PANEL
        .split("if !root.diagnostics-open : VerticalBox {")
        .nth(1)
        .expect("exclusive normal panels")
        .split("if root.diagnostics-open : Text")
        .next()
        .expect("normal panel body");
    assert!(normal.contains("if root.section == 2 : VerticalBox {"));
    assert!(normal.contains("text <=> root.annotation-search-text;"));
    assert!(normal.contains("text <=> root.layer-search-text;"));
    assert!(normal.contains("root.all-layers(true)"));
    assert!(normal.contains("root.all-layers(false)"));
    assert!(normal.contains("for row[index] in root.layers : VerticalBox"));
    assert!(normal.contains("root.layer(index, self.checked)"));
    assert!(PANEL.contains("if root.diagnostics-open : VerticalBox {"));
    assert!(PANEL.contains("for row in root.diagnostics : TextEdit {"));
}
