//! `tables/bim-relations.json` row codec (ADR-0042 D3 tail / D12).
//!
//! A plain JSON array like every other table, one row per edge:
//!
//! ```json
//! [{"kind": "aggregates",
//!   "parent": {"kind": "source", "connector": "ifc",
//!              "document_id": "<doc>", "id": "<id>"},
//!   "child":  {"kind": "entity", "entityId": 7},
//!   "source": {"kind": "source", "connector": "ifc",
//!              "document_id": "<doc>", "id": "<rel>"},
//!   "label": null}]
//! ```
//!
//! Endpoints reuse [`crate::bim::BimKey`] — the same `(connector, document_id,
//! id)` equivalence class the element `bim.source` block uses, or the element
//! itself when the source gave no identifier. That is why the source spelling
//! here is `document_id`, not a second camel-case dialect: one key shape, one
//! decoder.
//!
//! Strict in both directions:
//!
//! * every key is required (`source`/`label` are written as `null`, never
//!   omitted), an extra key is refused, and a wrong JSON type is refused;
//! * an endpoint object must carry exactly its own keys: a `source` endpoint
//!   has `kind/connector/document_id/id`, an `entity` endpoint has
//!   `kind/entityId` and nothing else (no `native_id` — that is element
//!   provenance, not a component key);
//! * `entityId` is a persistent identifier of the **entity family**; a bare
//!   sequence or another family is refused, exactly like an element row id;
//! * per-string lengths and the row/encoded budgets are enforced **before** an
//!   over-budget archive can be decoded: refusal, never truncation;
//! * a duplicate `(kind, parent, child)` row is refused.

use std::collections::BTreeMap;

use cad_domain::ymodel::id::{IdKind, PersistentId, PersistentIdOf};
use cad_domain::EntityId;
use serde_json::Value;

use super::bim::{budget, check_text, object, prefixed, take_text, text_bytes};
use super::elements::Reader;
use super::{table_entry, SnapshotError, TableSnapshot};
use crate::bim::{
    BimKey, BimRelation, BimRelationKind, BimRelations, BIM_INVALID, BIM_MAX_RELATIONS,
    BIM_MAX_RELATION_BYTES,
};
use crate::drawing::DrawingDatabase;
use crate::scope::TableKind;

/// The table entry this codec owns.
fn table() -> &'static str {
    table_entry(TableKind::BimRelations)
}

/// Encode every relation row, in `(kind, parent, child)` order.
pub(crate) fn encode_relations(db: &DrawingDatabase) -> Result<Vec<Value>, SnapshotError> {
    Ok(db.bim_relations.iter().map(encode_relation).collect())
}

/// Decode the relation table. An absent table is an empty relation set.
pub(crate) fn decode_relations(tables: &TableSnapshot) -> Result<BimRelations, SnapshotError> {
    let rows = tables.get(TableKind::BimRelations).unwrap_or(&[]);
    if rows.len() > BIM_MAX_RELATIONS {
        return Err(budget(format!(
            "{}: {} rows exceeds the {BIM_MAX_RELATIONS}-row budget",
            table(),
            rows.len()
        )));
    }
    let mut map: BTreeMap<(BimRelationKind, BimKey, BimKey), BimRelation> = BTreeMap::new();
    let mut text_total = 0usize;
    for (index, row) in rows.iter().enumerate() {
        // Bound the work before decoding: the entry has already been parsed by
        // the caller, but a hostile archive must not build an unbounded edge set.
        text_total = text_total.saturating_add(text_bytes(row));
        if text_total > BIM_MAX_RELATION_BYTES {
            return Err(budget(format!(
                "{}: row {index} pushes the relations over the \
                 {BIM_MAX_RELATION_BYTES}-byte budget",
                table()
            )));
        }
        let relation =
            decode_relation(row, format!("{} row {index}", table())).map_err(prefixed)?;
        let key = (
            relation.kind.clone(),
            relation.parent.clone(),
            relation.child.clone(),
        );
        if map.insert(key, relation).is_some() {
            return Err(SnapshotError::invalid(format!(
                "{BIM_INVALID}: {}: duplicate relation row at index {index}",
                table()
            )));
        }
    }
    let relations = BimRelations::from_rows(map);
    let encoded = relations_encoded_len(&relations).map_err(prefixed)?;
    if encoded > BIM_MAX_RELATION_BYTES {
        return Err(budget(format!(
            "{}: the table encodes to {encoded} bytes, over the \
             {BIM_MAX_RELATION_BYTES}-byte budget",
            table()
        )));
    }
    Ok(relations)
}

