//! `tables/guides.json` row codec (construction guides, minor 26).
//!
//! One row per guide in creation order:
//!
//! ```json
//! [{"point":{"x":0,"y":0,"z":0},"direction":{"x":1,"y":0,"z":0}}]
//! ```
//!
//! An absent table is an empty guide table. Rows are strict (finite coordinates,
//! normalized non-zero direction, no unknown keys) and the table is capped.

use cad_domain::guide::{GuideTable, GUIDE_CAP};
use serde_json::Value;

use super::{table_entry, SnapshotError, TableSnapshot};
use crate::drawing::DrawingDatabase;
use crate::scope::TableKind;

pub(crate) fn encode_guides(db: &DrawingDatabase) -> Vec<Value> {
    db.guides().to_json()
}

pub(crate) fn decode_guides(tables: &TableSnapshot) -> Result<GuideTable, SnapshotError> {
    let rows = tables.get(TableKind::Guides).unwrap_or(&[]);
    if rows.len() > GUIDE_CAP {
        return Err(SnapshotError::invalid(format!(
            "{}: {} rows exceed the {GUIDE_CAP} budget",
            table_entry(TableKind::Guides),
            rows.len()
        )));
    }
    GuideTable::from_json(rows).map_err(|error| SnapshotError::invalid(error.to_string()))
}
