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
use acadrust::{DwgReadOptions, DwgReader, EntityType, ReadStats};
use cad_db::{
    BlockDefinition, DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, EntityColor,
    EntityLineType, EntityLineWeight, EntityRenderAttributes, EntityTransparency, Layer, Layout,
    LineType as DbLineType, LinetypePattern, PaperViewport, Style,
};
use cad_domain::*;
use cad_geometry::{
    arbitrary_axis, tessellate_bspline, GradientDef, GradientKind, GradientStop, PatternLine,
    TessellationParams,
};
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

/// Bounds applied to an untrusted drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportLimits {
    pub max_file_bytes: usize,
    pub max_entities: usize,
    pub max_block_depth: usize,
}

impl Default for ImportLimits {
    fn default() -> Self {
        ImportLimits {
            max_file_bytes: 512 * 1024 * 1024,
            max_entities: 2_000_000,
            max_block_depth: 32,
        }
    }
}

pub struct ImportRequest {
    pub document: DocumentId,
    pub database: DatabaseId,
    pub bytes: Arc<[u8]>,
    pub limits: ImportLimits,
    pub generation: u64,
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

#[derive(Debug, Clone)]
pub struct ImportReport {
    pub identity: DocumentIdentity,
    pub capabilities: Vec<EntityCapability>,
    pub completeness: Completeness,
    pub diagnostics: Vec<Diagnostic>,
    /// Wall-clock parse/normalise time in milliseconds, when measured.
    pub parse_ms: Option<f64>,
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
        // A DWG begins with an "AC10xx" version signature. Refuse anything else
        // instead of handing arbitrary bytes to a failsafe parser that would
        // return an empty document and look like success.
        if !looks_like_dwg(&request.bytes) {
            return Err(CadError::CorruptData(
                "input is not a DWG (missing AC10xx signature); DXF is not handled by this importer".into(),
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

        let mut reader = DwgReader::from_stream(Cursor::new(request.bytes.to_vec()));
        reader.options = DwgReadOptions { failsafe: true };
        let outcome = reader
            .read_with_stats()
            .map_err(|e| CadError::CorruptData(format!("DWG read failed: {e}")))?;
        if cancelled() {
            return Err(CadError::Cancelled);
        }
        // A failsafe read that never reached the end of the stream is a
        // truncated/corrupt file. Return it as `CorruptData` rather than
        // publishing an empty drawing that merely *looks* successful (the
        // completeness report still carries the detail for direct builder use).
        if !outcome.stats.stream_completed {
            return Err(CadError::CorruptData(
                "DWG stream did not complete; file is truncated or corrupt".into(),
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

        let builder = ImporterBuilder::new(request, &outcome.document, outcome.stats, identity)
            .with_progress(cancelled, progress);
        builder.run().map(|mut drawing| {
            drawing.report.parse_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
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
    /// Effective display opacity of each layer, resolved from its DWG
    /// transparency (0 = opaque, 255 = transparent).
    layer_transparency: HashMap<LayerId, f32>,
    /// Resolved sRGB colour of each layer, used to substitute `ByLayer` on an
    /// entity (an acadrust `Color::Index` is resolved through its ACI table).
    layer_colors: HashMap<LayerId, [u8; 3]>,
    /// Resolved lineweight of each layer in millimetres.
    layer_lineweights: HashMap<LayerId, f32>,
    /// Root linetype name of each layer (`Layer.line_type`), looked up against
    /// the drawing's linetype table when an entity is `ByLayer`.
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
    /// Source scale handle value -> name, for resolving an annotative entity's
    /// per-scale context leaves (`ACDB_ANNOTATIONSCALES` list).
    scale_names: ScaleNames,
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
}

/// A block child classified for display, before nesting is resolved.
#[derive(Debug, Clone)]
struct BlockMember {
    render: SupportStatus,
    /// Block definitions referenced by this child (one per array cell), so
    /// nesting resolves through MINSERT arrays too.
    insert_blocks: Vec<BlockId>,
}

mod builder;
mod dynamic;
mod entity;
mod geometry;
mod hatch;
mod plot;
mod style;
mod support;

mod annotative;

use annotative::*;
use dynamic::*;
use geometry::*;
use hatch::*;
use plot::read_plot_settings;
use style::*;
use support::*;

#[cfg(test)]
mod tests;
