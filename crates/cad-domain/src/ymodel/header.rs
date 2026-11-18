//! The `document.json` entry: units, document identity, resource keys and the
//! persisted identifier high-water marks (ADR-0037 D5, D9, D10).
//!
//! The format rules this module enforces:
//!
//! 1. **The identifier counters are the allocator.** They are the next sequence
//!    number of every per-document family, persisted and restored verbatim.
//!    They are never rebuilt from `max(id) + 1`: deleting the largest id and
//!    reopening would hand that id to a new object, which the "never reuse"
//!    rule forbids. A counter is therefore `1 ..= MAX_SEQ + 1`, and
//!    [`DocumentHeader::mint`] fails at the ceiling instead of wrapping.
//! 2. **Counters are JSON numbers.** [`MAX_SEQ`] is 2^45 - 1 and the ceiling
//!    `MAX_SEQ + 1` is 2^45, far below 2^53, so a counter is exactly
//!    representable by every `f64`-backed JSON reader (`jq`, browser
//!    `JSON.parse`) — ADR-0037 D5's reason for keeping identifiers under 2^53.
//! 3. **A `u128` identity is a decimal string.** `DocumentIdentity::Temporary`
//!    and `DatabaseId` have no such bound, so they cross the format as decimal
//!    strings rather than as numbers a reader could round.
//! 4. **Units are named tokens**, never numbers: `"drawing"`, `"mm"`, `"m"`,
//!    `"in"`, `"ft"`. An unknown token refuses the document; there is no
//!    "assume millimetres" fallback (that assumption is the bug ADR-0037 D9
//!    pins as B31).
//!
//! # `DocumentIdentity`
//!
//! The type itself already lives at [`crate::DocumentIdentity`] and is the one
//! `cad_app::Document` holds. This module **re-exports** it instead of defining
//! a second copy: two `DocumentIdentity` types that can diverge would be a
//! format bug waiting to happen. What this module adds is the JSON codec; a
//! later slice that moves `cad_app::Document` onto `DocumentHeader` re-exports
//! the same type from here.

use std::fmt;

use serde_json::{Map, Number, Value};

use crate::ymodel::id::{IdKind, MAX_SEQ};
use crate::ymodel::json::{JsonError, ObjectReader};
use crate::{DatabaseId, Unit, UnitContext};

pub use crate::DocumentIdentity;

/// One past the largest sequence number a persisted identifier can carry.
pub const COUNTER_CEILING: u128 = MAX_SEQ + 1;

/// The nine per-document identifier high-water marks (ADR-0037 D5).
///
/// Each value is the sequence number the allocator will hand out **next**, so a
/// freshly created document starts every counter at `1` (sequence `0` is
/// reserved: model space is `ModelKey::ModelSpace`, and `LayoutId(0)`/
/// `BlockId(0)` are never allocated). The ceiling is [`COUNTER_CEILING`]: after
/// the last mint the counter holds `MAX_SEQ + 1` and further mints fail.
///
/// The order of the fields mirrors the adr's list; the JSON object is keyed by
/// name, and `serde_json`'s `BTreeMap` map sorts those keys, so the bytes do not
/// depend on this declaration order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdCounters {
    pub entity: u128,
    pub block: u128,
    pub layer: u128,
    pub layout: u128,
    pub style: u128,
    pub dim_style: u128,
    pub mleader_style: u128,
    pub linetype: u128,
    pub scale: u128,
}

impl Default for IdCounters {
    /// The state of a brand-new document: every family starts at sequence 1.
    fn default() -> Self {
        IdCounters {
            entity: 1,
            block: 1,
            layer: 1,
            layout: 1,
            style: 1,
            dim_style: 1,
            mleader_style: 1,
            linetype: 1,
            scale: 1,
        }
    }
}

