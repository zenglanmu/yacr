//! `tables/models.json`: model metadata (ADR-0037 D7).
//!
//! ```text
//! [{"key": 10, "kind": "model", "name": null, "owner": null},
//!  {"key": 1099511627775, "kind": "definitions", "name": "Definitions", "owner": null}]
//! ```
//!
//! `key` is the **raw** [`cad_domain::ymodel::PersistentId`] number
//! (`(kind << 45) | seq`, always ≤ 2^53 so an `f64`-based reader keeps it exact),
//! not the in-memory `ModelKey`: the two differ for paper layouts and block
//! models, whose sequences share one numeric space. The family is recovered from
//! `kind` — `ModelKey::from_persistent` refuses a layout/block record claiming
//! sequence 0 or the reserved definition-model sequence, so a malformed record is
//! an invalid archive rather than a silently shared model.
//!
//! Records are written **sorted by model id**: the shard map and the model list
//! then agree on one order, and two saves of one snapshot stay byte-identical
//! regardless of the order the database happened to collect them in.

use serde_json::{Map, Value};

use cad_db::{ModelKind, ModelRecord, StoreError};
use cad_domain::ymodel::{IdKind, ModelFamily, ModelKey, PersistentId};

use crate::json::{malformed, Reader};

const CONTEXT: &str = "tables/models.json";

/// The `tables/models.json` array, sorted by model id.
pub(crate) fn models_value(models: &[ModelRecord]) -> Value {
    let mut sorted: Vec<&ModelRecord> = models.iter().collect();
    sorted.sort_by_key(|model| model.key.persistent().raw());
    Value::Array(sorted.iter().map(|model| record_value(model)).collect())
}

fn record_value(model: &ModelRecord) -> Value {
    let mut map = Map::new();
    map.insert("key".to_string(), Value::from(model.key.persistent().raw()));
    map.insert(
        "kind".to_string(),
        Value::String(model.kind.as_str().to_string()),
    );
    map.insert(
        "name".to_string(),
        match &model.name {
            Some(name) => Value::String(name.clone()),
            None => Value::Null,
        },
    );
    map.insert(
        "owner".to_string(),
        match model.owner {
            Some(owner) => Value::from(owner.raw()),
            None => Value::Null,
        },
    );
    Value::Object(map)
}

/// Read `tables/models.json`.
pub(crate) fn models_from_value(value: &Value) -> Result<Vec<ModelRecord>, StoreError> {
    let Value::Array(items) = value else {
        return Err(malformed(CONTEXT, "expected an array of models"));
    };
    let mut models = Vec::with_capacity(items.len());
    for item in items {
        models.push(record_from_value(item)?);
    }
    Ok(models)
}

fn record_from_value(value: &Value) -> Result<ModelRecord, StoreError> {
    let mut reader = Reader::new(value, CONTEXT)?;
    let raw = match reader.take("key")? {
        Value::Number(number) => number
            .as_i64()
            .ok_or_else(|| malformed(CONTEXT, "\"key\" must be an integer"))?,
        _ => return Err(malformed(CONTEXT, "\"key\" must be an integer")),
    };
    let kind_name = reader.string("kind")?;
    let name = reader.opt_string("name")?;
    let owner = match reader.take("owner")? {
        Value::Null => None,
        Value::Number(number) => {
            let raw = number
                .as_i64()
                .ok_or_else(|| malformed(CONTEXT, "\"owner\" must be an integer or null"))?;
            Some(
                PersistentId::from_raw(raw)
                    .ok_or_else(|| malformed(CONTEXT, format!("owner id {raw} is not storable")))?,
            )
        }
        _ => return Err(malformed(CONTEXT, "\"owner\" must be an integer or null")),
    };
    reader.finish()?;

    let kind = ModelKind::parse_name(&kind_name)
        .ok_or_else(|| malformed(CONTEXT, format!("unknown model kind {kind_name:?}")))?;
    let id = PersistentId::from_raw(raw)
        .ok_or_else(|| malformed(CONTEXT, format!("model id {raw} is not storable")))?;
    if id.kind() != IdKind::Model {
        return Err(malformed(
            CONTEXT,
            format!("model id {raw} is not a Model identifier"),
        ));
    }
    let family = family_of(kind);
    let key = ModelKey::from_persistent(id, family).ok_or_else(|| {
        malformed(
            CONTEXT,
            format!("model id {raw} cannot be a {} model", kind.as_str()),
        )
    })?;
    Ok(ModelRecord {
        key,
        kind,
        name,
        owner,
    })
}

