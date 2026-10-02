//! Headless core CLI. Every data operation shares the application/database
//! command path used by the UI; there is no second business implementation.
//!
//! Spec v2.0 §19: operations return structured JSON with object IDs, transaction
//! results and diagnostics — never an opaque success string. Diagnostics are
//! redacted by `cad-diagnostics`; fixed-viewport rendering explicitly requires
//! a GPU environment and otherwise reports "not run".
//!
//! Machine contract (audit B30, N01 §5.3):
//!
//! * stdout carries **only** the pretty-printed JSON result document. Every
//!   human-facing string lives on stderr, so `--locale` can never change a
//!   machine key or value.
//! * On any failure the process prints
//!   `{schema_version, operation, error:{code,message,context}}` to stderr and
//!   exits non-zero. `code` is a stable machine key; `message` is localized.
//! * `--out <file>` writes the JSON result atomically (same-directory temp file
//!   followed by a rename), so a failed run never leaves a partial file.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cad_app::host::HostController;
use cad_app::{Command, CommandId, CommandPayload};
use cad_domain::*;

/// Machine schema version for CLI result and error documents.
///
/// This is a stable contract identifier: it does not change with `--locale`.
pub const CLI_SCHEMA_VERSION: u32 = 1;

/// Default locale for human-facing messages (simplified Chinese).
pub const DEFAULT_LOCALE: &str = "zh-CN";

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliOperation {
    Scan,
    ProxyReport,
    Measure,
    ImportNotes,
    ExportNotes,
    BuildRepresentation,
    FixedViewportRender,
    Benchmark,
}

impl CliOperation {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "scan" => Some(Self::Scan),
            "proxy-report" => Some(Self::ProxyReport),
            "measure" => Some(Self::Measure),
            "import-notes" => Some(Self::ImportNotes),
            "export-notes" => Some(Self::ExportNotes),
            "build-representation" => Some(Self::BuildRepresentation),
            "render" => Some(Self::FixedViewportRender),
            "benchmark" => Some(Self::Benchmark),
            _ => None,
        }
    }

    /// The stable machine key reported in result and error documents.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scan => "scan",
            Self::ProxyReport => "proxy-report",
            Self::Measure => "measure",
            Self::ImportNotes => "import-notes",
            Self::ExportNotes => "export-notes",
            Self::BuildRepresentation => "build-representation",
            Self::FixedViewportRender => "render",
            Self::Benchmark => "benchmark",
        }
    }
}

// ---------------------------------------------------------------------------
// Human locale (never affects machine output)
// ---------------------------------------------------------------------------

/// Locale for human-facing strings printed on stderr.
///
/// Machine output (stdout JSON and error `code`s) is locale-independent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Locale {
    #[default]
    ZhCn,
    En,
}

impl Locale {
    /// Normalize an input tag; unsupported tags fall back to `zh-CN`.
    pub fn parse(tag: &str) -> Self {
        let lower = tag.trim().to_ascii_lowercase();
        if lower == "en" || lower.starts_with("en-") {
            Locale::En
        } else {
            Locale::ZhCn
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Locale::ZhCn => "zh-CN",
            Locale::En => "en",
        }
    }
}

// ---------------------------------------------------------------------------
// Invocation
// ---------------------------------------------------------------------------

/// A versioned invocation record; re-running it reproduces the command.
#[derive(Debug, Clone)]
pub struct CliInvocation {
    pub schema_version: u32,
    pub operation: CliOperation,
    pub input: PathBuf,
    pub notes: Option<PathBuf>,
    /// Measure input in the input file's coordinate space.
    pub points: Vec<Point3>,
    /// Annotation import: attach despite a fingerprint mismatch.
    pub allow_fingerprint_mismatch: bool,
    /// Fonts to shape text with, as `(key, path)`.
    pub fonts: Vec<(String, PathBuf)>,
    /// Optional file that receives the JSON result atomically.
    pub out: Option<PathBuf>,
    /// Optional PNG that `render` writes the offscreen frame to.
    pub png: Option<PathBuf>,
    /// Offscreen frame width in pixels for `render`.
    pub render_width: u32,
    /// Offscreen frame height in pixels for `render`.
    pub render_height: u32,
    /// Locale for human-facing stderr messages (never machine output).
    pub locale: Locale,
}

/// Default offscreen frame width for `render`.
pub const DEFAULT_RENDER_WIDTH: u32 = 1280;
/// Default offscreen frame height for `render`.
pub const DEFAULT_RENDER_HEIGHT: u32 = 720;

