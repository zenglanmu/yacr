//! `SemanticGeometry` ⇄ payload JSON (ADR-0037 D4, D8, D10).
//!
//! The format rules this module enforces:
//!
//! 1. **One object per element, keyed by class.** A geometry is written as
//!    `{"class": "<type_key>", <field>: <value>, …}` where the field set is
//!    exactly [`super::descriptor`]'s descriptor for that class. The Rust type is
//!    a view of that object, never the other way round.
//! 2. **Decoding is strict, in both directions.** An unknown `class`, a missing
//!    field and an unknown extra field are all errors. Refusing unknown fields
//!    is deliberate (ADR-0037 D11): the writer re-serializes whole entries, so a
//!    field it does not model would be silently dropped on the next save. A
//!    newer minor version is refused at the container level
//!    (`ymodel.unsupported_minor`) for the same reason; the two rules agree.
//! 3. **The derived caches are persisted verbatim.** `Dimension`, `Hatch`,
//!    `MultiLeader` and `Table` carry their `display` (and `measurement`)
//!    because Phase 1 has no replacement cache that reproduces them byte for
//!    byte (ADR-0037 D8). Restoring them goes through the owning module's
//!    `from_persisted`, never through a synthesizing/validating constructor.
//! 4. **Byte strings are lowercase hex.** [`super::descriptor::FieldKind::Bytes`]
//!    documents why; an odd length or an uppercase digit is refused.
//!
//! Numeric encoding (including the `{"$f64": …}` escape for `NaN`/`±inf` and
//! the canonical sorted key order) lives in [`super::json`]; the class registry
//! lives in [`super::descriptor`].
//!
//! # Layout
//!
//! * [`basic`] — the analytic primitives, text/shape/insert, opaque, compound,
//!   image and mask.
//! * [`mesh`] — meshes, sub-element sources and the whole 3D body / B-rep tree.
//! * [`mesh_pbr`] — the nested `cad:Mesh.pbr` material object (ADR-0039).
//! * [`dimension`], [`hatch`], [`multileader`], [`table`] — the four families
//!   with their nested definition types.
//!
//! Splitting them keeps every file inside the repository's 1000-line ceiling.

use std::fmt;

use serde_json::{Map, Number, Value};

use crate::ymodel::id::{IdKind, PersistentId};
use crate::ymodel::json::{self, JsonError, ObjectReader};
use crate::SemanticGeometry;

mod attribute;
mod basic;
mod dimension;
mod hatch;
mod mesh;
mod mesh_pbr;
mod multileader;
mod table;
#[cfg(test)]
mod tests;

/// A payload codec failure.
///
/// The message always names the class (or nested shape) and field; it is the
/// diagnostic a connector or the store surfaces as
/// `ymodel.invalid_document`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadError {
    message: String,
}

impl PayloadError {
    /// A failure with its own message.
    pub fn new(message: impl Into<String>) -> Self {
        PayloadError {
            message: message.into(),
        }
    }

    /// A failure inside `context` (a class or field name).
    pub(crate) fn with(context: &str, error: impl fmt::Display) -> Self {
        PayloadError::new(format!("{context}: {error}"))
    }

