//! `tables/configurations.json` row codec (ADR-0043 P1).
//!
//! One row per named configuration overlay:
//!
//! ```json
//! [{"name":"thin","parent":null,
//!   "overrides":{"1":{"num":4.0}},
//!   "suppressed":[2]}]
//! ```
//!
//! * `name` — required, non-empty (the domain constructor validates the byte
//!   budget too);
//! * `parent` — **required** key: a string naming another configuration, or
//!   `null`. An absent key is refused, so the writer's shape is the only shape;
//! * `overrides` — required object; each key is a **canonical decimal** `seq`
//!   (`"01"`, `"0"`, `"+1"` and `"1.0"` are refused) and each value is the
//!   parameter expression codec (`snapshot/parameters.rs`);
//! * `suppressed` — required array of integer sequences `>= 1`, unique.
//!
//! Every key is required, an extra key is refused and duplicate configuration
//! names are refused. The table is validated against the **base parameter
//! table** (`ParamConfigTable::validate`: unknown parent, parent cycle, override
//! of a missing sequence), and the row/encoded budgets use the parameter
//! vocabulary (`param.budget_exceeded`), never the BIM one.

use std::collections::BTreeSet;

use cad_domain::param::{
    ParamConfig, ParamConfigTable, ParamError, ParamSeq, ParamTable, PARAM_BUDGET_EXCEEDED,
    PARAM_INVALID_VALUE,
};
use serde_json::{Map, Value};

use super::bim::object;
use super::elements::Reader;
use super::parameters::{decode_expr, encode_expr};
use super::{table_entry, SnapshotError, TableSnapshot};
use crate::drawing::{DrawingDatabase, PARAM_MAX_CONFIGS, PARAM_MAX_CONFIG_BYTES};
use crate::scope::TableKind;

/// The table entry this codec owns.
fn table() -> &'static str {
    table_entry(TableKind::Configurations)
}

fn budget(detail: impl std::fmt::Display) -> SnapshotError {
    SnapshotError::invalid(format!("{PARAM_BUDGET_EXCEEDED}: {detail}"))
}

fn invalid(detail: impl std::fmt::Display) -> SnapshotError {
    SnapshotError::invalid(format!("{PARAM_INVALID_VALUE}: {detail}"))
}

/// Encode every configuration row, in name order.
pub(crate) fn encode_configurations(db: &DrawingDatabase) -> Result<Vec<Value>, SnapshotError> {
    // The writer must refuse an archive its own reader would reject: every
    // overlay is checked against the base table (unknown parent, cycle,
    // override/suppression of a missing sequence) before encoding.
    db.param_configs()
        .validate(db.parameters())
        .map_err(|error| param_error(table(), error))?;
    let rows: Vec<Value> = db.param_configs().rows().map(encode_config).collect();
    if rows.len() > PARAM_MAX_CONFIGS {
        return Err(budget(format!(
            "{}: {} rows exceeds the {PARAM_MAX_CONFIGS}-row budget",
            table(),
            rows.len()
        )));
    }
    check_encoded_len(&rows)?;
    Ok(rows)
}

/// Decode the configuration table and validate it against `base`.
///
/// An absent table is an empty overlay set. `base` must be the already decoded
/// base parameter table: every override/suppression names a row in it.
pub(crate) fn decode_configurations(
    tables: &TableSnapshot,
    base: &ParamTable,
) -> Result<ParamConfigTable, SnapshotError> {
    let rows = tables.get(TableKind::Configurations).unwrap_or(&[]);
    if rows.len() > PARAM_MAX_CONFIGS {
        return Err(budget(format!(
            "{}: {} rows exceeds the {PARAM_MAX_CONFIGS}-row budget",
            table(),
            rows.len()
        )));
    }
    let mut configs = ParamConfigTable::new();
    for (index, row) in rows.iter().enumerate() {
        let context = format!("{} row {index}", table());
        let config = decode_config(row, context.clone())?;
        configs
            .insert(config)
            .map_err(|error| param_error(&context, error))?;
    }
    configs
        .validate(base)
        .map_err(|error| param_error(table(), error))?;
    check_encoded_len(rows)?;
    Ok(configs)
}

