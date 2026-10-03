//! Nested unknown-field capture and merge (audit B11 fidelity).
//!
//! Unknown top-level fields are preserved directly on
//! [`AnnotationFile::extensions_json`]; unknown fields *inside* an annotation
//! object (and its direct nested `geometry` / `style` / `precision` objects)
//! are preserved in a private top-level side-band
//! ([`NESTED_EXTENSIONS_KEY`]) so the public `cad-db` annotation type needs no
//! extra field. The side-band round-trips as ordinary preserved JSON, and
//! [`AnnotationService::encode`] expands it back into the annotation objects.

use super::*;

/// Collect unknown keys of one JSON object, preserving each value verbatim.
fn collect_unknown(object: &Map<String, Value>, known: &[&str]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (key, value) in object {
        if !known.contains(&key.as_str()) {
            out.insert(key.clone(), value.to_string());
        }
    }
    out
}

/// Capture the unknown nested fields of one decoded annotation.
pub(crate) fn capture_nested_extensions(
    annotation: &Map<String, Value>,
    geometry: &Map<String, Value>,
    precision_is_measurement: bool,
) -> AnnotationExtensions {
    // The geometry object's known key set depends on its `kind`: only a
    // measurement carries `plane`/`units`/`source`/`precision`.
    let known_geometry: &[&str] = if precision_is_measurement {
        &KNOWN_MEASUREMENT
    } else {
        &KNOWN_GEOMETRY
    };
    let mut extensions = AnnotationExtensions {
        annotation: collect_unknown(annotation, &KNOWN_ANNOTATION),
        geometry: collect_unknown(geometry, known_geometry),
        ..AnnotationExtensions::default()
    };
    if let Some(style) = annotation.get("style").and_then(Value::as_object) {
        extensions.style = collect_unknown(style, &KNOWN_STYLE);
    }
    if let Some(precision) = annotation.get("precision").and_then(Value::as_object) {
        extensions.precision = collect_unknown(precision, &KNOWN_PRECISION);
    }
    if precision_is_measurement {
        if let Some(precision) = geometry.get("precision").and_then(Value::as_object) {
            extensions.measurement_precision = collect_unknown(precision, &KNOWN_PRECISION);
        }
    }
    extensions
}

/// Merge the decoded [`NESTED_EXTENSIONS_KEY`] side-band into `target`.
///
/// The side-band is a normalization of what each annotation already carried
/// inline, so it may only add or agree with those entries. An entry that names
/// an annotation absent from the file, or disagrees with an inline capture, is
/// refused rather than silently dropped.
pub(crate) fn merge_nested_sideband(
    target: &mut NestedExtensions,
    payload: &Value,
    annotations: &[Annotation],
) -> CadResult<()> {
    let array = payload
        .as_array()
        .ok_or_else(|| corrupt(format!("'{NESTED_EXTENSIONS_KEY}' must be an array")))?;
    let known: std::collections::BTreeSet<AnnotationId> =
        annotations.iter().map(|a| a.id).collect();
    for entry in array {
        let object = entry
            .as_object()
            .ok_or_else(|| corrupt(format!("'{NESTED_EXTENSIONS_KEY}' entry must be an object")))?;
        let id = AnnotationId(decode_id(require(object, "id")?)?);
        if !known.contains(&id) {
            return Err(corrupt(format!(
                "'{NESTED_EXTENSIONS_KEY}' names annotation {id:?} not present in the file"
            )));
        }
        let extensions = decode_extensions(require(object, "extensions")?)?;
        match target.annotations.get(&id) {
            None => {
                target.annotations.insert(id, extensions);
            }
            Some(inline) if *inline == extensions => {}
            Some(_) => {
                return Err(corrupt(format!(
                    "'{NESTED_EXTENSIONS_KEY}' disagrees with inline fields for annotation {id:?}"
                )))
            }
        }
    }
    Ok(())
}

fn decode_extensions(value: &Value) -> CadResult<AnnotationExtensions> {
    let object = value
        .as_object()
        .ok_or_else(|| corrupt("nested extensions must be an object"))?;
    let mut extensions = AnnotationExtensions::default();
    for (key, field) in [
        ("annotation", &mut extensions.annotation),
        ("geometry", &mut extensions.geometry),
        ("style", &mut extensions.style),
        ("precision", &mut extensions.precision),
        (
            "measurement_precision",
            &mut extensions.measurement_precision,
        ),
    ] {
        match object.get(key) {
            None | Some(Value::Null) => {}
            Some(value) => {
                let map = value.as_object().ok_or_else(|| {
                    corrupt(format!("nested extensions '{key}' must be an object"))
                })?;
                for (k, v) in map {
                    let raw = v.as_str().ok_or_else(|| {
                        corrupt(format!(
                            "nested extension '{key}.{k}' must be a JSON string"
                        ))
                    })?;
                    field.insert(k.clone(), raw.to_string());
                }
            }
        }
    }
    Ok(extensions)
}

/// Merge preserved extensions into a freshly built encoded JSON object.
///
/// * a key colliding with one of `known` is [`CadError::InvalidInput`];
/// * a value that is not valid JSON is [`CadError::CorruptData`].
pub(crate) fn merge_extensions(
    object: &mut Map<String, Value>,
    known: &[&str],
    extensions: &BTreeMap<String, String>,
) -> CadResult<()> {
    for (key, raw) in extensions {
        if known.contains(&key.as_str()) {
            return Err(CadError::InvalidInput(format!(
                "extension field '{key}' collides with a schema field"
            )));
        }
        let parsed: Value = serde_json::from_str(raw).map_err(|_| {
            CadError::CorruptData(format!("extension field '{key}' is not valid JSON"))
        })?;
        object.insert(key.clone(), parsed);
    }
    Ok(())
}

/// The known keys of a `style` object.
pub(crate) const KNOWN_STYLE: [&str; 3] = ["rgba", "logical_width", "text_height"];
