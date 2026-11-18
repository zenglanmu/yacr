//! The numeric codec shared by every ymodel JSON entry (ADR-0037 D9).
//!
//! The format rule this module enforces: **JSON has no NaN and no infinity**,
//! and a document must never turn a stored non-finite value into a different
//! number. So:
//!
//! * every finite `f64` is written as a JSON number, using the shortest
//!   representation that parses back to the same bits;
//! * `NaN`, `+inf` and `-inf` are written as the documented escape
//!   `{"$f64": "nan"}`, `{"$f64": "inf"}`, `{"$f64": "-inf"}`;
//! * reading accepts a JSON number or exactly that escape. Any other object,
//!   any other key and any other escape string is an explicit error — there is
//!   no default value and no silent `0.0`. This is what lets a legacy row (see
//!   `RestoreStored` in `cad-db`) keep its stored `NaN` verbatim instead of
//!   failing to save, while an unknown escape still refuses the document.
//!
//! # Canonical key order
//!
//! `serde_json` is built here **without** the `preserve_order` feature, so its
//! `Map` is a `BTreeMap` and object keys serialize in sorted order. That is the
//! format's canonical key order: two writers given the same fields produce the
//! same bytes, and a re-encoding of a decoded payload is byte-stable. Do not
//! enable `preserve_order`; it would make the file's bytes depend on insertion
//! order and break that fixed point.
//!
//! # Why not strings everywhere
//!
//! ADR-0037 D5 keeps every identifier ≤ 2^53 so JSON numbers stay exact in
//! `f64`-based tooling (`jq`, browser `JSON.parse`). The same reasoning applies
//! to ordinary geometry scalars; only values that genuinely exceed 2^53 (a
//! `u128` identity) are written as decimal strings by their owning codec.

use std::fmt;

use serde_json::{Map, Number, Value};

use crate::Point3;

/// A numeric-codec failure.
///
/// Every variant names the field and the expected shape so a refused document
/// says which entry and which value were wrong; none of them carries a
/// recovery default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonError {
    /// A value that is neither a JSON number nor the `$f64` escape object.
    NotANumber,
    /// The `$f64` escape string is not one of `nan` / `inf` / `-inf`.
    UnknownEscape(String),
    /// An object used as a number has a key other than `$f64` (or more than one
    /// key). The payload lists what was found, sorted.
    UnknownEscapeKey(String),
    /// `$f64` maps to something that is not a string.
    EscapeNotAString,
    /// A value that had to be a JSON object is not one.
    ExpectedObject {
        /// Field being decoded.
        field: &'static str,
    },
    /// A required field is absent.
    MissingField {
        /// Field being decoded.
        field: &'static str,
    },
    /// An object carries a key this codec does not model. Phase 1 refuses it:
    /// the writer re-serializes whole entries and cannot preserve fields it does
    /// not model (ADR-0037 D11).
    UnknownField {
        /// Object being decoded.
        field: &'static str,
        /// The first unexpected key, in sorted order.
        found: String,
    },
    /// A value has the wrong JSON type.
    WrongType {
        /// Field being decoded.
        field: &'static str,
        /// The expected shape, for diagnostics.
        expected: &'static str,
    },
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JsonError::NotANumber => write!(f, "expected a JSON number or the $f64 escape"),
            JsonError::UnknownEscape(tag) => {
                write!(f, "unknown $f64 escape {tag:?} (expected nan, inf or -inf)")
            }
            JsonError::UnknownEscapeKey(keys) => write!(
                f,
                "a number escape must be the single key \"$f64\"; found object keys [{keys}]"
            ),
            JsonError::EscapeNotAString => write!(f, "\"$f64\" must map to a string"),
            JsonError::ExpectedObject { field } => {
                write!(f, "{field}: expected a JSON object")
            }
            JsonError::MissingField { field } => write!(f, "{field}: required field is missing"),
            JsonError::UnknownField { field, found } => write!(
                f,
                "{field}: unknown field {found:?} (this writer cannot preserve it)"
            ),
            JsonError::WrongType { field, expected } => {
                write!(f, "{field}: expected {expected}")
            }
        }
    }
}

impl std::error::Error for JsonError {}

