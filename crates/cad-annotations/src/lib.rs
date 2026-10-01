//! Versioned sidecar format and annotation business rules, never DWG writing.
//!
//! Spec v2.0 §3.4, §16.3: annotations live in their own versioned JSON bound to
//! a content fingerprint, are only mutated through the annotation database's
//! transaction path, and preserve unknown fields so a newer writer's data
//! survives an older reader.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use cad_db::{
    Annotation, AnnotationDatabase, AnnotationGeometry, AnnotationStyle, ChangeSet, EntityAnchor,
    MeasurementAlgorithm, MeasurementRecord,
};
use cad_domain::*;
use serde_json::{json, Map, Value};

pub const SCHEMA_VERSION: u32 = 1;

pub struct ViewBookmark {
    pub name: String,
    pub viewport: ViewportId,
    pub space: SpaceId,
    pub camera_transform: Transform3,
}

pub struct AnnotationFile {
    pub schema_version: u32,
    pub application_version: String,
    pub document_fingerprint: DocumentIdentity,
    pub document_name_hint: String,
    pub unit_context: UnitContext,
    pub annotations: Vec<Annotation>,
    pub view_bookmarks: Vec<ViewBookmark>,
    /// Raw JSON for unknown top-level fields, preserved verbatim.
    pub extensions_json: BTreeMap<String, String>,
}

pub enum FingerprintPolicy {
    RejectMismatch,
    ExplicitCoordinateMapping(Transform3),
    ImportUnanchored,
}

pub enum AnnotationCommand {
    Create(Annotation),
    Update(Annotation),
    Delete(AnnotationId),
}

#[derive(Default)]
pub struct AnnotationService;

static TX_COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_transaction() -> TransactionId {
    TransactionId(TX_COUNTER.fetch_add(1, Ordering::Relaxed) as u128)
}

impl AnnotationService {
    /// Apply a command as a single transaction and return its change set.
    pub fn apply(
        &self,
        database: &mut AnnotationDatabase,
        command: AnnotationCommand,
    ) -> CadResult<ChangeSet> {
        let reason = match &command {
            AnnotationCommand::Create(_) => "create annotation",
            AnnotationCommand::Update(_) => "update annotation",
            AnnotationCommand::Delete(_) => "delete annotation",
        };
        let tx = database.begin(reason, next_transaction())?;
        let mut tx = tx;
        match command {
            AnnotationCommand::Create(a) => {
                tx.insert_annotation(a)?;
            }
            AnnotationCommand::Update(a) => {
                tx.update_annotation(a)?;
            }
            AnnotationCommand::Delete(id) => {
                tx.delete_annotation(id)?;
            }
        }
        tx.commit()
    }

    /// Decode and migrate an annotation file.
    pub fn decode(
        &self,
        bytes: &[u8],
        identity: &DocumentIdentity,
        policy: FingerprintPolicy,
    ) -> CadResult<AnnotationFile> {
        let value: Value = serde_json::from_slice(bytes)
            .map_err(|e| CadError::CorruptData(format!("annotation JSON is malformed: {e}")))?;
        let object = value
            .as_object()
            .ok_or_else(|| CadError::CorruptData("annotation file is not a JSON object".into()))?;

        let schema_version = object
            .get("schema_version")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        if schema_version > SCHEMA_VERSION {
            return Err(CadError::Unsupported(format!(
                "annotation schema version {schema_version} is newer than this build supports ({SCHEMA_VERSION})"
            )));
        }

        let file_fingerprint = match object.get("document_fingerprint") {
            Some(Value::String(s)) => DocumentIdentity::Temporary(parse_uuid_bits(s)),
            Some(Value::Array(a)) => {
                let mut bytes = [0u8; 32];
                for (i, v) in a.iter().take(32).enumerate() {
                    bytes[i] = v.as_u64().unwrap_or(0) as u8;
                }
                DocumentIdentity::Sha256(bytes)
            }
            _ => DocumentIdentity::Temporary(0),
        };
        let matches = file_fingerprint == *identity;
        if !matches {
            match policy {
                FingerprintPolicy::RejectMismatch => {
                    return Err(CadError::InvalidInput(
                        "annotation file does not match the open drawing; supply an explicit mapping policy".into(),
                    ));
                }
                FingerprintPolicy::ImportUnanchored
                | FingerprintPolicy::ExplicitCoordinateMapping(_) => {}
            }
        }

        let mut annotations = Vec::new();
        if let Some(Value::Array(items)) = object.get("annotations") {
            for item in items {
                if let Some(a) = decode_annotation(item) {
                    annotations.push(a);
                }
            }
        }

        // Preserve unknown top-level fields verbatim.
        let known = [
            "schema_version",
            "application_version",
            "document_fingerprint",
            "document_name_hint",
            "unit_context",
            "annotations",
            "view_bookmarks",
        ];
        let mut extensions_json = BTreeMap::new();
        for (k, v) in object {
            if !known.contains(&k.as_str()) {
                extensions_json.insert(k.clone(), v.to_string());
            }
        }

        Ok(AnnotationFile {
            schema_version: schema_version.max(1),
            application_version: object
                .get("application_version")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            document_fingerprint: file_fingerprint,
            document_name_hint: object
                .get("document_name_hint")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            unit_context: decode_units(object.get("unit_context")),
            annotations,
            view_bookmarks: Vec::new(),
            extensions_json,
        })
    }

