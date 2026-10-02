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