    /// The message, for callers that map it onto a stable marker.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for PayloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PayloadError {}

impl From<JsonError> for PayloadError {
    fn from(error: JsonError) -> Self {
        PayloadError::new(error.to_string())
    }
}

/// Encode one semantic geometry as its payload object.
pub fn encode(geometry: &SemanticGeometry) -> Value {
    match geometry {
        SemanticGeometry::Line { start, end } => basic::encode_line(*start, *end),
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
            width,
        } => basic::encode_polyline(points, bulges, *closed, *width),
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => basic::encode_circle(*center, *normal, *radius),
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => basic::encode_arc(*center, *normal, *radius, *start, *sweep),
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => basic::encode_ellipse(*center, *normal, *major_axis, *ratio, *start, *sweep),
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
            closed,
        } => basic::encode_spline(*degree, knots, control_points, weights, *closed),
        SemanticGeometry::Point(point) => basic::encode_point(*point),
        SemanticGeometry::Mesh(mesh) => mesh::encode_mesh(mesh),
        SemanticGeometry::Body3d(body) => mesh::encode_body(body),
        SemanticGeometry::Insert {
            block,
            transform,
            attributes,
        } => basic::encode_insert(block.0, transform, attributes),
        SemanticGeometry::Text {
            text,
            position,
            style,
            height,
            rotation,
            font,
            h_align,
            v_align,
            wrap_width,
            width_factor,
            line_spacing,
        } => basic::encode_text(
            text,
            *position,
            style.0,
            *height,
            *rotation,
            font.as_deref(),
            *h_align,
            *v_align,
            *wrap_width,
            *width_factor,
            line_spacing,
        ),
        SemanticGeometry::Shape {
            shape_name,
            code,
            position,
            size,
            rotation,
            font,
        } => basic::encode_shape(
            shape_name,
            *code,
            *position,
            *size,
            *rotation,
            font.as_deref(),
        ),
        SemanticGeometry::Opaque {
            type_key,
            version,
            payload,
        } => basic::encode_opaque(type_key, *version, payload),
        SemanticGeometry::Compound(children) => basic::encode_compound(children),
        SemanticGeometry::Dimension(dimension) => dimension::encode_dimension(dimension),
        SemanticGeometry::Hatch(hatch) => hatch::encode_hatch(hatch),
        SemanticGeometry::MultiLeader(multileader) => multileader::encode_multileader(multileader),
        SemanticGeometry::Table(table) => table::encode_table(table),
        SemanticGeometry::Image {
            origin,
            u,
            v,
            pixels,
            file,
            clip,
            visible,
            adjust,
        } => basic::encode_image(
            *origin,
            *u,
            *v,
            *pixels,
            file.as_deref(),
            clip.as_ref(),
            *visible,
            adjust,
        ),
        SemanticGeometry::Mask { boundary, inverted } => basic::encode_mask(boundary, *inverted),
        SemanticGeometry::AttributeDefinition(definition) => {
            attribute::encode_attribute_definition(definition)
        }
    }
}

/// Decode a payload object into a semantic geometry.
///
/// An unknown `class`, a missing field or an extra field is an error; nothing is
/// defaulted.
pub fn decode(value: &Value) -> Result<SemanticGeometry, PayloadError> {
    let class = ObjectReader::new(value, "geometry")?.class()?.to_string();
    match class.as_str() {
        "cad:Line" => basic::decode_line(value),
        "cad:Polyline" => basic::decode_polyline(value),
        "cad:Circle" => basic::decode_circle(value),
        "cad:Arc" => basic::decode_arc(value),
        "cad:Ellipse" => basic::decode_ellipse(value),
        "cad:Spline" => basic::decode_spline(value),
        "cad:Point" => basic::decode_point(value),
        "cad:Mesh" => mesh::decode_mesh(value).map(SemanticGeometry::Mesh),
        "cad:Body3d" => mesh::decode_body(value),
        "cad:Insert" => basic::decode_insert(value),
        "cad:Text" => basic::decode_text(value),
        "cad:Shape" => basic::decode_shape(value),
        "cad:Opaque" => basic::decode_opaque(value),
        "cad:Compound" => basic::decode_compound(value),
        "cad:Dimension" => dimension::decode_dimension(value),
        "cad:Hatch" => hatch::decode_hatch(value),
        "cad:MultiLeader" => multileader::decode_multileader(value),
        "cad:Table" => table::decode_table(value),
        "cad:Image" => basic::decode_image(value),
        "cad:Mask" => basic::decode_mask(value),
        "cad:AttributeDefinition" => attribute::decode_attribute_definition(value),
        other => Err(PayloadError::new(format!(
            "unknown element class {other:?} (this build models 21 geometry classes)"
        ))),
    }
}

/// Build one payload object: the `class` key plus the descriptor's fields.
pub(crate) fn object(class: &'static str, fields: Vec<(&'static str, Value)>) -> Value {
    let mut map = Map::new();
    map.insert("class".to_string(), Value::String(class.to_string()));
    for (name, value) in fields {
        let previous = map.insert(name.to_string(), value);
        debug_assert!(previous.is_none(), "duplicate payload field {name}");
    }
    Value::Object(map)
}

