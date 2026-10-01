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

#[derive(Debug, Clone, PartialEq)]
pub struct ViewBookmark {
    pub name: String,
    pub viewport: ViewportId,
    pub space: SpaceId,
    pub camera_transform: Transform3,
}

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FingerprintPolicy {
    RejectMismatch,
    ExplicitCoordinateMapping(Transform3),
    ImportUnanchored,
}

/// A deterministic, diagnosable account of how a decoded file's fingerprint was
/// reconciled with the open drawing (audit B10).
///
/// Decoding never silently discards a mismatch: the caller can inspect this
/// report (via [`AnnotationService::decode_with_report`]) to explain an import
/// before committing it.
#[derive(Clone, Debug, PartialEq)]
pub struct FingerprintReport {
    /// The fingerprint stored in the sidecar.
    pub file_fingerprint: DocumentIdentity,
    /// The identity of the drawing the file is being decoded against.
    pub open_fingerprint: DocumentIdentity,
    /// Whether the two fingerprints were identical.
    pub matched: bool,
    /// Whether the returned file was rebound to the open drawing's identity
    /// (true exactly when a mismatch was accepted by policy).
    pub rebound: bool,
    /// The policy that was applied.
    pub policy: FingerprintPolicy,
    /// Number of anchors that were detached because the drawing did not match.
    pub anchors_detached: usize,
    /// Whether annotation geometry was transformed by an explicit mapping.
    pub geometry_transformed: bool,
    /// Number of view bookmarks transformed by an explicit mapping.
    pub bookmarks_transformed: usize,
}

/// The decoded file together with the fingerprint reconciliation report.
#[derive(Clone, Debug)]
pub struct DecodeOutcome {
    pub file: AnnotationFile,
    pub fingerprint: FingerprintReport,
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

/// A malformed sidecar value: reported, never silently coerced (audit B11).
fn corrupt(message: impl Into<String>) -> CadError {
    CadError::CorruptData(message.into())
}

/// Fetch a required field, naming it when absent.
fn require<'a>(object: &'a Map<String, Value>, field: &str) -> CadResult<&'a Value> {
    object
        .get(field)
        .ok_or_else(|| corrupt(format!("missing required field '{field}'")))
}

fn require_str<'a>(object: &'a Map<String, Value>, field: &str) -> CadResult<&'a str> {
    require(object, field)?
        .as_str()
        .ok_or_else(|| corrupt(format!("field '{field}' must be a string")))
}

fn require_u64(object: &Map<String, Value>, field: &str) -> CadResult<u64> {
    require(object, field)?
        .as_u64()
        .ok_or_else(|| corrupt(format!("field '{field}' must be an unsigned integer")))
}