/// The exact byte length the writer emits for this relation set.
///
/// Used on both sides of the budget: the builder refuses a set over 32 MiB
/// before it can be saved, and the reader refuses an archive that holds one.
pub(crate) fn relations_encoded_len(relations: &BimRelations) -> Result<usize, SnapshotError> {
    let rows: Vec<Value> = relations.iter().map(encode_relation).collect();
    serde_json::to_vec(&Value::Array(rows))
        .map(|bytes| bytes.len())
        .map_err(|error| SnapshotError::invalid(format!("{}: {error}", table())))
}

/// Encode one relation row. Every value is validated before it can be stored
/// ([`BimRelations::insert`] / `builder::set_bim_relations`), so encoding itself
/// cannot fail.
fn encode_relation(row: &BimRelation) -> Value {
    object(vec![
        ("kind", Value::from(row.kind.as_token())),
        ("parent", encode_key(&row.parent)),
        ("child", encode_key(&row.child)),
        (
            "source",
            row.source.as_ref().map_or(Value::Null, encode_key),
        ),
        (
            "label",
            row.label.as_deref().map_or(Value::Null, Value::from),
        ),
    ])
}

/// Encode one endpoint. `kind` is the tag; the rest is the [`BimKey`] payload.
fn encode_key(key: &BimKey) -> Value {
    match key {
        BimKey::Source {
            connector,
            document_id,
            id,
        } => object(vec![
            ("kind", Value::from("source")),
            ("connector", Value::from(connector.as_str())),
            ("document_id", Value::from(document_id.as_str())),
            ("id", Value::from(id.as_str())),
        ]),
        BimKey::Entity(entity) => object(vec![
            ("kind", Value::from("entity")),
            ("entityId", Value::from(entity.persistent().raw())),
        ]),
    }
}

/// Decode one relation row strictly.
fn decode_relation(value: &Value, context: String) -> Result<BimRelation, SnapshotError> {
    let mut reader = Reader::new(value, context)?;
    let kind_token = take_text(&mut reader, "kind", false)?;
    let kind = BimRelationKind::parse(&kind_token).ok_or_else(|| {
        reader.error(format!(
            "kind: unknown relation kind {kind_token:?} \
             (expected aggregates, nests, contains, group, connects or other:<token>)"
        ))
    })?;
    let parent = take_key(&mut reader, "parent")?;
    let child = take_key(&mut reader, "child")?;
    let source = take_optional_key(&mut reader, "source")?;
    let label = take_optional_text(&mut reader, "label")?;
    reader.finish()?;
    Ok(BimRelation {
        kind,
        parent,
        child,
        source,
        label,
    })
}

/// Decode a required endpoint object.
fn take_key(reader: &mut Reader, field: &'static str) -> Result<BimKey, SnapshotError> {
    let value = reader.take(field)?;
    decode_key(reader, &value, field)
}

/// Decode a required `null`-or-object field.
fn take_optional_key(
    reader: &mut Reader,
    field: &'static str,
) -> Result<Option<BimKey>, SnapshotError> {
    match reader.take(field)? {
        Value::Null => Ok(None),
        ref value @ Value::Object(_) => decode_key(reader, value, field).map(Some),
        _ => Err(reader.error(format!("{field}: expected an object or null"))),
    }
}

fn decode_key(
    reader: &Reader,
    value: &Value,
    field: &'static str,
) -> Result<BimKey, SnapshotError> {
    let mut nested = reader.child(value, field)?;
    let kind = take_text(&mut nested, "kind", false)?;
    let key = match kind.as_str() {
        "source" => BimKey::Source {
            connector: take_text(&mut nested, "connector", false)?,
            document_id: take_text(&mut nested, "document_id", false)?,
            id: take_text(&mut nested, "id", false)?,
        },
        "entity" => {
            let raw = nested.integer("entityId")?;
            let persistent = PersistentId::from_raw(raw).ok_or_else(|| {
                nested.error(format!(
                    "entityId: {raw} is not a persistent identifier this format knows"
                ))
            })?;
            if persistent.kind() != IdKind::Entity {
                return Err(nested.error(format!(
                    "entityId: {raw} is not an entity-family persistent id"
                )));
            }
            BimKey::Entity(EntityId(persistent.seq()))
        }
        other => {
            return Err(nested.error(format!(
                "kind: unknown relation endpoint kind {other:?} (expected source or entity)"
            )))
        }
    };
    nested.finish()?;
    Ok(key)
}

/// A required `null`-or-string field (a `null` label is "no label").
fn take_optional_text(
    reader: &mut Reader,
    field: &'static str,
) -> Result<Option<String>, SnapshotError> {
    match reader.take(field)? {
        Value::Null => Ok(None),
        Value::String(text) => {
            check_text(reader, field, &text, false)?;
            Ok(Some(text))
        }
        _ => Err(reader.error(format!("{field}: expected a string or null"))),
    }
}

#[cfg(test)]
mod tests;