/// Build a nested object: the given fields only, **no** `class` key.
///
/// Only a top-level element payload carries `class`; a nested value (a
/// transform matrix object, a hatch plane, a table cell) is a plain object.
pub(crate) fn fields(fields: Vec<(&'static str, Value)>) -> Value {
    let mut map = Map::new();
    for (name, value) in fields {
        let previous = map.insert(name.to_string(), value);
        debug_assert!(previous.is_none(), "duplicate payload field {name}");
    }
    Value::Object(map)
}

/// Encode one `IdRef` field: the persistent identifier as a JSON number.
///
/// The writer only ever sees ids the allocator minted (`seq <= MAX_SEQ`);
/// `PersistentId::expect` panics on a sequence this format cannot store, which
/// is a programming error, not a document one.
pub(crate) fn id_ref_value(kind: IdKind, seq: u128) -> Value {
    Value::Number(Number::from(PersistentId::expect(kind, seq).raw()))
}

// ---------------------------------------------------------------------------
// Typed reads. `json` supplies the numeric codecs; the shapes below are the
// geometry-level ones, so they live beside the payload codecs that use them.
// ---------------------------------------------------------------------------

impl ObjectReader {
    /// Start reading an element payload: the `class` discriminator is consumed
    /// here, so a field loop only sees the descriptor's fields.
    ///
    /// [`decode`] has already validated that the key exists and is a string.
    pub(crate) fn payload(value: &Value, context: &'static str) -> Result<Self, JsonError> {
        let mut reader = ObjectReader::new(value, context)?;
        let _ = reader.take("class")?;
        Ok(reader)
    }

    pub(crate) fn u32(&mut self, field: &'static str) -> Result<u32, JsonError> {
        let value = self.take(field)?;
        let raw = value.as_u64().ok_or(JsonError::WrongType {
            field,
            expected: "a non-negative integer",
        })?;
        u32::try_from(raw).map_err(|_| JsonError::WrongType {
            field,
            expected: "a u32",
        })
    }

    pub(crate) fn u64(&mut self, field: &'static str) -> Result<u64, JsonError> {
        let value = self.take(field)?;
        value.as_u64().ok_or(JsonError::WrongType {
            field,
            expected: "a non-negative integer",
        })
    }

    pub(crate) fn usize(&mut self, field: &'static str) -> Result<usize, JsonError> {
        let value = self.take(field)?;
        let raw = value.as_u64().ok_or(JsonError::WrongType {
            field,
            expected: "a non-negative integer",
        })?;
        usize::try_from(raw).map_err(|_| JsonError::WrongType {
            field,
            expected: "a usize",
        })
    }

    pub(crate) fn u8(&mut self, field: &'static str) -> Result<u8, JsonError> {
        let value = self.take(field)?;
        let raw = value.as_u64().ok_or(JsonError::WrongType {
            field,
            expected: "a byte",
        })?;
        u8::try_from(raw).map_err(|_| JsonError::WrongType {
            field,
            expected: "a byte",
        })
    }

    pub(crate) fn i16(&mut self, field: &'static str) -> Result<i16, JsonError> {
        let value = self.take(field)?;
        let raw = value.as_i64().ok_or(JsonError::WrongType {
            field,
            expected: "an integer",
        })?;
        i16::try_from(raw).map_err(|_| JsonError::WrongType {
            field,
            expected: "an i16",
        })
    }

    pub(crate) fn i32(&mut self, field: &'static str) -> Result<i32, JsonError> {
        let value = self.take(field)?;
        let raw = value.as_i64().ok_or(JsonError::WrongType {
            field,
            expected: "an integer",
        })?;
        i32::try_from(raw).map_err(|_| JsonError::WrongType {
            field,
            expected: "an i32",
        })
    }

    /// An integer that is `null` or a number.
    pub(crate) fn opt_i32(&mut self, field: &'static str) -> Result<Option<i32>, JsonError> {
        let value = self.take(field)?;
        if value.is_null() {
            return Ok(None);
        }
        let raw = value.as_i64().ok_or(JsonError::WrongType {
            field,
            expected: "an integer or null",
        })?;
        i32::try_from(raw)
            .map(Some)
            .map_err(|_| JsonError::WrongType {
                field,
                expected: "an i32 or null",
            })
    }

    /// A `u16` that is `null` or a number.
    pub(crate) fn opt_u16(&mut self, field: &'static str) -> Result<Option<u16>, JsonError> {
        let value = self.take(field)?;
        if value.is_null() {
            return Ok(None);
        }
        let raw = value.as_u64().ok_or(JsonError::WrongType {
            field,
            expected: "a non-negative integer or null",
        })?;
        u16::try_from(raw)
            .map(Some)
            .map_err(|_| JsonError::WrongType {
                field,
                expected: "a u16 or null",
            })
    }

    /// A `u64` that is `null` or a number.
    pub(crate) fn opt_u64(&mut self, field: &'static str) -> Result<Option<u64>, JsonError> {
        let value = self.take(field)?;
        if value.is_null() {
            return Ok(None);
        }
        value.as_u64().map(Some).ok_or(JsonError::WrongType {
            field,
            expected: "a non-negative integer or null",
        })
    }

    pub(crate) fn bool(&mut self, field: &'static str) -> Result<bool, JsonError> {
        let value = self.take(field)?;
        value.as_bool().ok_or(JsonError::WrongType {
            field,
            expected: "a boolean",
        })
    }

    pub(crate) fn string(&mut self, field: &'static str) -> Result<String, JsonError> {
        let value = self.take(field)?;
        match value {
            Value::String(text) => Ok(text),
            _ => Err(JsonError::WrongType {
                field,
                expected: "a string",
            }),
        }
    }

    /// A string that is `null` or a value.
    pub(crate) fn opt_string(&mut self, field: &'static str) -> Result<Option<String>, JsonError> {
        let value = self.take(field)?;
        match value {
            Value::Null => Ok(None),
            Value::String(text) => Ok(Some(text)),
            _ => Err(JsonError::WrongType {
                field,
                expected: "a string or null",
            }),
        }
    }

    pub(crate) fn vec_string(&mut self, field: &'static str) -> Result<Vec<String>, JsonError> {
        let value = self.take(field)?;
        let items = as_array(&value, field, "an array of strings")?;
        items
            .iter()
            .map(|item| match item {
                Value::String(text) => Ok(text.clone()),
                _ => Err(JsonError::WrongType {
                    field,
                    expected: "an array of strings",
                }),
            })
            .collect()
    }

    pub(crate) fn vec_point3(
        &mut self,
        field: &'static str,
    ) -> Result<Vec<crate::Point3>, JsonError> {
        let value = self.take(field)?;
        let items = as_array(&value, field, "an array of points")?.to_vec();
        items.iter().map(json::decode_point3).collect()
    }

    pub(crate) fn vec_f64(&mut self, field: &'static str) -> Result<Vec<f64>, JsonError> {
        let value = self.take(field)?;
        let items = as_array(&value, field, "an array of numbers")?.to_vec();
        items.iter().map(json::decode_f64).collect()
    }

    pub(crate) fn vec_f64x2(&mut self, field: &'static str) -> Result<Vec<[f64; 2]>, JsonError> {
        let value = self.take(field)?;
        let items = as_array(&value, field, "an array of two-number arrays")?.to_vec();
        items.iter().map(json::decode_f64x2).collect()
    }

    pub(crate) fn vec_f64x3(&mut self, field: &'static str) -> Result<Vec<[f64; 3]>, JsonError> {
        let value = self.take(field)?;
        let items = as_array(&value, field, "an array of three-number arrays")?.to_vec();
        items
            .iter()
            .map(|item| {
                let values = as_array(item, field, "a three-number array")?;
                if values.len() != 3 {
                    return Err(JsonError::WrongType {
                        field,
                        expected: "a three-number array",
                    });
                }
                Ok([
                    json::decode_f64(&values[0])?,
                    json::decode_f64(&values[1])?,
                    json::decode_f64(&values[2])?,
                ])
            })
            .collect()
    }

    pub(crate) fn vec_u32x3(&mut self, field: &'static str) -> Result<Vec<[u32; 3]>, JsonError> {
        let value = self.take(field)?;
        let items = as_array(&value, field, "an array of triangle index triples")?.to_vec();
        items
            .iter()
            .map(|item| {
                let values = as_array(item, field, "a three-index array")?;
                if values.len() != 3 {
                    return Err(JsonError::WrongType {
                        field,
                        expected: "a three-index array",
                    });
                }
                let index = |value: &Value| -> Result<u32, JsonError> {
                    let raw = value.as_u64().ok_or(JsonError::WrongType {
                        field,
                        expected: "a triangle index",
                    })?;
                    u32::try_from(raw).map_err(|_| JsonError::WrongType {
                        field,
                        expected: "a u32 index",
                    })
                };
                Ok([index(&values[0])?, index(&values[1])?, index(&values[2])?])
            })
            .collect()
    }

    pub(crate) fn u8x3(&mut self, field: &'static str) -> Result<[u8; 3], JsonError> {
        let value = self.take(field)?;
        decode_u8x3(&value, field)
    }

    pub(crate) fn opt_u8x3(&mut self, field: &'static str) -> Result<Option<[u8; 3]>, JsonError> {
        let value = self.take(field)?;
        if value.is_null() {
            return Ok(None);
        }
        decode_u8x3(&value, field).map(Some)
    }

    pub(crate) fn u8x3_list(&mut self, field: &'static str) -> Result<Vec<[u8; 3]>, JsonError> {
        let value = self.take(field)?;
        let items = as_array(&value, field, "an array of byte triples")?.to_vec();
        items.iter().map(|item| decode_u8x3(item, field)).collect()
    }

    /// An array of nested geometry objects.
    pub(crate) fn vec_geometry(
        &mut self,
        field: &'static str,
    ) -> Result<Vec<SemanticGeometry>, PayloadError> {
        let value = self.take(field)?;
        let items = as_array(&value, field, "an array of geometry objects")?.to_vec();
        items
            .iter()
            .map(|item| decode(item).map_err(|error| PayloadError::with(field, error)))
            .collect()
    }

    /// An `IdRef` field: a persistent id number of the declared family.
    pub(crate) fn id_seq(
        &mut self,
        field: &'static str,
        expected: IdKind,
    ) -> Result<u128, JsonError> {
        let value = self.take(field)?;
        decode_id_value(&value, field, expected)
    }

    /// A byte string written as lowercase hexadecimal.
    pub(crate) fn hex_bytes(&mut self, field: &'static str) -> Result<Vec<u8>, JsonError> {
        let text = self.string(field)?;
        decode_hex(field, &text)
    }
}

/// Decode one persistent-id number of the declared family.
pub(crate) fn decode_id_value(
    value: &Value,
    field: &'static str,
    expected: IdKind,
) -> Result<u128, JsonError> {
    let raw = value.as_i64().ok_or(JsonError::WrongType {
        field,
        expected: "a persistent identifier number",
    })?;
    let id = PersistentId::from_raw(raw).ok_or(JsonError::WrongType {
        field,
        expected: "a persistent identifier of a known family",
    })?;
    if id.kind() != expected {
        return Err(JsonError::WrongType {
            field,
            expected: "a persistent identifier of the declared family",
        });
    }
    Ok(id.seq())
}

fn as_array<'a>(
    value: &'a Value,
    field: &'static str,
    expected: &'static str,
) -> Result<&'a [Value], JsonError> {
    match value {
        Value::Array(items) => Ok(items),
        _ => Err(JsonError::WrongType { field, expected }),
    }
}

