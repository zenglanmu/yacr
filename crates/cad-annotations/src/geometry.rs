//! Annotation geometry, measurement metadata and precision codecs.
//!
//! Missing or invalid algorithm, units, source or precision must not be silently
//! approximated as analytic/UserPoints (audit B09/B11).

use super::*;

/// Known keys of the annotation object.
pub(crate) const KNOWN_ANNOTATION: [&str; 9] = [
    "id",
    "space",
    "geometry",
    "text",
    "style",
    "created_unix_ms",
    "modified_unix_ms",
    "anchor",
    "precision",
];

/// Known keys shared by every geometry object plus its `kind`.
pub(crate) const KNOWN_GEOMETRY: [&str; 10] = [
    "kind",
    "position",
    "points",
    "a",
    "b",
    "center",
    "axis_u",
    "axis_v",
    "algorithm",
    "value",
];

/// Known keys of a measurement geometry object (superset of [`KNOWN_GEOMETRY`]).
pub(crate) const KNOWN_MEASUREMENT: [&str; 14] = [
    "kind",
    "position",
    "points",
    "a",
    "b",
    "center",
    "axis_u",
    "axis_v",
    "algorithm",
    "value",
    "plane",
    "units",
    "source",
    "precision",
];

/// Known keys of an annotation-level or measurement-level `precision` object.
pub(crate) const KNOWN_PRECISION: [&str; 2] = ["kind", "error_bound"];

pub(crate) fn encode_geometry(geometry: &AnnotationGeometry) -> Value {
    match geometry {
        AnnotationGeometry::Text(p) => json!({ "kind": "text", "position": encode_point(*p) }),
        AnnotationGeometry::Leader(points) => {
            json!({ "kind": "leader", "points": points.iter().map(|p| encode_point(*p)).collect::<Vec<_>>() })
        }
        AnnotationGeometry::Rectangle(pair) => {
            json!({ "kind": "rectangle", "a": encode_point(pair[0]), "b": encode_point(pair[1]) })
        }
        AnnotationGeometry::Ellipse {
            center,
            axis_u,
            axis_v,
        } => json!({
            "kind": "ellipse",
            "center": encode_point(*center),
            "axis_u": encode_point(*axis_u),
            "axis_v": encode_point(*axis_v),
        }),
        AnnotationGeometry::Freehand(v) => {
            json!({ "kind": "freehand", "points": v.iter().map(|p| encode_point(*p)).collect::<Vec<_>>() })
        }
        AnnotationGeometry::Cloud(v) => {
            json!({ "kind": "cloud", "points": v.iter().map(|p| encode_point(*p)).collect::<Vec<_>>() })
        }
        AnnotationGeometry::Measurement(m) => json!({
            "kind": "measurement",
            "algorithm": encode_algorithm(&m.algorithm),
            "value": m.value,
            "points": m.inputs.iter().map(|p| encode_point(*p)).collect::<Vec<_>>(),
            "plane": m.plane.map(encode_plane),
            "units": encode_units(&m.units),
            "source": encode_geometry_source(&m.source),
            "precision": encode_precision(&m.precision),
        }),
    }
}

fn encode_algorithm(algorithm: &MeasurementAlgorithm) -> Value {
    json!(match algorithm {
        MeasurementAlgorithm::Distance2d => "Distance2d",
        MeasurementAlgorithm::Distance3d => "Distance3d",
        MeasurementAlgorithm::PolylineLength => "PolylineLength",
        MeasurementAlgorithm::Angle3Points => "Angle3Points",
        MeasurementAlgorithm::PlanarPolygonArea => "PlanarPolygonArea",
    })
}

fn decode_algorithm(value: Option<&Value>) -> CadResult<MeasurementAlgorithm> {
    match value.and_then(|v| v.as_str()) {
        Some("Distance2d") => Ok(MeasurementAlgorithm::Distance2d),
        Some("Distance3d") => Ok(MeasurementAlgorithm::Distance3d),
        Some("PolylineLength") => Ok(MeasurementAlgorithm::PolylineLength),
        Some("Angle3Points") => Ok(MeasurementAlgorithm::Angle3Points),
        Some("PlanarPolygonArea") => Ok(MeasurementAlgorithm::PlanarPolygonArea),
        Some(other) => Err(corrupt(format!("unknown measurement algorithm '{other}'"))),
        None => Err(corrupt("measurement algorithm is required")),
    }
}

