//! `tables/resources.json`: the logical name ⇄ archive-key mapping
//! (ADR-0040 D4).
//!
//! The file is a **plain JSON array of rows**, exactly like every other table,
//! one row per bundled external resource:
//!
//! ```json
//! [ { "key": "9f3a…c1.ttf", "kind": "font",  "name": "arial.ttf",
//!     "source": "C:/proj/fonts/arial.ttf" },
//!   { "key": "1b07…e4.dwg", "kind": "xref",  "name": "STRUCT",
//!     "source": null } ]
//! ```
//!
//! * `key` is the `resources/<kind>/<key>` entry name
//!   ([`cad_domain::is_valid_bundled_key`]'s form), never a path;
//! * `name` is the **logical reference** the document uses, so `(kind, name)` is
//!   unique and a second key claiming one name is refused;
//! * `source` is the collection-time path: diagnostics only, never used for
//!   resolution, `null` when no path was recorded (a reader also accepts the key
//!   being absent).
//!
//! The bytes are **not** here. They are `resources/<kind>/<key>` binary entries
//! and their integrity belongs to `manifest.json` alone, so the mapping and the
//! payload cannot disagree about a hash (D4).
//!
//! Kept beside `tables.rs` rather than inside it for the 1000-line file gate,
//! the same reason the element codecs live in their own modules: `tables.rs`
//! dispatches, this module owns one row shape.

use std::collections::BTreeMap;

use cad_domain::BundledKind;
use serde_json::{Map, Value};

use super::{table_entry, SnapshotError, TableSnapshot};
use crate::drawing::DrawingDatabase;
use crate::scope::TableKind;

/// The two maps one resource row feeds: the logical name and the (diagnostic)
/// collection-time source.
pub(crate) type ResourceTables = (
    BTreeMap<(BundledKind, String), String>,
    BTreeMap<(BundledKind, String), Option<String>>,
);

/// Encode the mapping rows for `db`, in `(kind, key)` order.
pub(crate) fn encode(db: &DrawingDatabase) -> Result<Vec<Value>, SnapshotError> {
    // A payload with no logical name could not be referenced by any row, and
    // `decode`/`from_snapshot` refuse such an orphan on the way back in: refuse
    // it here too instead of writing bytes the reader will reject.
    if let Some((kind, key)) = db
        .resources
        .iter()
        .find(|(id, _)| !db.resource_names.contains_key(*id))
        .map(|(id, _)| id)
    {
        return Err(SnapshotError::invalid(format!(
            "resources/{kind}/{key}: a bundled resource has no logical name"
        )));
    }
    Ok(db
        .resource_names
        .iter()
        .map(|((kind, key), name)| {
            let source = db
                .resource_sources
                .get(&(*kind, key.clone()))
                .cloned()
                .flatten();
            let mut row = Map::new();
            row.insert("key".to_string(), Value::from(key.as_str()));
            row.insert("kind".to_string(), Value::from(kind.as_str()));
            row.insert("name".to_string(), Value::from(name.as_str()));
            // One shape per row: no recorded path is `null`, not an omitted key.
            // A reader accepts both (D4).
            row.insert(
                "source".to_string(),
                source.map_or(Value::Null, Value::from),
            );
            Value::Object(row)
        })
        .collect())
}

/// Decode the mapping rows. An absent table is an empty mapping.
pub(crate) fn decode(tables: &TableSnapshot) -> Result<ResourceTables, SnapshotError> {
    let mut names = BTreeMap::new();
    let mut sources = BTreeMap::new();
    let mut by_name: BTreeMap<(BundledKind, String), String> = BTreeMap::new();
    for row in tables.get(TableKind::Resources).unwrap_or(&[]) {
        let (kind, key, name, source) = decode_row(row).map_err(|detail| row_error(row, detail))?;
        // `(kind, name)` is the uniqueness key (D4): a second key for one name
        // would make the name resolve to two payloads.
        if let Some(other) = by_name.insert((kind, name.clone()), key.clone()) {
            return Err(row_error(
                row,
                format!(
                    "{}: {kind} {name:?} is bound to both {other:?} and {key:?}",
                    cad_domain::ymodel::markers::RESOURCE_NAME_CONFLICT
                ),
            ));
        }
        if names.insert((kind, key.clone()), name).is_some() {
            return Err(row_error(row, format!("duplicate resource key {key:?}")));
        }
        sources.insert((kind, key), source);
    }
    Ok((names, sources))
}

fn decode_row(row: &Value) -> Result<(BundledKind, String, String, Option<String>), String> {
    let mut reader = Reader::new(row)?;
    let key = reader.string("key")?;
    let token = reader.string("kind")?;
    let kind = BundledKind::parse(&token).ok_or_else(|| {
        format!("kind: unknown resource kind {token:?} (expected font, xref or image)")
    })?;
    // The key is an archive entry name, so the reader enforces the same form the
    // writer did (ADR-0040 D1/D12: refuse, never guess a media type).
    if !cad_domain::is_valid_bundled_key(kind, &key) {
        return Err(format!(
            "key: {kind} {key:?} is not a valid bundled-resource key"
        ));
    }
    let name = reader.string("name")?;
    if name.trim().is_empty() {
        return Err(format!("name: {kind} {key:?} has an empty logical name"));
    }
    let source = reader.opt_string_if_present("source")?;
    reader.finish()?;
    Ok((kind, key, name, source))
}

/// Name the table (and the row's key when it carries one) in a refusal.
fn row_error(row: &Value, detail: impl std::fmt::Display) -> SnapshotError {
    let entry = table_entry(TableKind::Resources);
    match row.get("key").and_then(Value::as_str) {
        Some(key) => SnapshotError::invalid(format!("{entry} row {key:?}: {detail}")),
        None => SnapshotError::invalid(format!("{entry}: {detail}")),
    }
}

/// A strict reader for one row object: every field is taken by name, a required
/// one that is absent is an error, and [`Reader::finish`] refuses leftovers.
///
/// That is the crate's rule for table rows: this writer re-serializes whole
/// rows, so a field it does not model would be dropped by the next save.
struct Reader {
    map: Map<String, Value>,
}

impl Reader {
    fn new(value: &Value) -> Result<Self, String> {
        match value {
            Value::Object(map) => Ok(Reader { map: map.clone() }),
            _ => Err("resource: expected a JSON object".to_string()),
        }
    }

    fn take(&mut self, field: &str) -> Result<Value, String> {
        self.map
            .remove(field)
            .ok_or_else(|| format!("resource: required field `{field}` is missing"))
    }

    fn string(&mut self, field: &str) -> Result<String, String> {
        let value = self.take(field)?;
        value
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| format!("{field}: expected a string"))
    }

    /// `source` may be a string, `null`, or **absent** (all mean "no recorded
    /// path"). This is the only optional field in this table: do not spread it.
    fn opt_string_if_present(&mut self, field: &str) -> Result<Option<String>, String> {
        if !self.map.contains_key(field) {
            return Ok(None);
        }
        match self.take(field)? {
            Value::Null => Ok(None),
            value => value
                .as_str()
                .map(|text| Some(text.to_string()))
                .ok_or_else(|| format!("{field}: expected a string or null")),
        }
    }

    fn finish(self) -> Result<(), String> {
        match self.map.keys().next() {
            Some(key) => Err(format!("resource: unknown field `{key}`")),
            None => Ok(()),
        }
    }
}