impl CliInvocation {
    pub fn new(operation: CliOperation, input: impl Into<PathBuf>) -> Self {
        CliInvocation {
            schema_version: CLI_SCHEMA_VERSION,
            operation,
            input: input.into(),
            notes: None,
            points: Vec::new(),
            allow_fingerprint_mismatch: false,
            fonts: Vec::new(),
            out: None,
            png: None,
            render_width: DEFAULT_RENDER_WIDTH,
            render_height: DEFAULT_RENDER_HEIGHT,
            locale: Locale::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Structured errors
// ---------------------------------------------------------------------------

/// Stable machine error codes. These never change with locale.
pub mod error_code {
    pub const INVALID_INPUT: &str = "invalid_input";
    pub const UNSUPPORTED: &str = "unsupported";
    pub const NOT_IMPLEMENTED: &str = "not_implemented";
    pub const RESOURCE_MISSING: &str = "resource_missing";
    pub const CORRUPT_DATA: &str = "corrupt_data";
    pub const GPU_FAILURE: &str = "gpu_failure";
    pub const INVARIANT: &str = "invariant";
    pub const PERMISSION_DENIED: &str = "permission_denied";
    pub const CANCELLED: &str = "cancelled";
    pub const STALE_RESULT: &str = "stale_result";
    /// The operation's input contract was violated (bad option combination).
    pub const USAGE: &str = "usage";
    /// A result could not be written to `--out`.
    pub const OUTPUT_WRITE_FAILED: &str = "output_write_failed";
}

/// A structured CLI failure ready to be serialized to stderr.
///
/// `code` and `context` are stable machine data; `message` is human text whose
/// language may follow `locale`.
#[derive(Debug, Clone)]
pub struct CliError {
    pub code: String,
    pub message: String,
    pub context: serde_json::Value,
    /// Locale for the human-facing `message` line only.
    pub locale: Locale,
}

impl CliError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        CliError {
            code: code.into(),
            message: message.into(),
            context: serde_json::Value::Null,
            locale: Locale::default(),
        }
    }

    pub fn with_context(mut self, context: serde_json::Value) -> Self {
        self.context = context;
        self
    }

    /// Set the human-message locale. Machine keys are unaffected.
    pub fn with_locale(mut self, locale: Locale) -> Self {
        self.locale = locale;
        self
    }

    pub fn locale(&self) -> Locale {
        self.locale
    }

    /// A usage error (exit code 2) for bad argument combinations.
    pub fn usage(message: impl Into<String>) -> Self {
        CliError::new(error_code::USAGE, message)
    }

    /// The error document written to stderr.
    ///
    /// `operation` and every key are stable; only `error.message` is localized.
    pub fn to_json(&self, operation: CliOperation) -> serde_json::Value {
        serde_json::json!({
            "schema_version": CLI_SCHEMA_VERSION,
            "operation": operation.as_str(),
            "error": {
                "code": self.code,
                "message": self.message,
                "context": self.context,
            }
        })
    }

    /// Process exit status: 2 for usage errors, 1 for everything else.
    pub fn exit_code(&self) -> u8 {
        if self.code == error_code::USAGE {
            2
        } else {
            1
        }
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CliError {}

/// Map a domain error to a stable machine code.
pub fn error_code_for(error: &CadError) -> &'static str {
    match error {
        CadError::NotImplemented(_) => error_code::NOT_IMPLEMENTED,
        CadError::InvalidInput(_) => error_code::INVALID_INPUT,
        CadError::Unsupported(_) => error_code::UNSUPPORTED,
        CadError::ResourceMissing(_) => error_code::RESOURCE_MISSING,
        CadError::CorruptData(_) => error_code::CORRUPT_DATA,
        CadError::GpuFailure(_) => error_code::GPU_FAILURE,
        CadError::Invariant(_) => error_code::INVARIANT,
        CadError::PermissionDenied => error_code::PERMISSION_DENIED,
        CadError::Cancelled => error_code::CANCELLED,
        CadError::StaleResult => error_code::STALE_RESULT,
    }
}

/// Convert a domain error into a structured CLI error with redacted context.
///
/// Only the technical error kind is attached as context; file contents, paths
/// and identity values are never copied into the default machine document.
pub fn cli_error_from_domain(error: CadError) -> CliError {
    let code = error_code_for(&error);
    let message = error.to_string();
    CliError::new(code, message)
}

fn domain<T>(result: CadResult<T>) -> Result<T, CliError> {
    result.map_err(cli_error_from_domain)
}

// ---------------------------------------------------------------------------
// Atomic output
// ---------------------------------------------------------------------------

/// Write `contents` to `path` atomically.
///
/// The bytes go to a uniquely named temp file in the same directory, are
/// flushed and synced, and only then replace `path` with a rename. A failure at
/// any step removes the temp file and leaves any existing `path` untouched, so
/// a failed run never leaves a partial result file (audit B30).
pub fn write_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    let directory = path.parent().filter(|p| !p.as_os_str().is_empty());
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "output".to_string());
    let unique = format!(
        ".{}.{}.{}.tmp",
        file_name,
        std::process::id(),
        unique_counter()
    );
    let temp_path = match directory {
        Some(dir) => dir.join(unique),
        None => PathBuf::from(unique),
    };