    /// Encode an annotation file to JSON.
    pub fn encode(&self, file: &AnnotationFile) -> CadResult<Vec<u8>> {
        let mut root = Map::new();
        root.insert("schema_version".into(), json!(SCHEMA_VERSION));
        root.insert(
            "application_version".into(),
            json!(file.application_version),
        );
        root.insert(
            "document_fingerprint".into(),
            encode_identity(&file.document_fingerprint),
        );
        root.insert("document_name_hint".into(), json!(file.document_name_hint));
        root.insert("unit_context".into(), encode_units(&file.unit_context));
        let annotations: Vec<Value> = file.annotations.iter().map(encode_annotation).collect();
        root.insert("annotations".into(), Value::Array(annotations));
        root.insert("view_bookmarks".into(), Value::Array(Vec::new()));
        for (k, v) in &file.extensions_json {
            if let Ok(parsed) = serde_json::from_str::<Value>(v) {
                root.insert(k.clone(), parsed);
            }
        }
        serde_json::to_vec_pretty(&Value::Object(root))
            .map_err(|e| CadError::Invariant(format!("annotation encode failed: {e}")))
    }
}

fn encode_identity(identity: &DocumentIdentity) -> Value {
    match identity {
        DocumentIdentity::Sha256(bytes) => Value::Array(bytes.iter().map(|b| json!(*b)).collect()),
        DocumentIdentity::Temporary(bits) => json!(uuid_string(*bits)),
    }
}

fn uuid_string(bits: u128) -> String {
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        (bits >> 96) as u32,
        (bits >> 80) as u16,
        (bits >> 64) as u16,
        (bits >> 48) as u16,
        (bits & 0xffff_ffff_ffff) as u64
    )
}

fn parse_uuid_bits(s: &str) -> u128 {
    let hex: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    u128::from_str_radix(&hex, 16).unwrap_or(0)
}

fn encode_units(units: &UnitContext) -> Value {
    json!({
        "source": format!("{:?}", units.source),
        "display": format!("{:?}", units.display),
        "display_per_source": units.display_per_source,
        "decimal_places": units.decimal_places,
    })
}

fn decode_units(value: Option<&Value>) -> UnitContext {
    let Some(v) = value else {
        return UnitContext::drawing_units();
    };
    let display = match v.get("display").and_then(|d| d.as_str()) {
        Some("Millimeter") => Unit::Millimeter,
        Some("Meter") => Unit::Meter,
        Some("Inch") => Unit::Inch,
        Some("Foot") => Unit::Foot,
        _ => Unit::DrawingUnits,
    };
    UnitContext {
        source: display.clone(),
        display,
        display_per_source: v.get("display_per_source").and_then(|d| d.as_f64()),
        decimal_places: v
            .get("decimal_places")
            .and_then(|d| d.as_u64())
            .unwrap_or(3) as u8,
    }
}

