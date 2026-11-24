//! `cad:AttributeDefinition` and the `cad:Insert.attributes` array (ADR-0055).
//!
//! The definition is a top-level payload class; a value is a nested object inside
//! the `cad:Insert` array, so it carries no `class` key. Both are strict: a
//! missing or unknown field is an error, and the only relaxation is the
//! documented one on the array itself — an absent key means "no values" (a
//! minor-13 `INSERT`).
//!
//! The appearance object mirrors the element `render_attributes` shape for the
//! subset an attribute owns (transparency/colour/lineweight/linetype + scale), so
//! one style vocabulary is used everywhere. It is a nested shape owned by this
//! codec, not a `FieldKind` in the class registry.

use serde_json::{Map, Value};

use super::{fields, object, PayloadError};
use crate::attribute::{
    AttributeAppearance, AttributeDefinition, AttributeFlags, AttributeText, AttributeValue,
};
use crate::ymodel::id::IdKind;
use crate::ymodel::json::{self, ObjectReader};
use crate::{
    EntityColor, EntityLineType, EntityLineWeight, EntityTransparency, LayerId, LinetypePattern,
    SemanticGeometry, StyleId,
};

// ---------------------------------------------------------------------------
// `cad:AttributeDefinition`.
// ---------------------------------------------------------------------------

pub(super) fn encode_attribute_definition(definition: &AttributeDefinition) -> Value {
    object(
        "cad:AttributeDefinition",
        vec![
            ("tag", Value::String(definition.tag.clone())),
            ("prompt", Value::String(definition.prompt.clone())),
            (
                "default_value",
                Value::String(definition.default_value.clone()),
            ),
            ("flags", encode_flags(&definition.flags)),
            ("order", Value::from(definition.order)),
            ("text", encode_text(&definition.text)),
            (
                "source_handle",
                definition
                    .source_handle
                    .as_ref()
                    .map_or(Value::Null, |handle| Value::String(handle.clone())),
            ),
        ],
    )
}

pub(super) fn decode_attribute_definition(value: &Value) -> Result<SemanticGeometry, PayloadError> {
    let mut reader = ObjectReader::payload(value, "cad:AttributeDefinition")?;
    let tag = reader.string("tag")?;
    let prompt = reader.string("prompt")?;
    let default_value = reader.string("default_value")?;
    let flags = decode_flags(&reader.take("flags")?)?;
    let order = reader.u32("order")?;
    let text = decode_text(&reader.take("text")?)?;
    let source_handle = reader.opt_string("source_handle")?;
    reader.finish()?;
    Ok(SemanticGeometry::AttributeDefinition(Box::new(
        AttributeDefinition {
            tag,
            prompt,
            default_value,
            flags,
            order,
            text,
            source_handle,
        },
    )))
}

// ---------------------------------------------------------------------------
// `cad:Insert.attributes`.
// ---------------------------------------------------------------------------

pub(super) fn encode_attribute_values(values: &[AttributeValue]) -> Value {
    Value::Array(values.iter().map(encode_attribute_value).collect())
}

pub(super) fn decode_attribute_values(
    value: &Value,
    field: &'static str,
) -> Result<Vec<AttributeValue>, PayloadError> {
    let items = match value {
        Value::Array(items) => items,
        _ => {
            return Err(PayloadError::new(format!(
                "{field}: expected an array of attribute values"
            )));
        }
    };
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            decode_attribute_value(item)
                .map_err(|error| PayloadError::with(&format!("{field}[{index}]"), error))
        })
        .collect()
}

fn encode_attribute_value(value: &AttributeValue) -> Value {
    fields(vec![
        ("tag", Value::String(value.tag.clone())),
        ("value", Value::String(value.value.clone())),
        ("flags", encode_flags(&value.flags)),
        ("text", encode_text(&value.text)),
        ("layer", super::id_ref_value(IdKind::Layer, value.layer.0)),
        ("appearance", encode_appearance(&value.appearance)),
        (
            "source_handle",
            value
                .source_handle
                .as_ref()
                .map_or(Value::Null, |handle| Value::String(handle.clone())),
        ),
    ])
}

fn decode_attribute_value(value: &Value) -> Result<AttributeValue, PayloadError> {
    let mut reader = ObjectReader::new(value, "attribute_value")?;
    let tag = reader.string("tag")?;
    let value_text = reader.string("value")?;
    let flags = decode_flags(&reader.take("flags")?)?;
    let text = decode_text(&reader.take("text")?)?;
    let layer = LayerId(reader.id_seq("layer", IdKind::Layer)?);
    let appearance = decode_appearance(&reader.take("appearance")?)?;
    let source_handle = reader.opt_string("source_handle")?;
    reader.finish()?;
    Ok(AttributeValue {
        tag,
        value: value_text,
        flags,
        text,
        layer,
        appearance,
        source_handle,
    })
}

// ---------------------------------------------------------------------------
// Shared nested shapes.
// ---------------------------------------------------------------------------