    let write_result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temp_path)?;
        file.write_all(contents)?;
        file.flush()?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&temp_path);
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&temp_path, path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(error);
    }
    Ok(())
}

/// Monotonic counter so concurrent CLI invocations never share a temp name.
fn unique_counter() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Document loading
// ---------------------------------------------------------------------------

fn read_input(path: &Path) -> CadResult<Arc<[u8]>> {
    let bytes =
        std::fs::read(path).map_err(|e| CadError::InvalidInput(format!("read failed: {e}")))?;
    Ok(Arc::from(bytes.into_boxed_slice()))
}

fn load_document(invocation: &CliInvocation) -> CadResult<HostController> {
    let bytes = read_input(&invocation.input)?;
    let mut controller = HostController::with_demo_document([1920.0, 1080.0])?;
    controller.open_bytes(bytes, &invocation.input.display().to_string())?;
    Ok(controller)
}

// ---------------------------------------------------------------------------
// Shared JSON helpers
// ---------------------------------------------------------------------------

fn completeness_json(completeness: &Completeness) -> serde_json::Value {
    match completeness {
        Completeness::Complete => serde_json::json!({ "status": "complete" }),
        Completeness::Partial(items) => serde_json::json!({ "status": "partial", "items": items }),
        Completeness::Missing(items) => serde_json::json!({ "status": "missing", "items": items }),
        Completeness::Unverified => serde_json::json!({ "status": "unverified" }),
    }
}

fn support_name(status: SupportStatus) -> &'static str {
    match status {
        SupportStatus::NotImplemented => "not_implemented",
        SupportStatus::Unsupported => "unsupported",
        SupportStatus::Unverified => "unverified",
        SupportStatus::Partial => "partial",
        SupportStatus::Verified => "verified",
    }
}

fn units_name(units: &UnitContext) -> String {
    format!("{:?}", units.source)
}

/// Diagnostics flagged as entity-completeness relevant during import.
///
/// A partially- or un-rendered entity is never silently dropped: it contributes
/// a non-empty `completeness.items` or `completeness_issues` entry (audit B30).
fn is_completeness_diagnostic(code: &str) -> bool {
    code.starts_with("import.proxy") || code.starts_with("import.unknown")
}