fn encode_space(space: &SpaceId) -> Value {
    match space {
        SpaceId::Model => json!("Model"),
        SpaceId::Paper(id) => json!({ "Paper": id.0.to_string() }),
    }
}

fn decode_space(value: Option<&Value>) -> SpaceId {
    match value {
        Some(Value::Object(m)) => {
            let id = m
                .get("Paper")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u128>().ok())
                .unwrap_or(0);
            SpaceId::Paper(LayoutId(id))
        }
        _ => SpaceId::Model,
    }
}

fn encode_geometry(geometry: &AnnotationGeometry) -> Value {
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
            "algorithm": format!("{:?}", m.algorithm),
            "value": m.value,
            "points": m.inputs.iter().map(|p| encode_point(*p)).collect::<Vec<_>>(),
        }),
    }
}

fn decode_geometry(value: &Value) -> Option<AnnotationGeometry> {
    let kind = value.get("kind").and_then(|v| v.as_str())?;
    match kind {
        "text" => Some(AnnotationGeometry::Text(decode_point(
            value.get("position")?,
        )?)),
        "leader" => Some(AnnotationGeometry::Leader(decode_points(
            value.get("points")?,
        )?)),
        "freehand" => Some(AnnotationGeometry::Freehand(decode_points(
            value.get("points")?,
        )?)),
        "cloud" => Some(AnnotationGeometry::Cloud(decode_points(
            value.get("points")?,
        )?)),
        "rectangle" => Some(AnnotationGeometry::Rectangle([
            decode_point(value.get("a")?)?,
            decode_point(value.get("b")?)?,
        ])),
        "ellipse" => Some(AnnotationGeometry::Ellipse {
            center: decode_point(value.get("center")?)?,
            axis_u: decode_point(value.get("axis_u")?)?,
            axis_v: decode_point(value.get("axis_v")?)?,
        }),
        "measurement" => {
            let algorithm = match value.get("algorithm").and_then(|v| v.as_str()) {
                Some("Distance2d") => MeasurementAlgorithm::Distance2d,
                Some("PolylineLength") => MeasurementAlgorithm::PolylineLength,
                Some("Angle3Points") => MeasurementAlgorithm::Angle3Points,
                Some("PlanarPolygonArea") => MeasurementAlgorithm::PlanarPolygonArea,
                _ => MeasurementAlgorithm::Distance3d,
            };
            Some(AnnotationGeometry::Measurement(MeasurementRecord {
                algorithm,
                inputs: decode_points(value.get("points")?).unwrap_or_default(),
                plane: None,
                value: value.get("value").and_then(|v| v.as_f64()).unwrap_or(0.0),
                units: UnitContext::drawing_units(),
                source: GeometrySource::UserPoints,
                precision: Precision::Analytic,
            }))
        }
        _ => None,
    }
}

fn encode_point(p: Point3) -> Value {
    json!([p.x, p.y, p.z])
}

fn decode_point(value: &Value) -> Option<Point3> {
    let a = value.as_array()?;
    if a.len() < 3 {
        return None;
    }
    Some(Point3 {
        x: a[0].as_f64()?,
        y: a[1].as_f64()?,
        z: a[2].as_f64()?,
    })
}

fn decode_points(value: &Value) -> Option<Vec<Point3>> {
    value.as_array()?.iter().map(decode_point).collect()
}

fn encode_annotation(annotation: &Annotation) -> Value {
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
        "anchor": annotation.anchor.as_ref().map(|a| json!({
            "source_handle": a.source_handle,
            "fallback": encode_point(a.fallback),
        })),
        "precision": format!("{:?}", annotation.precision),
    })
}