impl IdCounters {
    /// The counter behind a per-document identifier family.
    ///
    /// `None` for the families that are **not** allocated per document: a Model
    /// id is derived from its layout/block, and SubCategory/Relationship/Aspect
    /// sequences are structural. Those have no high-water mark to persist.
    fn slot_mut(&mut self, kind: IdKind) -> Option<&mut u128> {
        match kind {
            IdKind::Entity => Some(&mut self.entity),
            IdKind::Block => Some(&mut self.block),
            IdKind::Layer => Some(&mut self.layer),
            IdKind::Layout => Some(&mut self.layout),
            IdKind::Style => Some(&mut self.style),
            IdKind::DimStyle => Some(&mut self.dim_style),
            IdKind::MLeaderStyle => Some(&mut self.mleader_style),
            IdKind::Linetype => Some(&mut self.linetype),
            IdKind::Scale => Some(&mut self.scale),
            IdKind::Model | IdKind::SubCategory | IdKind::Relationship | IdKind::Aspect => None,
        }
    }

    /// The counter value behind a per-document family (see
    /// [`DocumentHeader::next_id`]).
    pub fn counter(&self, kind: IdKind) -> Option<u128> {
        let mut copy = *self;
        copy.slot_mut(kind).map(|slot| *slot)
    }

    /// Refuse a counter that is outside `1 ..= MAX_SEQ + 1`.
    ///
    /// This is the load-time check: a document claiming `0` would hand out a
    /// reserved sequence, and one above the ceiling would produce an identifier
    /// this format cannot store. Neither is clamped — the document is refused.
    pub fn validate(&self) -> Result<(), HeaderError> {
        for (counter, value) in self.named() {
            if !(1..=COUNTER_CEILING).contains(&value) {
                return Err(HeaderError::CounterOutOfRange { counter, value });
            }
        }
        Ok(())
    }

    /// The counters with their format names, for validation and diagnostics.
    fn named(&self) -> [(&'static str, u128); 9] {
        [
            ("entity", self.entity),
            ("block", self.block),
            ("layer", self.layer),
            ("layout", self.layout),
            ("style", self.style),
            ("dim_style", self.dim_style),
            ("mleader_style", self.mleader_style),
            ("linetype", self.linetype),
            ("scale", self.scale),
        ]
    }

    /// The `document.json` form: a JSON number per counter.
    ///
    /// Numbers, not strings: see [`COUNTER_CEILING`] and the module docs (ADR
    /// D5 — every value stays exactly representable in an `f64`-backed reader).
    pub fn to_json_value(&self) -> Value {
        let mut map = Map::new();
        for (counter, value) in self.named() {
            map.insert(counter.to_string(), counter_value(value));
        }
        Value::Object(map)
    }

    /// Read the counters, refusing an unknown key or a non-integer value.
    pub fn from_json_value(value: &Value) -> Result<Self, HeaderError> {
        let mut reader = ObjectReader::new(value, "counters")?;
        let mut counters = IdCounters::default();
        for (counter, _) in counters.named() {
            let raw = reader.take(counter)?;
            let parsed =
                raw.as_u64()
                    .ok_or(HeaderError::NotAnInteger { field: counter })? as u128;
            counters.apply(counter, parsed)?;
        }
        reader.finish()?;
        Ok(counters)
    }

    fn apply(&mut self, counter: &'static str, value: u128) -> Result<(), HeaderError> {
        if value > COUNTER_CEILING {
            return Err(HeaderError::CounterOutOfRange { counter, value });
        }
        *self.slot_by_name(counter) = value;
        Ok(())
    }

    fn slot_by_name(&mut self, counter: &'static str) -> &mut u128 {
        match counter {
            "entity" => &mut self.entity,
            "block" => &mut self.block,
            "layer" => &mut self.layer,
            "layout" => &mut self.layout,
            "style" => &mut self.style,
            "dim_style" => &mut self.dim_style,
            "mleader_style" => &mut self.mleader_style,
            "linetype" => &mut self.linetype,
            "scale" => &mut self.scale,
            other => unreachable!("unknown counter {other}"),
        }
    }
}

/// The `document.json` header: everything about a document that is not an
/// element row.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentHeader {
    /// The in-memory database identity. Written as a decimal string because a
    /// `u128` has no 2^53 bound.
    pub database: DatabaseId,
    /// The source/display unit context and its exact conversion.
    pub units: UnitContext,
    /// The document's content identity (temporary serial or source `sha256`).
    pub identity: DocumentIdentity,
    /// The logical resource keys this document references (fonts, images),
    /// sorted or not — the header preserves the caller's order verbatim.
    pub resource_keys: Vec<String>,
    /// The next sequence number of every allocated family.
    pub next: IdCounters,
}

