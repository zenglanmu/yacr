//! PLY (Polygon File Format) and OFF (Object File Format) import adapters.
//!
//! Both formats are display-mesh formats: no layers, no blocks, no units and no
//! external references. This crate owns the parsers (hand-written; no third
//! party parsing dependency) and turns one file into exactly one
//! `SemanticGeometry::Mesh` entity on layer `0` in model space through
//! [`cad_import_core::MeshImport`]. The shared
//! scaffold calls `cad_db::validate_geometry`, so neither parser can publish a
//! mesh whose normals/colours do not match its vertices or whose triangles
//! reference unknown vertices.
//!
//! Losses are never silent:
//! * `n`-gons (`n > 3`) are fan-triangulated and recorded with
//!   [`ply::DIAG_NGON_FAN`] / [`off::DIAG_NGON_FAN`] plus `Completeness::Partial`,
//!   because fan triangulation can be wrong for concave polygons.
//! * Faces with fewer than three corners are skipped and reported (also
//!   `Partial`).
//! * Degenerate triangles (repeated index or zero area) are dropped and
//!   reported, never written to `cad-db`.
//! * A truncated payload (fewer element/vertex/face instances than declared) is
//!   [`CadError::CorruptData`] naming the shortfall. It is never a partial
//!   success that silently drops trailing data.
//!
//! Deliberate boundaries:
//! * PLY is one mesh per file, so one `DbEntity`; OFF likewise.
//! * Units are unknown (`UnitContext::drawing_units()`), never guessed.
//! * Meshes are draw/pick targets only; measure stays `Unsupported`.
//! * OFF is ASCII-only. Only `OFF` and `COFF` are accepted; other OFF-family
//!   prefixes are refused by name ([`off::ERR_VARIANT_REFUSED`]).

mod off;
mod ply;

pub use off::{
    OffImporter, DIAG_ALPHA_DROPPED, DIAG_COLORS_DROPPED, DIAG_DEGENERATE_TRIANGLE,
    DIAG_FACE_COLOR_NOT_IMPORTED, DIAG_FACE_TOO_FEW_VERTICES, DIAG_NGON_FAN, ERR_INDEX_RANGE,
    ERR_MALFORMED_HEADER, ERR_NON_ASCII, ERR_NON_FINITE, ERR_TRAILING_DATA, ERR_TRUNCATED,
    ERR_VARIANT_REFUSED, OFF_MESH_TYPE_KEY,
};
pub use ply::{
    PlyImporter, DIAG_COLORS_DROPPED as PLY_DIAG_COLORS_DROPPED,
    DIAG_DEGENERATE_TRIANGLE as PLY_DIAG_DEGENERATE_TRIANGLE,
    DIAG_FACE_TOO_FEW_VERTICES as PLY_DIAG_FACE_TOO_FEW_VERTICES,
    DIAG_NGON_FAN as PLY_DIAG_NGON_FAN, DIAG_NORMALS_DROPPED,
    ERR_INDEX_RANGE as PLY_ERR_INDEX_RANGE, ERR_MALFORMED_HEADER as PLY_ERR_MALFORMED_HEADER,
    ERR_MISSING_END_HEADER, ERR_NON_FINITE as PLY_ERR_NON_FINITE, ERR_TRAILING_BYTES,
    ERR_TRUNCATED as PLY_ERR_TRUNCATED, ERR_UNSUPPORTED_FORMAT, ERR_UNSUPPORTED_PROPERTY_TYPE,
    PLY_MESH_TYPE_KEY,
};

#[cfg(test)]
mod tests;

use cad_db::EntityColor;
use cad_domain::{CadError, CadResult, Diagnostic, DocumentIdentity, Mesh, Point3, UnitContext};
use cad_import_core::{
    ImportLimits, ImportPhase, ImportProgress, ImportProgressSink, ImportReport, ImportRequest,
    ImportedDrawing, MeshImport,
};

/// One non-fatal fact the parser found. `partial` marks lost/inexact geometry;
/// attribute-only losses use `false` (matching the STL normal note).
pub(crate) struct Note {
    pub code: &'static str,
    pub message: String,
    pub partial: bool,
}

/// Parser output, before it is written through [`MeshImport`].
pub(crate) struct ParsedMesh {
    pub vertices: Vec<Point3>,
    pub triangles: Vec<[u32; 3]>,
    pub normals: Vec<Point3>,
    pub colors: Vec<[u8; 3]>,
    pub notes: Vec<Note>,
}

/// Enforce the mesh limits against the final triangle count.
///
/// Counts are the real parsed numbers; the limit is echoed verbatim. Nothing is
/// truncated to fit.
pub(crate) fn check_mesh_limits(
    label: &str,
    triangles: usize,
    limits: &ImportLimits,
) -> CadResult<()> {
    if let Some(max) = limits.max_mesh_faces {
        if triangles > max {
            return Err(CadError::InvalidInput(format!(
                "{label} mesh has {triangles} triangle(s), over the {max} face import limit"
            )));
        }
    }
    let indices = triangles.saturating_mul(3);
    if let Some(max) = limits.max_mesh_face_indices {
        if indices > max {
            return Err(CadError::InvalidInput(format!(
                "{label} mesh has {indices} face corner indices, over the {max} face-index import limit"
            )));
        }
    }
    Ok(())
}

/// Write one parsed mesh through the shared scaffold and build the report.
#[allow(clippy::too_many_arguments)]
pub(crate) fn publish(
    request: &ImportRequest,
    progress: &dyn ImportProgressSink,
    identity: DocumentIdentity,
    started: web_time::Instant,
    parsed: ParsedMesh,
    type_key: &'static str,
    label: &str,
) -> CadResult<ImportedDrawing> {
    if parsed.triangles.is_empty() {
        return Err(CadError::InvalidInput(format!(
            "{label} file produced no drawable mesh geometry"
        )));
    }
    check_mesh_limits(label, parsed.triangles.len(), &request.limits)?;

    let mut import = MeshImport::new(request, type_key)?;
    for note in parsed.notes {
        import.record(
            Diagnostic {
                object: None,
                code: note.code.into(),
                message: note.message.clone(),
            },
            note.partial.then_some(note.message),
        );
    }

    let object = import.reserve_object()?;
    import.push_mesh(
        object,
        Mesh {
            vertices: parsed.vertices,
            triangles: parsed.triangles,
            normals: parsed.normals,
            face_sources: Vec::new(),
            colors: parsed.colors,
        },
        // Per-vertex colours, when present, are carried by the mesh; the entity
        // colour stays ByLayer so an absent colour never wins over them.
        EntityColor::ByLayer,
    )?;

    progress.report(ImportProgress::at(ImportPhase::Entities, 1, Some(1)));
    progress.report(ImportProgress::at(ImportPhase::Finishing, 1, Some(1)));

    let outcome = import.finish()?;
    let report = ImportReport {
        identity,
        capabilities: outcome.capabilities,
        completeness: outcome.completeness,
        diagnostics: outcome.diagnostics,
        parse_ms: Some(started.elapsed().as_secs_f64() * 1000.0),
        object_xdata: Vec::new(),
    };
    Ok(ImportedDrawing {
        database: outcome.database,
        // PLY and OFF carry no standard unit declaration this reader trusts.
        units: UnitContext::drawing_units(),
        report,
        vendor_records: Vec::new(),
    })
}