/// Encode a finite `f64` as a JSON number, or a non-finite one as the `$f64`
/// escape.
pub fn encode_f64(value: f64) -> Value {
    match Number::from_f64(value) {
        Some(number) => Value::Number(number),
        None => {
            let tag = if value.is_nan() {
                "nan"
            } else if value > 0.0 {
                "inf"
            } else {
                "-inf"
            };
            let mut map = Map::new();
            map.insert("$f64".to_string(), Value::String(tag.to_string()));
            Value::Object(map)
        }
    }
}

/// Decode a value written by [`encode_f64`].
///
/// A JSON number decodes to `f64`. The escape object must be exactly
/// `{"$f64": "nan" | "inf" | "-inf"}`; everything else is refused, including an
/// unknown escape string and an object with another key.
pub fn decode_f64(value: &Value) -> Result<f64, JsonError> {
    if let Some(number) = value.as_f64() {
        return Ok(number);
    }
    let Value::Object(map) = value else {
        return Err(JsonError::NotANumber);
    };
    if map.len() != 1 {
        return Err(JsonError::UnknownEscapeKey(joined_keys(map)));
    }
    match map.get("$f64") {
        Some(Value::String(tag)) => match tag.as_str() {
            "nan" => Ok(f64::NAN),
            "inf" => Ok(f64::INFINITY),
            "-inf" => Ok(f64::NEG_INFINITY),
            other => Err(JsonError::UnknownEscape(other.to_string())),
        },
        Some(_) => Err(JsonError::EscapeNotAString),
        None => Err(JsonError::UnknownEscapeKey(joined_keys(map))),
    }
}

/// Encode a point as `{"x": …, "y": …, "z": …}`.
pub fn encode_point3(point: Point3) -> Value {
    let mut map = Map::new();
    map.insert("x".to_string(), encode_f64(point.x));
    map.insert("y".to_string(), encode_f64(point.y));
    map.insert("z".to_string(), encode_f64(point.z));
    Value::Object(map)
}

/// Decode a point written by [`encode_point3`].
///
/// The object must carry exactly `x`, `y` and `z`; a missing component and an
/// extra key are both errors.
pub fn decode_point3(value: &Value) -> Result<Point3, JsonError> {
    let Value::Object(map) = value else {
        return Err(JsonError::ExpectedObject { field: "point3" });
    };
    let mut map = map.clone();
    let x = take(map.remove("x"), "point3.x")?;
    let y = take(map.remove("y"), "point3.y")?;
    let z = take(map.remove("z"), "point3.z")?;
    finish_map(&map, "point3")?;
    Ok(Point3 { x, y, z })
}

/// Encode a two-component vector as a JSON array `[a, b]`.
pub fn encode_f64x2(value: [f64; 2]) -> Value {
    Value::Array(vec![encode_f64(value[0]), encode_f64(value[1])])
}

/// Decode a two-component vector written by [`encode_f64x2`].
pub fn decode_f64x2(value: &Value) -> Result<[f64; 2], JsonError> {
    let Value::Array(items) = value else {
        return Err(JsonError::WrongType {
            field: "f64x2",
            expected: "an array of two numbers",
        });
    };
    if items.len() != 2 {
        return Err(JsonError::WrongType {
            field: "f64x2",
            expected: "an array of two numbers",
        });
    }
    Ok([decode_f64(&items[0])?, decode_f64(&items[1])?])
}

/// Encode an optional scalar: `null` for `None`, the number codec otherwise.
pub fn encode_opt_f64(value: Option<f64>) -> Value {
    value.map_or(Value::Null, encode_f64)
}

/// Decode an optional scalar written by [`encode_opt_f64`].
pub fn decode_opt_f64(value: &Value) -> Result<Option<f64>, JsonError> {
    if value.is_null() {
        return Ok(None);
    }
    Ok(Some(decode_f64(value)?))
}

/// Encode an optional point: `null` for `None`, the point codec otherwise.
pub fn encode_opt_point3(value: Option<Point3>) -> Value {
    value.map_or(Value::Null, encode_point3)
}

/// Decode an optional point written by [`encode_opt_point3`].
pub fn decode_opt_point3(value: &Value) -> Result<Option<Point3>, JsonError> {
    if value.is_null() {
        return Ok(None);
    }
    Ok(Some(decode_point3(value)?))
}