impl DocumentHeader {
    /// The current high-water mark of a per-document family.
    ///
    /// `None` for a family that is not allocated per document (Model,
    /// SubCategory, Relationship, Aspect); those derive their sequence from the
    /// row that owns them, so they have no counter to advance.
    pub fn next_id(&self, kind: IdKind) -> Option<u128> {
        self.next.counter(kind)
    }

    /// Allocate one sequence number of `kind`, then advance its counter.
    ///
    /// The returned value is the one the counter held; the counter saturates at
    /// [`COUNTER_CEILING`]. Once it is there — i.e. the last call returned
    /// `MAX_SEQ` — every further call fails with
    /// [`HeaderError::CannotMint`]. Wrapping or reusing a sequence would break
    /// both "ids are never reused" and the shard rule.
    pub fn mint(&mut self, kind: IdKind) -> Result<u128, HeaderError> {
        let counter = match self.next.slot_mut(kind) {
            Some(slot) => *slot,
            None => {
                return Err(HeaderError::NotAllocated(kind));
            }
        };
        if !(1..=MAX_SEQ).contains(&counter) {
            return Err(HeaderError::CannotMint { kind, counter });
        }
        if let Some(slot) = self.next.slot_mut(kind) {
            *slot = counter + 1;
        }
        Ok(counter)
    }

    /// The `document.json` object.
    pub fn to_json_value(&self) -> Value {
        let mut map = Map::new();
        map.insert(
            "database".to_string(),
            Value::String(self.database.0.to_string()),
        );
        map.insert("units".to_string(), self.units.to_json_value());
        map.insert("identity".to_string(), self.identity.to_json_value());
        map.insert(
            "resource_keys".to_string(),
            Value::Array(
                self.resource_keys
                    .iter()
                    .map(|key| Value::String(key.clone()))
                    .collect(),
            ),
        );
        map.insert("next".to_string(), self.next.to_json_value());
        Value::Object(map)
    }

    /// Read a `document.json` object. Unknown keys, missing keys and malformed
    /// values are all refused.
    pub fn from_json_value(value: &Value) -> Result<Self, HeaderError> {
        let mut reader = ObjectReader::new(value, "document")?;
        let database = database_from_value(&reader.take("database")?)?;
        let units = UnitContext::from_json_value(&reader.take("units")?)?;
        let identity = DocumentIdentity::from_json_value(&reader.take("identity")?)?;
        let resource_keys = strings_from_value(&reader.take("resource_keys")?)?;
        let next = IdCounters::from_json_value(&reader.take("next")?)?;
        reader.finish()?;
        let header = DocumentHeader {
            database,
            units,
            identity,
            resource_keys,
            next,
        };
        header.next.validate()?;
        Ok(header)
    }
}

impl UnitContext {
    /// The unit context as a `document.json` object.
    pub fn to_json_value(&self) -> Value {
        let mut map = Map::new();
        map.insert("source".to_string(), unit_to_value(&self.source));
        map.insert("display".to_string(), unit_to_value(&self.display));
        map.insert(
            "display_per_source".to_string(),
            crate::ymodel::json::encode_opt_f64(self.display_per_source),
        );
        map.insert(
            "decimal_places".to_string(),
            Value::Number(Number::from(self.decimal_places)),
        );
        Value::Object(map)
    }

    /// Read a unit context written by [`Self::to_json_value`].
    pub fn from_json_value(value: &Value) -> Result<Self, HeaderError> {
        let mut reader = ObjectReader::new(value, "units")?;
        let source = unit_from_value(&reader.take("source")?)?;
        let display = unit_from_value(&reader.take("display")?)?;
        let display_per_source = reader.opt_f64("display_per_source")?;
        let decimal_places =
            reader
                .take("decimal_places")?
                .as_u64()
                .ok_or(HeaderError::NotAnInteger {
                    field: "decimal_places",
                })?;
        reader.finish()?;
        let decimal_places =
            u8::try_from(decimal_places).map_err(|_| HeaderError::NotAnInteger {
                field: "decimal_places",
            })?;
        Ok(UnitContext {
            source,
            display,
            display_per_source,
            decimal_places,
        })
    }
}

