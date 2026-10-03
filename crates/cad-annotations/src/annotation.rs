//! Annotation, anchor and style codecs.
//!
//! Decoding rejects any missing or malformed required field instead of
//! defaulting it (audit B09/B11).

use super::*;

/// Decode an annotation timestamp.
///
/// Both the canonical integer Unix milliseconds and an RFC 3339 string are
/// accepted, because a newer writer may emit either. A malformed string, an
/// out-of-range instant or any other JSON type is [`CadError::CorruptData`];
/// encode always writes integer milliseconds as the canonical form.
pub(crate) fn decode_timestamp(value: &Value, field: &str) -> CadResult<i64> {
    if let Some(ms) = value.as_i64() {
        return Ok(ms);
    }
    let text = value.as_str().ok_or_else(|| {
        corrupt(format!(
            "{field} must be integer Unix milliseconds or an RFC 3339 string"
        ))
    })?;
    parse_rfc3339_millis(text)
        .ok_or_else(|| corrupt(format!("{field} is not a valid RFC 3339 timestamp")))
}

/// Parse an RFC 3339 date-time into Unix milliseconds.
///
/// Supports the UTC designators `Z`/`z`, `+hh:mm`/`-hh:mm` offsets, optional
/// fractional seconds and `T`/`t`/space separators. Returns `None` for
/// malformed input or a year/day beyond the `i64` millisecond range.
fn parse_rfc3339_millis(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() < 20 {
        return None;
    }
    let year = parse_digits(&bytes[0..4])? as i64;
    if bytes[4] != b'-' {
        return None;
    }
    let month = parse_digits(&bytes[5..7])?;
    if bytes[7] != b'-' {
        return None;
    }
    let day = parse_digits(&bytes[8..10])?;
    if !matches!(bytes[10], b'T' | b't' | b' ') {
        return None;
    }
    let hour = parse_digits(&bytes[11..13])?;
    if bytes[13] != b':' {
        return None;
    }
    let minute = parse_digits(&bytes[14..16])?;
    if bytes[16] != b':' {
        return None;
    }
    let second = parse_digits(&bytes[17..19])?;

    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if day > days_in_month(year, month) || hour > 23 || minute > 59 || second > 59 {
        return None;
    }

    // Optional fractional seconds.
    let mut millis = 0i64;
    let mut cursor = 19;
    if bytes.get(cursor).is_some_and(|b| *b == b'.') {
        cursor += 1;
        let start = cursor;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
        if cursor == start {
            return None;
        }
        let digits = &text[start..cursor];
        let mut fraction = String::with_capacity(3);
        for ch in digits.chars().take(3) {
            fraction.push(ch);
        }
        while fraction.len() < 3 {
            fraction.push('0');
        }
        millis = fraction.parse::<i64>().ok()?;
    }

    // Offset: `Z`/`z` or `±hh:mm`.
    let offset_minutes = match bytes.get(cursor) {
        Some(b'Z') | Some(b'z') => {
            cursor += 1;
            0i64
        }
        Some(sign @ (b'+' | b'-')) => {
            let sign = if *sign == b'+' { 1i64 } else { -1i64 };
            let off_hour = parse_digits(bytes.get(cursor + 1..cursor + 3)? as &[u8])? as i64;
            if bytes.get(cursor + 3) != Some(&b':') {
                return None;
            }
            let off_minute = parse_digits(bytes.get(cursor + 4..cursor + 6)? as &[u8])? as i64;
            if off_hour > 23 || off_minute > 59 {
                return None;
            }
            cursor += 6;
            sign * (off_hour * 60 + off_minute)
        }
        _ => return None,
    };
    if cursor != bytes.len() {
        return None;
    }

    let days = days_from_civil(year, month, day);
    let seconds = days
        .checked_mul(86_400)?
        .checked_add(i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second))?
        .checked_sub(offset_minutes * 60)?;
    seconds.checked_mul(1000)?.checked_add(millis)
}

