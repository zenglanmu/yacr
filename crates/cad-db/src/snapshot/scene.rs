//! `tables/scenes.json`: the persisted scene rows (`.ymodel` minor 20).
//!
//! The row shape and its strictness live in [`cad_domain::scene`] (a named-view
//! object plus a `style` token); this module only maps the snapshot entry to a
//! [`cad_domain::SceneTable`] and back. An absent table is an empty document
//! table, and the writer emits the entry only when it holds a row.

use serde_json::Value;

use cad_domain::scene::SceneTable;

use super::{table_entry, SnapshotError, TableSnapshot};
use crate::drawing::DrawingDatabase;
use crate::scope::TableKind;

fn table() -> &'static str {
    table_entry(TableKind::Scenes)
}

/// Encode every scene, oldest first.
pub(crate) fn encode_scenes(db: &DrawingDatabase) -> Vec<Value> {
    db.scenes().to_json()
}

/// Decode the scene table; every refusal is a malformed document.
pub(crate) fn decode_scenes(tables: &TableSnapshot) -> Result<SceneTable, SnapshotError> {
    let rows = tables.get(TableKind::Scenes).unwrap_or(&[]);
    SceneTable::from_json(rows)
        .map_err(|error| SnapshotError::invalid(format!("{}: {error}", table())))
}
