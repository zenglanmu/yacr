//! Small pieces every entry codec in this crate shares: a strict object reader,
//! lowercase hex and the `sha256` helper.
//!
//! The reader mirrors `cad_domain::ymodel::json::ObjectReader` (which is
//! crate-private there): fields are *taken* by name and [`Reader::finish`]
//! refuses anything left over, because the writer re-serializes whole entries
//! and cannot preserve a field it does not model (ADR-0037 D11). Duplicating the
//! rule here rather than exporting the domain type keeps the domain's API
//! surface small; the two must stay behaviourally identical.

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use cad_db::StoreError;

/// A strict reader for one JSON object.
pub(crate) struct Reader {
    map: Map<String, Value>,
    context: &'static str,
}

impl Reader {
    /// Start reading `value`, which must be a JSON object.
    pub(crate) fn new(value: &Value, context: &'static str) -> Result<Reader, StoreError> {
        match value {
            Value::Object(map) => Ok(Reader {
                map: map.clone(),
                context,
            }),
            _ => Err(malformed(context, "expected a JSON object")),
        }
    }

    /// Take one field's raw value.
    pub(crate) fn take(&mut self, field: &'static str) -> Result<Value, StoreError> {
        self.map
            .remove(field)
            .ok_or_else(|| malformed(self.context, format!("required field {field:?} is missing")))
    }

    /// Take a required string.
    pub(crate) fn string(&mut self, field: &'static str) -> Result<String, StoreError> {
        match self.take(field)? {
            Value::String(text) => Ok(text),
            _ => Err(malformed(
                self.context,
                format!("{field:?} must be a string"),
            )),
        }
    }

    /// Take a string that may be `null`. The key itself is required: this writer
    /// always emits every field.
    pub(crate) fn opt_string(&mut self, field: &'static str) -> Result<Option<String>, StoreError> {
        match self.take(field)? {
            Value::String(text) => Ok(Some(text)),
            Value::Null => Ok(None),
            _ => Err(malformed(
                self.context,
                format!("{field:?} must be a string or null"),
            )),
        }
    }

    /// Take a string that may be `null` **or absent**; both mean "no value".
    ///
    /// Only a format field explicitly documented as optional may use this: the
    /// reader otherwise refuses a missing field, because the writer emits every
    /// field it models.
    pub(crate) fn string_if_present(
        &mut self,
        field: &'static str,
    ) -> Result<Option<String>, StoreError> {
        if !self.map.contains_key(field) {
            return Ok(None);
        }
        self.opt_string(field)
    }

    /// Take a required non-negative integer.
    pub(crate) fn u64(&mut self, field: &'static str) -> Result<u64, StoreError> {
        match self.take(field)? {
            Value::Number(number) => number.as_u64().ok_or_else(|| {
                malformed(
                    self.context,
                    format!("{field:?} must be a non-negative integer"),
                )
            }),
            _ => Err(malformed(
                self.context,
                format!("{field:?} must be a non-negative integer"),
            )),
        }
    }

    /// Take a required boolean.
    pub(crate) fn bool(&mut self, field: &'static str) -> Result<bool, StoreError> {
        match self.take(field)? {
            Value::Bool(value) => Ok(value),
            _ => Err(malformed(
                self.context,
                format!("{field:?} must be a boolean"),
            )),
        }
    }

    /// Take a required array.
    pub(crate) fn array(&mut self, field: &'static str) -> Result<Vec<Value>, StoreError> {
        match self.take(field)? {
            Value::Array(items) => Ok(items),
            _ => Err(malformed(
                self.context,
                format!("{field:?} must be an array"),
            )),
        }
    }

    /// Take a required object.
    pub(crate) fn object(&mut self, field: &'static str) -> Result<Value, StoreError> {
        match self.take(field)? {
            value @ Value::Object(_) => Ok(value),
            _ => Err(malformed(
                self.context,
                format!("{field:?} must be an object"),
            )),
        }
    }

    /// Refuse the object if any field was not taken.
    pub(crate) fn finish(self) -> Result<(), StoreError> {
        match self.map.keys().next() {
            Some(found) => Err(malformed(
                self.context,
                format!("unknown field {found:?} (this writer cannot preserve it)"),
            )),
            None => Ok(()),
        }
    }
}

/// A malformed-archive error, i.e. [`cad_domain::ymodel::markers::INVALID_DOCUMENT`].
///
/// `detail` is anything printable (a `&str`, a format argument, or the error type
/// of a lower layer) so a refused document always names what was wrong.
pub(crate) fn malformed(context: &str, detail: impl std::fmt::Display) -> StoreError {
    StoreError::invalid(format!("{context}: {detail}"))
}

/// Lowercase hexadecimal, the one spelling of every hash in this format.
pub(crate) fn hex_lower(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    out
}

/// Lowercase hex SHA-256 of one entry's uncompressed bytes.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256Sum::new();
    hasher.update(bytes);
    hasher.hex()
}

/// A streaming SHA-256, for entry bodies that must not be buffered (a preserved
/// `raw/source-*.bin` can be hundreds of megabytes).
pub(crate) struct Sha256Sum(Sha256);

impl Sha256Sum {
    pub(crate) fn new() -> Sha256Sum {
        Sha256Sum(Sha256::new())
    }

    pub(crate) fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    /// Lowercase hexadecimal digest.
    pub(crate) fn hex(self) -> String {
        hex_lower(&self.0.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_is_lowercase_and_padded() {
        assert_eq!(hex_lower(&[0x00, 0x0f, 0xa5, 0xff]), "000fa5ff");
    }

    #[test]
    fn sha256_matches_the_known_vector() {
        // The empty-string vector every crypto library agrees on.
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn the_reader_takes_by_name_and_refuses_leftovers() {
        let value = serde_json::json!({"a": "x", "b": 7});
        let mut reader = Reader::new(&value, "t").expect("object");
        assert_eq!(reader.string("a").expect("a"), "x");
        assert_eq!(reader.u64("b").expect("b"), 7);
        reader.finish().expect("no leftovers");

        let mut reader = Reader::new(&value, "t").expect("object");
        assert!(reader.string("b").is_err(), "wrong type is refused");
        assert!(reader.finish().is_err(), "an untaken field is refused");

        let mut reader = Reader::new(&value, "t").expect("object");
        assert!(reader.string("missing").is_err());
        assert!(Reader::new(&Value::Null, "t").is_err());
    }

    #[test]
    fn null_is_only_accepted_for_optional_strings() {
        let value = serde_json::json!({"a": null});
        let mut reader = Reader::new(&value, "t").expect("object");
        assert_eq!(reader.opt_string("a").expect("null"), None);
        let value = serde_json::json!({"a": null});
        let mut reader = Reader::new(&value, "t").expect("object");
        assert!(reader.string("a").is_err());
    }
}