fn encode_geometry_source(source: &GeometrySource) -> Value {
    json!(match source {
        GeometrySource::Analytic => "Analytic",
        GeometrySource::DirectMesh => "DirectMesh",
        GeometrySource::ProxyCache => "ProxyCache",
        GeometrySource::KernelMesh => "KernelMesh",
        GeometrySource::UserPoints => "UserPoints",
    })
}

fn decode_geometry_source(value: Option<&Value>) -> CadResult<GeometrySource> {
    match value.and_then(|v| v.as_str()) {
        Some("Analytic") => Ok(GeometrySource::Analytic),
        Some("DirectMesh") => Ok(GeometrySource::DirectMesh),
        Some("ProxyCache") => Ok(GeometrySource::ProxyCache),
        Some("KernelMesh") => Ok(GeometrySource::KernelMesh),
        Some("UserPoints") => Ok(GeometrySource::UserPoints),
        Some(other) => Err(corrupt(format!("unknown geometry source '{other}'"))),
        None => Err(corrupt("measurement geometry source is required")),
    }
}

pub(crate) fn encode_precision(precision: &Precision) -> Value {
    match precision {
        Precision::Analytic => json!({ "kind": "analytic" }),
        Precision::Approximate { error_bound } => {
            json!({ "kind": "approximate", "error_bound": error_bound })
        }
        Precision::Unknown => json!({ "kind": "unknown" }),
    }
}

pub(crate) fn decode_precision(value: Option<&Value>) -> CadResult<Precision> {
    let v = value.ok_or_else(|| corrupt("precision is required"))?;
    let object = v
        .as_object()
        .ok_or_else(|| corrupt("precision must be an object"))?;
    match require_str(object, "kind")? {
        "analytic" => Ok(Precision::Analytic),
        "approximate" => match object.get("error_bound") {
            None | Some(Value::Null) => Ok(Precision::Approximate { error_bound: None }),
            Some(b) => {
                let n = b
                    .as_f64()
                    .ok_or_else(|| corrupt("precision error_bound must be a number"))?;
                if !n.is_finite() {
                    return Err(corrupt("precision error_bound must be finite"));
                }
                Ok(Precision::Approximate {
                    error_bound: Some(n),
                })
            }
        },
        "unknown" => Ok(Precision::Unknown),
        other => Err(corrupt(format!("unknown precision kind '{other}'"))),
    }
}

/// Decode a geometry object, also reporting whether it carries a nested
/// `precision` object (measurement geometries do; other kinds do not).
pub(crate) fn decode_geometry_with_kind(value: &Value) -> CadResult<(AnnotationGeometry, bool)> {
    let object = value
        .as_object()
        .ok_or_else(|| corrupt("geometry must be an object"))?;
    let kind = require_str(object, "kind")?;
    let precision_is_measurement = kind == "measurement";
    let geometry = match kind {
        "text" => AnnotationGeometry::Text(decode_point(require(object, "position")?)?),
        "leader" => AnnotationGeometry::Leader(decode_points(require(object, "points")?)?),
        "freehand" => AnnotationGeometry::Freehand(decode_points(require(object, "points")?)?),
        "cloud" => AnnotationGeometry::Cloud(decode_points(require(object, "points")?)?),
        "rectangle" => AnnotationGeometry::Rectangle([
            decode_point(require(object, "a")?)?,
            decode_point(require(object, "b")?)?,
        ]),
        "ellipse" => AnnotationGeometry::Ellipse {
            center: decode_point(require(object, "center")?)?,
            axis_u: decode_point(require(object, "axis_u")?)?,
            axis_v: decode_point(require(object, "axis_v")?)?,
        },
        "measurement" => {
            // Missing/invalid algorithm, units, source or precision must not be
            // silently approximated as analytic/UserPoints (audit B09).
            let algorithm = decode_algorithm(object.get("algorithm"))?;
            let units = decode_units(object.get("units"))?;
            let source = decode_geometry_source(object.get("source"))?;
            let precision = decode_precision(object.get("precision"))?;
            AnnotationGeometry::Measurement(MeasurementRecord {
                algorithm,
                inputs: decode_points(require(object, "points")?)?,
                plane: match object.get("plane") {
                    None | Some(Value::Null) => None,
                    Some(plane) => Some(decode_plane(plane)?),
                },
                value: require_f64(object, "value")?,
                units,
                source,
                precision,
            })
        }
        other => return Err(corrupt(format!("unknown geometry kind '{other}'"))),
    };
    Ok((geometry, precision_is_measurement))
}