fn check_encoded_len(rows: &[Value]) -> Result<(), SnapshotError> {
    let encoded = serde_json::to_vec(rows)
        .map(|bytes| bytes.len())
        .map_err(|error| SnapshotError::invalid(format!("{}: {error}", table())))?;
    if encoded > PARAM_MAX_CONFIG_BYTES {
        return Err(budget(format!(
            "{}: the table encodes to {encoded} bytes, over the \
             {PARAM_MAX_CONFIG_BYTES}-byte budget",
            table()
        )));
    }
    Ok(())
}

/// Encode one configuration row. `parent` is written as `null`, never omitted.
fn encode_config(config: &ParamConfig) -> Value {
    let mut overrides = Map::new();
    for (seq, expr) in &config.overrides {
        overrides.insert(seq.0.to_string(), encode_expr(expr));
    }
    let suppressed = config
        .suppressed
        .iter()
        .map(|seq| Value::from(seq.0))
        .collect();
    object(vec![
        ("name", Value::from(config.name.as_str())),
        (
            "parent",
            config.parent.as_deref().map_or(Value::Null, Value::from),
        ),
        ("overrides", Value::Object(overrides)),
        ("suppressed", Value::Array(suppressed)),
    ])
}

/// Decode one configuration row strictly.
fn decode_config(value: &Value, context: String) -> Result<ParamConfig, SnapshotError> {
    let mut reader = Reader::new(value, context.clone())?;
    let name = reader.string("name")?;
    let parent = match reader.take("parent")? {
        Value::Null => None,
        Value::String(parent) => Some(parent),
        _ => return Err(reader.error("parent: expected a string or null")),
    };
    let overrides = reader.take("overrides")?;
    let suppressed = reader.take("suppressed")?;
    reader.finish()?;

    let mut config = ParamConfig::new(name).map_err(|error| param_error(&context, error))?;
    if let Some(parent) = parent {
        config = config
            .with_parent(parent)
            .map_err(|error| param_error(&context, error))?;
    }
    let Value::Object(overrides) = overrides else {
        return Err(invalid(format!("{context}.overrides: expected an object")));
    };
    for (key, expr) in &overrides {
        let seq = parse_seq_key(key).ok_or_else(|| {
            invalid(format!(
                "{context}.overrides: {key:?} is not a canonical decimal sequence"
            ))
        })?;
        let decoded = decode_expr(expr, format!("{context}.overrides.{key}"))?;
        config
            .set_override(ParamSeq(seq), decoded)
            .map_err(|error| param_error(&context, error))?;
    }
    let Value::Array(suppressed) = suppressed else {
        return Err(invalid(format!("{context}.suppressed: expected an array")));
    };
    let mut seen = BTreeSet::new();
    for item in &suppressed {
        let raw = item.as_u64().ok_or_else(|| {
            invalid(format!(
                "{context}.suppressed: expected an integer sequence, got {item}"
            ))
        })?;
        if raw == 0 {
            return Err(invalid(format!(
                "{context}.suppressed: a sequence must be at least 1"
            )));
        }
        if !seen.insert(raw) {
            return Err(invalid(format!(
                "{context}.suppressed: duplicate sequence {raw}"
            )));
        }
        config.suppress(ParamSeq(raw));
    }
    Ok(config)
}

/// A canonical decimal sequence key: no sign, no leading zero, parseable `u64`.
fn parse_seq_key(key: &str) -> Option<u64> {
    let raw: u64 = key.parse().ok()?;
    (raw >= 1 && raw.to_string() == key).then_some(raw)
}

fn param_error(context: &str, error: ParamError) -> SnapshotError {
    SnapshotError::invalid(format!("{context}: {}: {}", error.code(), error.detail()))
}

#[cfg(test)]
mod tests;