impl DocumentIdentity {
    /// The identity as a tagged object:
    /// `{"kind":"temporary","value":"<decimal>"}` or
    /// `{"kind":"sha256","value":"<64 lowercase hex>"}`.
    pub fn to_json_value(&self) -> Value {
        let mut map = Map::new();
        match self {
            DocumentIdentity::Temporary(serial) => {
                map.insert("kind".to_string(), Value::String("temporary".to_string()));
                // A `u128` exceeds 2^53, so it crosses as a decimal string.
                map.insert("value".to_string(), Value::String(serial.to_string()));
            }
            DocumentIdentity::Sha256(bytes) => {
                map.insert("kind".to_string(), Value::String("sha256".to_string()));
                map.insert("value".to_string(), Value::String(hex_lower(bytes)));
            }
        }
        Value::Object(map)
    }

    /// Read an identity written by [`Self::to_json_value`].
    pub fn from_json_value(value: &Value) -> Result<Self, HeaderError> {
        let mut reader = ObjectReader::new(value, "identity")?;
        let kind = match reader.take("kind")? {
            Value::String(kind) => kind,
            _ => {
                return Err(HeaderError::Json(JsonError::WrongType {
                    field: "identity.kind",
                    expected: "a string",
                }));
            }
        };
        let raw = reader.take("value")?;
        reader.finish()?;
        let text = match raw {
            Value::String(text) => text,
            _ => {
                return Err(HeaderError::Json(JsonError::WrongType {
                    field: "identity.value",
                    expected: "a string",
                }));
            }
        };
        match kind.as_str() {
            "temporary" => Ok(DocumentIdentity::Temporary(parse_u128(
                "identity.value",
                &text,
            )?)),
            "sha256" => Ok(DocumentIdentity::Sha256(parse_hex(&text)?)),
            other => Err(HeaderError::UnknownIdentityKind(other.to_string())),
        }
    }
}

/// The token a [`Unit`] is written as.
pub fn unit_token(unit: &Unit) -> &'static str {
    match unit {
        Unit::DrawingUnits => "drawing",
        Unit::Millimeter => "mm",
        Unit::Meter => "m",
        Unit::Inch => "in",
        Unit::Foot => "ft",
    }
}

/// The [`Unit`] a token names, or the token that was not understood.
pub fn unit_from_token(token: &str) -> Result<Unit, HeaderError> {
    match token {
        "drawing" => Ok(Unit::DrawingUnits),
        "mm" => Ok(Unit::Millimeter),
        "m" => Ok(Unit::Meter),
        "in" => Ok(Unit::Inch),
        "ft" => Ok(Unit::Foot),
        other => Err(HeaderError::UnknownUnit(other.to_string())),
    }
}

/// A header (or counter) codec failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeaderError {
    /// A nested value failed the shared numeric codec.
    Json(JsonError),
    /// A field that must be a non-negative integer is not one.
    NotAnInteger {
        /// Field being decoded.
        field: &'static str,
    },
    /// A counter is outside `1 ..= MAX_SEQ + 1`.
    CounterOutOfRange {
        /// Counter name as written in `document.json`.
        counter: &'static str,
        /// The offending value.
        value: u128,
    },
    /// The counter is exhausted (or was `0`): no sequence can be minted.
    CannotMint {
        /// Identifier family.
        kind: IdKind,
        /// The counter that refused.
        counter: u128,
    },
    /// The family has no per-document counter (Model, SubCategory, …).
    NotAllocated(IdKind),
    /// `units.source`/`units.display` is not one of the five tokens.
    UnknownUnit(String),
    /// `identity.kind` is not `temporary` or `sha256`.
    UnknownIdentityKind(String),
    /// `identity.value` is not 64 lowercase hex digits.
    BadSha256(String),
    /// A decimal-string integer is not a plain base-10 `u128`.
    BadDecimal {
        /// Field being decoded.
        field: &'static str,
        /// The offending text.
        text: String,
    },
}

