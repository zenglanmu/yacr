//! `tables/sections.json`: the optional document section plane (minor 32).
//!
//! One row when a plane exists, no table when it does not. The codec is strict
//! and bounded to one row; an absent table means "no section plane".

use serde_json::Value;

use cad_domain::section::SectionPlane;

use super::{table_entry, SnapshotError, TableSnapshot};
use crate::drawing::DrawingDatabase;
use crate::scope::TableKind;

fn table() -> &'static str {
    table_entry(TableKind::Sections)
}

/// Encode the one row, or nothing when the document has no plane.
pub(crate) fn encode_section(db: &DrawingDatabase) -> Vec<Value> {
    db.section_plane()
        .map(|plane| plane.to_json())
        .into_iter()
        .collect()
}

/// Decode the optional plane; an absent table is `None`.
pub(crate) fn decode_section(
    tables: &TableSnapshot,
) -> Result<Option<SectionPlane>, SnapshotError> {
    let rows = tables.get(TableKind::Sections).unwrap_or(&[]);
    match rows {
        [] => Ok(None),
        [row] => SectionPlane::from_json(row)
            .map(Some)
            .map_err(|error| SnapshotError::invalid(format!("{}: {error}", table()))),
        _ => Err(SnapshotError::invalid(format!(
            "{}: holds {} rows; the section plane is one row",
            table(),
            rows.len()
        ))),
    }
}