/// The table a model id's sequence must be resolved against.
const fn family_of(kind: ModelKind) -> ModelFamily {
    match kind {
        ModelKind::ModelSpace => ModelFamily::ModelSpace,
        ModelKind::PaperSpace => ModelFamily::Paper,
        ModelKind::Block => ModelFamily::Block,
        ModelKind::Definitions => ModelFamily::Definitions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_domain::ymodel::PersistentIdOf;
    use cad_domain::{BlockId, EntityId, LayoutId};

    fn records() -> Vec<ModelRecord> {
        vec![
            ModelRecord {
                key: ModelKey::ModelSpace,
                kind: ModelKind::ModelSpace,
                name: Some("Model".into()),
                owner: None,
            },
            ModelRecord {
                key: ModelKey::Paper(LayoutId(3)),
                kind: ModelKind::PaperSpace,
                name: Some("Layout A".into()),
                owner: Some(EntityId(11).persistent()),
            },
            ModelRecord {
                key: ModelKey::Block(BlockId(2)),
                kind: ModelKind::Block,
                name: None,
                owner: None,
            },
            ModelRecord {
                key: ModelKey::Definitions,
                kind: ModelKind::Definitions,
                name: Some("Definitions".into()),
                owner: None,
            },
        ]
    }

    fn round_trip(models: &[ModelRecord]) -> Vec<ModelRecord> {
        let value = models_value(models);
        let text = serde_json::to_string(&value).expect("serialize");
        let parsed: Value = serde_json::from_str(&text).expect("parse");
        models_from_value(&parsed).expect("decode")
    }

    #[test]
    fn every_model_kind_round_trips() {
        let models = records();
        let mut expected = models.clone();
        expected.sort_by_key(|model| model.key.persistent().raw());
        assert_eq!(round_trip(&models), expected);
    }

    #[test]
    fn the_written_array_is_sorted_by_model_id() {
        let mut reversed = records();
        reversed.reverse();
        let value = models_value(&reversed);
        let keys: Vec<i64> = value
            .as_array()
            .expect("array")
            .iter()
            .map(|model| model["key"].as_i64().expect("number"))
            .collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(keys, sorted);
    }

    #[test]
    fn a_layout_record_cannot_claim_the_definition_model() {
        let value = serde_json::json!([{
            "key": ModelKey::Definitions.persistent().raw(),
            "kind": "paper",
            "name": null,
            "owner": null
        }]);
        assert!(models_from_value(&value).is_err());

        // …nor sequence 0, which is reserved for model space.
        let value = serde_json::json!([{
            "key": ModelKey::ModelSpace.persistent().raw(),
            "kind": "block",
            "name": null,
            "owner": null
        }]);
        assert!(models_from_value(&value).is_err());
    }

    #[test]
    fn malformed_records_are_refused() {
        for value in [
            serde_json::json!({}),
            serde_json::json!([{"key": 1, "kind": "model", "name": null}]),
            serde_json::json!([{"key": 1, "kind": "space", "name": null, "owner": null}]),
            serde_json::json!([{"key": -1, "kind": "model", "name": null, "owner": null}]),
            serde_json::json!([{"key": 1, "kind": "model", "name": 7, "owner": null}]),
            serde_json::json!([{"key": 1, "kind": "model", "name": null, "owner": -1}]),
            serde_json::json!([{
                "key": EntityId(1).persistent().raw(),
                "kind": "model",
                "name": null,
                "owner": null
            }]),
            serde_json::json!([{"key": 1, "kind": "model", "name": null, "owner": null, "x": 0}]),
        ] {
            assert!(models_from_value(&value).is_err(), "accepted {value}");
        }
    }
}
