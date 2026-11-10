//! Format dispatcher for the shared import contract.
//!
//! `cad-app` and every carrier open bytes through one [`Importer`]. This module
//! sniffs the format once ([`cad_import_core::detect_format`]) and delegates to
//! the adapter that owns it. It is intentionally a plain dispatcher: no limits,
//! progress, report or cancellation policy is duplicated here, so the adapters
//! remain the single place each format's behaviour is defined.
//!
//! `ImportRequest::xref` is DWG-only. OBJ, STL, PLY, OFF, STEP, IFC and RVT
//! imports ignore it; those formats have no CAD external references, and
//! reporting otherwise would invent a resolution that never happened.

use cad_domain::{CadError, CadResult};
use cad_import_acadrust::AcadrustImporter;
use cad_import_core::{
    detect_format, DrawingFormat, ImportProgressSink, ImportedDrawing, NoopProgress,
};
use cad_import_ifc::IfcImporter;
use cad_import_mesh::{OffImporter, PlyImporter};
use cad_import_obj::ObjImporter;
use cad_import_rvt::RvtImporter;
use cad_import_step::StepImporter;
use cad_import_stl::StlImporter;

// Re-exported so call sites name one module (`cad_app::import`) instead of
// mixing the neutral contract crate with the dispatcher.
pub use cad_import_core::{ImportLimits, ImportPhase, ImportProgress, ImportRequest, Importer};

/// The single user-facing refusal for bytes no supported reader claims.
pub const UNSUPPORTED_INPUT: &str = "input is not a supported drawing: expected a DWG \
     signature, an ASCII/binary DXF section stream, an ISO 10303-21 STEP or IFC file, an \
     OLE compound file from Autodesk Revit (.rvt/.rte; .rfa/.rft families carry no geometry), \
     a Wavefront OBJ file, an STL file, a PLY file, or an OFF/COFF file";

/// Sniff-and-delegate importer used by every carrier.
#[derive(Default)]
pub struct FormatImporter {
    acadrust: AcadrustImporter,
    step: StepImporter,
    ifc: IfcImporter,
    obj: ObjImporter,
    stl: StlImporter,
    ply: PlyImporter,
    off: OffImporter,
    rvt: RvtImporter,
}

impl FormatImporter {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Importer for FormatImporter {
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
        match detect_format(&request.bytes) {
            DrawingFormat::Dwg | DrawingFormat::Dxf => self
                .acadrust
                .import_with_progress(request, cancelled, progress),
            DrawingFormat::Step => self.step.import_with_progress(request, cancelled, progress),
            // IFC shares the Part 21 container with STEP but has its own schema
            // and importer. Detection separates it by FILE_SCHEMA, so the STEP
            // adapter (and cadmpeg) never sees these bytes.
            DrawingFormat::Ifc => self.ifc.import_with_progress(request, cancelled, progress),
            DrawingFormat::Obj => self.obj.import_with_progress(request, cancelled, progress),
            DrawingFormat::Stl => self.stl.import_with_progress(request, cancelled, progress),
            DrawingFormat::Ply => self.ply.import_with_progress(request, cancelled, progress),
            DrawingFormat::Off => self.off.import_with_progress(request, cancelled, progress),
            // The OLE signature cannot tell a project from a family; the RVT
            // adapter reads the streams and refuses `.rfa`/`.rft` by name.
            // glTF is never a user-visible format and is never detected here.
            DrawingFormat::Rvt => self.rvt.import_with_progress(request, cancelled, progress),
            DrawingFormat::Unknown => Err(CadError::CorruptData(UNSUPPORTED_INPUT.into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_domain::{DatabaseId, DocumentId};
    use std::sync::Arc;

    fn request(bytes: Vec<u8>) -> ImportRequest {
        ImportRequest {
            document: DocumentId(1),
            database: DatabaseId(1),
            bytes: Arc::from(bytes.into_boxed_slice()),
            limits: ImportLimits::default(),
            generation: 0,
            xref: None,
        }
    }

    fn fixture(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/ifc")
            .join(name);
        std::fs::read(path).expect("fixture is committed")
    }

    fn fixture_at(directory: &str, name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(directory)
            .join(name);
        std::fs::read(path).expect("fixture is committed")
    }

    #[test]
    fn ifc_is_routed_to_the_ifc_importer_not_streamed_into_step() {
        let drawing = FormatImporter::new()
            .import(&request(fixture("synthetic-wall.ifc")), &|| false)
            .expect("the dispatcher imports IFC");
        assert!(
            drawing.database.entities().next().is_some(),
            "the IFC lane produced geometry"
        );
    }

    #[test]
    fn ifc_bytes_without_geometry_hit_the_ifc_lane_and_fail_honestly() {
        // A header-only IFC file reaches the IFC importer (the STEP adapter
        // would refuse this with `import.step.ifc_schema_refused`); it must be a
        // CorruptData naming the IFC reason, never an empty success.
        let bytes = b"ISO-10303-21;\nHEADER;\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\nENDSEC;";
        let error = FormatImporter::new()
            .import(&request(bytes.to_vec()), &|| false)
            .err()
            .expect("a header-only IFC file has no geometry");
        match error {
            CadError::CorruptData(message) => assert!(
                message.contains(cad_import_ifc::NO_GEOMETRY_PRODUCTS),
                "the IFC lane must answer: {message}"
            ),
            other => panic!("expected CorruptData, got {other:?}"),
        }
    }

    #[test]
    fn revit_is_routed_to_the_rvt_importer_and_refuses_empty_geometry() {
        // The OLE signature alone cannot tell a project from a family; the
        // dispatcher must reach the RVT lane, and the scaffold must be the
        // stable no-geometry refusal, not the generic UNSUPPORTED_INPUT.
        let error = match FormatImporter::new().import(
            &request(fixture_at("rvt", "synthetic-scaffold-2024.rvt")),
            &|| false,
        ) {
            Ok(_) => panic!("the RVT scaffold has no geometry"),
            Err(error) => error,
        };
        match error {
            CadError::Unsupported(message) => assert!(
                message.contains(cad_import_rvt::NO_GEOMETRY),
                "the RVT lane must answer: {message}"
            ),
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn the_gltf_intermediate_is_not_a_user_format() {
        // `rvt` -> GLB is an implementation detail. GLB bytes fall through to
        // Unknown and the single unsupported-input message; there is no glTF
        // capability row, no detector arm and no picker entry.
        let error = match FormatImporter::new()
            .import(&request(b"glTF\x02\x00\x00\x00".to_vec()), &|| false)
        {
            Ok(_) => panic!("glTF is not a supported drawing"),
            Err(error) => error,
        };
        match error {
            CadError::CorruptData(message) => assert_eq!(message, UNSUPPORTED_INPUT),
            other => panic!("expected CorruptData, got {other:?}"),
        }
    }

    #[test]
    fn ply_is_routed_to_the_ply_importer() {
        let drawing = FormatImporter::new()
            .import(
                &request(fixture_at("ply", "synthetic-attrs-ascii.ply")),
                &|| false,
            )
            .expect("the dispatcher imports PLY");
        assert_eq!(drawing.database.entities().count(), 1);
    }

    #[test]
    fn off_is_routed_to_the_off_importer() {
        let drawing = FormatImporter::new()
            .import(
                &request(fixture_at("off", "synthetic-quad-triangle.off")),
                &|| false,
            )
            .expect("the dispatcher imports OFF");
        assert_eq!(drawing.database.entities().count(), 1);
    }
}
