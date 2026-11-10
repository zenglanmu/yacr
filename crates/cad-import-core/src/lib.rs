//! Format-neutral import contract shared by every drawing importer.
//!
//! Spec v2.0 §7.1: the request/limits/progress/report vocabulary a carrier uses
//! to drive an import lives here, and a concrete format adapter (for example the
//! DWG/DXF adapter) implements [`Importer`] on top of it. Nothing in
//! this crate knows a file format, a parser crate or a UI toolkit, so a second
//! format can reuse the exact same call-site contract.
//!
//! `cad-db` stays the sole authority: [`ImportedDrawing::database`] is the built
//! database, and every other field is derived metadata about that build.

mod detect;
pub use detect::{detect_format, ifc_schema, step_file_schema, DrawingFormat, OFF_FAMILY_PREFIXES};

/// Stable refusal code for ISO 10303-21 input whose `FILE_SCHEMA` is IFC when
/// it is handed to the **STEP** adapter.
///
/// IFC has its own importer lane (`cad-import-ifc`); the dispatcher routes it
/// there. The STEP adapter repeats the `FILE_SCHEMA` check so a direct caller
/// cannot decode IFC as AP203/AP214/AP242 either, and refuses by name before
/// cadmpeg is invoked at all.
pub const IFC_SCHEMA_REFUSED: &str = "import.step.ifc_schema_refused";

mod mesh_import;
pub use mesh_import::{MeshImport, MeshImportOutcome};

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cad_db::DrawingDatabase;
use cad_domain::{
    CadResult, Completeness, DatabaseId, Diagnostic, DocumentId, DocumentIdentity,
    EntityCapability, UnitContext,
};
use cad_resources::{ResourceLimits, XrefFileSource};

/// Bounds applied to an untrusted drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportLimits {
    pub max_file_bytes: usize,
    pub max_entities: usize,
    pub max_block_depth: usize,
    /// Faces retained from one MESH entity by the format reader.
    ///
    /// `None` means the reader's own behaviour (its system-derived budget), so
    /// the default does not silently change what is accepted or rejected. A host
    /// that wants a tighter bound sets `Some(n)`; `Some(usize::MAX)` removes the
    /// cap.
    pub max_mesh_faces: Option<usize>,
    /// Face-corner indices retained from one MESH entity by the format reader.
    ///
    /// `None` means the reader's own behaviour (its system-derived budget); see
    /// [`ImportLimits::max_mesh_faces`].
    pub max_mesh_face_indices: Option<usize>,
    /// Upper bound on the decompressed bytes one archive-backed read may
    /// allocate for its sections and system pages.
    ///
    /// `None` means the reader's own behaviour (per-section allocation, no
    /// shared budget), so the default does not silently change what is accepted.
    pub max_decompressed_bytes: Option<usize>,
}

impl Default for ImportLimits {
    fn default() -> Self {
        ImportLimits {
            max_file_bytes: 512 * 1024 * 1024,
            max_entities: 2_000_000,
            max_block_depth: 32,
            max_mesh_faces: None,
            max_mesh_face_indices: None,
            max_decompressed_bytes: None,
        }
    }
}

pub struct ImportRequest {
    pub document: DocumentId,
    pub database: DatabaseId,
    pub bytes: Arc<[u8]>,
    pub limits: ImportLimits,
    pub generation: u64,
    /// External-reference loading policy and host file access.
    ///
    /// `None` means this import may not read referenced files: every xref block
    /// is recorded as `BrowserUnavailable` (never as a silently empty block).
    /// The host supplies a [`XrefOptions`] with a [`cad_resources::XrefFileSource`]
    /// to make xref content available. This is import-only; loaded xref content
    /// is merged into the host database and never re-read afterwards.
    pub xref: Option<Arc<XrefOptions>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportStage {
    Reading,
    Parsing,
    Normalizing,
    Proxy,
    Indexing,
    Representations,
    Uploading,
}

/// Fine-grained phase of an in-progress import (spec F01).
///
/// Distinct from [`ImportStage`], which enumerates the *pipeline* stages a host
/// may schedule; this enumerates the boundaries the synchronous importer
/// actually reaches so a progress bar is driven by real work, never a timer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImportPhase {
    /// Bytes are being validated/identified before the format reader runs.
    Reading,
    /// The format reader is running (the coarse, uninterruptible window).
    Parsing,
    /// The layer/style/layout/block tables are being copied into the database.
    Tables,
    /// Model, block and paper-space entities are being normalised and inserted.
    Entities,
    /// A block's display status is being resolved to a fixpoint.
    Resolving,
    /// The database builder is finalising indexes and is about to publish.
    Finishing,
}