fn import_completeness_issues(
    report: &cad_import_acadrust::ImportReport,
) -> Vec<serde_json::Value> {
    report
        .diagnostics
        .iter()
        .filter(|d| is_completeness_diagnostic(&d.code))
        .map(|d| {
            serde_json::json!({
                "code": d.code,
                "message": cad_diagnostics::redact_text(&d.message),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

fn run_scan(controller: &HostController) -> CadResult<serde_json::Value> {
    let document = controller
        .application
        .workspace
        .documents
        .get(&controller.document_id)
        .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
    let bounds = document.drawing.bounds().map(|(min, max)| {
        serde_json::json!({ "min": [min.x, min.y, min.z], "max": [max.x, max.y, max.z] })
    });
    let (completeness, diagnostics) = match &controller.last_import_report {
        Some(report) => (
            completeness_json(&report.completeness),
            report
                .diagnostics
                .iter()
                .map(|d| {
                    serde_json::json!({
                        "code": d.code,
                        "message": cad_diagnostics::redact_text(&d.message),
                    })
                })
                .collect::<Vec<_>>(),
        ),
        None => (serde_json::json!({ "status": "unverified" }), Vec::new()),
    };
    Ok(serde_json::json!({
        "schema_version": CLI_SCHEMA_VERSION,
        "operation": CliOperation::Scan.as_str(),
        "entities": document.drawing.entity_count(),
        "model_entities": document.drawing.model_space().len(),
        "block_definitions": document.drawing.blocks().count(),
        "layers": document.drawing.layers().count(),
        "bounds": bounds,
        "units": units_name(&document.units),
        "completeness": completeness,
        "diagnostics": diagnostics,
    }))
}

fn run_proxy_report(controller: &HostController) -> CadResult<serde_json::Value> {
    let report = controller
        .last_import_report
        .as_ref()
        .ok_or_else(|| CadError::InvalidInput("no import report; open a DWG first".into()))?;
    // Every proxy/unknown diagnostic is surfaced, not just a subset: a proxy
    // without a cache is a completeness issue, not silent success (audit B30).
    let proxy_diagnostics = import_completeness_issues(report);
    let types: Vec<_> = report
        .capabilities
        .iter()
        .map(|capability| {
            serde_json::json!({
                "type": capability.type_key,
                "read": support_name(capability.read),
                "semantic": support_name(capability.semantic),
                "render": support_name(capability.render),
                "pick": support_name(capability.pick),
                "measure": support_name(capability.measure),
            })
        })
        .collect();
    Ok(serde_json::json!({
        "schema_version": CLI_SCHEMA_VERSION,
        "operation": CliOperation::ProxyReport.as_str(),
        "completeness": completeness_json(&report.completeness),
        "entity_types": types,
        "proxy_diagnostics": proxy_diagnostics,
        "note": "entity types are reported per capability; absence of a type is not a compatibility claim",
    }))
}

fn run_measure(
    controller: &mut HostController,
    invocation: &CliInvocation,
) -> CadResult<serde_json::Value> {
    if invocation.points.len() < 2 {
        return Err(CadError::InvalidInput(
            "measure needs two (distance), three (angle) or more (length) points".into(),
        ));
    }
    let command = Command {
        schema_version: 1,
        id: CommandId::Measure,
        document: controller.document_id,
        viewport: controller.viewport_id,
        payload: CommandPayload::Points(invocation.points.clone()),
    };
    let outcome = controller.execute(command)?;
    let units = controller
        .application
        .workspace
        .documents
        .get(&controller.document_id)
        .map(|d| units_name(&d.units))
        .unwrap_or_else(|| "unknown".to_string());
    let measurement = outcome.measurement.as_ref().map(measurement_json);
    Ok(serde_json::json!({
        "schema_version": CLI_SCHEMA_VERSION,
        "operation": CliOperation::Measure.as_str(),
        "input_points": invocation.points.iter().map(|p| [p.x, p.y, p.z]).collect::<Vec<_>>(),
        "measurement": measurement,
        "results": outcome.diagnostics.iter().map(|d| serde_json::json!({
            "code": d.code,
            "message": d.message,
        })).collect::<Vec<_>>(),
        "units": units,
    }))
}

/// Structured measurement record: numeric values are locale-independent and
/// carry no formatted human text (N01 §5.3 item 7).
fn measurement_json(record: &cad_db::MeasurementRecord) -> serde_json::Value {
    let plane = record.plane.map(|plane| {
        serde_json::json!({
            "origin": [plane.origin.x, plane.origin.y, plane.origin.z],
            "u": [plane.u.x, plane.u.y, plane.u.z],
            "v": [plane.v.x, plane.v.y, plane.v.z],
        })
    });
    serde_json::json!({
        "algorithm": format!("{:?}", record.algorithm),
        "inputs": record.inputs.iter().map(|p| [p.x, p.y, p.z]).collect::<Vec<_>>(),
        "plane": plane,
        "value": record.value,
        "units": units_name(&record.units),
        "source": format!("{:?}", record.source),
        "precision": format!("{:?}", record.precision),
    })
}

fn run_export_notes(
    controller: &mut HostController,
    invocation: &CliInvocation,
) -> CadResult<serde_json::Value> {
    let target = invocation
        .notes
        .clone()
        .unwrap_or_else(|| invocation.input.with_extension("cadnotes.json"));
    // Take the bundle and the revision atomically; the write and the
    // saved-marking are bound to this exact revision (audit B07/B30).
    let (json, revision) = controller.prepare_annotation_export()?;
    let bytes = json.as_bytes();
    write_atomic(&target, bytes).map_err(|e| {
        CadError::InvalidInput(format!(
            "annotation write failed: {}",
            cad_diagnostics::redact_text(&e.to_string())
        ))
    })?;
    // Only mark saved after the bytes are durably in place.
    controller.confirm_annotation_export(revision)?;
    Ok(serde_json::json!({
        "schema_version": CLI_SCHEMA_VERSION,
        "operation": CliOperation::ExportNotes.as_str(),
        "annotations": controller
            .application
            .workspace
            .documents
            .get(&controller.document_id)
            .map(|d| d.annotations.len())
            .unwrap_or(0),
        "bytes": bytes.len(),
        "revision": revision.0,
        "saved": true,
    }))
}

fn run_import_notes(
    controller: &mut HostController,
    invocation: &CliInvocation,
) -> CadResult<serde_json::Value> {
    let source = invocation
        .notes
        .clone()
        .ok_or_else(|| CadError::InvalidInput("import-notes needs --notes <file>".into()))?;
    let text = std::fs::read_to_string(&source)
        .map_err(|e| CadError::InvalidInput(format!("annotation read failed: {e}")))?;
    let policy = if invocation.allow_fingerprint_mismatch {
        cad_annotations::FingerprintPolicy::ImportUnanchored
    } else {
        cad_annotations::FingerprintPolicy::RejectMismatch
    };
    let count = controller.import_annotations_json(&text, policy)?;
    Ok(serde_json::json!({
        "schema_version": CLI_SCHEMA_VERSION,
        "operation": CliOperation::ImportNotes.as_str(),
        "imported": count,
        "undo_recorded": controller.application.can_undo(&controller.document_id),
    }))
}

/// The representation context shared by `build-representation` and `render`.
///
/// Both must build from the same provider registry, document id, tolerance
/// policy and task stamp; only this constructor is allowed to define that, so
/// the two paths cannot silently drift apart.
fn representation_context(
    controller: &HostController,
    fonts: Option<&Arc<cad_representation::FontEngine>>,
) -> cad_representation::RepresentationContext {
    let mut context = cad_representation::RepresentationContext::new(
        controller.document_id,
        TolerancePolicy::default(),
        TaskStamp::new(controller.document_id, controller.session.generation),
    );
    if let Some(fonts) = fonts {
        context = context.with_fonts(fonts.clone());
    }
    context
}

fn run_build_representation(
    controller: &HostController,
    fonts: Option<&Arc<cad_representation::FontEngine>>,
) -> CadResult<serde_json::Value> {
    let document = controller
        .application
        .workspace
        .documents
        .get(&controller.document_id)
        .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
    let registry = cad_representation::ProviderRegistry::with_default_provider();
    let context = representation_context(controller, fonts);
    let (mut lines, mut meshes, mut texts, mut instances, mut images) = (0usize, 0, 0, 0, 0);
    let mut vertices = 0usize;
    let mut failures: Vec<serde_json::Value> = Vec::new();
    for entity in document.drawing.model_space() {
        match registry.build_expanded(&document.drawing, entity, &context) {
            Ok(representation) => {
                for fragment in &representation.fragments {
                    match &fragment.primitive {
                        cad_representation::DisplayPrimitive::Lines(points) => {
                            lines += 1;
                            vertices += points.len();
                        }
                        cad_representation::DisplayPrimitive::Mesh(mesh) => {
                            meshes += 1;
                            vertices += mesh.vertices.len();
                        }
                        cad_representation::DisplayPrimitive::Text { .. } => texts += 1,
                        cad_representation::DisplayPrimitive::Instance { .. } => instances += 1,
                        cad_representation::DisplayPrimitive::Image { .. } => images += 1,
                    }
                }
            }
            Err(error) => failures.push(serde_json::json!({
                "entity": entity.id.0.to_string(),
                "error": error.to_string(),
            })),
        }
    }
    let primitives = lines + meshes + texts + instances + images;
    Ok(serde_json::json!({
        "schema_version": CLI_SCHEMA_VERSION,
        "operation": CliOperation::BuildRepresentation.as_str(),
        "primitives": primitives,
        "vertices": vertices,
        "kind_counts": {
            "lines": lines,
            "meshes": meshes,
            "texts": texts,
            "instances": instances,
            "images": images,
        },
        "failures": failures,
    }))
}

/// Native fixed-viewport render.
///
/// Imports the drawing through the shared app path, builds the same display
/// representation as `build-representation`, batches it through `SceneCache`,
/// uploads it to a real headless wgpu device, renders one frame, reads it back
/// and optionally writes a PNG.
///
/// Never an empty success: a machine with no adapter fails with
/// `CadError::GpuFailure`, a drawing with no drawable bounds fails with
/// `CadError::InvalidInput`, and the PNG is written only after a frame exists.
#[cfg(not(target_arch = "wasm32"))]
fn run_render(invocation: &CliInvocation) -> CadResult<serde_json::Value> {
    use cad_render_wgpu::headless::{create_headless_gpu, encode_png, HeadlessGpu};
    use cad_render_wgpu::{BackendPreference, Camera2d, RenderTarget, Renderer};

    let controller = load_document(invocation)?;
    let document = controller
        .application
        .workspace
        .documents
        .get(&controller.document_id)
        .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;

    let registry = cad_representation::ProviderRegistry::with_default_provider();
    let context = representation_context(&controller, None);

    // One delta over every model-space entity. `SceneCache::build` skips
    // Text/Instance/Image (documented) and returns line/mesh batches only.
    let mut cache = cad_scene::SceneCache::new(Default::default());
    let mut delta = cad_scene::SceneDelta {
        stamp: context.stamp.clone(),
        added: Vec::new(),
        removed_chunks: Vec::new(),
    };
    for entity in document.drawing.model_space() {
        let representation = registry.build_expanded(&document.drawing, entity, &context)?;
        let built = cache.build(&representation, context.stamp.clone())?;
        delta.added.extend(built.added);
    }

    // Fit to what is actually drawn, not `drawing.bounds()`: the database
    // bounds include material the renderer does not draw (text, unplaced or
    // block-definition geometry), which leaves the framed image off-centre and
    // small. The scene batches already carry world-space points
    // (`local_origin + vertex`), so derive the fit from those.
    let mut fit: Option<(f64, f64, f64, f64)> = None; // min_x, min_y, max_x, max_y
    for batch in &delta.added {
        let ox = batch.local_origin.x;
        let oy = batch.local_origin.y;
        for vertex in &batch.vertices {
            let x = ox + vertex[0] as f64;
            let y = oy + vertex[1] as f64;
            if !x.is_finite() || !y.is_finite() {
                continue;
            }
            fit = Some(match fit {
                None => (x, y, x, y),
                Some((min_x, min_y, max_x, max_y)) => {
                    (min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y))
                }
            });
        }
    }
    let (min_x, min_y, max_x, max_y) =
        fit.ok_or_else(|| CadError::InvalidInput("no drawable geometry to render".into()))?;

    let width = invocation.render_width;
    let height = invocation.render_height;
    let span_x = (max_x - min_x).abs();
    let span_y = (max_y - min_y).abs();
    let world_per_px = (span_x / (width as f64 * 0.9))
        .max(span_y / (height as f64 * 0.9))
        .max(1e-9);
    let camera = Camera2d {
        center: Point3 {
            x: (min_x + max_x) / 2.0,
            y: (min_y + max_y) / 2.0,
            z: 0.0,
        },
        world_per_px,
        z_plane: 0.0,
    };

    let HeadlessGpu {
        device,
        queue,
        adapter,
    } = create_headless_gpu(BackendPreference::Auto)?;
    let mut renderer = Renderer::new(BackendPreference::Auto);
    // A large drawing is tens of thousands of small draw calls; a CPU software
    // adapter can legitimately exceed the interactive 1 s submission bound.
    // Headless evidence waits longer instead of misreporting a device loss.
    renderer.set_poll_timeout(std::time::Duration::from_secs(600));
    renderer.initialize_with_device(device, queue)?;
    renderer.upload(&delta)?;
    let target = RenderTarget::new(width, height);
    let frame = renderer
        .render(camera, &target)
        .map_err(|error| CadError::GpuFailure(error.message().to_string()))?;
    let image = renderer
        .read_target_rgba()
        .map_err(|error| CadError::GpuFailure(error.message().to_string()))?;

    let background = image.pixel(0, 0);
    let non_background = image.count_differing_from(background, 8);
    let coverage = non_background as f64 / (width as f64 * height as f64);

    // Only after a successful frame: encode and atomically write, so a failure
    // never leaves a partial PNG.
    let png_json = match &invocation.png {
        Some(path) => {
            let bytes = encode_png(&image)
                .map_err(|e| CadError::Invariant(format!("PNG encode failed: {e}")))?;
            write_atomic(path, &bytes).map_err(|e| {
                CadError::InvalidInput(format!(
                    "PNG write failed: {}",
                    cad_diagnostics::redact_text(&e.to_string())
                ))
            })?;
            serde_json::json!({
                "path": path.display().to_string(),
                "bytes": bytes.len(),
            })
        }
        None => serde_json::Value::Null,
    };

    let completeness = match &controller.last_import_report {
        Some(report) => completeness_json(&report.completeness),
        None => serde_json::json!({ "status": "unverified" }),
    };
    let scene_vertices: usize = delta.added.iter().map(|batch| batch.vertices.len()).sum();

    Ok(serde_json::json!({
        "schema_version": CLI_SCHEMA_VERSION,
        "operation": CliOperation::FixedViewportRender.as_str(),
        "adapter": {
            "backend": adapter.backend,
            "name": adapter.name,
            "device_type": adapter.device_type,
            "driver": adapter.driver,
            "driver_info": adapter.driver_info,
        },
        "width": width,
        "height": height,
        "png": png_json,
        "pixels": {
            "non_background": non_background,
            "coverage": coverage,
            "distinct_colors": image.distinct_colors(),
        },
        "frame": {
            "draw_calls": frame.draw_calls,
            "vertices": frame.vertices,
            "triangles": frame.triangles,
            "opaque_batches": frame.opaque_batches,
            "transparent_batches": frame.transparent_batches,
            "invisible_batches": frame.invisible_batches,
        },
        "scene": {
            "batches": delta.added.len(),
            "vertices": scene_vertices,
        },
        "completeness": completeness,
        "note": "software/headless frame; not a compatibility or performance claim",
    }))
}

fn run_benchmark(
    controller: &HostController,
    invocation: &CliInvocation,
    fonts: Option<&Arc<cad_representation::FontEngine>>,
) -> CadResult<serde_json::Value> {
    let bytes = std::fs::metadata(&invocation.input)
        .map(|m| m.len())
        .unwrap_or(0);
    let start = std::time::Instant::now();
    let built = run_build_representation(controller, fonts)?;
    let build_ms = start.elapsed().as_secs_f64() * 1000.0;
    Ok(serde_json::json!({
        "schema_version": CLI_SCHEMA_VERSION,
        "operation": CliOperation::Benchmark.as_str(),
        "file_bytes": bytes,
        "representation_build_ms": build_ms,
        "representation": built,
        "environment": {
            "release_build": !cfg!(debug_assertions),
            "gpu": "not required for geometry benchmark",
        },
    }))
}

/// Load the `--font` entries into a shaping engine, if any were given.
fn load_fonts(
    entries: &[(String, PathBuf)],
) -> CadResult<Option<Arc<cad_representation::FontEngine>>> {
    if entries.is_empty() {
        return Ok(None);
    }
    let mut engine = cad_representation::FontEngine::new();
    let mut keys = Vec::new();
    for (key, path) in entries {
        let bytes = std::fs::read(path)
            .map_err(|e| CadError::InvalidInput(format!("font read failed: {e}")))?;
        engine.register(key, Arc::from(bytes.into_boxed_slice()))?;
        keys.push(key.clone());
    }
    // Any registered font can stand in for a missing one.
    engine.set_fallback(keys);
    Ok(Some(Arc::new(engine)))
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// Execute one CLI operation and produce the pretty-printed JSON document.
///
/// The returned error is structured and carries a stable machine `code`.
pub fn run(invocation: &CliInvocation) -> Result<String, CliError> {
    if invocation.operation == CliOperation::FixedViewportRender {
        // The browser has no headless device to own: it keeps the explicit
        // "unsupported" answer (the host canvas supplies the device instead).
        #[cfg(target_arch = "wasm32")]
        return Err(cli_error_from_domain(CadError::Unsupported(
            "fixed-viewport rendering requires a GPU device; wasm receives its device from the host canvas"
                .into(),
        )));
        // Native: drive the real headless renderer and report a structured
        // frame, or fail explicitly (no adapter, no drawable geometry).
        #[cfg(not(target_arch = "wasm32"))]
        return serde_json::to_string_pretty(&domain(run_render(invocation))?)
            .map_err(|e| CliError::new(error_code::INVARIANT, format!("cli encode failed: {e}")));
    }
    let fonts = domain(load_fonts(&invocation.fonts))?;
    let mut controller = domain(load_document(invocation))?;
    let value = match invocation.operation {
        CliOperation::Scan => domain(run_scan(&controller))?,
        CliOperation::ProxyReport => domain(run_proxy_report(&controller))?,
        CliOperation::Measure => domain(run_measure(&mut controller, invocation))?,
        CliOperation::ImportNotes => domain(run_import_notes(&mut controller, invocation))?,
        CliOperation::ExportNotes => domain(run_export_notes(&mut controller, invocation))?,
        CliOperation::BuildRepresentation => {
            domain(run_build_representation(&controller, fonts.as_ref()))?
        }
        CliOperation::Benchmark => domain(run_benchmark(&controller, invocation, fonts.as_ref()))?,
        CliOperation::FixedViewportRender => unreachable!(),
    };
    serde_json::to_string_pretty(&value)
        .map_err(|e| CliError::new(error_code::INVARIANT, format!("cli encode failed: {e}")))
}

/// Run an operation with an explicit `--out` file written atomically.
///
/// On success the JSON is returned. When `out` is `Some`, the bytes are written
/// to that path atomically (temp file + rename) before this returns; stdout
/// containment is the caller's responsibility.
pub fn run_to_output(invocation: &CliInvocation, out: Option<&Path>) -> Result<String, CliError> {
    let json = run(invocation)?;
    if let Some(path) = out {
        write_atomic(path, json.as_bytes()).map_err(|e| {
            CliError::new(
                error_code::OUTPUT_WRITE_FAILED,
                format!("failed to write --out: {}", e),
            )
            .with_context(serde_json::json!({ "path": path.display().to_string() }))
        })?;
    }
    Ok(json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_operation_names_parse() {
        for name in [
            "scan",
            "proxy-report",
            "measure",
            "import-notes",
            "export-notes",
            "build-representation",
            "render",
            "benchmark",
        ] {
            assert!(CliOperation::parse(name).is_some(), "{name}");
        }
        assert!(CliOperation::parse("nope").is_none());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn render_of_missing_input_fails_before_any_gpu_work() {
        // Import happens before device creation, so a missing file is an
        // `invalid_input` failure on every machine — adapter or not.
        let invocation = CliInvocation::new(CliOperation::FixedViewportRender, "missing.dwg");
        let error = run(&invocation).unwrap_err();
        assert_eq!(error.code, error_code::INVALID_INPUT);
        assert_eq!(error.exit_code(), 1);
    }

    #[cfg(target_arch = "wasm32")]
    #[test]
    fn render_on_wasm_is_explicitly_unsupported() {
        // wasm has no headless device: the operation stays explicitly not run.
        let invocation = CliInvocation::new(CliOperation::FixedViewportRender, "missing.dwg");
        let error = run(&invocation).unwrap_err();
        assert_eq!(error.code, error_code::UNSUPPORTED);
        assert_eq!(error.exit_code(), 1);
    }

    #[test]
    fn scan_of_missing_file_is_an_input_error() {
        let invocation = CliInvocation::new(CliOperation::Scan, "definitely-missing.dwg");
        let error = run(&invocation).unwrap_err();
        assert_eq!(error.code, error_code::INVALID_INPUT);
    }

    #[test]
    fn locale_normalizes_and_falls_back() {
        assert_eq!(Locale::parse("en"), Locale::En);
        assert_eq!(Locale::parse("en-US"), Locale::En);
        assert_eq!(Locale::parse("zh-CN"), Locale::ZhCn);
        assert_eq!(Locale::parse("fr"), Locale::ZhCn);
        assert_eq!(Locale::default(), Locale::ZhCn);
    }

    #[test]
    fn error_document_has_stable_schema() {
        let error = CliError::usage("bad option").with_context(serde_json::json!({"flag": "--x"}));
        let value = error.to_json(CliOperation::Measure);
        assert_eq!(value["schema_version"], CLI_SCHEMA_VERSION);
        assert_eq!(value["operation"], "measure");
        assert_eq!(value["error"]["code"], error_code::USAGE);
        assert_eq!(value["error"]["message"], "bad option");
        assert_eq!(value["error"]["context"]["flag"], "--x");
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn atomic_write_leaves_no_temp_on_failure() {
        let dir = std::env::temp_dir().join(format!("yacr-atomic-{}", unique_counter()));
        std::fs::create_dir_all(&dir).unwrap();
        // A directory cannot be replaced by rename, so this must fail cleanly.
        let blocked = dir.join("target-dir");
        std::fs::create_dir_all(&blocked).unwrap();
        assert!(write_atomic(&blocked, b"nope").is_err());
        // No stray temp files remain.
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp files left: {leftovers:?}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
