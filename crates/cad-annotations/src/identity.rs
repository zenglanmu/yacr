//! Document fingerprint and annotation id encoding.
//!
//! Fingerprints and ids are decoded with strict, exact typing (audit B11): a
//! missing, malformed or wrong-typed value is corruption, never coerced into a
//! plausible-looking identity.

use super::*;

/// Decode a document fingerprint with strict, exact typing (audit B11).
///
/// A missing, malformed or wrong-typed fingerprint is corruption; it is never
/// coerced into `Temporary(0)`.
pub(crate) fn decode_identity(value: Option<&Value>) -> CadResult<DocumentIdentity> {
    match value {
        Some(Value::String(s)) => parse_uuid_strict(s)
            .map(DocumentIdentity::Temporary)
            .ok_or_else(|| corrupt("document_fingerprint is not a valid UUID string")),
        Some(Value::Array(a)) => {
            // A SHA-256 fingerprint is exactly 32 bytes in 0..=255; a short or
            // over-long array must not be zero-padded or truncated into a
            // plausible-looking identity (audit B11).
            if a.len() != 32 {
                return Err(corrupt("document_fingerprint must be 32 bytes"));
            }
            let mut bytes = [0u8; 32];
            for (i, v) in a.iter().enumerate() {
                let n = v
                    .as_u64()
                    .ok_or_else(|| corrupt("document_fingerprint byte is not an integer"))?;
                bytes[i] = u8::try_from(n)
                    .map_err(|_| corrupt("document_fingerprint byte out of range"))?;
            }
            Ok(DocumentIdentity::Sha256(bytes))
        }
        Some(_) => Err(corrupt(
            "document_fingerprint must be a UUID string or a 32-byte array",
        )),
        None => Err(corrupt("annotation file has no document_fingerprint")),
    }
}

pub(crate) fn encode_identity(identity: &DocumentIdentity) -> Value {
    match identity {
        DocumentIdentity::Sha256(bytes) => Value::Array(bytes.iter().map(|b| json!(*b)).collect()),
        DocumentIdentity::Temporary(bits) => json!(uuid_string(*bits)),
    }
}

/// Decode an annotation id with strict UUID validation (audit B11).
///
/// A malformed id is an error, not a silent `AnnotationId(0)`.
pub(crate) fn decode_id(value: &Value) -> CadResult<u128> {
    let s = value
        .as_str()
        .ok_or_else(|| corrupt("annotation id must be a string"))?;
    parse_uuid_strict(s).ok_or_else(|| corrupt("annotation id is not a valid UUID"))
}