fn take(value: Option<Value>, field: &'static str) -> Result<f64, JsonError> {
    match value {
        // The inner error already names the offending value (an unknown escape
        // string, a wrong key set); only the "key absent" case needs the path.
        Some(value) => decode_f64(&value),
        None => Err(JsonError::MissingField { field }),
    }
}

fn joined_keys(map: &Map<String, Value>) -> String {
    map.keys().cloned().collect::<Vec<_>>().join(", ")
}

fn finish_map(map: &Map<String, Value>, field: &'static str) -> Result<(), JsonError> {
    match map.keys().next() {
        Some(key) => Err(JsonError::UnknownField {
            field,
            found: key.clone(),
        }),
        None => Ok(()),
    }
}

/// A strict reader for one JSON object.
///
/// Fields are *taken* by name: a required field that is absent is
/// [`JsonError::MissingField`], and [`ObjectReader::finish`] refuses the object
/// if anything is left over. That is the whole "unknown fields are refused"
/// rule of ADR-0037 D11 — the writer re-serializes whole entries, so a field it
/// does not model would be dropped on the next save.
///
/// The reader is generic over the value types: [`json`](self) supplies the
/// numeric codecs, `ymodel::payload` extends it with the geometry-shaped takes.
pub(crate) struct ObjectReader {
    map: Map<String, Value>,
    context: &'static str,
}

impl ObjectReader {
    /// Start reading `value`, which must be a JSON object.
    pub(crate) fn new(value: &Value, context: &'static str) -> Result<Self, JsonError> {
        match value {
            Value::Object(map) => Ok(ObjectReader {
                map: map.clone(),
                context,
            }),
            _ => Err(JsonError::ExpectedObject { field: context }),
        }
    }

    /// Read the `class` discriminator without consuming it.
    pub(crate) fn class(&self) -> Result<&str, JsonError> {
        match self.map.get("class") {
            Some(Value::String(class)) => Ok(class),
            Some(_) => Err(JsonError::WrongType {
                field: "class",
                expected: "a string",
            }),
            None => Err(JsonError::MissingField { field: "class" }),
        }
    }

    /// Take a required field's raw value, leaving the rest for [`Self::finish`].
    pub(crate) fn take(&mut self, field: &'static str) -> Result<Value, JsonError> {
        self.map
            .remove(field)
            .ok_or(JsonError::MissingField { field })
    }

    /// Refuse the object if any field was not taken.
    pub(crate) fn finish(self) -> Result<(), JsonError> {
        finish_map(&self.map, self.context)
    }

    /// A required scalar.
    pub(crate) fn f64(&mut self, field: &'static str) -> Result<f64, JsonError> {
        decode_f64(&self.take(field)?)
    }

    /// A scalar that is `null` or a number.
    ///
    /// The key itself is required: a payload object always writes every
    /// descriptor field (`null` for `None`), so an absent key means the entry
    /// was not produced by this writer.
    pub(crate) fn opt_f64(&mut self, field: &'static str) -> Result<Option<f64>, JsonError> {
        decode_opt_f64(&self.take(field)?)
    }

    /// A required point.
    pub(crate) fn point3(&mut self, field: &'static str) -> Result<Point3, JsonError> {
        decode_point3(&self.take(field)?)
    }

    /// A point that is `null` or an object (the key is required; see
    /// [`Self::opt_f64`]).
    pub(crate) fn opt_point3(&mut self, field: &'static str) -> Result<Option<Point3>, JsonError> {
        decode_opt_point3(&self.take(field)?)
    }

