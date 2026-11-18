//! `tables/models.json`: model metadata (ADR-0037 D7).
//!
//! ```text
//! [{"key": 351843720888320, "kind": "model", "name": null, "owner": null},
//!  {"key": 492581209243649, "kind": "paper", "name": "Sheet",
//!   "owner": 140737488355329}]
//! ```
//!
//! `key` is the **raw** [`cad_domain::ymodel::PersistentId`] number
//! (`(kind << 45) | seq`, always ≤ 2^53 so an `f64`-based reader keeps it
//! exact). Each model family has its own kind — model space 10, paper 14,
//! block 15, definitions 16 — so the id alone identifies the model and a paper
//! layout and a block may carry the same sequence. The record's `kind` is kept
//! because it makes the file readable, but it must **agree** with the id:
//! [`ModelKey::from_persistent`] decodes the family from the id and a record
//! whose two spellings disagree is an invalid archive, never a silently
//! retyped model.
//!
//! Records are written **sorted by model id**: the shard map and the model list
//! then agree on one order, and two saves of one snapshot stay byte-identical
//! regardless of the order the database happened to collect them in.

use serde_json::{Map, Value};

use cad_db::{ModelKind, ModelRecord, StoreError};
use cad_domain::ymodel::{ModelKey, PersistentId};

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
    let key = ModelKey::from_persistent(id)
        .ok_or_else(|| malformed(CONTEXT, format!("model id {raw} is not a Model identifier")))?;
    let derived = model_kind(key);
    if derived != kind {
        return Err(malformed(
            CONTEXT,
            format!(
                "model id {raw} encodes a {} model but records kind {kind_name:?}",
                derived.as_str()
            ),
        ));
    }
    Ok(ModelRecord {
        key,
        kind,
        name,
        owner,
    })
}

/// The `kind` a decoded model id must be recorded as.
const fn model_kind(key: ModelKey) -> ModelKind {
    match key {
        ModelKey::ModelSpace => ModelKind::ModelSpace,
        ModelKey::Paper(_) => ModelKind::PaperSpace,
        ModelKey::Block(_) => ModelKind::Block,
        ModelKey::Definitions => ModelKind::Definitions,
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
    fn a_record_kind_must_agree_with_its_model_id() {
        // A paper record whose id is the definition-model id.
        let value = serde_json::json!([{
            "key": ModelKey::Definitions.persistent().raw(),
            "kind": "paper",
            "name": null,
            "owner": null
        }]);
        assert!(models_from_value(&value).is_err());

        // A block record whose id is the model-space id.
        let value = serde_json::json!([{
            "key": ModelKey::ModelSpace.persistent().raw(),
            "kind": "block",
            "name": null,
            "owner": null
        }]);
        assert!(models_from_value(&value).is_err());

        // A layout record whose id encodes a block model with the same sequence:
        // the family comes from the id now, and a disagreement is refused.
        let value = serde_json::json!([{
            "key": ModelKey::Block(BlockId(1)).persistent().raw(),
            "kind": "paper",
            "name": null,
            "owner": null
        }]);
        assert!(models_from_value(&value).is_err());

        // The same sequence under the right kind decodes.
        let value = serde_json::json!([{
            "key": ModelKey::Paper(LayoutId(1)).persistent().raw(),
            "kind": "paper",
            "name": null,
            "owner": null
        }]);
        assert_eq!(
            models_from_value(&value).expect("decode"),
            vec![ModelRecord {
                key: ModelKey::Paper(LayoutId(1)),
                kind: ModelKind::PaperSpace,
                name: None,
                owner: None,
            }]
        );
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
