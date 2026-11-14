//! MicroStation DGN (V7 ISFF and V8 OLE compound file) **vector** import.
//!
//! [`cadkit_dgn`] 0.5.2 decodes both containers into a format-neutral
//! `cadkit_core::Document`. This adapter maps that document into the `cad-db`
//! vector model: levels become layers, linetypes/styles/shared-cell definitions
//! become their tables, and every supported entity becomes a neutral
//! [`cad_domain::SemanticGeometry`]. It is not the mesh path: a DGN line, arc,
//! circle, spline or text stays measurable, drawable and editable.
//!
//! The boundary is explicit, never an empty success:
//!
//! * only the **primary design model** is imported; every other model is a
//!   [`MODEL_SKIPPED`] diagnostic and a `Partial` reason;
//! * `cadkit-dgn` decodes **no** level colour/weight/linetype/freeze/lock bit,
//!   so layer rows carry name + visibility only and **no** `LayerAttributes` row
//!   is written ([`LEVEL_SYMBOLOGY_UNDECODED`], always `Partial`);
//! * DGN tags/attributes, rasters, unknown element types and every `EntityKind`
//!   without a vector mapping are reported and skipped, never fabricated;
//! * a shared-cell instance whose block has no definition is skipped
//!   ([`BLOCK_MISSING`]); no empty block is invented;
//! * a file whose every element is unsupported is
//!   [`CadError::Unsupported`] with [`UNKNOWN_ELEMENTS_ONLY`]; a readable file
//!   with no drawable row is [`NO_GEOMETRY`]. Neither is an empty success.
//!
//! Everything used here is pure Rust with no platform dependency, so the
//! `dom-canvas` wasm session can link this crate.
//!
//! Tested only against the two committed GDAL fixtures (`fixtures/dgn/`); real
//! V8i/Connect models, device rendering, pixels and performance are NOT
//! verified by these contract tests.

mod map;
#[cfg(test)]
mod tests;
mod transform;

use cad_domain::{CadError, CadResult, UnitContext};
use cad_import_core::{
    compute_identity, ImportLimits, ImportPhase, ImportProgress, ImportProgressSink, ImportReport,
    ImportRequest, ImportedDrawing, Importer, NoopProgress,
};

/// Stable refusal code: the DGN reader produced no drawable entity.
pub const NO_GEOMETRY: &str = "import.dgn.no_geometry";
/// Stable refusal code: the input is not a DGN V7/V8 file.
pub const NOT_A_DGN_FILE: &str = "import.dgn.not_a_dgn_file";
/// Stable diagnostic code: an element type this build does not model.
pub const UNKNOWN_ELEMENT: &str = "import.dgn.unknown_element";
/// Stable diagnostic code: DGN tag/attribute values have no storage target.
pub const TAGS_NOT_STORED: &str = "import.dgn.tags_not_stored";
/// Stable diagnostic code: DGN groups were flattened into their members.
pub const GROUP_FLATTENED: &str = "import.dgn.group_flattened";
/// Stable diagnostic code: a non-primary DGN model was not imported.
pub const MODEL_SKIPPED: &str = "import.dgn.model_skipped";
/// Stable diagnostic code: level colour/freeze/lock flags are not decoded.
pub const LEVEL_SYMBOLOGY_UNDECODED: &str = "import.dgn.level_symbology_undecoded";
/// Stable diagnostic code: a raster reference carries placement only.
pub const RASTER_NOT_LOADED: &str = "import.dgn.raster_not_loaded";
/// Stable diagnostic code: a fit-point-only spline became a polyline.
pub const SPLINE_FIT_POINTS: &str = "import.dgn.spline_fit_points_polyline";
/// Stable diagnostic code: multiline text markup was flattened.
pub const TEXT_FORMATTING: &str = "import.dgn.text_formatting_flattened";
/// Stable diagnostic code: a source entity was rejected before insertion.
pub const GEOMETRY_REJECTED: &str = "import.dgn.geometry_rejected";
/// Stable diagnostic code: an entity names an absent linetype.
pub const LINETYPE_UNKNOWN: &str = "import.dgn.linetype_unknown";
/// Stable diagnostic code: an entity names an absent level/layer.
pub const LAYER_UNKNOWN: &str = "import.dgn.layer_unknown";
/// Stable diagnostic code: the master unit is outside the domain's unit set.
pub const UNITS_UNSUPPORTED: &str = "import.dgn.units_unsupported";
/// Stable refusal code: the entity import limit was exceeded.
pub const ENTITY_LIMIT: &str = "import.dgn.limits_exceeded";
/// Stable diagnostic code: text alignment beyond left/center/right was lost.
pub const TEXT_ALIGN_UNSUPPORTED: &str = "import.dgn.text.align_unsupported";
/// Stable diagnostic code: text on a non-world-Z plane lost its normal.
pub const TEXT_PLANE_NOT_SUPPORTED: &str = "import.dgn.text.plane_not_supported";
/// Stable diagnostic code: a bad text width factor became the neutral 1.0.
pub const TEXT_WIDTH_FACTOR: &str = "import.dgn.text.width_factor";
/// Stable diagnostic code: an `EntityKind` has no vector mapping.
pub const ELEMENT_NOT_MODELLED: &str = "import.dgn.element_not_modelled";
/// Stable diagnostic code: a shared cell instance names no definition.
pub const BLOCK_MISSING: &str = "import.dgn.block_missing";
/// Stable diagnostic code: a shared cell instance is mirrored.
pub const CELL_MIRRORED_Z: &str = "import.dgn.cell_mirrored_z";
/// Stable diagnostic code: group/block nesting exceeded the import limit.
pub const DEPTH_LIMIT: &str = "import.dgn.depth_limit";
/// Stable refusal code: every element of the file is an unsupported type.
pub const UNKNOWN_ELEMENTS_ONLY: &str = "import.dgn.unknown_elements_only";
/// Informational code: the reader's 8-bit text decoding choice.
pub const TEXT_ENCODING: &str = "import.dgn.text_encoding";
/// Stable diagnostic code: a linetype pattern could not be scaled from UOR.
pub const LINETYPE_PATTERN_UNSCALED: &str = "import.dgn.linetype_pattern_unscaled";