impl fmt::Display for HeaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HeaderError::Json(error) => write!(f, "{error}"),
            HeaderError::NotAnInteger { field } => {
                write!(f, "{field}: expected a non-negative integer")
            }
            HeaderError::CounterOutOfRange { counter, value } => write!(
                f,
                "{counter}: counter {value} is outside 1..={COUNTER_CEILING}"
            ),
            HeaderError::CannotMint { kind, counter } => write!(
                f,
                "cannot mint a {:?} id: counter {counter} is outside 1..={MAX_SEQ}",
                kind
            ),
            HeaderError::NotAllocated(kind) => {
                write!(f, "{kind:?} ids are derived, not allocated")
            }
            HeaderError::UnknownUnit(token) => {
                write!(
                    f,
                    "unknown unit {token:?} (expected drawing, mm, m, in or ft)"
                )
            }
            HeaderError::UnknownIdentityKind(kind) => {
                write!(f, "unknown document identity kind {kind:?}")
            }
            HeaderError::BadSha256(text) => write!(
                f,
                "document identity {text:?} is not 64 lowercase hexadecimal digits"
            ),
            HeaderError::BadDecimal { field, text } => {
                write!(f, "{field}: {text:?} is not a decimal u128")
            }
        }
    }
}

impl std::error::Error for HeaderError {}

impl From<JsonError> for HeaderError {
    fn from(error: JsonError) -> Self {
        HeaderError::Json(error)
    }
}

fn counter_value(value: u128) -> Value {
    // `COUNTER_CEILING` is 2^45, so the cast cannot truncate.
    Value::Number(Number::from(value as u64))
}

fn unit_to_value(unit: &Unit) -> Value {
    Value::String(unit_token(unit).to_string())
}

fn unit_from_value(value: &Value) -> Result<Unit, HeaderError> {
    match value {
        Value::String(token) => unit_from_token(token),
        _ => Err(HeaderError::Json(JsonError::WrongType {
            field: "units",
            expected: "a unit token string",
        })),
    }
}

fn database_from_value(value: &Value) -> Result<DatabaseId, HeaderError> {
    match value {
        Value::String(text) => Ok(DatabaseId(parse_u128("database", text)?)),
        _ => Err(HeaderError::Json(JsonError::WrongType {
            field: "database",
            expected: "a decimal string",
        })),
    }
}

fn strings_from_value(value: &Value) -> Result<Vec<String>, HeaderError> {
    let Value::Array(items) = value else {
        return Err(HeaderError::Json(JsonError::WrongType {
            field: "resource_keys",
            expected: "an array of strings",
        }));
    };
    items
        .iter()
        .map(|item| match item {
            Value::String(text) => Ok(text.clone()),
            _ => Err(HeaderError::Json(JsonError::WrongType {
                field: "resource_keys",
                expected: "an array of strings",
            })),
        })
        .collect()
}

fn parse_u128(field: &'static str, text: &str) -> Result<u128, HeaderError> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(HeaderError::BadDecimal {
            field,
            text: text.to_string(),
        });
    }
    text.parse::<u128>().map_err(|_| HeaderError::BadDecimal {
        field,
        text: text.to_string(),
    })
}

fn hex_lower(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    out
}

