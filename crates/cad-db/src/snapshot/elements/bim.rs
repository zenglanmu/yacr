//! The optional `bim` block of one element row (ADR-0042 D2).
//!
//! ```json
//! {"bim": {
//!   "category": "IfcWall",
//!   "name": "Basic Wall:Generic 200mm",
//!   "source": {"connector": "ifc", "document_id": "2O2Fr$t4…",
//!              "id": "2O2Fr$t4…", "native_id": "22"},
//!   "level": "level:ifc:<doc>:<id>",
//!   "type": "type:ifc:<doc>:<id>",
//!   "material": "material:ifc:<doc>:<id>",
//!   "psets": [{"name": "Pset_WallCommon", "properties": [
//!     {"name": "IsExternal", "value": {"kind": "bool", "value": true}}]}],
//!   "evidence": "declared"}}
//! ```
//!
//! Two rules are the whole compatibility story (D2/D6):
//!
//! 1. an element with no BIM data **omits the key**. It is never written as
//!    `"bim": null`: a minor-5 reader is strict about unknown element fields, so
//!    a null placeholder would make the row unreadable there for no gain;
//! 2. the block is only *read* when the key exists. A minor ≤ 5 row is the same
//!    object minus this key, which decodes as [`None`] rather than an error.
//!
//! Everything else is strict, like the envelope around it: every key of the
//! block is required (`null` for an absent value), an extra key is refused, each
//! string is length-capped and every property-set/property count is capped
//! before anything is stored.

use serde_json::Value;

use super::{fields, Reader};
use crate::bim::BimElement;
use crate::snapshot::bim::{encode_evidence, encode_psets, encode_source};
use crate::snapshot::bim::{take_evidence, take_opt_text, take_psets, take_source};
use crate::snapshot::SnapshotError;

/// Encode one `bim` block.
pub(crate) fn encode(element: &BimElement) -> Result<Value, SnapshotError> {
    Ok(fields(vec![
        (
            "category",
            element.category.as_deref().map_or(Value::Null, Value::from),
        ),
        (
            "name",
            element.name.as_deref().map_or(Value::Null, Value::from),
        ),
        ("source", encode_source(&element.source)),
        (
            "level",
            element.level.as_deref().map_or(Value::Null, Value::from),
        ),
        (
            "type",
            element
                .element_type
                .as_deref()
                .map_or(Value::Null, Value::from),
        ),
        (
            "material",
            element.material.as_deref().map_or(Value::Null, Value::from),
        ),
        ("psets", encode_psets(&element.psets)),
        ("evidence", encode_evidence(element.evidence)),
    ]))
}

/// Decode one `bim` block.
pub(crate) fn decode(value: &Value) -> Result<BimElement, SnapshotError> {
    let mut reader = Reader::new(value, "cad:Element.bim")?;
    let element = BimElement {
        category: take_opt_text(&mut reader, "category")?,
        name: take_opt_text(&mut reader, "name")?,
        source: take_source(&mut reader, "source")?,
        level: take_opt_text(&mut reader, "level")?,
        element_type: take_opt_text(&mut reader, "type")?,
        material: take_opt_text(&mut reader, "material")?,
        psets: take_psets(&mut reader, "psets")?,
        evidence: take_evidence(&mut reader, "evidence")?,
    };
    reader.finish()?;
    Ok(element)
}