/// The DGN import adapter.
#[derive(Default)]
pub struct DgnImporter;

impl DgnImporter {
    pub fn new() -> Self {
        Self
    }

    /// Whether `bytes` are a DGN V7 design file or a CFB whose root holds
    /// `Dgn~H`. This is the dispatcher's OLE disambiguation probe: it reads the
    /// compound-file directory, so the caller must apply the file-size limit
    /// first.
    pub fn sniff(bytes: &[u8]) -> bool {
        cadkit_dgn::sniff(bytes)
    }
}

impl Importer for DgnImporter {
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
        // A direct caller can hand this adapter anything; the reader is only
        // invoked when its own header probe recognizes the bytes.
        if !cadkit_dgn::sniff(&request.bytes) {
            return Err(CadError::CorruptData(format!(
                "{NOT_A_DGN_FILE}: the input has neither the DGN V7 design-file header nor a \
                 compound file whose root holds the Dgn~H stream; it is not a DGN file"
            )));
        }

        let started = web_time::Instant::now();
        let identity = compute_identity(&request.bytes);
        progress.report(ImportProgress {
            phase: ImportPhase::Reading,
            entities_done: 0,
            entities_total: None,
            bytes: Some(request.bytes.len() as u64),
            message: None,
        });

        let options = cadkit_core::ReadOptions::default()
            .with_limits(cadkit_limits(&request.limits))
            .with_keep_raw(false);
        progress.report(ImportProgress::at(ImportPhase::Parsing, 0, None));
        let doc = cadkit_dgn::read(&request.bytes, &options).map_err(read_error)?;
        if cancelled() {
            return Err(CadError::Cancelled);
        }

        progress.report(ImportProgress::at(ImportPhase::Tables, 0, None));
        let mut mapper = map::Mapper::new(request.database, &doc, &request.limits, cancelled);
        mapper.map_tables()?;

        let total = doc.entity_count();
        progress.report(ImportProgress::at(ImportPhase::Entities, 0, Some(total)));
        mapper.map_blocks()?;
        mapper.map_primary_model()?;
        if cancelled() {
            return Err(CadError::Cancelled);
        }
        progress.report(ImportProgress::at(
            ImportPhase::Finishing,
            mapper.inserted_count(),
            Some(total),
        ));

        let parse_ms = started.elapsed().as_secs_f64() * 1000.0;
        let (database, units, report): (_, UnitContext, ImportReport) =
            mapper.finish(identity, parse_ms)?;
        Ok(ImportedDrawing {
            database,
            units,
            report,
            // DGN has no Tianzheng records and no CAD external references; the
            // import contract's xref policy is ignored.
            vendor_records: Vec::new(),
        })
    }
}

/// Build `cadkit` limits from the import contract.
///
/// `max_vertices`/`max_total_vertices` stay at the reader's defaults: the import
/// contract has no per-entity vertex budget. This is a documented residual gap,
/// and the input/object/depth/decompression budgets below are the enforced ones.
fn cadkit_limits(limits: &ImportLimits) -> cadkit_core::Limits {
    let mut cadkit = cadkit_core::Limits::default()
        .with_max_input_bytes(limits.max_file_bytes as u64)
        .with_max_objects(limits.max_entities as u64)
        .with_max_depth(limits.max_block_depth as u32);
    if let Some(bytes) = limits.max_decompressed_bytes {
        cadkit = cadkit.with_max_decompressed_bytes(bytes as u64);
    }
    cadkit
}

/// Convert a reader error without turning it into an empty document.
fn read_error(error: cadkit_core::Error) -> CadError {
    match error {
        cadkit_core::Error::LimitExceeded { .. } => {
            CadError::InvalidInput(format!("{ENTITY_LIMIT}: {error}"))
        }
        cadkit_core::Error::UnknownFormat => CadError::CorruptData(format!(
            "{NOT_A_DGN_FILE}: the DGN reader did not recognize the input"
        )),
        other => CadError::CorruptData(format!(
            "{NOT_A_DGN_FILE}: cadkit-dgn could not read the file: {other}"
        )),
    }
}
