//! Annotation, anchor and style codecs.
//!
//! Decoding rejects any missing or malformed required field instead of
//! defaulting it (audit B09/B11).

use super::*;

pub(crate) fn encode_annotation(annotation: &Annotation) -> Value {
    json!({
        "id": uuid_string(annotation.id.0),
        "space": encode_space(&annotation.space),
        "geometry": encode_geometry(&annotation.geometry),
        "text": annotation.text,
        "style": {
            "rgba": annotation.style.rgba,
            "logical_width": annotation.style.logical_width,
            "text_height": annotation.style.text_height,
        },
        "created_unix_ms": annotation.created_unix_ms,
        "modified_unix_ms": annotation.modified_unix_ms,
        "anchor": annotation.anchor.as_ref().map(encode_anchor),
        "precision": encode_precision(&annotation.precision),
    })
}

fn encode_anchor(anchor: &EntityAnchor) -> Value {
    json!({
        "source_handle": anchor.source_handle,
        "instance": encode_instance_path(&anchor.instance),
        "sub_element": anchor.sub_element.as_ref().map(|s| json!({
            "source_key": s.source_key,
            "topology_revision": s.topology_revision.0,
        })),
        "fallback": encode_point(anchor.fallback),
        "status": encode_anchor_status(&anchor.status),
    })
}

fn encode_instance_path(path: &InstancePath) -> Value {
    Value::Array(path.0.iter().map(|e| json!(e.0.to_string())).collect())
}

fn decode_instance_path(value: Option<&Value>) -> CadResult<InstancePath> {
    let items = value
        .ok_or_else(|| corrupt("anchor instance path is required"))?
        .as_array()
        .ok_or_else(|| corrupt("anchor instance path must be an array"))?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let raw = item
            .as_str()
            .ok_or_else(|| corrupt("anchor instance id must be a string"))?;
        let id = raw
            .parse::<u128>()
            .map_err(|_| corrupt("anchor instance id is not a valid integer"))?;
        out.push(EntityId(id));
    }
    Ok(InstancePath(out))
}

fn encode_anchor_status(status: &AnchorStatus) -> Value {
    json!(match status {
        AnchorStatus::Valid => "Valid",
        AnchorStatus::Stale => "Stale",
        AnchorStatus::Unresolved => "Unresolved",
    })
}

fn decode_anchor_status(value: Option<&Value>) -> CadResult<AnchorStatus> {
    match value.and_then(|v| v.as_str()) {
        Some("Valid") => Ok(AnchorStatus::Valid),
        Some("Stale") => Ok(AnchorStatus::Stale),
        Some("Unresolved") => Ok(AnchorStatus::Unresolved),
        Some(other) => Err(corrupt(format!("unknown anchor status '{other}'"))),
        None => Err(corrupt("anchor status is required")),
    }
}

/// Decode one annotation, rejecting any missing or malformed required field
/// instead of defaulting it (audit B09/B11).
pub(crate) fn decode_annotation(value: &Value) -> CadResult<Annotation> {
    let object = value
        .as_object()
        .ok_or_else(|| corrupt("annotation must be an object"))?;
    let geometry = decode_geometry(require(object, "geometry")?)?;
    let id = decode_id(require(object, "id")?)?;
    let style_value = require(object, "style")?;
    let style_object = style_value
        .as_object()
        .ok_or_else(|| corrupt("annotation style must be an object"))?;
    let style = AnnotationStyle {
        rgba: decode_rgba(style_object)?,
        logical_width: require_f64(style_object, "logical_width")?,
        text_height: require_f64(style_object, "text_height")?,
    };
    if !style.logical_width.is_finite() || !style.text_height.is_finite() {
        return Err(corrupt("annotation style must be finite"));
    }
    let created = require(object, "created_unix_ms")?
        .as_i64()
        .ok_or_else(|| corrupt("created_unix_ms must be a signed integer"))?;
    let modified = require(object, "modified_unix_ms")?
        .as_i64()
        .ok_or_else(|| corrupt("modified_unix_ms must be a signed integer"))?;
    let anchor = match object.get("anchor") {
        None | Some(Value::Null) => None,
        Some(anchor) => Some(decode_anchor(anchor)?),
    };
    Ok(Annotation {
        id: AnnotationId(id),
        space: decode_space(object.get("space"))?,
        geometry,
        text: require_str(object, "text")?.to_string(),
        style,
        created_unix_ms: created,
        modified_unix_ms: modified,
        anchor,
        precision: decode_precision(object.get("precision"))?,
    })
}

fn decode_rgba(style: &Map<String, Value>) -> CadResult<[u8; 4]> {
    let a = require(style, "rgba")?
        .as_array()
        .ok_or_else(|| corrupt("style rgba must be an array"))?;
    if a.len() != 4 {
        return Err(corrupt("style rgba must have 4 channels"));
    }
    let mut out = [0u8; 4];
    for (i, v) in a.iter().enumerate() {
        let n = v
            .as_u64()
            .ok_or_else(|| corrupt("style rgba channel must be an unsigned integer"))?;
        out[i] = u8::try_from(n).map_err(|_| corrupt("style rgba channel out of range"))?;
    }
    Ok(out)
}

fn decode_anchor(value: &Value) -> CadResult<EntityAnchor> {
    let object = value
        .as_object()
        .ok_or_else(|| corrupt("anchor must be an object"))?;
    let source_handle = require_str(object, "source_handle")?.to_string();
    if source_handle.trim().is_empty() {
        return Err(corrupt("anchor source_handle must not be empty"));
    }
    let sub_element = match object.get("sub_element") {
        None | Some(Value::Null) => None,
        Some(s) => {
            let sub = s
                .as_object()
                .ok_or_else(|| corrupt("anchor sub_element must be an object"))?;
            Some(SubElementId {
                source_key: require_str(sub, "source_key")?.to_string(),
                topology_revision: Revision(require_u64(sub, "topology_revision")?),
            })
        }
    };
    Ok(EntityAnchor {
        source_handle,
        instance: decode_instance_path(object.get("instance"))?,
        sub_element,
        fallback: decode_point(require(object, "fallback")?)?,
        status: decode_anchor_status(object.get("status"))?,
    })
}
