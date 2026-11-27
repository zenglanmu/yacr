//! `tables/display.json`: the per-document display settings (minor 22).
//!
//! One row object (`shadows` + `fog`). The writer emits the table only when the
//! record differs from the default, so an ordinary document keeps its lower
//! minor. The codec is strict: an absent table is the default, more than one row
//! or an unknown field is a malformed document.

use serde_json::Value;

use cad_domain::display::DisplaySettings;

use super::{table_entry, SnapshotError, TableSnapshot};
use crate::drawing::DrawingDatabase;
use crate::scope::TableKind;

fn table() -> &'static str {
    table_entry(TableKind::Display)
}

/// Encode the one row, or nothing for the default record.
pub(crate) fn encode_display(db: &DrawingDatabase) -> Vec<Value> {
    let settings = db.display_settings();
    if settings.is_default() {
        return Vec::new();
    }
    vec![settings.to_json()]
}

/// Decode the record; an absent table is the default.
pub(crate) fn decode_display(tables: &TableSnapshot) -> Result<DisplaySettings, SnapshotError> {
    let rows = tables.get(TableKind::Display).unwrap_or(&[]);
    match rows {
        [] => Ok(DisplaySettings::default()),
        [row] => DisplaySettings::from_json(row)
            .map_err(|error| SnapshotError::invalid(format!("{}: {error}", table()))),
        _ => Err(SnapshotError::invalid(format!(
            "{}: holds {} rows; the display record is one row",
            table(),
            rows.len()
        ))),
    }
}
