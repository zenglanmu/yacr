//! `tables/relations.json` row codec (ADR-0043 P3).
//!
//! ```json
//! [{"source":{"entityId":1},"target":{"entityId":2},
//!   "reference":{"kind":"face","key":"f1"},"state":"in_context","label":null}]
//! ```
//!
//! Strict in both directions: exactly five keys, entity-family persistent ids,
//! a `face|edge|vertex` reference with a non-empty bounded key, a known
//! `InContextState` token and an optional label. The `(source, target,
//! reference)` triple is unique. Endpoint existence and `source != target` are
//! database rules checked at the builder `finish()` gate; this codec owns the
//! shape and the row/byte budgets.

use cad_domain::relation::{InContextState, RelationReference};
use cad_domain::ymodel::id::{IdKind, PersistentId, PersistentIdOf};
use cad_domain::EntityId;
use serde_json::Value;

use super::bim::object;
use super::elements::Reader;
use super::{table_entry, SnapshotError, TableSnapshot};
use crate::drawing::DrawingDatabase;
use crate::relation::{
    InContextRelation, InContextRelations, REL_MAX_BYTES, REL_MAX_REFERENCE_KEY_BYTES,
    REL_MAX_RELATIONS,
};
use crate::scope::TableKind;

fn table() -> &'static str {
    table_entry(TableKind::Relations)
}

fn budget(detail: impl std::fmt::Display) -> SnapshotError {
    SnapshotError::invalid(format!("rel.budget_exceeded: {detail}"))
}

fn invalid(detail: impl std::fmt::Display) -> SnapshotError {
    SnapshotError::invalid(format!("rel.invalid: {detail}"))
}

/// Encode every relation row in deterministic key order.
pub(crate) fn encode_relations_in_context(
    db: &DrawingDatabase,
) -> Result<Vec<Value>, SnapshotError> {
    // The save path refuses what the reader's builder gate would: dangling
    // endpoints never reach disk.
    crate::relation::validate_in_context_relations(db)
        .map_err(|error| SnapshotError::invalid(format!("{}: {error}", table())))?;
    let rows: Vec<Value> = db.relations().iter().map(encode_relation).collect();
    check_budgets(&rows)?;
    Ok(rows)
}

/// Decode the relation table. An absent table is an empty relation set.
pub(crate) fn decode_relations_in_context(
    tables: &TableSnapshot,
) -> Result<InContextRelations, SnapshotError> {
    let rows = tables.get(TableKind::Relations).unwrap_or(&[]);
    if rows.len() > REL_MAX_RELATIONS {
        return Err(budget(format!(
            "{}: {} rows exceeds the {REL_MAX_RELATIONS}-row budget",
            table(),
            rows.len()
        )));
    }
    let mut relations = InContextRelations::new();
    for (index, row) in rows.iter().enumerate() {
        let context = format!("{} row {index}", table());
        let relation = decode_relation(row, context.clone())?;
        relations
            .insert(relation)
            .map_err(|error| SnapshotError::invalid(format!("{context}: {error}")))?;
    }
    check_budgets(rows)?;
    Ok(relations)
}

fn check_budgets(rows: &[Value]) -> Result<(), SnapshotError> {
    if rows.len() > REL_MAX_RELATIONS {
        return Err(budget(format!(
            "{}: {} rows exceeds the {REL_MAX_RELATIONS}-row budget",
            table(),
            rows.len()
        )));
    }
    let encoded = serde_json::to_vec(rows)
        .map(|bytes| bytes.len())
        .map_err(|error| SnapshotError::invalid(format!("{}: {error}", table())))?;
    if encoded > REL_MAX_BYTES {
        return Err(budget(format!(
            "{}: the table encodes to {encoded} bytes, over the {REL_MAX_BYTES}-byte budget",
            table()
        )));
    }
    Ok(())
}

fn encode_relation(relation: &InContextRelation) -> Value {
    object(vec![
        ("source", encode_entity(relation.source)),
        ("target", encode_entity(relation.target)),
        (
            "reference",
            object(vec![
                ("kind", Value::from(relation.reference.token())),
                ("key", Value::from(relation.reference.key())),
            ]),
        ),
        ("state", Value::from(relation.state.token())),
        (
            "label",
            relation.label.as_deref().map_or(Value::Null, Value::from),
        ),
    ])
}

fn decode_relation(value: &Value, context: String) -> Result<InContextRelation, SnapshotError> {
    let mut reader = Reader::new(value, context.clone())?;
    let source_value = reader.take("source")?;
    let source = decode_entity(&source_value, format!("{context}.source"))?;
    let target_value = reader.take("target")?;
    let target = decode_entity(&target_value, format!("{context}.target"))?;
    let reference_value = reader.take("reference")?;
    let reference = decode_reference(&reference_value, format!("{context}.reference"))?;
    let token = reader.string("state")?;
    let state = InContextState::parse(&token)
        .ok_or_else(|| reader.error(format!("state: unknown in-context state {token:?}")))?;
    let label = match reader.take("label")? {
        Value::Null => None,
        Value::String(label) => {
            if label.len() > REL_MAX_REFERENCE_KEY_BYTES {
                return Err(budget(format!(
                    "{context}.label: {} bytes over the {REL_MAX_REFERENCE_KEY_BYTES}-byte budget",
                    label.len()
                )));
            }
            Some(label)
        }
        _ => return Err(reader.error("label: expected a string or null")),
    };
    reader.finish()?;
    Ok(InContextRelation {
        source,
        target,
        reference,
        state,
        label,
    })
}

fn decode_reference(value: &Value, context: String) -> Result<RelationReference, SnapshotError> {
    let mut reader = Reader::new(value, context)?;
    let kind = reader.string("kind")?;
    let key = reader.string("key")?;
    reader.finish()?;
    if key.trim().is_empty() {
        return Err(invalid("reference key must not be empty"));
    }
    if key.len() > REL_MAX_REFERENCE_KEY_BYTES {
        return Err(budget(format!(
            "reference key is {} bytes, over the {REL_MAX_REFERENCE_KEY_BYTES}-byte budget",
            key.len()
        )));
    }
    RelationReference::parse(&kind, key).ok_or_else(|| {
        invalid(format!(
            "unknown reference kind {kind:?} (expected face, edge or vertex)"
        ))
    })
}

fn encode_entity(id: EntityId) -> Value {
    object(vec![("entityId", Value::from(id.persistent().raw()))])
}

fn decode_entity(value: &Value, context: String) -> Result<EntityId, SnapshotError> {
    let mut reader = Reader::new(value, context)?;
    let raw = reader.integer("entityId")?;
    let persistent = PersistentId::from_raw(raw).ok_or_else(|| {
        reader.error(format!(
            "entityId: {raw} is not a persistent identifier this format knows"
        ))
    })?;
    if persistent.kind() != IdKind::Entity {
        return Err(reader.error(format!(
            "entityId: {raw} is not an entity-family persistent id"
        )));
    }
    reader.finish()?;
    Ok(EntityId(persistent.seq()))
}

#[cfg(test)]
mod tests;