fn encode_flags(flags: &AttributeFlags) -> Value {
    fields(vec![
        ("invisible", Value::Bool(flags.invisible)),
        ("constant", Value::Bool(flags.constant)),
        ("verify", Value::Bool(flags.verify)),
        ("preset", Value::Bool(flags.preset)),
        ("locked_position", Value::Bool(flags.locked_position)),
        ("annotative", Value::Bool(flags.annotative)),
    ])
}

fn decode_flags(value: &Value) -> Result<AttributeFlags, PayloadError> {
    let mut reader = ObjectReader::new(value, "attribute_flags")?;
    let flags = AttributeFlags {
        invisible: reader.bool("invisible")?,
        constant: reader.bool("constant")?,
        verify: reader.bool("verify")?,
        preset: reader.bool("preset")?,
        locked_position: reader.bool("locked_position")?,
        annotative: reader.bool("annotative")?,
    };
    reader.finish()?;
    Ok(flags)
}

fn encode_text(text: &AttributeText) -> Value {
    fields(vec![
        ("position", json::encode_point3(text.position)),
        ("height", json::encode_f64(text.height)),
        ("rotation", json::encode_f64(text.rotation)),
        ("style", super::id_ref_value(IdKind::Style, text.style.0)),
        ("width_factor", json::encode_f64(text.width_factor)),
        ("h_align", super::basic::encode_align_h(text.h_align)),
        ("v_align", super::basic::align_v_token(text.v_align)),
        ("wrap_width", json::encode_opt_f64(text.wrap_width)),
        (
            "line_spacing",
            super::basic::encode_line_spacing(&text.line_spacing),
        ),
        (
            "font",
            text.font
                .as_ref()
                .map_or(Value::Null, |font| Value::String(font.clone())),
        ),
        ("oblique", json::encode_f64(text.oblique)),
    ])
}

fn decode_text(value: &Value) -> Result<AttributeText, PayloadError> {
    let mut reader = ObjectReader::new(value, "attribute_text")?;
    let position = reader.point3("position")?;
    let height = reader.f64("height")?;
    let rotation = reader.f64("rotation")?;
    let style = StyleId(reader.id_seq("style", IdKind::Style)?);
    let width_factor = reader.f64("width_factor")?;
    let h_align = super::basic::decode_align_h(&reader.take("h_align")?)?;
    let v_align = super::basic::decode_align_v(&reader.take("v_align")?)?;
    let wrap_width = reader.opt_f64("wrap_width")?;
    let line_spacing = super::basic::decode_line_spacing(&reader.take("line_spacing")?)?;
    let font = reader.opt_string("font")?;
    let oblique = reader.f64("oblique")?;
    reader.finish()?;
    Ok(AttributeText {
        position,
        height,
        rotation,
        style,
        width_factor,
        h_align,
        v_align,
        wrap_width,
        line_spacing,
        font,
        oblique,
    })
}

// Appearance. Deliberately the same tagged shapes the element
// `render_attributes` codec writes (`{"kind":…}` variants), so a reader that
// knows an entity's colour also knows an attribute's.

fn encode_appearance(appearance: &AttributeAppearance) -> Value {
    fields(vec![
        ("transparency", encode_transparency(appearance.transparency)),
        ("color", encode_color(appearance.color)),
        ("lineweight", encode_lineweight(appearance.lineweight)),
        ("linetype", encode_linetype(&appearance.linetype)),
        (
            "linetype_scale",
            json::encode_f64(appearance.linetype_scale),
        ),
    ])
}

fn decode_appearance(value: &Value) -> Result<AttributeAppearance, PayloadError> {
    let mut reader = ObjectReader::new(value, "attribute_appearance")?;
    let transparency = decode_transparency(&reader.take("transparency")?)?;
    let color = decode_color(&reader.take("color")?)?;
    let lineweight = decode_lineweight(&reader.take("lineweight")?)?;
    let linetype = decode_linetype(&reader.take("linetype")?)?;
    let linetype_scale = reader.f64("linetype_scale")?;
    reader.finish()?;
    Ok(AttributeAppearance {
        transparency,
        color,
        lineweight,
        linetype,
        linetype_scale,
    })
}

fn encode_transparency(value: EntityTransparency) -> Value {
    match value {
        EntityTransparency::Explicit(opacity) => variant(
            "explicit",
            vec![("value", json::encode_f64(opacity as f64))],
        ),
        EntityTransparency::ByBlock => variant("byblock", Vec::new()),
        EntityTransparency::ByLayer => variant("bylayer", Vec::new()),
    }
}

fn decode_transparency(value: &Value) -> Result<EntityTransparency, PayloadError> {
    variant_object(value, "transparency", |reader| {
        match reader.string("kind")?.as_str() {
            "explicit" => Ok(EntityTransparency::Explicit(reader.f64("value")? as f32)),
            "byblock" => Ok(EntityTransparency::ByBlock),
            "bylayer" => Ok(EntityTransparency::ByLayer),
            other => Err(PayloadError::new(format!(
                "transparency: unknown kind {other:?} (expected explicit, byblock or bylayer)"
            ))),
        }
    })
}