fn decode_annotation(value: &Value) -> Option<Annotation> {
    let geometry = decode_geometry(value.get("geometry")?)?;
    let id = value
        .get("id")
        .and_then(|v| v.as_str())
        .map(parse_uuid_bits)
        .unwrap_or(0);
    let rgba = value
        .get("style")
        .and_then(|s| s.get("rgba"))
        .and_then(|r| r.as_array())
        .map(|a| {
            let mut out = [0u8; 4];
            for (i, v) in a.iter().take(4).enumerate() {
                out[i] = v.as_u64().unwrap_or(0) as u8;
            }
            out
        })
        .unwrap_or([0xE5, 0x39, 0x35, 0xFF]);
    let anchor = value.get("anchor").and_then(|a| {
        Some(EntityAnchor {
            source_handle: a.get("source_handle")?.as_str()?.to_string(),
            instance: InstancePath::default(),
            sub_element: None,
            fallback: decode_point(a.get("fallback")?)?,
            status: cad_db::AnchorStatus::Valid,
        })
    });
    Some(Annotation {
        id: AnnotationId(id),
        space: decode_space(value.get("space")),
        geometry,
        text: value
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        style: AnnotationStyle {
            rgba,
            logical_width: 2.0,
            text_height: 2.5,
        },
        created_unix_ms: value
            .get("created_unix_ms")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        modified_unix_ms: value
            .get("modified_unix_ms")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        anchor,
        precision: Precision::Analytic,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ann(id: u128, text: &str) -> Annotation {
        Annotation {
            id: AnnotationId(id),
            space: SpaceId::Model,
            geometry: AnnotationGeometry::Text(Point3 {
                x: 1.0,
                y: 2.0,
                z: 0.0,
            }),
            text: text.to_string(),
            style: AnnotationStyle::default(),
            created_unix_ms: 10,
            modified_unix_ms: 20,
            anchor: None,
            precision: Precision::Analytic,
        }
    }

    #[test]
    fn create_then_delete_round_trips_through_the_database() {
        let service = AnnotationService;
        let mut db = AnnotationDatabase::new(DatabaseId(1));
        let changes = service
            .apply(&mut db, AnnotationCommand::Create(ann(1, "hello")))
            .unwrap();
        assert_eq!(changes.after, Revision(1));
        assert_eq!(db.len(), 1);
        service
            .apply(&mut db, AnnotationCommand::Delete(AnnotationId(1)))
            .unwrap();
        assert_eq!(db.len(), 0);
    }

    #[test]
    fn encode_decode_round_trip_preserves_annotation() {
        let service = AnnotationService;
        let identity = DocumentIdentity::Sha256([7u8; 32]);
        let mut extensions = BTreeMap::new();
        extensions.insert("future_field".to_string(), "{\"a\":1}".to_string());
        let file = AnnotationFile {
            schema_version: SCHEMA_VERSION,
            application_version: "test".into(),
            document_fingerprint: identity.clone(),
            document_name_hint: "sample.dwg".into(),
            unit_context: UnitContext::drawing_units(),
            annotations: vec![ann(1, "hello")],
            view_bookmarks: Vec::new(),
            extensions_json: extensions,
        };
        let bytes = service.encode(&file).unwrap();
        let decoded = service
            .decode(&bytes, &identity, FingerprintPolicy::RejectMismatch)
            .unwrap();
        assert_eq!(decoded.annotations.len(), 1);
        assert_eq!(decoded.annotations[0].text, "hello");
        assert!(decoded.extensions_json.contains_key("future_field"));
    }

    #[test]
    fn mismatched_fingerprint_is_refused_by_default() {
        let service = AnnotationService;
        let file = AnnotationFile {
            schema_version: SCHEMA_VERSION,
            application_version: "test".into(),
            document_fingerprint: DocumentIdentity::Sha256([1u8; 32]),
            document_name_hint: "a.dwg".into(),
            unit_context: UnitContext::drawing_units(),
            annotations: Vec::new(),
            view_bookmarks: Vec::new(),
            extensions_json: BTreeMap::new(),
        };
        let bytes = service.encode(&file).unwrap();
        let other = DocumentIdentity::Sha256([2u8; 32]);
        assert!(service
            .decode(&bytes, &other, FingerprintPolicy::RejectMismatch)
            .is_err());
        assert!(service
            .decode(&bytes, &other, FingerprintPolicy::ImportUnanchored)
            .is_ok());
    }

    #[test]
    fn newer_schema_is_rejected() {
        let json = br#"{"schema_version": 99, "annotations": []}"#;
        let service = AnnotationService;
        let identity = DocumentIdentity::Sha256([0u8; 32]);
        assert!(matches!(
            service.decode(json, &identity, FingerprintPolicy::ImportUnanchored),
            Err(CadError::Unsupported(_))
        ));
    }
}
