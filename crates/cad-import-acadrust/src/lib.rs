//! Sole acadrust integration boundary. No third-party entity escapes.
//!
//! Spec v2.0 §7.1: read bytes → acadrust parse → walk model/paper space and
//! block definitions → proxy supplement → normalise into the database. The
//! library is never modified and nothing here assumes internal hooks.

use std::collections::{BTreeSet, HashMap};
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
    EntityLineWeight, EntityRenderAttributes, EntityTransparency, Layer, Layout, PaperViewport,
    Style,
};
use cad_domain::*;
use cad_geometry::{arbitrary_axis, tessellate_bspline, PatternLine, TessellationParams};
use cad_proxy::{DecodeLimits, ProxyPlayer, ProxySource};

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
        let started = std::time::Instant::now();
        let identity = compute_identity(&request.bytes);

        let mut reader = DwgReader::from_stream(Cursor::new(request.bytes.to_vec()));
        reader.options = DwgReadOptions { failsafe: true };
        let outcome = reader
            .read_with_stats()
            .map_err(|e| CadError::CorruptData(format!("DWG read failed: {e}")))?;
        if cancelled() {
            return Err(CadError::Cancelled);
        }

        let builder = ImporterBuilder::new(request, &outcome.document, outcome.stats, identity);
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
    style_ids: HashMap<String, StyleId>,
    /// Lower-cased style name -> primary font file name.
    style_fonts: HashMap<String, String>,
    block_ids: HashMap<String, BlockId>,
    /// Block name -> the block's insertion base point. An INSERT maps the
    /// block's base point onto its insertion point, so expansion subtracts it
    /// (`world = insert * (local - base)`, audit B31).
    block_base_points: HashMap<String, Point3>,
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
mod entity;
mod geometry;
mod hatch;
mod style;
mod support;

use geometry::*;
use hatch::*;
use style::*;
use support::*;

#[cfg(test)]
mod tests;