fn require_f64(object: &Map<String, Value>, field: &str) -> CadResult<f64> {
    require(object, field)?
        .as_f64()
        .ok_or_else(|| corrupt(format!("field '{field}' must be a number")))
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
    ///
    /// Thin wrapper over [`AnnotationService::decode_with_report`] that discards
    /// the fingerprint report. Use the report form when the caller must explain
    /// (or log) how a mismatch was reconciled.
    pub fn decode(
        &self,
        bytes: &[u8],
        identity: &DocumentIdentity,
        policy: FingerprintPolicy,
    ) -> CadResult<AnnotationFile> {
        Ok(self.decode_with_report(bytes, identity, policy)?.file)
    }

    /// Decode an annotation file, returning the fingerprint reconciliation
    /// report alongside it (audit B10).
    ///
    /// The format is versioned JSON. This build reads schema version
    /// [`SCHEMA_VERSION`] (currently 1); newer versions are refused with
    /// [`CadError::Unsupported`]. Unknown top-level fields are preserved verbatim
    /// so a newer writer's data survives an older reader. Malformed values are
    /// reported as [`CadError::CorruptData`] and never silently coerced
    /// (audit B11).
    pub fn decode_with_report(
        &self,
        bytes: &[u8],
        identity: &DocumentIdentity,
        policy: FingerprintPolicy,
    ) -> CadResult<DecodeOutcome> {
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

        // A missing fingerprint is corruption, not an implicit match with
        // `Temporary(0)` (audit B11).
        let file_fingerprint = decode_identity(object.get("document_fingerprint"))?;
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

        let annotations_value = object.get("annotations").ok_or_else(|| {
            CadError::CorruptData("annotation file has no 'annotations' array".into())
        })?;
        let items = annotations_value
            .as_array()
            .ok_or_else(|| CadError::CorruptData("'annotations' must be an array".into()))?;
        let mut annotations = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            // A malformed annotation is reported, not silently dropped
            // (audit B11): a caller must never believe a lossy import is OK.
            let a = decode_annotation(item)
                .map_err(|e| corrupt(format!("annotation at index {index}: {}", e)))?;
            annotations.push(a);
        }
        // Duplicate ids would silently collapse on import; refuse them instead.
        let mut seen_ids = std::collections::BTreeSet::new();
        for annotation in &annotations {
            if !seen_ids.insert(annotation.id) {
                return Err(corrupt(format!(
                    "annotation id {:?} appears more than once",
                    annotation.id
                )));
            }
        }

        // Preserve unknown top-level fields verbatim.
        let mut extensions_json = BTreeMap::new();
        for (k, v) in object {
            if !KNOWN_TOP_LEVEL.contains(&k.as_str()) {
                extensions_json.insert(k.clone(), v.to_string());
            }
        }

        // Decode bookmarks before the fingerprint policy so an explicit mapping
        // can move their camera transforms too (audit B10).
        let mut view_bookmarks = match object.get("view_bookmarks") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(items)) => items
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    decode_bookmark(b)
                        .map_err(|e| corrupt(format!("view bookmark at index {i}: {}", e)))
                })
                .collect::<CadResult<Vec<_>>>()?,
            Some(_) => {
                return Err(CadError::CorruptData(
                    "'view_bookmarks' must be an array".into(),
                ))
            }
        };

        // Apply the fingerprint policy to the decoded content (audit B10). A
        // mismatch must not leave anchors pointing into a document they were not
        // bound to, and an explicit mapping must actually move the geometry.
        let mut anchors_detached = 0usize;
        let mut geometry_transformed = false;
        let mut bookmarks_transformed = 0usize;
        if !matches {
            match policy {
                FingerprintPolicy::RejectMismatch => unreachable!("rejected above"),
                FingerprintPolicy::ImportUnanchored => {
                    for annotation in &mut annotations {
                        if annotation.anchor.take().is_some() {
                            anchors_detached += 1;
                        }
                    }
                }
                FingerprintPolicy::ExplicitCoordinateMapping(transform) => {
                    let mapping = CoordinateMapping::validate(&transform)?;
                    for annotation in &mut annotations {
                        if annotation.anchor.is_some() {
                            anchors_detached += 1;
                        }
                        mapping.apply_annotation(annotation)?;
                    }
                    geometry_transformed = !annotations.is_empty();
                    for bookmark in &mut view_bookmarks {
                        bookmark.camera_transform =
                            mapping.map_transform(&bookmark.camera_transform)?;
                        bookmarks_transformed += 1;
                    }
                }
            }
        }

        let outcome_file = AnnotationFile {
            schema_version: schema_version.max(1),
            application_version: match object.get("application_version") {
                None | Some(Value::Null) => String::new(),
                Some(v) => v
                    .as_str()
                    .ok_or_else(|| corrupt("'application_version' must be a string"))?
                    .to_string(),
            },
            document_fingerprint: if matches {
                file_fingerprint.clone()
            } else {
                // The mismatch was accepted by policy, so the imported content
                // now belongs to the open drawing; rebind the identity instead
                // of re-emitting a fingerprint that can never match (audit B10).
                identity.clone()
            },
            document_name_hint: match object.get("document_name_hint") {
                None | Some(Value::Null) => String::new(),
                Some(v) => v
                    .as_str()
                    .ok_or_else(|| corrupt("'document_name_hint' must be a string"))?
                    .to_string(),
            },
            unit_context: decode_units(object.get("unit_context"))?,
            annotations,
            view_bookmarks: std::mem::take(&mut view_bookmarks),
            extensions_json,
        };

        Ok(DecodeOutcome {
            file: outcome_file,
            fingerprint: FingerprintReport {
                file_fingerprint,
                open_fingerprint: identity.clone(),
                matched: matches,
                rebound: !matches,
                policy,
                anchors_detached,
                geometry_transformed,
                bookmarks_transformed,
            },
        })
    }

    /// Encode an annotation file to JSON.
    ///
    /// The output always carries the current [`SCHEMA_VERSION`]; a file claiming
    /// a newer version is refused rather than silently downgraded. Extension
    /// fields that collide with a known schema field are refused rather than
    /// silently dropped, so an export can never report success while losing data
    /// (audit B11).
    pub fn encode(&self, file: &AnnotationFile) -> CadResult<Vec<u8>> {
        if file.schema_version > SCHEMA_VERSION {
            return Err(CadError::Unsupported(format!(
                "cannot encode annotation schema version {} (this build writes {SCHEMA_VERSION})",
                file.schema_version
            )));
        }
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
            // (audit B11): refuse rather than drop, so the caller learns the
            // export would be lossy instead of silently succeeding.
            if KNOWN_TOP_LEVEL.contains(&k.as_str()) {
                return Err(CadError::InvalidInput(format!(
                    "extension field '{k}' collides with a schema field"
                )));
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

    /// Atomically export `file` for the current database revision.
    ///
    /// The bytes are produced first; only if encoding fully succeeds is the
    /// captured revision marked saved. A failed encode therefore returns an
    /// error, leaves the database dirty and does not advance its revision
    /// (audit B07/F09). Hosts that must wait for a durable write should instead
    /// call [`AnnotationService::encode`] and confirm with
    /// [`AnnotationDatabase::mark_exported`] once the write succeeds.
    pub fn export_sidecar(
        &self,
        database: &mut AnnotationDatabase,
        file: &AnnotationFile,
    ) -> CadResult<Vec<u8>> {
        let revision = database.revision();
        let bytes = self.encode(file)?;
        database.mark_exported(revision)?;
        Ok(bytes)
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

    /// Transform a camera/placement transform by composing `mapping` on the
    /// left: the mapped world of the file feeds the original transform.
    fn map_transform(&self, t: &Transform3) -> CadResult<Transform3> {
        let composed = self.transform.matrix_mul(t);
        for row in &composed.matrix {
            for v in row {
                if !v.is_finite() {
                    return Err(CadError::InvalidInput(
                        "coordinate mapping produced a non-finite transform".into(),
                    ));
                }
            }
        }
        Ok(composed)
    }
}

/// Decode a document fingerprint with strict, exact typing (audit B11).
///
/// A missing, malformed or wrong-typed fingerprint is corruption; it is never
/// coerced into `Temporary(0)`.
fn decode_identity(value: Option<&Value>) -> CadResult<DocumentIdentity> {
    match value {
        Some(Value::String(s)) => parse_uuid_strict(s)
            .map(DocumentIdentity::Temporary)
            .ok_or_else(|| corrupt("document_fingerprint is not a valid UUID string")),
        Some(Value::Array(a)) => {
            // A SHA-256 fingerprint is exactly 32 bytes in 0..=255; a short or
            // over-long array must not be zero-padded or truncated into a
            // plausible-looking identity (audit B11).
            if a.len() != 32 {
                return Err(corrupt("document_fingerprint must be 32 bytes"));
            }
            let mut bytes = [0u8; 32];
            for (i, v) in a.iter().enumerate() {
                let n = v
                    .as_u64()
                    .ok_or_else(|| corrupt("document_fingerprint byte is not an integer"))?;
                bytes[i] = u8::try_from(n)
                    .map_err(|_| corrupt("document_fingerprint byte out of range"))?;
            }
            Ok(DocumentIdentity::Sha256(bytes))
        }
        Some(_) => Err(corrupt(
            "document_fingerprint must be a UUID string or a 32-byte array",
        )),
        None => Err(corrupt("annotation file has no document_fingerprint")),
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

fn decode_unit(value: Option<&Value>) -> CadResult<Unit> {
    match value {
        // A missing unit is intentionally Unknown (DrawingUnits); this is the
        // documented "unreadable source stays Unknown" rule (audit B09).
        None | Some(Value::Null) => Ok(Unit::DrawingUnits),
        Some(v) => match v.as_str() {
            Some("DrawingUnits") => Ok(Unit::DrawingUnits),
            Some("Millimeter") => Ok(Unit::Millimeter),
            Some("Meter") => Ok(Unit::Meter),
            Some("Inch") => Ok(Unit::Inch),
            Some("Foot") => Ok(Unit::Foot),
            // An unrecognized unit string is corruption, not silence.
            Some(other) => Err(corrupt(format!("unknown unit '{other}'"))),
            None => Err(corrupt("unit must be a string")),
        },
    }
}

/// Decode a unit context, bounded and strict (audit B09/B11).
///
/// `decimal_places` is range-checked instead of being truncated by an `as u8`
/// cast; `display_per_source` must be finite when present.
fn decode_units(value: Option<&Value>) -> CadResult<UnitContext> {
    let Some(v) = value else {
        return Ok(UnitContext::drawing_units());
    };
    let object = v
        .as_object()
        .ok_or_else(|| corrupt("unit_context must be an object"))?;
    let decimal_places = match object.get("decimal_places") {
        None | Some(Value::Null) => 3u8,
        Some(d) => {
            let raw = d
                .as_u64()
                .ok_or_else(|| corrupt("decimal_places must be an unsigned integer"))?;
            u8::try_from(raw).map_err(|_| corrupt("decimal_places does not fit in 8 bits"))?
        }
    };
    let display_per_source = match object.get("display_per_source") {
        None | Some(Value::Null) => None,
        Some(d) => {
            let n = d
                .as_f64()
                .ok_or_else(|| corrupt("display_per_source must be a number"))?;
            if !n.is_finite() {
                return Err(corrupt("display_per_source must be finite"));
            }
            Some(n)
        }
    };
    Ok(UnitContext {
        source: decode_unit(object.get("source"))?,
        display: decode_unit(object.get("display"))?,
        display_per_source,
        decimal_places,
    })
}

fn encode_space(space: &SpaceId) -> Value {
    match space {
        SpaceId::Model => json!("Model"),
        SpaceId::Paper(id) => json!({ "Paper": id.0.to_string() }),
        SpaceId::Block(id) => json!({ "Block": id.0.to_string() }),
    }
}

/// Decode a space id strictly: a malformed or unknown space is corruption, not
/// a silent `Model`/`Paper(0)` (audit B11).
fn decode_space(value: Option<&Value>) -> CadResult<SpaceId> {
    match value {
        Some(Value::String(s)) if s == "Model" => Ok(SpaceId::Model),
        Some(Value::Object(m)) => {
            if let Some(block) = m.get("Block") {
                let raw = block
                    .as_str()
                    .ok_or_else(|| corrupt("space 'Block' id must be a string"))?;
                let id = raw
                    .parse::<u128>()
                    .map_err(|_| corrupt("space 'Block' id is not a valid integer"))?;
                return Ok(SpaceId::Block(BlockId(id)));
            }
            if let Some(paper) = m.get("Paper") {
                let raw = paper
                    .as_str()
                    .ok_or_else(|| corrupt("space 'Paper' id must be a string"))?;
                let id = raw
                    .parse::<u128>()
                    .map_err(|_| corrupt("space 'Paper' id is not a valid integer"))?;
                return Ok(SpaceId::Paper(LayoutId(id)));
            }
            Err(corrupt("space object must name 'Paper' or 'Block'"))
        }
        Some(_) => Err(corrupt("space must be \"Model\" or an object")),
        None => Err(corrupt("missing required field 'space'")),
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

fn decode_bookmark(value: &Value) -> CadResult<ViewBookmark> {
    let object = value
        .as_object()
        .ok_or_else(|| corrupt("view bookmark must be an object"))?;
    let viewport_raw = require_str(object, "viewport")?;
    let viewport = viewport_raw
        .parse::<u128>()
        .map_err(|_| corrupt("view bookmark 'viewport' is not a valid integer"))?;
    Ok(ViewBookmark {
        name: require_str(object, "name")?.to_string(),
        viewport: ViewportId(viewport),
        space: decode_space(object.get("space"))?,
        camera_transform: decode_transform(require(object, "camera_transform")?)?,
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

fn decode_transform(value: &Value) -> CadResult<Transform3> {
    let rows = value
        .as_array()
        .ok_or_else(|| corrupt("transform must be a 4x4 array"))?;
    if rows.len() != 4 {
        return Err(corrupt("transform must have 4 rows"));
    }
    let mut matrix = [[0.0f64; 4]; 4];
    for (i, row) in rows.iter().enumerate() {
        let cols = row
            .as_array()
            .ok_or_else(|| corrupt("transform row must be an array"))?;
        if cols.len() != 4 {
            return Err(corrupt("transform row must have 4 columns"));
        }
        for (j, v) in cols.iter().enumerate() {
            let n = v
                .as_f64()
                .ok_or_else(|| corrupt("transform entry must be a number"))?;
            if !n.is_finite() {
                return Err(corrupt("transform entry must be finite"));
            }
            matrix[i][j] = n;
        }
    }
    Ok(Transform3 { matrix })
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

fn decode_plane(value: &Value) -> CadResult<WorkPlane> {
    let object = value
        .as_object()
        .ok_or_else(|| corrupt("work plane must be an object"))?;
    Ok(WorkPlane {
        origin: decode_point(require(object, "origin")?)?,
        u: decode_point(require(object, "u")?)?,
        v: decode_point(require(object, "v")?)?,
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

fn encode_precision(precision: &Precision) -> Value {
    match precision {
        Precision::Analytic => json!({ "kind": "analytic" }),
        Precision::Approximate { error_bound } => {
            json!({ "kind": "approximate", "error_bound": error_bound })
        }
        Precision::Unknown => json!({ "kind": "unknown" }),
    }
}

fn decode_precision(value: Option<&Value>) -> CadResult<Precision> {
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

fn decode_geometry(value: &Value) -> CadResult<AnnotationGeometry> {
    let object = value
        .as_object()
        .ok_or_else(|| corrupt("geometry must be an object"))?;
    let kind = require_str(object, "kind")?;
    match kind {
        "text" => Ok(AnnotationGeometry::Text(decode_point(require(
            object, "position",
        )?)?)),
        "leader" => Ok(AnnotationGeometry::Leader(decode_points(require(
            object, "points",
        )?)?)),
        "freehand" => Ok(AnnotationGeometry::Freehand(decode_points(require(
            object, "points",
        )?)?)),
        "cloud" => Ok(AnnotationGeometry::Cloud(decode_points(require(
            object, "points",
        )?)?)),
        "rectangle" => Ok(AnnotationGeometry::Rectangle([
            decode_point(require(object, "a")?)?,
            decode_point(require(object, "b")?)?,
        ])),
        "ellipse" => Ok(AnnotationGeometry::Ellipse {
            center: decode_point(require(object, "center")?)?,
            axis_u: decode_point(require(object, "axis_u")?)?,
            axis_v: decode_point(require(object, "axis_v")?)?,
        }),
        "measurement" => {
            // Missing/invalid algorithm, units, source or precision must not be
            // silently approximated as analytic/UserPoints (audit B09).
            let algorithm = decode_algorithm(object.get("algorithm"))?;
            let units = decode_units(object.get("units"))?;
            let source = decode_geometry_source(object.get("source"))?;
            let precision = decode_precision(object.get("precision"))?;
            Ok(AnnotationGeometry::Measurement(MeasurementRecord {
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
            }))
        }
        other => Err(corrupt(format!("unknown geometry kind '{other}'"))),
    }
}

fn encode_point(p: Point3) -> Value {
    json!([p.x, p.y, p.z])
}

fn decode_point(value: &Value) -> CadResult<Point3> {
    let a = value
        .as_array()
        .ok_or_else(|| corrupt("point must be an array"))?;
    if a.len() != 3 {
        return Err(corrupt("point must have exactly 3 coordinates"));
    }
    let read = |i: usize| {
        a[i].as_f64()
            .ok_or_else(|| corrupt("point coordinate must be a number"))
    };
    let p = Point3 {
        x: read(0)?,
        y: read(1)?,
        z: read(2)?,
    };
    if !p.x.is_finite() || !p.y.is_finite() || !p.z.is_finite() {
        return Err(corrupt("point coordinate must be finite"));
    }
    Ok(p)
}

fn decode_points(value: &Value) -> CadResult<Vec<Point3>> {
    value
        .as_array()
        .ok_or_else(|| corrupt("points must be an array"))?
        .iter()
        .map(decode_point)
        .collect()
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
fn decode_annotation(value: &Value) -> CadResult<Annotation> {
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

/// Decode an annotation id with strict UUID validation (audit B11).
///
/// A malformed id is an error, not a silent `AnnotationId(0)`.
fn decode_id(value: &Value) -> CadResult<u128> {
    let s = value
        .as_str()
        .ok_or_else(|| corrupt("annotation id must be a string"))?;
    parse_uuid_strict(s).ok_or_else(|| corrupt("annotation id is not a valid UUID"))
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

    /// A matching all-zero SHA-256 fingerprint, so a test can reach the
    /// annotation-level decoders without tripping the fingerprint check.
    const ZERO_FP: &str = r#""document_fingerprint":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"#;

    #[test]
    fn unknown_measurement_algorithm_is_rejected_not_approximated() {
        // A future algorithm must not be silently downgraded to Distance2d.
        let service = AnnotationService;
        let identity = DocumentIdentity::Sha256([0u8; 32]);
        let json = format!(
            r#"{{"schema_version":1,{ZERO_FP}"annotations":[{{"id":"00000000-0000-0000-0000-000000000001",
            "space":"Model","geometry":{{"kind":"measurement","algorithm":"Future4d","value":1.0,"points":[[0,0,0]],
            "units":{{"source":"Millimeter","display":"Millimeter","decimal_places":3}},
            "source":"UserPoints","precision":{{"kind":"analytic"}}}},"text":"t","style":{{"rgba":[1,2,3,4],
            "logical_width":1.0,"text_height":1.0}},"created_unix_ms":0,"modified_unix_ms":0,"precision":{{"kind":"analytic"}}}}]}}"#
        );
        assert!(matches!(
            service.decode(
                json.as_bytes(),
                &identity,
                FingerprintPolicy::ImportUnanchored
            ),
            Err(CadError::CorruptData(_))
        ));
    }

    #[test]
    fn malformed_annotation_id_is_rejected() {
        let service = AnnotationService;
        let identity = DocumentIdentity::Sha256([0u8; 32]);
        let json = format!(
            r#"{{"schema_version":1,{ZERO_FP}"annotations":[{{"id":"not-a-uuid",
            "space":"Model","geometry":{{"kind":"text","position":[0,0,0]}},"text":"t","style":{{"rgba":[1,2,3,4],
            "logical_width":1.0,"text_height":1.0}},"created_unix_ms":0,"modified_unix_ms":0,"precision":{{"kind":"analytic"}}}}]}}"#
        );
        assert!(matches!(
            service.decode(
                json.as_bytes(),
                &identity,
                FingerprintPolicy::ImportUnanchored
            ),
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