fn parse_digits(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let mut value = 0u32;
    for b in bytes {
        value = value.checked_mul(10)?.checked_add(u32::from(b - b'0'))?;
    }
    Some(value)
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = i64::from(month);
    let d = i64::from(day);
    let doy = (153 * (if month > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub(crate) fn encode_annotation(
    annotation: &Annotation,
    ext: &AnnotationExtensions,
) -> CadResult<Value> {
    let geometry = encode_geometry_value(&annotation.geometry, ext)?;
    let mut style = Map::new();
    style.insert("rgba".into(), json!(annotation.style.rgba));
    style.insert(
        "logical_width".into(),
        json!(annotation.style.logical_width),
    );
    style.insert("text_height".into(), json!(annotation.style.text_height));
    merge_extensions(&mut style, &KNOWN_STYLE, &ext.style)?;

    let mut precision = encode_precision(&annotation.precision);
    if let Some(object) = precision.as_object_mut() {
        merge_extensions(object, &KNOWN_PRECISION, &ext.precision)?;
    }

    let mut object = Map::new();
    object.insert("id".into(), json!(uuid_string(annotation.id.0)));
    object.insert("space".into(), encode_space(&annotation.space));
    object.insert("geometry".into(), geometry);
    object.insert("text".into(), json!(annotation.text));
    object.insert("style".into(), Value::Object(style));
    object.insert("created_unix_ms".into(), json!(annotation.created_unix_ms));
    object.insert(
        "modified_unix_ms".into(),
        json!(annotation.modified_unix_ms),
    );
    object.insert(
        "anchor".into(),
        annotation
            .anchor
            .as_ref()
            .map(encode_anchor)
            .unwrap_or(Value::Null),
    );
    object.insert("precision".into(), precision);
    merge_extensions(&mut object, &KNOWN_ANNOTATION, &ext.annotation)?;
    Ok(Value::Object(object))
}

/// Encode a geometry object, expanding the preserved unknown nested fields of
/// its geometry and (for measurements) precision objects.
fn encode_geometry_value(
    geometry: &AnnotationGeometry,
    ext: &AnnotationExtensions,
) -> CadResult<Value> {
    let mut object = match encode_geometry(geometry) {
        Value::Object(object) => object,
        _ => unreachable!("encode_geometry always yields an object"),
    };
    merge_extensions(&mut object, &KNOWN_GEOMETRY, &ext.geometry)?;
    if let AnnotationGeometry::Measurement(_) = geometry {
        if let Some(precision) = object.get_mut("precision").and_then(Value::as_object_mut) {
            merge_extensions(precision, &KNOWN_PRECISION, &ext.measurement_precision)?;
        }
    }
    Ok(Value::Object(object))
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
///
/// Returns the annotation and the unknown nested fields preserved for it.
pub(crate) fn decode_annotation(value: &Value) -> CadResult<(Annotation, AnnotationExtensions)> {
    let object = value
        .as_object()
        .ok_or_else(|| corrupt("annotation must be an object"))?;
    let geometry_value = require(object, "geometry")?;
    let geometry_object = geometry_value
        .as_object()
        .ok_or_else(|| corrupt("geometry must be an object"))?;
    let (geometry, precision_is_measurement) = decode_geometry_with_kind(geometry_value)?;
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
    let created = decode_timestamp(require(object, "created_unix_ms")?, "created_unix_ms")?;
    let modified = decode_timestamp(require(object, "modified_unix_ms")?, "modified_unix_ms")?;
    let anchor = match object.get("anchor") {
        None | Some(Value::Null) => None,
        Some(anchor) => Some(decode_anchor(anchor)?),
    };
    let nested_extensions =
        capture_nested_extensions(object, geometry_object, precision_is_measurement);
    Ok((
        Annotation {
            id: AnnotationId(id),
            space: decode_space(object.get("space"))?,
            geometry,
            text: require_str(object, "text")?.to_string(),
            style,
            created_unix_ms: created,
            modified_unix_ms: modified,
            anchor,
            precision: decode_precision(object.get("precision"))?,
        },
        nested_extensions,
    ))
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
