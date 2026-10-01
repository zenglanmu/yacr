//! Versioned sidecar format and annotation business rules, never DWG writing.
//!
//! Spec v2.0 §3.4, §16.3: annotations live in their own versioned JSON bound to
//! a content fingerprint, are only mutated through the annotation database's
//! transaction path, and preserve unknown fields so a newer writer's data
//! survives an older reader.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use cad_db::{
    AnchorStatus, Annotation, AnnotationDatabase, AnnotationGeometry, AnnotationStyle, ChangeSet,
    EntityAnchor, MeasurementAlgorithm, MeasurementRecord,
};
use cad_domain::*;
use serde_json::{json, Map, Value};

pub const SCHEMA_VERSION: u32 = 1;

/// Top-level schema fields that extension data must never override.
const KNOWN_TOP_LEVEL: [&str; 7] = [
    "schema_version",
    "application_version",
    "document_fingerprint",
    "document_name_hint",
    "unit_context",
    "annotations",
    "view_bookmarks",
];

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

        let schema_version = match object.get("schema_version") {
            Some(v) => {
                let raw = v.as_u64().ok_or_else(|| {
                    CadError::CorruptData("schema_version must be an unsigned integer".into())
                })?;
                u32::try_from(raw).map_err(|_| {
                    CadError::CorruptData("schema_version does not fit in 32 bits".into())
                })?
            }
            None => {
                return Err(CadError::CorruptData(
                    "annotation file has no schema_version".into(),
                ))
            }
        };
        if schema_version == 0 {
            return Err(CadError::CorruptData(
                "annotation schema_version 0 is not valid".into(),
            ));
        }
        if schema_version > SCHEMA_VERSION {
            return Err(CadError::Unsupported(format!(
                "annotation schema version {schema_version} is newer than this build supports ({SCHEMA_VERSION})"
            )));
        }

        let file_fingerprint = match object.get("document_fingerprint") {
            Some(Value::String(s)) => DocumentIdentity::Temporary(parse_uuid_bits(s)),
            Some(Value::Array(a)) => {
                // A SHA-256 fingerprint is exactly 32 bytes in 0..=255; a short
                // or over-long array must not be zero-padded or truncated into a
                // plausible-looking identity (audit B11).
                if a.len() != 32 {
                    return Err(CadError::CorruptData(
                        "document_fingerprint must be 32 bytes".into(),
                    ));
                }
                let mut bytes = [0u8; 32];
                for (i, v) in a.iter().enumerate() {
                    let n = v.as_u64().ok_or_else(|| {
                        CadError::CorruptData("document_fingerprint byte is not an integer".into())
                    })?;
                    bytes[i] = u8::try_from(n).map_err(|_| {
                        CadError::CorruptData("document_fingerprint byte out of range".into())
                    })?;
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
            for (index, item) in items.iter().enumerate() {
                // A malformed annotation is reported, not silently dropped
                // (audit B11): a caller must never believe a lossy import is OK.
                let a = decode_annotation(item).ok_or_else(|| {
                    CadError::CorruptData(format!("annotation at index {index} is invalid"))
                })?;
                annotations.push(a);
            }
        }

        // Preserve unknown top-level fields verbatim.
        let mut extensions_json = BTreeMap::new();
        for (k, v) in object {
            if !KNOWN_TOP_LEVEL.contains(&k.as_str()) {
                extensions_json.insert(k.clone(), v.to_string());
            }
        }

        // Apply the fingerprint policy to the decoded content (audit B10). A
        // mismatch must not leave anchors pointing into a document they were not
        // bound to, and an explicit mapping must actually move the geometry.
        if !matches {
            match policy {
                FingerprintPolicy::RejectMismatch => unreachable!("rejected above"),
                FingerprintPolicy::ImportUnanchored => {
                    for annotation in &mut annotations {
                        annotation.anchor = None;
                    }
                }
                FingerprintPolicy::ExplicitCoordinateMapping(transform) => {
                    let mapping = CoordinateMapping::validate(&transform)?;
                    for annotation in &mut annotations {
                        mapping.apply_annotation(annotation)?;
                    }
                }
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
            view_bookmarks: match object.get("view_bookmarks") {
                Some(Value::Array(items)) => items
                    .iter()
                    .enumerate()
                    .map(|(i, b)| {
                        decode_bookmark(b).ok_or_else(|| {
                            CadError::CorruptData(format!("view bookmark at index {i} is invalid"))
                        })
                    })
                    .collect::<CadResult<Vec<_>>>()?,
                _ => Vec::new(),
            },
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
        let bookmarks: Vec<Value> = file.view_bookmarks.iter().map(encode_bookmark).collect();
        root.insert("view_bookmarks".into(), Value::Array(bookmarks));
        for (k, v) in &file.extensions_json {
            // Never let a preserved/unknown field override a schema field
            // (audit B11): extension keys that collide with the known schema
            // are dropped rather than allowed to corrupt the output.
            if KNOWN_TOP_LEVEL.contains(&k.as_str()) {
                continue;
            }
            if let Ok(parsed) = serde_json::from_str::<Value>(v) {
                root.insert(k.clone(), parsed);
            } else {
                return Err(CadError::CorruptData(format!(
                    "extension field '{k}' is not valid JSON"
                )));
            }
        }
        serde_json::to_vec_pretty(&Value::Object(root))
            .map_err(|e| CadError::Invariant(format!("annotation encode failed: {e}")))
    }
}

/// A validated affine mapping applied when importing a mismatched file.
struct CoordinateMapping {
    transform: Transform3,
}

impl CoordinateMapping {
    /// Reject non-finite or singular (non-invertible) mappings (audit B10).
    fn validate(transform: &Transform3) -> CadResult<Self> {
        for row in &transform.matrix {
            for v in row {
                if !v.is_finite() {
                    return Err(CadError::InvalidInput(
                        "coordinate mapping contains non-finite values".into(),
                    ));
                }
            }
        }
        if transform.determinant().abs() < 1e-12 {
            return Err(CadError::InvalidInput(
                "coordinate mapping is singular and cannot be applied".into(),
            ));
        }
        Ok(CoordinateMapping {
            transform: *transform,
        })
    }

    fn map_point(&self, p: Point3) -> CadResult<Point3> {
        let out = self.transform.apply_point(p);
        if !out.x.is_finite() || !out.y.is_finite() || !out.z.is_finite() {
            return Err(CadError::InvalidInput(
                "coordinate mapping produced a non-finite point".into(),
            ));
        }
        Ok(out)
    }

    /// Move all geometry and anchor fallbacks, then mark anchors Unresolved:
    /// the match was not verified, so they must not claim `Valid`.
    fn apply_annotation(&self, annotation: &mut Annotation) -> CadResult<()> {
        annotation.geometry = self.map_geometry(&annotation.geometry)?;
        if let Some(anchor) = annotation.anchor.as_mut() {
            anchor.fallback = self.map_point(anchor.fallback)?;
            anchor.status = AnchorStatus::Unresolved;
        }
        Ok(())
    }

    fn map_geometry(&self, geometry: &AnnotationGeometry) -> CadResult<AnnotationGeometry> {
        let map = |p: Point3| self.map_point(p);
        Ok(match geometry {
            AnnotationGeometry::Text(p) => AnnotationGeometry::Text(map(*p)?),
            AnnotationGeometry::Leader(v) => {
                AnnotationGeometry::Leader(v.iter().map(|p| map(*p)).collect::<CadResult<_>>()?)
            }
            AnnotationGeometry::Rectangle(pair) => {
                AnnotationGeometry::Rectangle([map(pair[0])?, map(pair[1])?])
            }
            AnnotationGeometry::Ellipse {
                center,
                axis_u,
                axis_v,
            } => AnnotationGeometry::Ellipse {
                center: map(*center)?,
                // Axes are directions: use the linear part without translation.
                axis_u: self.map_vector(*axis_u)?,
                axis_v: self.map_vector(*axis_v)?,
            },
            AnnotationGeometry::Freehand(v) => {
                AnnotationGeometry::Freehand(v.iter().map(|p| map(*p)).collect::<CadResult<_>>()?)
            }
            AnnotationGeometry::Cloud(v) => {
                AnnotationGeometry::Cloud(v.iter().map(|p| map(*p)).collect::<CadResult<_>>()?)
            }
            AnnotationGeometry::Measurement(m) => {
                let mut m = m.clone();
                m.inputs = m.inputs.iter().map(|p| map(*p)).collect::<CadResult<_>>()?;
                if let Some(plane) = m.plane.as_mut() {
                    plane.origin = map(plane.origin)?;
                    plane.u = self.map_vector(plane.u)?;
                    plane.v = self.map_vector(plane.v)?;
                }
                AnnotationGeometry::Measurement(m)
            }
        })
    }

    fn map_vector(&self, v: Point3) -> CadResult<Point3> {
        let origin = self.map_point(Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        })?;
        let tip = self.map_point(v)?;
        let out = Point3 {
            x: tip.x - origin.x,
            y: tip.y - origin.y,
            z: tip.z - origin.z,
        };
        if !out.x.is_finite() || !out.y.is_finite() || !out.z.is_finite() {
            return Err(CadError::InvalidInput(
                "coordinate mapping produced a non-finite vector".into(),
            ));
        }
        Ok(out)
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
    parse_uuid_strict(s).unwrap_or(0)
}

fn encode_units(units: &UnitContext) -> Value {
    json!({
        "source": encode_unit(&units.source),
        "display": encode_unit(&units.display),
        "display_per_source": units.display_per_source,
        "decimal_places": units.decimal_places,
    })
}

fn encode_unit(unit: &Unit) -> Value {
    json!(match unit {
        Unit::DrawingUnits => "DrawingUnits",
        Unit::Millimeter => "Millimeter",
        Unit::Meter => "Meter",
        Unit::Inch => "Inch",
        Unit::Foot => "Foot",
    })
}

fn decode_unit(value: Option<&Value>) -> Option<Unit> {
    match value.and_then(|v| v.as_str()) {
        Some("DrawingUnits") => Some(Unit::DrawingUnits),
        Some("Millimeter") => Some(Unit::Millimeter),
        Some("Meter") => Some(Unit::Meter),
        Some("Inch") => Some(Unit::Inch),
        Some("Foot") => Some(Unit::Foot),
        _ => None,
    }
}

fn decode_units(value: Option<&Value>) -> UnitContext {
    let Some(v) = value else {
        return UnitContext::drawing_units();
    };
    // Never silently coerce a missing source into the display unit (audit B09):
    // an unreadable source stays Unknown (DrawingUnits).
    let source = decode_unit(v.get("source")).unwrap_or(Unit::DrawingUnits);
    let display = decode_unit(v.get("display")).unwrap_or(Unit::DrawingUnits);
    UnitContext {
        source,
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
        SpaceId::Block(id) => json!({ "Block": id.0.to_string() }),
    }
}

fn decode_space(value: Option<&Value>) -> SpaceId {
    match value {
        Some(Value::Object(m)) => {
            if let Some(block) = m.get("Block").and_then(|v| v.as_str()) {
                return SpaceId::Block(BlockId(block.parse::<u128>().unwrap_or(0)));
            }
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

fn encode_bookmark(bookmark: &ViewBookmark) -> Value {
    json!({
        "name": bookmark.name,
        "viewport": bookmark.viewport.0.to_string(),
        "space": encode_space(&bookmark.space),
        "camera_transform": encode_transform(&bookmark.camera_transform),
    })
}

fn decode_bookmark(value: &Value) -> Option<ViewBookmark> {
    Some(ViewBookmark {
        name: value.get("name")?.as_str()?.to_string(),
        viewport: ViewportId(value.get("viewport")?.as_str()?.parse::<u128>().ok()?),
        space: decode_space(value.get("space")),
        camera_transform: decode_transform(value.get("camera_transform")?)?,
    })
}

fn encode_transform(t: &Transform3) -> Value {
    Value::Array(
        t.matrix
            .iter()
            .map(|row| Value::Array(row.iter().map(|v| json!(v)).collect()))
            .collect(),
    )
}

fn decode_transform(value: &Value) -> Option<Transform3> {
    let rows = value.as_array()?;
    if rows.len() != 4 {
        return None;
    }
    let mut matrix = [[0.0f64; 4]; 4];
    for (i, row) in rows.iter().enumerate() {
        let cols = row.as_array()?;
        if cols.len() != 4 {
            return None;
        }
        for (j, v) in cols.iter().enumerate() {
            let n = v.as_f64()?;
            if !n.is_finite() {
                return None;
            }
            matrix[i][j] = n;
        }
    }
    Some(Transform3 { matrix })
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

fn encode_plane(plane: WorkPlane) -> Value {
    json!({
        "origin": encode_point(plane.origin),
        "u": encode_point(plane.u),
        "v": encode_point(plane.v),
    })
}

fn decode_plane(value: &Value) -> Option<WorkPlane> {
    Some(WorkPlane {
        origin: decode_point(value.get("origin")?)?,
        u: decode_point(value.get("u")?)?,
        v: decode_point(value.get("v")?)?,
    })
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

fn decode_algorithm(value: Option<&Value>) -> Option<MeasurementAlgorithm> {
    match value.and_then(|v| v.as_str()) {
        Some("Distance2d") => Some(MeasurementAlgorithm::Distance2d),
        Some("Distance3d") => Some(MeasurementAlgorithm::Distance3d),
        Some("PolylineLength") => Some(MeasurementAlgorithm::PolylineLength),
        Some("Angle3Points") => Some(MeasurementAlgorithm::Angle3Points),
        Some("PlanarPolygonArea") => Some(MeasurementAlgorithm::PlanarPolygonArea),
        _ => None,
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

fn decode_geometry_source(value: Option<&Value>) -> Option<GeometrySource> {
    match value.and_then(|v| v.as_str()) {
        Some("Analytic") => Some(GeometrySource::Analytic),
        Some("DirectMesh") => Some(GeometrySource::DirectMesh),
        Some("ProxyCache") => Some(GeometrySource::ProxyCache),
        Some("KernelMesh") => Some(GeometrySource::KernelMesh),
        Some("UserPoints") => Some(GeometrySource::UserPoints),
        _ => None,
    }
}

fn encode_precision(precision: &Precision) -> Value {
    match precision {
        Precision::Analytic => json!({ "kind": "analytic" }),
        Precision::Approximate { error_bound } => {
            json!({ "kind": "approximate", "error_bound": error_bound })
        }
        Precision::Unknown => json!({ "kind": "unknown" }),
    }
}

fn decode_precision(value: Option<&Value>) -> Option<Precision> {
    let v = value?;
    match v.get("kind").and_then(|k| k.as_str()) {
        Some("analytic") => Some(Precision::Analytic),
        Some("approximate") => Some(Precision::Approximate {
            error_bound: v.get("error_bound").and_then(|b| b.as_f64()),
        }),
        Some("unknown") => Some(Precision::Unknown),
        _ => None,
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
            // Missing algorithm/units/source/precision must not be silently
            // approximated as analytic/UserPoints (audit B09).
            let algorithm = decode_algorithm(value.get("algorithm"))?;
            let units = value.get("units").map(|u| decode_units(Some(u)));
            let units = units.unwrap_or_else(UnitContext::drawing_units);
            let source = decode_geometry_source(value.get("source"))?;
            let precision = decode_precision(value.get("precision")).unwrap_or(Precision::Unknown);
            Some(AnnotationGeometry::Measurement(MeasurementRecord {
                algorithm,
                inputs: decode_points(value.get("points")?)?,
                plane: value.get("plane").and_then(decode_plane),
                value: value.get("value").and_then(|v| v.as_f64())?,
                units,
                source,
                precision,
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
    if a.len() != 3 {
        return None;
    }
    let p = Point3 {
        x: a[0].as_f64()?,
        y: a[1].as_f64()?,
        z: a[2].as_f64()?,
    };
    if !p.x.is_finite() || !p.y.is_finite() || !p.z.is_finite() {
        return None;
    }
    Some(p)
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

fn decode_instance_path(value: Option<&Value>) -> Option<InstancePath> {
    let Some(Value::Array(items)) = value else {
        return Some(InstancePath::default());
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let raw = item.as_str()?;
        out.push(EntityId(raw.parse::<u128>().ok()?));
    }
    Some(InstancePath(out))
}

fn encode_anchor_status(status: &AnchorStatus) -> Value {
    json!(match status {
        AnchorStatus::Valid => "Valid",
        AnchorStatus::Stale => "Stale",
        AnchorStatus::Unresolved => "Unresolved",
    })
}

fn decode_anchor_status(value: Option<&Value>) -> Option<AnchorStatus> {
    match value.and_then(|v| v.as_str()) {
        Some("Valid") => Some(AnchorStatus::Valid),
        Some("Stale") => Some(AnchorStatus::Stale),
        Some("Unresolved") => Some(AnchorStatus::Unresolved),
        _ => None,
    }
}

fn decode_annotation(value: &Value) -> Option<Annotation> {
    let geometry = decode_geometry(value.get("geometry")?)?;
    let id = decode_id(value.get("id")?)?;
    let rgba = decode_rgba(value.get("style"))?;
    let style = AnnotationStyle {
        rgba,
        logical_width: value
            .get("style")
            .and_then(|s| s.get("logical_width"))
            .and_then(|v| v.as_f64())?,
        text_height: value
            .get("style")
            .and_then(|s| s.get("text_height"))
            .and_then(|v| v.as_f64())?,
    };
    if !style.logical_width.is_finite() || !style.text_height.is_finite() {
        return None;
    }
    let created = value.get("created_unix_ms").and_then(|v| v.as_i64())?;
    let modified = value.get("modified_unix_ms").and_then(|v| v.as_i64())?;
    let anchor = match value.get("anchor") {
        None | Some(Value::Null) => None,
        Some(anchor) => Some(decode_anchor(anchor)?),
    };
    let precision = decode_precision(value.get("precision")).unwrap_or(Precision::Unknown);
    Some(Annotation {
        id: AnnotationId(id),
        space: decode_space(value.get("space")),
        geometry,
        text: value
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        style,
        created_unix_ms: created,
        modified_unix_ms: modified,
        anchor,
        precision,
    })
}

fn decode_rgba(style: Option<&Value>) -> Option<[u8; 4]> {
    let a = style?.get("rgba")?.as_array()?;
    if a.len() != 4 {
        return None;
    }
    let mut out = [0u8; 4];
    for (i, v) in a.iter().enumerate() {
        out[i] = u8::try_from(v.as_u64()?).ok()?;
    }
    Some(out)
}

fn decode_anchor(value: &Value) -> Option<EntityAnchor> {
    let source_handle = value.get("source_handle")?.as_str()?.to_string();
    if source_handle.is_empty() {
        return None;
    }
    let instance = decode_instance_path(value.get("instance"))?;
    let sub_element = match value.get("sub_element") {
        None | Some(Value::Null) => None,
        Some(s) => Some(SubElementId {
            source_key: s.get("source_key")?.as_str()?.to_string(),
            topology_revision: Revision(s.get("topology_revision")?.as_u64()?),
        }),
    };
    Some(EntityAnchor {
        source_handle,
        instance,
        sub_element,
        fallback: decode_point(value.get("fallback")?)?,
        status: decode_anchor_status(value.get("status")).unwrap_or(AnchorStatus::Unresolved),
    })
}

/// Decode an annotation id with strict UUID validation (audit B11).
///
/// A malformed id is an error, not a silent `AnnotationId(0)`.
fn decode_id(value: &Value) -> Option<u128> {
    let s = value.as_str()?;
    parse_uuid_strict(s)
}

fn parse_uuid_strict(s: &str) -> Option<u128> {
    // Accept canonical 8-4-4-4-12 UUIDs and bare 32-hex-digit strings.
    let hex: String = s.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u128::from_str_radix(&hex, 16).ok()
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

    fn round_trip(annotation: Annotation) -> Annotation {
        let service = AnnotationService;
        let identity = DocumentIdentity::Sha256([7u8; 32]);
        let file = AnnotationFile {
            schema_version: SCHEMA_VERSION,
            application_version: "test".into(),
            document_fingerprint: identity.clone(),
            document_name_hint: "sample.dwg".into(),
            unit_context: UnitContext::drawing_units(),
            annotations: vec![annotation],
            view_bookmarks: Vec::new(),
            extensions_json: BTreeMap::new(),
        };
        let bytes = service.encode(&file).unwrap();
        let decoded = service
            .decode(&bytes, &identity, FingerprintPolicy::RejectMismatch)
            .unwrap();
        decoded.annotations.into_iter().next().unwrap()
    }

    #[test]
    fn full_fidelity_round_trip_preserves_all_fields() {
        // Every geometry variant, style, precision and a multi-INSERT anchor
        // must survive the round trip byte-for-byte on the struct (audit B09).
        let style = AnnotationStyle {
            rgba: [1, 2, 3, 4],
            logical_width: 7.5,
            text_height: 3.25,
        };
        let geoms = vec![
            AnnotationGeometry::Text(Point3 {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            }),
            AnnotationGeometry::Leader(vec![
                Point3::default(),
                Point3 {
                    x: 9.0,
                    y: 8.0,
                    z: 0.0,
                },
            ]),
            AnnotationGeometry::Rectangle([
                Point3::default(),
                Point3 {
                    x: 5.0,
                    y: 6.0,
                    z: 0.0,
                },
            ]),
            AnnotationGeometry::Ellipse {
                center: Point3::default(),
                axis_u: Point3 {
                    x: 4.0,
                    y: 0.0,
                    z: 0.0,
                },
                axis_v: Point3 {
                    x: 0.0,
                    y: 2.0,
                    z: 0.0,
                },
            },
            AnnotationGeometry::Freehand(vec![Point3::default()]),
            AnnotationGeometry::Cloud(vec![Point3::default()]),
            AnnotationGeometry::Measurement(MeasurementRecord {
                algorithm: MeasurementAlgorithm::PlanarPolygonArea,
                inputs: vec![
                    Point3::default(),
                    Point3 {
                        x: 1.0,
                        y: 0.0,
                        z: 0.0,
                    },
                ],
                plane: Some(WorkPlane {
                    origin: Point3::default(),
                    u: Point3 {
                        x: 1.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    v: Point3 {
                        x: 0.0,
                        y: 1.0,
                        z: 0.0,
                    },
                }),
                value: 12.5,
                units: UnitContext {
                    source: Unit::Millimeter,
                    display: Unit::Meter,
                    display_per_source: Some(0.001),
                    decimal_places: 4,
                },
                source: GeometrySource::ProxyCache,
                precision: Precision::Approximate {
                    error_bound: Some(0.01),
                },
            }),
        ];
        for geometry in geoms {
            let mut a = ann(0x1234_5678_9abc_def0_1234_5678_9abc_def0, "x");
            a.geometry = geometry;
            a.style = style.clone();
            a.space = SpaceId::Paper(LayoutId(9));
            a.precision = Precision::Approximate {
                error_bound: Some(0.5),
            };
            a.created_unix_ms = 111;
            a.modified_unix_ms = 222;
            let decoded = round_trip(a.clone());
            assert_eq!(decoded, a);
        }
    }

    #[test]
    fn multi_insert_anchor_keeps_instance_and_status() {
        let mut a = ann(1, "anchored");
        a.anchor = Some(EntityAnchor {
            source_handle: "2F".into(),
            instance: InstancePath(vec![EntityId(10), EntityId(20)]),
            sub_element: Some(SubElementId {
                source_key: "edge-3".into(),
                topology_revision: Revision(4),
            }),
            fallback: Point3 {
                x: 1.0,
                y: 2.0,
                z: 0.0,
            },
            status: AnchorStatus::Stale,
        });
        assert_eq!(round_trip(a.clone()), a);
    }

    #[test]
    fn unknown_measurement_algorithm_is_rejected_not_approximated() {
        // A future algorithm must not be silently downgraded to Distance2d.
        let service = AnnotationService;
        let identity = DocumentIdentity::Sha256([0u8; 32]);
        let json =
            br#"{"schema_version":1,"annotations":[{"id":"00000000-0000-0000-0000-000000000001",
            "geometry":{"kind":"measurement","algorithm":"Future4d","value":1.0,"points":[[0,0,0]],
            "source":"UserPoints","precision":{"kind":"analytic"}},"style":{"rgba":[1,2,3,4],
            "logical_width":1.0,"text_height":1.0},"created_unix_ms":0,"modified_unix_ms":0}]}"#;
        assert!(matches!(
            service.decode(json, &identity, FingerprintPolicy::ImportUnanchored),
            Err(CadError::CorruptData(_))
        ));
    }

    #[test]
    fn malformed_annotation_id_is_rejected() {
        let service = AnnotationService;
        let identity = DocumentIdentity::Sha256([0u8; 32]);
        let json = br#"{"schema_version":1,"annotations":[{"id":"not-a-uuid",
            "geometry":{"kind":"text","position":[0,0,0]},"style":{"rgba":[1,2,3,4],
            "logical_width":1.0,"text_height":1.0},"created_unix_ms":0,"modified_unix_ms":0}]}"#;
        assert!(matches!(
            service.decode(json, &identity, FingerprintPolicy::ImportUnanchored),
            Err(CadError::CorruptData(_))
        ));
    }

    #[test]
    fn short_fingerprint_array_is_rejected() {
        let service = AnnotationService;
        let identity = DocumentIdentity::Sha256([0u8; 32]);
        let json = br#"{"schema_version":1,"document_fingerprint":[1,2,3],"annotations":[]}"#;
        assert!(matches!(
            service.decode(json, &identity, FingerprintPolicy::ImportUnanchored),
            Err(CadError::CorruptData(_))
        ));
    }

    #[test]
    fn explicit_mapping_moves_geometry_and_unresolves_anchors() {
        let service = AnnotationService;
        let identity = DocumentIdentity::Sha256([0u8; 32]);
        let mut a = ann(1, "note");
        a.geometry = AnnotationGeometry::Text(Point3 {
            x: 1.0,
            y: 2.0,
            z: 0.0,
        });
        a.anchor = Some(EntityAnchor {
            source_handle: "A".into(),
            instance: InstancePath::default(),
            sub_element: None,
            fallback: Point3 {
                x: 1.0,
                y: 2.0,
                z: 0.0,
            },
            status: AnchorStatus::Valid,
        });
        let file = AnnotationFile {
            schema_version: SCHEMA_VERSION,
            application_version: "t".into(),
            document_fingerprint: DocumentIdentity::Sha256([9u8; 32]),
            document_name_hint: "a.dwg".into(),
            unit_context: UnitContext::drawing_units(),
            annotations: vec![a],
            view_bookmarks: Vec::new(),
            extensions_json: BTreeMap::new(),
        };
        let bytes = service.encode(&file).unwrap();
        let mapping = Transform3::translation(Point3 {
            x: 10.0,
            y: -3.0,
            z: 0.0,
        });
        let decoded = service
            .decode(
                &bytes,
                &identity,
                FingerprintPolicy::ExplicitCoordinateMapping(mapping),
            )
            .unwrap();
        let out = &decoded.annotations[0];
        assert_eq!(
            out.geometry,
            AnnotationGeometry::Text(Point3 {
                x: 11.0,
                y: -1.0,
                z: 0.0
            })
        );
        let anchor = out.anchor.as_ref().unwrap();
        assert_eq!(anchor.fallback.x, 11.0);
        assert_eq!(anchor.fallback.y, -1.0);
        assert_eq!(anchor.status, AnchorStatus::Unresolved);
    }

    #[test]
    fn unanchored_import_clears_anchors() {
        let service = AnnotationService;
        let identity = DocumentIdentity::Sha256([0u8; 32]);
        let mut a = ann(1, "note");
        a.anchor = Some(EntityAnchor {
            source_handle: "A".into(),
            instance: InstancePath::default(),
            sub_element: None,
            fallback: Point3::default(),
            status: AnchorStatus::Valid,
        });
        let file = AnnotationFile {
            schema_version: SCHEMA_VERSION,
            application_version: "t".into(),
            document_fingerprint: DocumentIdentity::Sha256([9u8; 32]),
            document_name_hint: "a.dwg".into(),
            unit_context: UnitContext::drawing_units(),
            annotations: vec![a],
            view_bookmarks: Vec::new(),
            extensions_json: BTreeMap::new(),
        };
        let bytes = service.encode(&file).unwrap();
        let decoded = service
            .decode(&bytes, &identity, FingerprintPolicy::ImportUnanchored)
            .unwrap();
        assert!(decoded.annotations[0].anchor.is_none());
    }

    #[test]
    fn singular_mapping_is_rejected() {
        let service = AnnotationService;
        let identity = DocumentIdentity::Sha256([0u8; 32]);
        let file = AnnotationFile {
            schema_version: SCHEMA_VERSION,
            application_version: "t".into(),
            document_fingerprint: DocumentIdentity::Sha256([9u8; 32]),
            document_name_hint: "a.dwg".into(),
            unit_context: UnitContext::drawing_units(),
            annotations: vec![ann(1, "note")],
            view_bookmarks: Vec::new(),
            extensions_json: BTreeMap::new(),
        };
        let bytes = service.encode(&file).unwrap();
        let mut matrix = [[0.0f64; 4]; 4];
        matrix[3][3] = 1.0; // all-zero linear part => singular
        let singular = Transform3 { matrix };
        assert!(matches!(
            service.decode(
                &bytes,
                &identity,
                FingerprintPolicy::ExplicitCoordinateMapping(singular)
            ),
            Err(CadError::InvalidInput(_))
        ));
    }
}
