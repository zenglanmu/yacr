// Source contracts only; runtime selection, clipboard and layout are not verified.
const PANEL: &str = include_str!("../ui/panels.slint");
const APP: &str = include_str!("../ui/app.slint");

fn property_rows() -> &'static str {
    PANEL
        .split("for row in root.properties : VerticalBox {")
        .nth(1)
        .expect("real property rows")
        .split("if root.diagnostics-open : Text")
        .next()
        .expect("property section")
}

#[test]
fn property_values_are_complete_selectable_read_only_and_catalog_labelled() {
    let rows = property_rows();
    assert!(rows.contains("TextEdit {"));
    assert!(rows.contains("text: row.value;"));
    assert!(rows.contains("text: row.key;"));
    assert!(rows.contains("accessible-label: row.key;"));
    assert!(rows.contains("read-only: true; enabled: true; wrap: word-wrap;"));
    assert!(rows.contains("accessible-action-set-value(value) => { }"));
    assert!(!rows.contains("self.text ="));
    assert!(!rows.contains("overflow: elide"));
    assert!(!rows.contains("text <=>"));
    assert!(!rows.contains("edited =>"));
    assert!(rows.contains("min-width: 0px"));
    assert!(rows.contains("preferred-height: root.phone ? 96px : 72px"));
    assert!(PANEL.contains("clicked => { root.clear-selection(); }"));
    assert!(PANEL.contains("enabled: root.properties.length > 0"));
    assert!(PANEL.contains("text: root.labels[8]"));
}

#[test]
fn diagnostics_render_every_real_field_without_title_as_empty_state() {
    let rows = PANEL
        .split("for row in root.diagnostics : TextEdit {")
        .nth(1)
        .expect("real diagnostic rows");
    assert!(rows.contains(
        "text: row.severity + \"\\n\" + row.code + \"\\n\" + row.description + \"\\n\" + row.object + \"\\n\" + row.details;"
    ));
    assert!(rows.contains("read-only: true; enabled: true; wrap: word-wrap;"));
    assert!(rows.contains("accessible-label: row.code;"));
    assert!(rows.contains("accessible-action-set-value(value) =>"));
    assert!(!rows.contains("self.text ="));
    assert!(!rows.contains("overflow: elide"));
    assert!(PANEL.contains("in property <string> diagnostics-empty-label;"));
    let empty = PANEL
        .split("if root.diagnostics-open && root.diagnostics.length == 0 : Text {")
        .nth(1)
        .expect("explicit diagnostic empty state")
        .split('}')
        .next()
        .expect("empty state body");
    assert!(empty.contains("text: root.diagnostics-empty-label;"));
    assert!(!empty.contains("root.labels[12]"));
}

#[test]
fn diagnostics_are_exclusive_without_reindexing_layer_search() {
    let normal = PANEL
        .split("if !root.diagnostics-open : VerticalBox {")
        .nth(1)
        .expect("exclusive normal sections")
        .split("if root.diagnostics-open : Text")
        .next()
        .expect("normal section body");
    for section in 0..=1 {
        assert!(normal.contains(&format!("if root.section == {section}")));
    }
    assert!(normal.contains("for row[index] in root.layers : VerticalBox"));
    assert!(normal.contains("if root.layer-matches(row.name) : CheckBox"));
    assert!(normal.contains("root.layer(index, self.checked)"));
    assert!(normal.contains("text <=> root.layer-search-text"));
    assert!(PANEL.contains("ScrollView {"));
}

#[test]
fn both_panel_hosts_distinguish_diagnostics_title_from_empty_label() {
    assert_eq!(APP.matches("root.diagnostics-drawer-title];").count(), 2);
    assert_eq!(
        APP.matches("diagnostics-empty-label: root.diagnostics-empty-label;")
            .count(),
        2
    );
}
