//! `tables/bim-definitions.json` row codec (ADR-0042 D3).
//!
//! A plain JSON array of shared definition rows, exactly like every other
//! table, one row per level, type or material:
//!
//! ```json
//! [ { "id": "level:ifc:<doc>:<id>", "kind": "level", "name": "Level 1",
//!     "elevation": 0.0,
//!     "source": {"connector": "ifc", "document_id": "<doc>", "id": "<id>",
//!                "native_id": null},
//!     "evidence": "declared" },
//!   { "id": "type:ifc:<doc>:<id>", "kind": "type", "name": "Basic Wall",
//!     "class": "IfcWallType", "category": "IfcWall", "psets": [], … },
//!   { "id": "material:ifc:<doc>:<id>", "kind": "material", "name": "Concrete", … } ]
//! ```
//!
//! The element-side `bim` block lives in [`super::elements::bim`]; this module
//! also owns the nested shapes it reuses (source, property sets, values,
//! evidence) so the two cannot drift.
//!
//! Strict in both directions, like every table codec:
//!
//! * every key is required (a `None` value is written as `null`, never omitted),
//!   an extra key is refused, and a value of the wrong JSON type is refused;
//! * a `kind`-specific key on the wrong row kind (`elevation` on a material) is
//!   an unknown field, so a reader cannot silently ignore it;
//! * per-string length and per-element property-set/property counts are capped
//!   **before** they are stored, and the decoded table is re-measured against
//!   the 32 MiB budget: an oversized archive is refused, never truncated;
//! * a duplicate id is refused (two rows one element reference could mean).

use std::collections::BTreeMap;

use cad_domain::ymodel::json;
use serde_json::{Map, Value};

use super::elements::Reader;
use super::{table_entry, SnapshotError, TableSnapshot};
use crate::bim::{
    BimDefinition, BimDefinitions, BimEvidence, BimProperty, BimPropertySet, BimSource, BimValue,
    BIM_BUDGET_EXCEEDED, BIM_INVALID, BIM_MAX_DEFINITIONS, BIM_MAX_DEFINITION_BYTES,
    BIM_MAX_PROPERTIES_PER_SET, BIM_MAX_STRING_BYTES,
};
use crate::drawing::DrawingDatabase;
use crate::scope::TableKind;

/// The table entry this codec owns.
fn table() -> &'static str {
    table_entry(TableKind::BimDefinitions)
}

/// Encode every definition row, in id order.
pub(crate) fn encode_definitions(db: &DrawingDatabase) -> Result<Vec<Value>, SnapshotError> {
    db.bim_definitions
        .iter()
        .map(|row| Ok(encode_definition(row)))
        .collect()
}

/// Decode the definition table. An absent table is an empty definition set.
pub(crate) fn decode_definitions(tables: &TableSnapshot) -> Result<BimDefinitions, SnapshotError> {
    let rows = tables.get(TableKind::BimDefinitions).unwrap_or(&[]);
    if rows.len() > BIM_MAX_DEFINITIONS {
        return Err(budget(format!(
            "{}: {} rows exceeds the {BIM_MAX_DEFINITIONS}-row budget",
            table(),
            rows.len()
        )));
    }
    let mut map: BTreeMap<String, BimDefinition> = BTreeMap::new();
    let mut text_total = 0usize;
    for (index, row) in rows.iter().enumerate() {
        // Bound the work before decoding: the entry's `Value` has already been
        // parsed by the caller, but a hostile archive must not make this codec
        // build a decoded table of unbounded size. The per-string and per-row
        // caps alone would allow 100k rows of 64 MiB each.
        text_total = text_total.saturating_add(text_bytes(row));
        if text_total > BIM_MAX_DEFINITION_BYTES {
            return Err(budget(format!(
                "{}: row {index} pushes the definitions over the \
                 {BIM_MAX_DEFINITION_BYTES}-byte budget",
                table()
            )));
        }
        let definition =
            decode_definition(row, format!("{} row {index}", table())).map_err(prefixed)?;
        let id = definition.id().to_string();
        if map.insert(id.clone(), definition).is_some() {
            return Err(SnapshotError::invalid(format!(
                "{BIM_INVALID}: {}: definition id {id:?} appears twice",
                table()
            )));
        }
    }
    let definitions = BimDefinitions::from_rows(map);
    let encoded = definitions_encoded_len(&definitions).map_err(prefixed)?;
    if encoded > BIM_MAX_DEFINITION_BYTES {
        return Err(budget(format!(
            "{}: the table encodes to {encoded} bytes, over the \
             {BIM_MAX_DEFINITION_BYTES}-byte budget",
            table()
        )));
    }
    Ok(definitions)
}

