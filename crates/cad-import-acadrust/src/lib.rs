//! Sole acadrust integration boundary. No third-party entity escapes.
//!
//! Spec v2.0 §7.1: read bytes → acadrust parse → walk model/paper space and
//! block definitions → proxy supplement → normalise into the database. The
//! library is never modified and nothing here assumes internal hooks.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Cursor;
use std::sync::Arc;

use acadrust::entities::EntityCommon;
use acadrust::entities::{
    AttachmentPoint, BoundaryEdge, DimensionBase, Hatch, TextHorizontalAlignment,
    TextVerticalAlignment,
};
use acadrust::{DwgReadOptions, DwgReader, DxfReader, EntityType, ReadStats};
use cad_db::{
    sanitize_linetype_scale, BlockDefinition, DbEntity, DbObject, DrawingDatabase,
    DrawingDatabaseBuilder, EntityColor, EntityLineType, EntityLineWeight, EntityRenderAttributes,
    EntityTransparency, ExternalReference, ExternalReferenceStatus, Layer, LayerAttributes,
    LayerColor, LayerLineWeight, Layout, LineType as DbLineType, LinetypePattern, PaperViewport,
    Style,
};
use cad_domain::hatch::GradientStop;
use cad_domain::*;
use cad_geometry::{arbitrary_axis, tessellate_bspline, TessellationParams};
use cad_proxy::{DecodeLimits, ProxyPlayer, ProxySource};

// `std::time::Instant::now()` panics on wasm32-unknown-unknown ("time not
// implemented on this platform"); the browser gets `Performance.now()` through
// `web-time`. Native keeps the std type.
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

mod solid;
pub use solid::{
    acis_exchange, acis_raw_payload, sab_to_brep, sat_to_brep, solid_exchange_from_entity,
};

pub mod export;
pub mod vendor;
pub use vendor::{VendorExtraction, VendorRecord, VendorSource};

/// Bounds applied to an untrusted drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportLimits {
    pub max_file_bytes: usize,
    pub max_entities: usize,
    pub max_block_depth: usize,
    /// Faces retained from one MESH entity by the DWG reader.
    ///
    /// `None` means acadrust's own behaviour (its system-derived budget), so the
    /// default does not silently change what is accepted or rejected. A host that
    /// wants a tighter bound sets `Some(n)`; `Some(usize::MAX)` removes the cap.
    pub max_mesh_faces: Option<usize>,
    /// Face-corner indices retained from one MESH entity by the DWG reader.
    ///
    /// `None` means acadrust's own behaviour (its system-derived budget); see
    /// [`ImportLimits::max_mesh_faces`].
    pub max_mesh_face_indices: Option<usize>,
    /// Upper bound on the decompressed bytes one DWG read may allocate for its
    /// sections and system pages.
    ///
    /// `None` means acadrust's own behaviour (per-section allocation, no shared
    /// budget), so the default does not silently change what is accepted.
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