/// One progress tick emitted by an importer.
///
/// Counts are always real. `entities_total` is `None` while the source total is
/// unknown (for example before the format reader has produced a document); it
/// must never be filled with a fabricated guess. `bytes` is only set once a byte
/// count is actually known (the request's own size during reading).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportProgress {
    pub phase: ImportPhase,
    /// Entities normalised and inserted so far (monotonic non-decreasing).
    pub entities_done: usize,
    /// Total entities expected, when the format exposes it.
    pub entities_total: Option<usize>,
    /// Source bytes known so far, when measurable.
    pub bytes: Option<u64>,
    /// Optional human-facing note for this tick (never a fabricated count).
    pub message: Option<String>,
}

impl ImportProgress {
    /// A phase boundary at a known entity count, with no byte/message payload.
    pub fn at(phase: ImportPhase, entities_done: usize, entities_total: Option<usize>) -> Self {
        ImportProgress {
            phase,
            entities_done,
            entities_total,
            bytes: None,
            message: None,
        }
    }
}

/// Receives [`ImportProgress`] ticks during an import.
///
/// A blanket impl covers closures, so tests and hosts can pass `&|p| { .. }`
/// without naming this trait.
pub trait ImportProgressSink {
    fn report(&self, progress: ImportProgress);
}

impl<F: Fn(ImportProgress)> ImportProgressSink for F {
    fn report(&self, progress: ImportProgress) {
        self(progress)
    }
}

/// The progress sink used when a caller does not want ticks.
///
/// Explicit and cheap: it drops every tick and never allocates. Importers owned
/// by the core default to this rather than requiring an `Option` at each site.
pub struct NoopProgress;

impl ImportProgressSink for NoopProgress {
    fn report(&self, _progress: ImportProgress) {}
}

/// One application name found on object-level EED (XDATA) in the source.
///
/// This is read-only metadata decoded by the format reader: yacr reports it so a
/// host can see what an object carried, but it is **not** applied to geometry,
/// layers or any other imported state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectXdataApplication {
    /// Registered application name (`1001` group).
    pub application: String,
    /// Number of objects that carry at least one record for this application.
    pub objects: usize,
}

#[derive(Debug, Clone)]
pub struct ImportReport {
    pub identity: DocumentIdentity,
    pub capabilities: Vec<EntityCapability>,
    pub completeness: Completeness,
    pub diagnostics: Vec<Diagnostic>,
    /// Wall-clock parse/normalise time in milliseconds, when measured.
    pub parse_ms: Option<f64>,
    /// Decoded object-level EED (XDATA) applications, sorted by name.
    ///
    /// Reported only: the import never applies these values to geometry. Empty
    /// when the source carries no object xdata.
    pub object_xdata: Vec<ObjectXdataApplication>,
}

impl ImportReport {
    /// Short human-facing completeness label for the UI status line.
    pub fn completeness_label(&self) -> String {
        match &self.completeness {
            Completeness::Complete => "完整".to_string(),
            Completeness::Partial(items) => format!("部分（{} 项）", items.len()),
            Completeness::Missing(items) => format!("缺失（{} 项）", items.len()),
            Completeness::Unverified => "未验证".to_string(),
        }
    }
}

/// Which source shape produced one vendor record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VendorSource {
    /// A vendor-class object the reader could not model.
    UnknownEntity,
    /// A proxy entity whose class resolves to a vendor class.
    ProxyEntity,
}

impl VendorSource {
    /// Stable byte for the ABI frame.
    pub fn wire_code(self) -> u8 {
        match self {
            Self::UnknownEntity => 0,
            Self::ProxyEntity => 1,
        }
    }
}

/// One vendor record, byte-for-byte what the plugin needs.
///
/// The import copies bytes only; it never interprets the vendor layout. This is a
/// payload pass-through, not a second document model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VendorRecord {
    /// The vendor class name (for example a `TCH_*` class).
    pub class_name: String,
    /// The source entity handle.
    pub handle: u64,
    /// DWG version as a stable decimal code (`1027` for AC1027).
    pub dwg_version: u32,
    /// Which source shape produced this record.
    pub source: VendorSource,
    /// Main stream bytes (common data + class body, bit-aligned at bit 0).
    pub main: Vec<u8>,
    /// Exact main-stream bit length.
    pub main_bits: u32,
    /// Text stream bytes (bit-aligned at bit 0), empty before R2007.
    pub text: Vec<u8>,
    /// Exact text-stream bit length.
    pub text_bits: u32,
    /// Handle-stream bytes (bit-aligned at bit 0).
    pub handle_stream: Vec<u8>,
    /// Exact handle-stream bit length.
    pub handle_bits: u32,
    /// Bits of common data the class-body decoder must skip in `main`.
    pub main_skip_bits: u32,
    /// Bits of common data the class-body decoder must skip in `text`.
    pub text_skip_bits: u32,
    /// Bits of common data the class-body decoder must skip in `handle_stream`.
    pub handle_skip_bits: u32,
    /// Handles referenced by a proxy record (soft/hard pointers).
    pub references: Vec<u64>,
}

