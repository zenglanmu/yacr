//! `tables/features.json` row codec (ADR-0043 P2).
//!
//! ```json
//! [{"id":{"kind":"entity","entityId":123},"order":0,"operation":"extrude",
//!   "inputs":[{"kind":"entity","entityId":122}],
//!   "parameters":{"depth":{"ref":7}}}]
//! ```
//!
//! Strict in both directions: exactly five keys, entity-family persistent ids
//! (the row names existing element rows), a known operation token, and the same
//! slot→expression codec as an element `params` block. Duplicate ids/orders,
//! unknown operations and the row/byte budgets are refusals.
//!
//! Existence of the feature element and its inputs is a **database** rule
//! (checked at the builder `finish()` gate); this codec only owns the shape.

use cad_domain::ymodel::id::{IdKind, PersistentId, PersistentIdOf};
use cad_domain::EntityId;
use serde_json::Value;

use super::bim::object;
use super::elements::Reader;
use super::parameters::{decode_bindings, encode_bindings};
use super::{table_entry, SnapshotError, TableSnapshot};
use crate::drawing::DrawingDatabase;
use crate::feature::{
    FeatureDefinition, FeatureDefinitions, FeatureOperation, FEATURE_MAX_BYTES,
    FEATURE_MAX_DEFINITIONS,
};
use crate::scope::TableKind;

fn table() -> &'static str {
    table_entry(TableKind::Features)
}

fn budget(detail: impl std::fmt::Display) -> SnapshotError {
    SnapshotError::invalid(format!("feature.budget_exceeded: {detail}"))
}

/// Encode every feature row in evaluation order.
pub(crate) fn encode_features(db: &DrawingDatabase) -> Result<Vec<Value>, SnapshotError> {
    // The save path refuses what the reader's builder gate would: dangling
    // feature elements/inputs and unresolvable slot maps never reach disk.
    crate::feature::validate_features(db)
        .map_err(|error| SnapshotError::invalid(format!("{}: {error}", table())))?;
    let rows: Vec<Value> = db.features().ordered().map(encode_feature).collect();
    check_budgets(&rows)?;
    Ok(rows)
}

/// Decode the feature table. An absent table is an empty definition set.
pub(crate) fn decode_features(tables: &TableSnapshot) -> Result<FeatureDefinitions, SnapshotError> {
    let rows = tables.get(TableKind::Features).unwrap_or(&[]);
    if rows.len() > FEATURE_MAX_DEFINITIONS {
        return Err(budget(format!(
            "{}: {} rows exceeds the {FEATURE_MAX_DEFINITIONS}-row budget",
            table(),
            rows.len()
        )));
    }
    let mut features = FeatureDefinitions::new();
    for (index, row) in rows.iter().enumerate() {
        let context = format!("{} row {index}", table());
        let feature = decode_feature(row, context.clone())?;
        features
            .insert(feature)
            .map_err(|error| SnapshotError::invalid(format!("{context}: {error}")))?;
    }
    check_budgets(rows)?;
    Ok(features)
}

fn check_budgets(rows: &[Value]) -> Result<(), SnapshotError> {
    if rows.len() > FEATURE_MAX_DEFINITIONS {
        return Err(budget(format!(
            "{}: {} rows exceeds the {FEATURE_MAX_DEFINITIONS}-row budget",
            table(),
            rows.len()
        )));
    }
    let encoded = serde_json::to_vec(rows)
        .map(|bytes| bytes.len())
        .map_err(|error| SnapshotError::invalid(format!("{}: {error}", table())))?;
    if encoded > FEATURE_MAX_BYTES {
        return Err(budget(format!(
            "{}: the table encodes to {encoded} bytes, over the {FEATURE_MAX_BYTES}-byte budget",
            table()
        )));
    }
    Ok(())
}

fn encode_feature(feature: &FeatureDefinition) -> Value {
    let inputs = feature.inputs.iter().map(|id| encode_entity(*id)).collect();
    object(vec![
        ("id", encode_entity(feature.id)),
        ("order", Value::from(feature.order)),
        ("operation", Value::from(feature.operation.as_token())),
        ("inputs", Value::Array(inputs)),
        ("parameters", encode_bindings(&feature.parameters)),
    ])
}

fn decode_feature(value: &Value, context: String) -> Result<FeatureDefinition, SnapshotError> {
    let mut reader = Reader::new(value, context.clone())?;
    let id_value = reader.take("id")?;
    let id = decode_entity(&id_value, format!("{context}.id"))?;
    let raw = reader.integer("order")?;
    let order = u32::try_from(raw)
        .map_err(|_| reader.error(format!("order: {raw} is not a u32 evaluation order")))?;
    let token = reader.string("operation")?;
    let operation = FeatureOperation::parse(&token)
        .ok_or_else(|| reader.error(format!("operation: unknown feature operation {token:?}")))?;
    let inputs = reader
        .array("inputs")?
        .into_iter()
        .enumerate()
        .map(|(index, value)| decode_entity(&value, format!("{context}.inputs[{index}]")))
        .collect::<Result<Vec<_>, SnapshotError>>()?;
    let parameters_value = reader.take("parameters")?;
    let parameters = decode_bindings(&parameters_value, format!("{context}.parameters"))?;
    reader.finish()?;
    Ok(FeatureDefinition {
        id,
        order,
        operation,
        inputs,
        parameters,
    })
}

fn encode_entity(id: EntityId) -> Value {
    object(vec![
        ("kind", Value::from("entity")),
        ("entityId", Value::from(id.persistent().raw())),
    ])
}

/// One `{"kind":"entity","entityId":N}` object.
fn decode_entity(value: &Value, context: String) -> Result<EntityId, SnapshotError> {
    let mut nested = Reader::new(value, context)?;
    let kind = nested.string("kind")?;
    if kind != "entity" {
        return Err(nested.error(format!(
            "kind: expected \"entity\" (a feature row attaches to an element), got {kind:?}"
        )));
    }
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
    nested.finish()?;
    Ok(EntityId(persistent.seq()))
}

#[cfg(test)]
mod tests;
