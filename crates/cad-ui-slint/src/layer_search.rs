//! Presentation-only layer search; the authoritative row order stays untouched.
use crate::YacrWindow;
use slint::{ComponentHandle, Model};

pub(crate) fn refresh(ui: &YacrWindow) {
    let query = ui.get_layer_search_text().to_lowercase();
    let count = ui
        .get_layer_rows()
        .iter()
        .filter(|row| matches(&row.name, &query))
        .count();
    ui.set_layer_matching_count(i32::try_from(count).unwrap_or(i32::MAX));
}

fn matches(name: &str, lowercase_query: &str) -> bool {
    name.to_lowercase().contains(lowercase_query)
}

pub(crate) fn connect(ui: &YacrWindow) {
    let weak = ui.as_weak();
    ui.on_layer_search_edited(move |_| {
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
    fn search_accepts_empty_case_insensitive_and_unicode_substrings() {
        assert!(matches("Walls", ""));
        assert!(matches("Walls", "wall"));
        assert!(matches("Étage 墙", "étage"));
        assert!(matches("Étage 墙", "墙"));
        assert!(!matches("Walls", "roof"));
    }
}