/// Read exactly 64 lowercase hex digits into a 32-byte array.
fn parse_hex(text: &str) -> Result<[u8; 32], HeaderError> {
    let bad = || HeaderError::BadSha256(text.to_string());
    if text.len() != 64 {
        return Err(bad());
    }
    let mut out = [0u8; 32];
    let chunks: &[[u8; 2]] = match text.as_bytes().as_chunks::<2>() {
        (chunks, []) => chunks,
        _ => return Err(bad()),
    };
    for (index, chunk) in chunks.iter().enumerate() {
        let mut byte = 0u8;
        for digit in chunk {
            let value = match digit {
                b'0'..=b'9' => digit - b'0',
                b'a'..=b'f' => digit - b'a' + 10,
                // Uppercase is refused: the writer emits lowercase, and one
                // canonical spelling keeps the saved bytes byte-stable.
                _ => return Err(bad()),
            };
            byte = (byte << 4) | value;
        }
        out[index] = byte;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ymodel::id::MAX_SEQ;

    fn drawing_units() -> UnitContext {
        UnitContext::drawing_units()
    }

    fn header() -> DocumentHeader {
        DocumentHeader {
            database: DatabaseId(1),
            units: UnitContext {
                source: Unit::Millimeter,
                display: Unit::Meter,
                display_per_source: Some(0.001),
                decimal_places: 4,
            },
            identity: DocumentIdentity::Temporary(u128::MAX),
            resource_keys: Vec::new(),
            next: IdCounters::default(),
        }
    }

    fn round_trip(header: &DocumentHeader) -> DocumentHeader {
        let text = serde_json::to_string(&header.to_json_value()).expect("serialize");
        let parsed: Value = serde_json::from_str(&text).expect("parse");
        DocumentHeader::from_json_value(&parsed).expect("decode")
    }

    #[test]
    fn header_round_trips_with_drawing_units_and_no_conversion() {
        let header = DocumentHeader {
            database: DatabaseId(7),
            units: drawing_units(),
            identity: DocumentIdentity::Temporary(1),
            resource_keys: Vec::new(),
            next: IdCounters::default(),
        };
        assert_eq!(round_trip(&header), header);
        assert_eq!(header.units.display_per_source, None);

        // The default context is what "unknown units stay drawing units" means.
        let decoded = round_trip(&DocumentHeader {
            units: UnitContext::default(),
            ..header
        });
        assert_eq!(decoded.units.source, Unit::DrawingUnits);
        assert_eq!(decoded.units.display, Unit::DrawingUnits);
        assert_eq!(decoded.units.display_per_source, None);
    }

    #[test]
    fn header_round_trips_both_identity_kinds_and_a_populated_resource_list() {
        let mut header = header();
        header.resource_keys = vec!["arial.ttf".to_string(), "logo.png".to_string()];
        assert_eq!(round_trip(&header), header);

        let sha = DocumentIdentity::Sha256([0xab; 32]);
        let mut hashed = header.clone();
        hashed.identity = sha.clone();
        assert_eq!(round_trip(&hashed), hashed);
        // 64 lowercase hex characters, not 32 bytes as an array of numbers.
        let encoded = sha.to_json_value();
        let value = encoded
            .get("value")
            .and_then(Value::as_str)
            .expect("string");
        assert_eq!(value.len(), 64);
        assert_eq!(value, "ab".repeat(32));

        // A temporary identity above 2^53 crosses as a decimal string.
        let encoded = DocumentIdentity::Temporary(u128::MAX).to_json_value();
        assert_eq!(
            encoded.get("value").and_then(Value::as_str),
            Some("340282366920938463463374607431768211455")
        );
    }

    #[test]
    fn units_use_tokens_and_refuse_unknown_ones() {
        for (unit, token) in [
            (Unit::DrawingUnits, "drawing"),
            (Unit::Millimeter, "mm"),
            (Unit::Meter, "m"),
            (Unit::Inch, "in"),
            (Unit::Foot, "ft"),
        ] {
            assert_eq!(unit_token(&unit), token);
            assert_eq!(unit_from_token(token).expect("known"), unit);
        }
        let context = UnitContext {
            source: Unit::Foot,
            display: Unit::Inch,
            display_per_source: Some(12.0),
            decimal_places: 2,
        };
        assert_eq!(
            context.to_json_value()["source"],
            Value::String("ft".into())
        );
        let error = UnitContext::from_json_value(&serde_json::json!({
            "source": "cubits",
            "display": "mm",
            "display_per_source": null,
            "decimal_places": 3
        }))
        .expect_err("unknown unit");
        assert_eq!(error, HeaderError::UnknownUnit("cubits".to_string()));
    }

    #[test]
    fn counters_round_trip_at_both_ends_of_the_range() {
        let low = IdCounters::default();
        assert_eq!(IdCounters::from_json_value(&low.to_json_value()), Ok(low));

        let high = IdCounters {
            entity: COUNTER_CEILING,
            block: COUNTER_CEILING,
            layer: MAX_SEQ,
            layout: 1,
            style: 1000,
            dim_style: 2,
            mleader_style: 3,
            linetype: 4,
            scale: 5,
        };
        assert_eq!(IdCounters::from_json_value(&high.to_json_value()), Ok(high));
        assert!(high.validate().is_ok());

        // Numbers, not strings: the ceiling is 2^45, exact in f64.
        let encoded = high.to_json_value();
        assert!(encoded["entity"].is_u64());
        assert_eq!(encoded["entity"].as_u64(), Some(COUNTER_CEILING as u64));
    }

    #[test]
    fn validate_refuses_zero_and_above_the_ceiling() {
        let mut counters = IdCounters {
            entity: 0,
            ..IdCounters::default()
        };
        assert_eq!(
            counters.validate(),
            Err(HeaderError::CounterOutOfRange {
                counter: "entity",
                value: 0
            })
        );
        counters.entity = COUNTER_CEILING + 1;
        assert_eq!(
            counters.validate(),
            Err(HeaderError::CounterOutOfRange {
                counter: "entity",
                value: COUNTER_CEILING + 1
            })
        );
        // The decoder refuses the same value instead of storing it.
        let value = IdCounters {
            layer: COUNTER_CEILING + 1,
            ..IdCounters::default()
        };
        assert!(IdCounters::from_json_value(&value.to_json_value()).is_err());
        // A header whose next counters are malformed is refused on read.
        let mut header = self::header();
        header.next.scale = 0;
        let text = serde_json::to_string(&header.to_json_value()).expect("serialize");
        let parsed: Value = serde_json::from_str(&text).expect("parse");
        assert!(DocumentHeader::from_json_value(&parsed).is_err());
    }

    #[test]
    fn mint_returns_the_counter_then_advances_and_fails_at_the_ceiling() {
        let mut header = self::header();
        assert_eq!(header.next_id(IdKind::Entity), Some(1));
        assert_eq!(header.mint(IdKind::Entity), Ok(1));
        assert_eq!(header.next_id(IdKind::Entity), Some(2));
        assert_eq!(header.mint(IdKind::Entity), Ok(2));
        assert_eq!(header.next.entity, 3);

        header.next.entity = MAX_SEQ;
        assert_eq!(header.mint(IdKind::Entity), Ok(MAX_SEQ));
        assert_eq!(header.next.entity, COUNTER_CEILING);
        assert_eq!(
            header.mint(IdKind::Entity),
            Err(HeaderError::CannotMint {
                kind: IdKind::Entity,
                counter: COUNTER_CEILING
            })
        );
        // A failed mint does not move the counter or wrap it.
        assert_eq!(header.next.entity, COUNTER_CEILING);

        // A derived family has no counter, so it cannot be minted.
        assert_eq!(header.next_id(IdKind::Model), None);
        assert_eq!(
            header.mint(IdKind::Model),
            Err(HeaderError::NotAllocated(IdKind::Model))
        );
    }

    #[test]
    fn identity_and_header_refuse_malformed_values() {
        let error = DocumentIdentity::from_json_value(&serde_json::json!({
            "kind": "sha256",
            "value": "AB".repeat(32)
        }))
        .expect_err("uppercase hex");
        assert!(matches!(error, HeaderError::BadSha256(_)), "{error:?}");

        let error = DocumentIdentity::from_json_value(&serde_json::json!({
            "kind": "sha256",
            "value": "ab"
        }))
        .expect_err("short hex");
        assert!(matches!(error, HeaderError::BadSha256(_)), "{error:?}");

        let error = DocumentIdentity::from_json_value(&serde_json::json!({
            "kind": "uuid",
            "value": "1"
        }))
        .expect_err("unknown kind");
        assert_eq!(error, HeaderError::UnknownIdentityKind("uuid".into()));

        // An unknown field is refused: the writer could not preserve it.
        let value = serde_json::json!({
            "database": "1",
            "units": {
                "source": "mm",
                "display": "mm",
                "display_per_source": null,
                "decimal_places": 3
            },
            "identity": { "kind": "temporary", "value": "1" },
            "resource_keys": [],
            "next": IdCounters::default().to_json_value(),
            "future_field": true
        });
        let error = DocumentHeader::from_json_value(&value).expect_err("unknown field");
        assert!(
            matches!(error, HeaderError::Json(JsonError::UnknownField { .. })),
            "{error:?}"
        );
    }
}