pub struct ImportedDrawing {
    pub database: DrawingDatabase,
    pub units: UnitContext,
    pub report: ImportReport,
    /// `TCH_*` / TCH-proxy records copied for the WebView vendor pass.
    ///
    /// Empty when the drawing carries no Tianzheng objects. These bytes are not
    /// interpreted here and never enter `cad-db`; the plugin decodes them and the
    /// host validates the result.
    pub vendor_records: Vec<VendorRecord>,
}

pub trait Importer {
    fn import(
        &self,
        request: &ImportRequest,
        cancelled: &dyn Fn() -> bool,
    ) -> CadResult<ImportedDrawing>;

    /// Import while reporting real progress at phase boundaries.
    ///
    /// The default implementation delegates to [`Importer::import`] and emits
    /// nothing: existing callers and implementations keep compiling unchanged.
    /// An importer that can observe phase boundaries overrides this; the
    /// synchronous format importers do.
    fn import_with_progress(
        &self,
        request: &ImportRequest,
        cancelled: &dyn Fn() -> bool,
        _progress: &dyn ImportProgressSink,
    ) -> CadResult<ImportedDrawing> {
        self.import(request, cancelled)
    }
}

/// Default total byte budget for all referenced files in one import (1 GiB).
pub const DEFAULT_XREF_BYTE_BUDGET: u64 = 1024 * 1024 * 1024;

/// Host policy and file access for external references.
///
/// Construct with [`XrefOptions::new`] or a struct literal, optionally tune the
/// public fields, and pass it as `ImportRequest::xref`. One value drives the
/// whole nested tree: a referenced drawing's own xrefs are resolved with the
/// same source, roots and budget.
///
/// ```ignore
/// let mut options = XrefOptions::new(source, "/drawings");
/// options.roots = vec![PathBuf::from("/drawings/lib")];
/// options.max_depth = 4;
/// ```
pub struct XrefOptions {
    /// Carrier that actually reads referenced bytes.
    pub source: Arc<dyn XrefFileSource>,
    /// Directory relative references in the host drawing resolve against.
    /// Nested imports replace this with the referenced file's parent directory.
    pub drawing_dir: PathBuf,
    /// Additional lexical roots an absolute reference may live under.
    pub roots: Vec<PathBuf>,
    /// Maximum nesting depth. `0` allows only the host's direct attachments.
    pub max_depth: usize,
    /// Total bytes every referenced file in this tree may consume.
    pub total_bytes_budget: u64,
}

impl XrefOptions {
    /// Create options for a host drawing at `drawing_dir`.
    pub fn new(source: Arc<dyn XrefFileSource>, drawing_dir: impl Into<PathBuf>) -> Self {
        Self {
            source,
            drawing_dir: drawing_dir.into(),
            roots: Vec::new(),
            max_depth: ResourceLimits::default().max_xref_depth,
            total_bytes_budget: DEFAULT_XREF_BYTE_BUDGET,
        }
    }

    /// Add allowed absolute roots, keeping the configured budget/depth.
    pub fn with_roots(mut self, roots: Vec<PathBuf>) -> Self {
        self.roots = roots;
        self
    }

    /// Replace the total byte budget for this tree.
    pub fn with_budget(mut self, total_bytes_budget: u64) -> Self {
        self.total_bytes_budget = total_bytes_budget;
        self
    }

    /// Replace the nesting depth limit.
    pub fn with_max_depth(mut self, max_depth: usize) -> Self {
        self.max_depth = max_depth;
        self
    }

    /// Options for one referenced drawing, keeping source/roots/depth.
    ///
    /// `drawing_dir` becomes the referenced file's parent directory and the
    /// budget is the caller-supplied remaining tree budget (the session tracks
    /// it, so this crate never guesses one).
    pub fn nested(&self, path: &Path, remaining_bytes_budget: u64) -> Self {
        let drawing_dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.drawing_dir.clone());
        XrefOptions {
            source: Arc::clone(&self.source),
            drawing_dir,
            roots: self.roots.clone(),
            max_depth: self.max_depth,
            total_bytes_budget: remaining_bytes_budget,
        }
    }
}

/// SHA-256 content identity of the imported bytes.
///
/// Kept here so every importer reports the same identity for the same bytes.
pub fn compute_identity(bytes: &[u8]) -> DocumentIdentity {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&hasher.finalize());
    DocumentIdentity::Sha256(out)
}