/// Translate import limits into acadrust's DWG read options.
///
/// The three resource fields are copied verbatim: a `None` limit stays `None`,
/// which is acadrust's own behaviour rather than an invented budget.
pub(crate) fn dwg_read_options(limits: &ImportLimits) -> DwgReadOptions {
    DwgReadOptions {
        failsafe: true,
        max_mesh_faces: limits.max_mesh_faces,
        max_mesh_face_indices: limits.max_mesh_face_indices,
        max_decompressed_bytes: limits.max_decompressed_bytes,
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
    /// Bytes are being validated/identified before the DWG reader runs.
    Reading,
    /// The acadrust DWG reader is running (the coarse, uninterruptible window).
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
/// unknown (for example before the DWG reader has produced a document); it must
/// never be filled with a fabricated guess. `bytes` is only set once a byte
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

/// The cancellation predicate for callers that never cancel.
///
/// Lets [`ImporterBuilder`] be constructed directly in tests without an
/// `Option` at each poll site.
pub(crate) static NEVER_CANCELLED: fn() -> bool = || false;

/// One application name found on object-level EED (XDATA) in the source.
///
/// This is read-only metadata decoded by acadrust: yacr reports it so a host can
/// see what an object carried, but it is **not** applied to geometry, layers or
/// any other imported state.
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

pub struct ImportedDrawing {
    pub database: DrawingDatabase,
    pub units: UnitContext,
    pub report: ImportReport,
    /// `TCH_*` / TCH-proxy records copied for the WebView vendor pass.
    ///
    /// Empty when the drawing carries no Tianzheng objects. These bytes are not
    /// interpreted here and never enter `cad-db`; the plugin decodes them and the
    /// host validates the result.
    pub vendor_records: Vec<vendor::VendorRecord>,
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
    /// synchronous acadrust importer does (see [`AcadrustImporter`]).
    fn import_with_progress(
        &self,
        request: &ImportRequest,
        cancelled: &dyn Fn() -> bool,
        _progress: &dyn ImportProgressSink,
    ) -> CadResult<ImportedDrawing> {
        self.import(request, cancelled)
    }
}

/// The acadrust-backed importer.
#[derive(Default)]
pub struct AcadrustImporter {
    pub proxy_limits: DecodeLimits,
}

impl AcadrustImporter {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Importer for AcadrustImporter {
    fn import(
        &self,
        request: &ImportRequest,
        cancelled: &dyn Fn() -> bool,
    ) -> CadResult<ImportedDrawing> {
        self.import_with_progress(request, cancelled, &NoopProgress)
    }

    fn import_with_progress(
        &self,
        request: &ImportRequest,
        cancelled: &dyn Fn() -> bool,
        progress: &dyn ImportProgressSink,
    ) -> CadResult<ImportedDrawing> {
        if cancelled() {
            return Err(CadError::Cancelled);
        }
        if request.bytes.len() > request.limits.max_file_bytes {
            return Err(CadError::InvalidInput(format!(
                "file is {} bytes, over the {} byte import limit",
                request.bytes.len(),
                request.limits.max_file_bytes
            )));
        }
        let dwg = looks_like_dwg(&request.bytes);
        let dxf = request.bytes.starts_with(b"AutoCAD Binary DXF\r\n\x1a\0") || {
            let prefix = String::from_utf8_lossy(&request.bytes[..request.bytes.len().min(4096)]);
            let mut lines = prefix.lines().map(str::trim);
            matches!((lines.next(), lines.next()), (Some("0"), Some("SECTION")))
        };
        if !dwg && !dxf {
            return Err(CadError::CorruptData(
                "input is neither a DWG signature nor an ASCII/binary DXF section stream".into(),
            ));
        }
        let started = Instant::now();
        let identity = compute_identity(&request.bytes);

        // Read start: the byte count is real (it is the request itself).
        progress.report(ImportProgress {
            phase: ImportPhase::Reading,
            entities_done: 0,
            entities_total: None,
            bytes: Some(request.bytes.len() as u64),
            message: None,
        });

        let outcome = if dwg {
            let mut reader = DwgReader::from_stream(Cursor::new(request.bytes.to_vec()));
            // acadrust 0.6.8 exposes read resource limits on `DwgReadOptions`
            // (`max_mesh_faces`, `max_mesh_face_indices`, `max_decompressed_bytes`).
            // They are threaded through `ImportLimits` here; a `None` limit keeps
            // acadrust's own behaviour, so the defaults stay unchanged.
            reader.options = dwg_read_options(&request.limits);
            reader.read_with_stats()
        } else {
            // DXF defaults to strict reading: malformed records must not become empty success.
            DxfReader::from_reader(Cursor::new(request.bytes.to_vec()))
                .and_then(|reader| reader.read_with_stats())
        }
        .map_err(|e| CadError::CorruptData(format!("drawing read failed: {e}")))?;
        if cancelled() {
            return Err(CadError::Cancelled);
        }
        // A failsafe read that never reached the end of the stream is a
        // truncated/corrupt file. Return it as `CorruptData` rather than
        // publishing an empty drawing that merely *looks* successful (the
        // completeness report still carries the detail for direct builder use).
        if !outcome.stats.stream_completed {
            return Err(CadError::CorruptData(
                "drawing stream did not complete; file is truncated or corrupt".into(),
            ));
        }

        // Read end: the document is parsed, so the total entity count is known.
        let entities_total = outcome.document.model_space_entities().count()
            + outcome
                .document
                .block_records
                .iter()
                .filter(|b| !is_space_block_name(&b.name))
                .map(|b| outcome.document.entities_in_block(&b.name).count())
                .sum::<usize>()
            + outcome
                .document
                .block_records
                .iter()
                .filter(|b| is_paper_space_name(&b.name))
                .map(|b| outcome.document.entities_in_block(&b.name).count())
                .sum::<usize>();
        progress.report(ImportProgress {
            phase: ImportPhase::Parsing,
            entities_done: 0,
            entities_total: Some(entities_total),
            bytes: Some(request.bytes.len() as u64),
            message: None,
        });

        let vendor::VendorExtraction {
            records: vendor_records,
            skipped: vendor_skipped,
            truncated: vendor_truncated,
        } = vendor::extract_vendor_records(&outcome.document);
        let builder = ImporterBuilder::new(request, &outcome.document, outcome.stats, identity)
            .with_progress(cancelled, progress);
        builder.run().map(move |mut drawing| {
            drawing.report.parse_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
            drawing.vendor_records = vendor_records;
            if vendor_skipped > 0 || vendor_truncated {
                drawing.report.diagnostics.push(Diagnostic {
                    object: None,
                    code: "vendor.record_skipped".to_string(),
                    message: format!(
                        "{vendor_skipped} vendor record(s) skipped (truncated={vendor_truncated})"
                    ),
                });
            }
            drawing
        })
    }
}

struct ImporterBuilder<'a> {
    request: &'a ImportRequest,
    acad: &'a acadrust::CadDocument,
    stats: ReadStats,
    identity: DocumentIdentity,
    /// Polled at every phase/batch boundary; a true result aborts with
    /// [`CadError::Cancelled`] without finishing the database.
    cancelled: &'a dyn Fn() -> bool,
    progress: &'a dyn ImportProgressSink,
    builder: DrawingDatabaseBuilder,
    diagnostics: Vec<Diagnostic>,
    capabilities: HashMap<String, EntityCapability>,
    layer_ids: HashMap<String, LayerId>,
    /// Root linetype name of each layer (`Layer.line_type`), used only to report
    /// the import-time "layer names a linetype this table does not define"
    /// diagnostic. The value itself is **not** flattened onto entities: a
    /// `ByLayer` entity keeps the symbol and the representation resolves the
    /// owning layer's name against the linetype table at draw time.
    layer_linetypes: HashMap<LayerId, String>,
    /// Linetype name (lower-cased) -> resolved dash pattern, from the drawing's
    /// `LineType` table. Nothing is fabricated; unknown names are reported.
    linetype_patterns: HashMap<String, LinetypePattern>,
    /// Linetype name -> table identity, so named entries are recorded in the
    /// database even when no entity references them.
    linetype_ids: HashMap<String, LinetypeId>,
    /// Source linetype handle value -> name, so an entity that carries only a
    /// handle (R13/R14 with no resolvable table name) can still be resolved.
    linetype_names_by_handle: HashMap<u64, String>,
    /// `true` when a source linetype carried shape/text glyphs that this build
    /// cannot draw; surfaces as a `Partial` reason.
    complex_linetypes: BTreeSet<String>,
    style_ids: HashMap<String, StyleId>,
    /// Lower-cased style name -> primary font file name.
    style_fonts: HashMap<String, String>,
    /// DXF STYLE-table XDATA font hints (`1000`), used only as the last fallback
    /// after acadrust's typed `true_type_font` and `font_file`. Empty for
    /// DWG/non-DXF inputs.
    dxf_style_fonts: HashMap<String, String>,
    /// Source scale handle value -> name, for resolving an annotative entity's
    /// per-scale context leaves (`ACDB_ANNOTATIONSCALES` list).
    scale_names: ScaleNames,
    /// Reasons already pushed as import diagnostics, so a per-entity loss is
    /// reported once per import rather than once per entity (ADR 0019 decision H
    /// uses it for the four MTEXT reasons).
    reported_reasons: BTreeSet<String>,
    block_ids: HashMap<String, BlockId>,
    /// Block name -> the block's insertion base point. An INSERT maps the
    /// block's base point onto its insertion point, so expansion subtracts it
    /// (`world = insert * (local - base)`, audit B31).
    block_base_points: HashMap<String, Point3>,
    /// Per block: source handle value -> imported entity id, and -> actual
    /// visible flag. Used to map a visibility parameter's handles, including
    /// the evaluated-anonymous-block path resolved per INSERT.
    block_member_ids: HashMap<BlockId, BTreeMap<u64, EntityId>>,
    block_member_visible: HashMap<BlockId, BTreeMap<u64, bool>>,
    layout_ids: HashMap<String, LayoutId>,
    next_entity: u128,
    next_object: u128,
    entity_total: usize,
    dropped: usize,
    proxy: ProxyPlayer,
    model_layout: LayoutId,
    /// Per block: the display status of each child, with the referenced block
    /// when the child is an INSERT (so nesting can be resolved).
    block_members: HashMap<BlockId, Vec<BlockMember>>,
    /// Resolved render status of each block definition (weakest child).
    block_status: HashMap<BlockId, SupportStatus>,
    /// Weakest render status and offending types over model-space entities.
    model_render: SupportStatus,
    model_drawable: bool,
    model_render_types: BTreeSet<String>,
    /// Running model-space bounding box, used to clip RAY/XLINE construct lines
    /// (a fixed viewport has no per-view clip).
    model_bounds: Option<(Point3, Point3)>,
    /// Block names on the current dimension picture-block (`*D...`) expansion
    /// stack. A member DIMENSION that references a block already being expanded
    /// is refused by the guard in `dimension_semantics` rather than recursing
    /// without bound (review B3).
    dimension_picture_stack: Vec<String>,
    /// XREF block records detected in [`ImporterBuilder::read_blocks`], keyed
    /// by their host block id. Kept beside the table rows so expansion and
    /// status resolution never have to guess from a name.
    xref_blocks: HashMap<BlockId, XrefBlock>,
    /// Terminal or in-flight status of every xref block. A non-`Loaded` entry
    /// seeds [`ImporterBuilder::resolve_block_status`] so a missing reference
    /// can never settle as `Verified`.
    xref_block_status: HashMap<BlockId, ExternalReferenceStatus>,
    /// Every inserted entity that references a block definition, recorded at
    /// push/splice time so the post-resolution pass can re-evaluate the final
    /// block-status fixpoint (including display-tree INSERTs).
    insert_chains: Vec<InsertChain>,
    /// Non-loaded xref cause keyed by the block that owns it: host xref blocks
    /// and the spliced blocks of a nested reference. Used to name the real
    /// reference in `import.xref_insert_missing` instead of guessing.
    xref_causes: HashMap<BlockId, XrefCause>,
    /// Xref blocks whose status is only a `Missing` placeholder until
    /// [`ImporterBuilder::resolve_xrefs`] runs. Expansion treats them as known
    /// blocks (not as a missing reference) so a successfully loaded reference is
    /// not reported as missing before it is attached.
    xref_pending: BTreeSet<BlockId>,
    /// Xref blocks whose provenance row was already inserted, keyed by
    /// [`BlockId`] (not by name: a nested namespaced name may legitimately
    /// collide with a host name). The builder accepts an external reference
    /// exactly once; an unresolved xref inserts its terminal row when
    /// resolution finishes.
    xref_recorded: BTreeSet<BlockId>,
    /// Next unused host block id. Initialised after [`ImporterBuilder::read_blocks`]
    /// and shared by every spliced xref so a referenced id is never reused.
    next_block: u128,
    /// Next unused host layer id for spliced xref layers. Initialised after
    /// [`ImporterBuilder::read_layers`].
    next_layer: u128,
    /// Host layer target name -> layer id for xref-dependent rows, including
    /// rows the host drawing already declared. A later splice reuses the row
    /// (VISRETAIN-like host override) instead of duplicating or overwriting it.
    xref_layer_names: HashMap<String, LayerId>,
    /// Materialised member ids of each host block, used when an xref block's
    /// definition is replaced with the merged member list.
    block_entities: HashMap<BlockId, Vec<EntityId>>,
}

/// Source facts of one external-reference block.
///
/// Kept beside the imported table row so status resolution and INSERT expansion
/// can answer without re-reading the acadrust document.
#[derive(Debug, Clone)]
struct XrefBlock {
    /// Block table name (the external-reference map key).
    name: String,
    /// Path exactly as the source declared it, before any trimming/folding.
    raw_path: String,
    /// Source marked this attachment as an overlay (as opposed to attach).
    overlay: bool,
    /// Source marked the attachment as currently unloaded.
    unloaded: bool,
}

/// A block child classified for display, before nesting is resolved.
#[derive(Debug, Clone)]
struct BlockMember {
    render: SupportStatus,
    /// Block definitions referenced by this child (one per array cell), so
    /// nesting resolves through MINSERT arrays too.
    insert_blocks: Vec<BlockId>,
}

/// One inserted entity that reaches block definitions (directly, through an
/// array, or through a display tree such as a dimension picture block).
///
/// Recorded so [`ImporterBuilder::reclassify_xref_inserts`] can apply the
/// **final** block-status fixpoint to the report/capabilities; entity
/// classification runs before xref resolution and therefore cannot see it.
#[derive(Debug, Clone)]
struct InsertChain {
    object: ObjectId,
    class_name: String,
    space: SpaceId,
    insert_blocks: Vec<BlockId>,
}

/// A non-loaded external reference reachable through a block.
#[derive(Debug, Clone)]
struct XrefCause {
    name: String,
    path: String,
    status: ExternalReferenceStatus,
}

mod builder;
mod construct_line;
mod dimension;
mod dynamic;
mod entity;
mod format_report;
mod geometry;
mod hatch;
mod mleader_style;
mod plot;
mod primitives;
mod style;
mod support;
mod xref;

pub use xref::XrefOptions;
use xref::{xref_current_depth, xref_support_status};

mod annotative;

use annotative::*;
use dynamic::*;
use geometry::*;
use hatch::*;
use plot::read_plot_settings;
use primitives::*;
use style::*;
use support::*;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_display_dimension_multileader;
#[cfg(test)]
mod tests_display_hatch_boundary;
#[cfg(test)]
mod tests_display_mline_tolerance;
#[cfg(test)]
mod tests_display_multileader;
#[cfg(test)]
mod tests_display_table_extended;
#[cfg(test)]
mod tests_multileader_adversarial;
#[cfg(test)]
mod tests_multileader_created;
#[cfg(test)]
mod tests_multileader_export;
#[cfg(test)]
mod tests_multileader_fixtures;
#[cfg(test)]
mod tests_xref;
