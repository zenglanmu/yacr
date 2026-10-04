//! Presentation-only search over the authoritative annotation row model.
use crate::YacrWindow;
use slint::{ComponentHandle, Model};

pub(crate) fn refresh(ui: &YacrWindow) {
    let query = ui.get_annotation_search_text().to_lowercase();
    let count = ui
        .get_annotation_rows()
        .iter()
        .filter(|row| matches(&row.kind, &row.text, &query))
        .count();
    ui.set_annotation_matching_count(i32::try_from(count).unwrap_or(i32::MAX));
}

fn matches(kind: &str, text: &str, lowercase_query: &str) -> bool {
    kind.to_lowercase().contains(lowercase_query) || text.to_lowercase().contains(lowercase_query)
}

pub(crate) fn connect(ui: &YacrWindow) {
    let weak = ui.as_weak();
    ui.on_annotation_search_edited(move |_| {
        if let Some(ui) = weak.upgrade() {
            refresh(&ui);
        }
    });
    refresh(ui);
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn search_matches_text_or_type_without_synthetic_cross_field_matches() {
        assert!(matches("Leader", "Roof revision", ""));
        assert!(matches("Leader", "Roof revision", "leader"));
        assert!(matches("Leader", "Roof revision", "roof"));
        assert!(matches("文字", "Étage 北墙", "étage"));
        assert!(matches("文字", "Étage 北墙", "北墙"));
        assert!(!matches("Leader", "Roof revision", "cloud"));
        assert!(!matches("Leader", "Roof revision", "leader roof"));
    }
}
