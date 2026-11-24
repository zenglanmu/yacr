//! `tables/groups.json` row codec (persisted entity groups).
//!
//! One row per group, in sequence order:
//!
//! ```json
//! [{"seq":1,"name":"Walls","description":"exterior","selectable":true,
//!   "members":[2,3]}]
//! ```
//!
//! `description` is omitted when empty (never written as a null placeholder);
//! `selectable` and `members` are always present. Members are persistent entity
//! identifiers in capture order; the decoder refuses a duplicate member and a
//! non-entity kind, and the builder `finish()` gate refuses a dangling one.
//!
//! Strict in both directions: every required key is present, an extra key is
//! refused, a wrong JSON type is refused, and names are unique
//! case-insensitively (the table's one user-visible resolver rule).

use cad_domain::ymodel::id::{IdKind, PersistentId, PersistentIdOf};
use cad_domain::EntityId;
use serde_json::Value;

use super::bim::object;
use super::elements::Reader;
use super::{table_entry, SnapshotError, TableSnapshot};
use crate::drawing::{DrawingDatabase, GroupRow, GroupSeq};
use crate::scope::TableKind;

fn table() -> &'static str {
    table_entry(TableKind::Groups)
}

fn invalid(detail: impl std::fmt::Display) -> SnapshotError {
    SnapshotError::invalid(format!("group.invalid_value: {detail}"))
}

/// Encode every group row, in sequence order.
pub(crate) fn encode_groups(db: &DrawingDatabase) -> Result<Vec<Value>, SnapshotError> {
    Ok(db.groups().map(encode_row).collect())
}

/// Decode the group table. An absent table is an empty group table.
pub(crate) fn decode_groups(tables: &TableSnapshot) -> Result<Vec<GroupRow>, SnapshotError> {
    let rows = tables.get(TableKind::Groups).unwrap_or(&[]);
    let mut decoded = Vec::with_capacity(rows.len());
    let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (index, row) in rows.iter().enumerate() {
        let context = format!("{} row {index}", table());
        let row = decode_row(row, context.clone())?;
        if !names.insert(row.name.trim().to_ascii_lowercase()) {
            return Err(invalid(format!(
                "{context}: duplicate group name {:?}",
                row.name
            )));
        }
        decoded.push(row);
    }
    Ok(decoded)
}

fn encode_row(row: &GroupRow) -> Value {
    let mut entries = vec![
        ("seq", Value::from(row.seq.0)),
        ("name", Value::from(row.name.as_str())),
    ];
    if !row.description.is_empty() {
        entries.push(("description", Value::from(row.description.as_str())));
    }
    entries.push(("selectable", Value::Bool(row.selectable)));
    entries.push((
        "members",
        Value::Array(
            row.members
                .iter()
                .map(|member| Value::from(member.persistent().raw()))
                .collect(),
        ),
    ));
    object(entries)
}

fn decode_row(value: &Value, context: String) -> Result<GroupRow, SnapshotError> {
    let mut reader = Reader::new(value, context.clone())?;
    let raw = reader.integer("seq")?;
    let seq = u64::try_from(raw)
        .ok()
        .filter(|seq| *seq > 0)
        .map(GroupSeq)
        .ok_or_else(|| reader.error(format!("seq: {raw} is not a positive group sequence")))?;
    let name = reader.string("name")?;
    if name.trim().is_empty() {
        return Err(reader.error("name: a group name cannot be empty"));
    }
    if name != name.trim() {
        return Err(reader.error(format!(
            "name: {name:?} is not trimmed (the writer stores trimmed names)"
        )));
    }
    let description = match reader.opt_take("description") {
        Some(Value::String(text)) => text,
        Some(_) => return Err(reader.error("description: expected a string")),
        None => String::new(),
    };
    let selectable = reader.bool("selectable")?;
    let members = reader
        .array("members")?
        .into_iter()
        .enumerate()
        .map(|(index, value)| decode_member(&value, format!("{context}.members[{index}]")))
        .collect::<Result<Vec<_>, SnapshotError>>()?;
    reader.finish()?;
    Ok(GroupRow {
        seq,
        name,
        description,
        selectable,
        members,
    })
}

fn decode_member(value: &Value, context: String) -> Result<EntityId, SnapshotError> {
    let raw = value
        .as_i64()
        .ok_or_else(|| SnapshotError::invalid(format!("{context}: expected a JSON integer")))?;
    let persistent = PersistentId::from_raw(raw)
        .ok_or_else(|| SnapshotError::invalid(format!("{context}: {raw} is not an identifier")))?;
    if persistent.kind() != IdKind::Entity {
        return Err(SnapshotError::invalid(format!(
            "{context}: {raw} is not an entity identifier"
        )));
    }
    Ok(EntityId::from_seq(persistent.seq()))
}