/// The exact byte length the writer emits for this definition set.
///
/// Used on both sides of the budget: the builder refuses a set over 32 MiB
/// before it can be saved, and the reader refuses an archive that holds one.
pub(crate) fn definitions_encoded_len(
    definitions: &BimDefinitions,
) -> Result<usize, SnapshotError> {
    let rows: Vec<Value> = definitions.iter().map(encode_definition).collect();
    serde_json::to_vec(&Value::Array(rows))
        .map(|bytes| bytes.len())
        .map_err(|error| SnapshotError::invalid(format!("{}: {error}", table())))
}

/// Encode one definition row. Every value is validated before it can be stored
/// (`BimDefinitions::insert` / `builder::set_bim_definitions`), so encoding
/// itself cannot fail.
pub(crate) fn encode_definition(row: &BimDefinition) -> Value {
    match row {
        BimDefinition::Level {
            id,
            name,
            elevation,
            source,
            evidence,
        } => object(vec![
            ("id", Value::from(id.as_str())),
            ("kind", Value::from(row.kind().as_str())),
            ("name", Value::from(name.as_str())),
            ("elevation", elevation.map_or(Value::Null, json::encode_f64)),
            ("source", encode_source(source)),
            ("evidence", encode_evidence(*evidence)),
        ]),
        BimDefinition::Type {
            id,
            name,
            class,
            category,
            psets,
            source,
            evidence,
        } => object(vec![
            ("id", Value::from(id.as_str())),
            ("kind", Value::from(row.kind().as_str())),
            ("name", Value::from(name.as_str())),
            ("class", class.as_deref().map_or(Value::Null, Value::from)),
            (
                "category",
                category.as_deref().map_or(Value::Null, Value::from),
            ),
            ("psets", encode_psets(psets)),
            ("source", encode_source(source)),
            ("evidence", encode_evidence(*evidence)),
        ]),
        BimDefinition::Material {
            id,
            name,
            source,
            evidence,
        } => object(vec![
            ("id", Value::from(id.as_str())),
            ("kind", Value::from(row.kind().as_str())),
            ("name", Value::from(name.as_str())),
            ("source", encode_source(source)),
            ("evidence", encode_evidence(*evidence)),
        ]),
    }
}

/// Decode one definition row strictly.
fn decode_definition(value: &Value, context: String) -> Result<BimDefinition, SnapshotError> {
    let mut reader = Reader::new(value, context)?;
    let id = take_text(&mut reader, "id", false)?;
    let kind_token = take_text(&mut reader, "kind", false)?;
    let kind = crate::bim::BimDefinitionKind::parse(&kind_token).ok_or_else(|| {
        reader.error(format!(
            "kind: unknown definition kind {kind_token:?} (expected level, type or material)"
        ))
    })?;
    if !id.starts_with(kind.id_prefix()) || id.len() == kind.id_prefix().len() {
        return Err(reader.error(format!(
            "id: {id:?} does not name a {} definition (expected {}{{…}})",
            kind.as_str(),
            kind.id_prefix()
        )));
    }
    let name = take_text(&mut reader, "name", false)?;
    let row = match kind {
        crate::bim::BimDefinitionKind::Level => BimDefinition::Level {
            id,
            name,
            elevation: reader.opt_f64("elevation")?,
            source: take_source(&mut reader, "source")?,
            evidence: take_evidence(&mut reader, "evidence")?,
        },
        crate::bim::BimDefinitionKind::Type => BimDefinition::Type {
            id,
            name,
            class: take_opt_text(&mut reader, "class")?,
            category: take_opt_text(&mut reader, "category")?,
            psets: take_psets(&mut reader, "psets")?,
            source: take_source(&mut reader, "source")?,
            evidence: take_evidence(&mut reader, "evidence")?,
        },
        crate::bim::BimDefinitionKind::Material => BimDefinition::Material {
            id,
            name,
            source: take_source(&mut reader, "source")?,
            evidence: take_evidence(&mut reader, "evidence")?,
        },
    };
    reader.finish()?;
    Ok(row)
}

// ------------------------------------------------------- shared nested shapes ---

