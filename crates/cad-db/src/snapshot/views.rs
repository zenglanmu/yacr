//! `tables/views.json`: the persisted named-view rows (`.ymodel` minor 18).
//!
//! The row shape and its strictness live in [`cad_domain::view`]; this module
//! only maps the [`TableSnapshot`] entry to a [`cad_domain::NamedViewTable`] and
//! back. An absent table is an empty document table (a document written before
//! minor 18), and the writer emits the entry only when the table holds a row.

use serde_json::Value;

use cad_domain::NamedViewTable;

use super::{table_entry, SnapshotError, TableSnapshot};
use crate::drawing::DrawingDatabase;
use crate::scope::TableKind;

/// The table entry this codec owns.
fn table() -> &'static str {
    table_entry(TableKind::Views)
}

/// Encode every named view, oldest first.
pub(crate) fn encode_views(db: &DrawingDatabase) -> Vec<Value> {
    db.named_views().to_json()
}

/// Decode the named-view table.
///
/// Every refusal (duplicate name, over-cap table, unknown field, invalid camera)
/// is a malformed document; a row is never silently skipped.
pub(crate) fn decode_views(tables: &TableSnapshot) -> Result<NamedViewTable, SnapshotError> {
    let rows = tables.get(TableKind::Views).unwrap_or(&[]);
    NamedViewTable::from_json(rows)
        .map_err(|error| SnapshotError::invalid(format!("{}: {error}", table())))
}
