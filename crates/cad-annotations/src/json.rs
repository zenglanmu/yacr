//! Shared JSON access helpers.
//!
//! Malformed sidecar values are reported, never silently coerced (audit B11).

use super::*;

/// A malformed sidecar value: reported, never silently coerced (audit B11).
pub(crate) fn corrupt(message: impl Into<String>) -> CadError {
    CadError::CorruptData(message.into())
}

/// Fetch a required field, naming it when absent.
pub(crate) fn require<'a>(object: &'a Map<String, Value>, field: &str) -> CadResult<&'a Value> {
    object
        .get(field)
        .ok_or_else(|| corrupt(format!("missing required field '{field}'")))
}

pub(crate) fn require_str<'a>(object: &'a Map<String, Value>, field: &str) -> CadResult<&'a str> {
    require(object, field)?
        .as_str()
        .ok_or_else(|| corrupt(format!("field '{field}' must be a string")))
}

pub(crate) fn require_u64(object: &Map<String, Value>, field: &str) -> CadResult<u64> {
    require(object, field)?
        .as_u64()
        .ok_or_else(|| corrupt(format!("field '{field}' must be an unsigned integer")))
}

pub(crate) fn require_f64(object: &Map<String, Value>, field: &str) -> CadResult<f64> {
    require(object, field)?
        .as_f64()
        .ok_or_else(|| corrupt(format!("field '{field}' must be a number")))
}

/// Parse the raw annotation list out of [`NESTED_EXTENSIONS_KEY`].
///
/// The side-band is an object with a `version` and an `annotations` array;
/// wrapping the array avoids JSON-string escaping of the appended payload.
pub(crate) fn parse_nested_extensions(raw: &str) -> CadResult<Value> {
    let payload: Value = serde_json::from_str(raw).map_err(|e| {
        corrupt(format!(
            "extension field '{NESTED_EXTENSIONS_KEY}' is not valid JSON: {e}"
        ))
    })?;
    let object = payload.as_object().ok_or_else(|| {
        corrupt(format!(
            "extension field '{NESTED_EXTENSIONS_KEY}' must be an object"
        ))
    })?;
    let version = object
        .get("version")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            corrupt(format!(
                "extension field '{NESTED_EXTENSIONS_KEY}' has no version"
            ))
        })?;
    if version != u64::from(NESTED_EXTENSIONS_VERSION) {
        return Err(corrupt(format!(
            "extension field '{NESTED_EXTENSIONS_KEY}' has unsupported version {version}"
        )));
    }
    match object.get("annotations") {
        Some(annotations) if annotations.is_array() => Ok(annotations.clone()),
        _ => Err(corrupt(format!(
            "extension field '{NESTED_EXTENSIONS_KEY}' has no annotations array"
        ))),
    }
}