/// Encode a source block: all four keys, `null` for an unknown part.
pub(crate) fn encode_source(source: &BimSource) -> Value {
    object(vec![
        ("connector", Value::from(source.connector.as_str())),
        (
            "document_id",
            source
                .document_id
                .as_deref()
                .map_or(Value::Null, Value::from),
        ),
        ("id", source.id.as_deref().map_or(Value::Null, Value::from)),
        (
            "native_id",
            source.native_id.as_deref().map_or(Value::Null, Value::from),
        ),
    ])
}

/// Encode the property-set array.
pub(crate) fn encode_psets(psets: &[BimPropertySet]) -> Value {
    Value::Array(
        psets
            .iter()
            .map(|set| {
                object(vec![
                    ("name", Value::from(set.name.as_str())),
                    (
                        "properties",
                        Value::Array(set.properties.iter().map(encode_property).collect()),
                    ),
                ])
            })
            .collect(),
    )
}

fn encode_property(property: &BimProperty) -> Value {
    object(vec![
        ("name", Value::from(property.name.as_str())),
        ("value", encode_value(&property.value)),
    ])
}

fn encode_value(value: &BimValue) -> Value {
    match value {
        BimValue::Text(text) => object(vec![
            ("kind", Value::from("text")),
            ("value", Value::from(text.as_str())),
        ]),
        BimValue::Real {
            value,
            ifc_type,
            unit,
        } => object(vec![
            ("kind", Value::from("real")),
            ("value", json::encode_f64(*value)),
            (
                "ifc_type",
                ifc_type.as_deref().map_or(Value::Null, Value::from),
            ),
            ("unit", unit.as_deref().map_or(Value::Null, Value::from)),
        ]),
        BimValue::Integer(value) => object(vec![
            ("kind", Value::from("integer")),
            ("value", Value::from(*value)),
        ]),
        BimValue::Bool(value) => object(vec![
            ("kind", Value::from("bool")),
            ("value", Value::Bool(*value)),
        ]),
        BimValue::Null => object(vec![("kind", Value::from("null"))]),
    }
}

pub(crate) fn encode_evidence(evidence: BimEvidence) -> Value {
    Value::from(evidence.as_str())
}

/// Decode a source block.
pub(crate) fn take_source(
    reader: &mut Reader,
    field: &'static str,
) -> Result<BimSource, SnapshotError> {
    let value = reader.take(field)?;
    let mut nested = reader.child(&value, field)?;
    let source = BimSource {
        connector: take_text(&mut nested, "connector", false)?,
        document_id: take_opt_text(&mut nested, "document_id")?,
        id: take_opt_text(&mut nested, "id")?,
        native_id: take_opt_text(&mut nested, "native_id")?,
    };
    nested.finish()?;
    Ok(source)
}

/// Decode the property-set array.
pub(crate) fn take_psets(
    reader: &mut Reader,
    field: &'static str,
) -> Result<Vec<BimPropertySet>, SnapshotError> {
    let items = reader.array(field)?;
    if items.len() > crate::bim::BIM_MAX_PSETS_PER_ELEMENT {
        return Err(budget(format!(
            "{}: {field} holds {} sets, over the {}-set budget",
            reader.context(),
            items.len(),
            crate::bim::BIM_MAX_PSETS_PER_ELEMENT
        )));
    }
    items
        .into_iter()
        .enumerate()
        .map(|(index, item)| {
            let mut set_reader =
                Reader::new(&item, format!("{}.{field}[{index}]", reader.context()))?;
            let name = take_text(&mut set_reader, "name", false)?;
            let properties = set_reader.array("properties")?;
            if properties.len() > BIM_MAX_PROPERTIES_PER_SET {
                return Err(budget(format!(
                    "{}: {} properties, over the {BIM_MAX_PROPERTIES_PER_SET}-property budget",
                    set_reader.context(),
                    properties.len()
                )));
            }
            let properties = properties
                .into_iter()
                .enumerate()
                .map(|(property_index, property)| {
                    let mut property_reader = Reader::new(
                        &property,
                        format!("{}.properties[{property_index}]", set_reader.context()),
                    )?;
                    let property_name = take_text(&mut property_reader, "name", false)?;
                    let value = take_value(&mut property_reader, "value")?;
                    property_reader.finish()?;
                    Ok(BimProperty {
                        name: property_name,
                        value,
                    })
                })
                .collect::<Result<Vec<_>, SnapshotError>>()?;
            set_reader.finish()?;
            Ok(BimPropertySet { name, properties })
        })
        .collect()
}