fn encode_color(value: EntityColor) -> Value {
    match value {
        EntityColor::Explicit([r, g, b]) => {
            let channels = vec![Value::from(r), Value::from(g), Value::from(b)];
            variant("explicit", vec![("value", Value::Array(channels))])
        }
        EntityColor::ByBlock => variant("byblock", Vec::new()),
        EntityColor::ByLayer => variant("bylayer", Vec::new()),
    }
}

fn decode_color(value: &Value) -> Result<EntityColor, PayloadError> {
    variant_object(value, "color", |reader| {
        match reader.string("kind")?.as_str() {
            "explicit" => {
                let items = match reader.take("value")? {
                    Value::Array(items) if items.len() == 3 => items,
                    _ => {
                        return Err(PayloadError::new(
                            "color.value: expected an array of three bytes",
                        ));
                    }
                };
                let mut channels = [0u8; 3];
                for (slot, item) in channels.iter_mut().zip(&items) {
                    let raw = item.as_u64().ok_or_else(|| {
                        PayloadError::new("color.value: expected a byte (0..=255)")
                    })?;
                    *slot = u8::try_from(raw)
                        .map_err(|_| PayloadError::new("color.value: expected a byte (0..=255)"))?;
                }
                Ok(EntityColor::Explicit(channels))
            }
            "byblock" => Ok(EntityColor::ByBlock),
            "bylayer" => Ok(EntityColor::ByLayer),
            other => Err(PayloadError::new(format!(
                "color: unknown kind {other:?} (expected explicit, byblock or bylayer)"
            ))),
        }
    })
}

fn encode_lineweight(value: EntityLineWeight) -> Value {
    match value {
        EntityLineWeight::Explicit(millimeters) => variant(
            "explicit",
            vec![("value", json::encode_f64(millimeters as f64))],
        ),
        EntityLineWeight::Default => variant("default", Vec::new()),
        EntityLineWeight::ByBlock => variant("byblock", Vec::new()),
        EntityLineWeight::ByLayer => variant("bylayer", Vec::new()),
    }
}

fn decode_lineweight(value: &Value) -> Result<EntityLineWeight, PayloadError> {
    variant_object(value, "lineweight", |reader| {
        match reader.string("kind")?.as_str() {
            "explicit" => Ok(EntityLineWeight::Explicit(reader.f64("value")? as f32)),
            "default" => Ok(EntityLineWeight::Default),
            "byblock" => Ok(EntityLineWeight::ByBlock),
            "bylayer" => Ok(EntityLineWeight::ByLayer),
            other => Err(PayloadError::new(format!(
                "lineweight: unknown kind {other:?} (expected explicit, default, byblock or bylayer)"
            ))),
        }
    })
}

fn encode_linetype(value: &EntityLineType) -> Value {
    match value {
        EntityLineType::Explicit {
            name,
            pattern,
            scale,
        } => {
            let elements = pattern.elements.iter().copied().map(json::encode_f64);
            let pattern = fields(vec![
                ("elements", Value::Array(elements.collect())),
                ("cycle", json::encode_f64(pattern.cycle)),
            ]);
            variant(
                "explicit",
                vec![
                    ("name", Value::String(name.clone())),
                    ("pattern", pattern),
                    ("scale", json::encode_f64(*scale)),
                ],
            )
        }
        EntityLineType::ByBlock => variant("byblock", Vec::new()),
        EntityLineType::ByLayer => variant("bylayer", Vec::new()),
    }
}

fn decode_linetype(value: &Value) -> Result<EntityLineType, PayloadError> {
    variant_object(value, "linetype", |reader| {
        match reader.string("kind")?.as_str() {
            "explicit" => {
                let name = reader.string("name")?;
                let pattern_value = reader.take("pattern")?;
                let pattern = {
                    let mut pattern_reader = ObjectReader::new(&pattern_value, "linetype_pattern")?;
                    let elements = pattern_reader.vec_f64("elements")?;
                    let cycle = pattern_reader.f64("cycle")?;
                    pattern_reader.finish()?;
                    LinetypePattern { elements, cycle }
                };
                let scale = reader.f64("scale")?;
                Ok(EntityLineType::Explicit {
                    name,
                    pattern,
                    scale,
                })
            }
            "byblock" => Ok(EntityLineType::ByBlock),
            "bylayer" => Ok(EntityLineType::ByLayer),
            other => Err(PayloadError::new(format!(
                "linetype: unknown kind {other:?} (expected explicit, byblock or bylayer)"
            ))),
        }
    })
}

fn variant(kind: &str, entries: Vec<(&'static str, Value)>) -> Value {
    let mut map = Map::new();
    map.insert("kind".to_string(), Value::String(kind.to_string()));
    for (name, value) in entries {
        let previous = map.insert(name.to_string(), value);
        debug_assert!(previous.is_none(), "duplicate variant field {name}");
    }
    Value::Object(map)
}

fn variant_object<T>(
    value: &Value,
    context: &'static str,
    read: impl FnOnce(&mut ObjectReader) -> Result<T, PayloadError>,
) -> Result<T, PayloadError> {
    let mut reader = ObjectReader::new(value, context)?;
    let decoded = read(&mut reader)?;
    reader.finish()?;
    Ok(decoded)
}