    /// A required two-component vector.
    pub(crate) fn f64x2(&mut self, field: &'static str) -> Result<[f64; 2], JsonError> {
        decode_f64x2(&self.take(field)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip_bits(value: f64) {
        let encoded = encode_f64(value);
        let text = serde_json::to_string(&encoded).expect("serialize");
        let parsed: Value = serde_json::from_str(&text).expect("parse");
        let decoded = decode_f64(&parsed).expect("decode");
        assert_eq!(
            decoded.to_bits(),
            value.to_bits(),
            "{value:?} -> {text} -> {decoded:?}"
        );
    }

    #[test]
    fn finite_values_are_bit_exact_through_a_json_string() {
        for value in [
            0.0,
            -0.0,
            0.1,
            1.0 / 3.0,
            f64::MAX,
            f64::MIN,
            f64::MIN_POSITIVE,
            5e-324,
            1e308,
            -1e-308,
            2.2250738585072014e-308,
        ] {
            round_trip_bits(value);
        }
        assert_eq!((-0.0f64).to_bits(), 0x8000_0000_0000_0000);
    }

    #[test]
    fn non_finite_values_use_the_documented_escape() {
        for (value, tag) in [
            (f64::NAN, "nan"),
            (f64::INFINITY, "inf"),
            (f64::NEG_INFINITY, "-inf"),
        ] {
            let encoded = encode_f64(value);
            assert_eq!(
                encoded,
                serde_json::json!({ "$f64": tag }),
                "{value:?} must use the {tag} escape"
            );
            let text = serde_json::to_string(&encoded).expect("serialize");
            let parsed: Value = serde_json::from_str(&text).expect("parse");
            let decoded = decode_f64(&parsed).expect("decode");
            if value.is_nan() {
                assert!(decoded.is_nan());
            } else {
                assert_eq!(decoded, value);
            }
        }
    }

    #[test]
    fn unknown_escapes_are_refused_not_defaulted() {
        let error = decode_f64(&serde_json::json!({ "$f64": "snan" })).expect_err("unknown escape");
        assert_eq!(error, JsonError::UnknownEscape("snan".to_string()));
        let error = decode_f64(&serde_json::json!({ "$f64": 1 })).expect_err("not a string");
        assert_eq!(error, JsonError::EscapeNotAString);
        // A different key is not a number escape.
        let error = decode_f64(&serde_json::json!({ "$float": "nan" })).expect_err("wrong key");
        assert!(matches!(error, JsonError::UnknownEscapeKey(_)), "{error:?}");
        // Two keys.
        let error =
            decode_f64(&serde_json::json!({ "$f64": "nan", "x": 1 })).expect_err("two keys");
        assert!(matches!(error, JsonError::UnknownEscapeKey(_)), "{error:?}");
        // A string is not a number either.
        assert_eq!(
            decode_f64(&Value::String("1.5".into())).expect_err("string"),
            JsonError::NotANumber
        );
    }

    #[test]
    fn points_round_trip_and_reject_extra_keys() {
        let point = Point3 {
            x: 1.5,
            y: f64::INFINITY,
            z: -0.0,
        };
        let text = serde_json::to_string(&encode_point3(point)).expect("serialize");
        assert!(text.contains("\"$f64\":\"inf\""), "{text}");
        // Keys are sorted by the BTreeMap-backed map, which is the canonical order.
        assert!(text.starts_with("{\"x\":"), "{text}");
        let parsed: Value = serde_json::from_str(&text).expect("parse");
        let decoded = decode_point3(&parsed).expect("decode");
        assert_eq!(decoded.x, point.x);
        assert!(decoded.y.is_infinite());
        assert_eq!(decoded.z.to_bits(), point.z.to_bits());

        let error = decode_point3(&serde_json::json!({ "x": 0.0, "y": 0.0, "z": 0.0, "w": 1.0 }))
            .expect_err("extra key");
        assert_eq!(
            error,
            JsonError::UnknownField {
                field: "point3",
                found: "w".to_string()
            }
        );
        let error = decode_point3(&serde_json::json!({ "x": 0.0, "y": 0.0 })).expect_err("missing");
        assert_eq!(error, JsonError::MissingField { field: "point3.z" });
    }

    #[test]
    fn optional_scalars_and_pairs_round_trip() {
        assert!(encode_opt_f64(None).is_null());
        assert_eq!(decode_opt_f64(&Value::Null).expect("null"), None);
        let value = decode_opt_f64(&encode_opt_f64(Some(f64::NAN))).expect("nan");
        assert!(value.expect("some").is_nan());

        assert!(encode_opt_point3(None).is_null());
        assert_eq!(decode_opt_point3(&Value::Null).expect("null"), None);

        let pair = [f64::MIN_POSITIVE, -1.5];
        let encoded = encode_f64x2(pair);
        let text = serde_json::to_string(&encoded).expect("serialize");
        let parsed: Value = serde_json::from_str(&text).expect("parse");
        assert_eq!(decode_f64x2(&parsed).expect("decode"), pair);
        assert!(decode_f64x2(&serde_json::json!([1.0, 2.0, 3.0])).is_err());
    }
}