fn take_value(reader: &mut Reader, field: &'static str) -> Result<BimValue, SnapshotError> {
    let value = reader.take(field)?;
    let mut nested = Reader::new(&value, format!("{}.{field}", reader.context()))?;
    let kind = take_text(&mut nested, "kind", false)?;
    let value = match kind.as_str() {
        "text" => BimValue::Text(take_text(&mut nested, "value", true)?),
        "real" => BimValue::Real {
            value: nested.f64("value")?,
            ifc_type: take_opt_text(&mut nested, "ifc_type")?,
            unit: take_opt_text(&mut nested, "unit")?,
        },
        "integer" => BimValue::Integer(nested.integer("value")?),
        "bool" => BimValue::Bool(nested.bool("value")?),
        // A null value has no `value` key; `finish` refuses one, and it refuses
        // any other stray key on the other variants too.
        "null" => BimValue::Null,
        other => {
            return Err(nested.error(format!(
                "kind: unknown property value kind {other:?} \
                 (expected text, real, integer, bool or null)"
            )))
        }
    };
    nested.finish()?;
    Ok(value)
}

pub(crate) fn take_evidence(
    reader: &mut Reader,
    field: &'static str,
) -> Result<BimEvidence, SnapshotError> {
    let token = reader.string(field)?;
    BimEvidence::parse(&token).ok_or_else(|| {
        reader.error(format!(
            "{field}: unknown evidence {token:?} (expected declared or heuristic)"
        ))
    })
}

pub(crate) fn take_opt_text(
    reader: &mut Reader,
    field: &'static str,
) -> Result<Option<String>, SnapshotError> {
    match reader.opt_take(field) {
        None => Ok(None),
        Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => {
            check_text(reader, field, &text, false)?;
            Ok(Some(text))
        }
        Some(_) => Err(reader.error(format!("{field}: expected a string or null"))),
    }
}

pub(crate) fn check_text(
    reader: &Reader,
    field: &str,
    value: &str,
    allow_empty: bool,
) -> Result<(), SnapshotError> {
    if !allow_empty && value.trim().is_empty() {
        return Err(reader.error(format!("{field}: empty")));
    }
    if value.len() > BIM_MAX_STRING_BYTES {
        return Err(budget(format!(
            "{}: {field}: {} bytes over the {BIM_MAX_STRING_BYTES}-byte string budget",
            reader.context(),
            value.len()
        )));
    }
    Ok(())
}

/// A required, length-checked string.
pub(crate) fn take_text(
    reader: &mut Reader,
    field: &'static str,
    allow_empty: bool,
) -> Result<String, SnapshotError> {
    let value = reader.string(field)?;
    check_text(reader, field, &value, allow_empty)?;
    Ok(value)
}

/// A budget refusal: `ymodel.invalid_document` whose detail carries the stable
/// `ymodel.bim.budget_exceeded` prefix (ADR-0042 D7: a read-side over-budget
/// archive is an invalid document, not a different failure class).
pub(crate) fn budget(detail: impl std::fmt::Display) -> SnapshotError {
    SnapshotError::invalid(format!("{BIM_BUDGET_EXCEEDED}: {detail}"))
}

/// The UTF-8 bytes of every string inside one JSON value.
///
/// The read-side pre-check: summed over the definition rows, it bounds the text
/// this codec is willing to decode **before** it allocates the decoded rows. It
/// is smaller than the exact encoded length (no punctuation/keys), so the exact
/// check still runs on the way out.
pub(crate) fn text_bytes(value: &Value) -> usize {
    match value {
        Value::String(text) => text.len(),
        Value::Array(items) => items.iter().map(text_bytes).sum(),
        Value::Object(map) => map.values().map(text_bytes).sum(),
        _ => 0,
    }
}

/// Add the stable `ymodel.bim.invalid` prefix to a shape refusal, once.
pub(crate) fn prefixed(error: SnapshotError) -> SnapshotError {
    if error.detail.starts_with(BIM_INVALID)
        || error.detail.starts_with(BIM_BUDGET_EXCEEDED)
        || error.detail.starts_with(crate::bim::BIM_DANGLING_REFERENCE)
    {
        error
    } else {
        SnapshotError::invalid(format!("{BIM_INVALID}: {}", error.detail))
    }
}

/// A plain JSON object with exactly the given fields.
pub(crate) fn object(entries: Vec<(&str, Value)>) -> Value {
    let mut map = Map::new();
    for (name, value) in entries {
        let previous = map.insert(name.to_string(), value);
        debug_assert!(previous.is_none(), "duplicate object field {name}");
    }
    Value::Object(map)
}