fn decode_u8x3(value: &Value, field: &'static str) -> Result<[u8; 3], JsonError> {
    let items = as_array(value, field, "a byte triple")?;
    if items.len() != 3 {
        return Err(JsonError::WrongType {
            field,
            expected: "a byte triple",
        });
    }
    let byte = |value: &Value| -> Result<u8, JsonError> {
        let raw = value.as_u64().ok_or(JsonError::WrongType {
            field,
            expected: "a byte",
        })?;
        u8::try_from(raw).map_err(|_| JsonError::WrongType {
            field,
            expected: "a byte",
        })
    };
    Ok([byte(&items[0])?, byte(&items[1])?, byte(&items[2])?])
}

pub(crate) fn encode_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    out
}

fn decode_hex(field: &'static str, text: &str) -> Result<Vec<u8>, JsonError> {
    if !text.len().is_multiple_of(2) {
        return Err(JsonError::WrongType {
            field,
            expected: "an even number of lowercase hex digits",
        });
    }
    let mut out = Vec::with_capacity(text.len() / 2);
    let chunks: &[[u8; 2]] = match text.as_bytes().as_chunks::<2>() {
        (chunks, []) => chunks,
        _ => {
            return Err(JsonError::WrongType {
                field,
                expected: "an even number of lowercase hex digits",
            });
        }
    };
    for chunk in chunks {
        let mut byte = 0u8;
        for digit in chunk {
            let value = match digit {
                b'0'..=b'9' => digit - b'0',
                b'a'..=b'f' => digit - b'a' + 10,
                _ => {
                    return Err(JsonError::WrongType {
                        field,
                        expected: "lowercase hex digits",
                    });
                }
            };
            byte = (byte << 4) | value;
        }
        out.push(byte);
    }
    Ok(out)
}
