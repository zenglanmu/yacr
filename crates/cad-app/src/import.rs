//! Format dispatcher for the shared import contract.
//!
//! `cad-app` and every carrier open bytes through one [`Importer`]. This module
//! sniffs the format once ([`cad_import_core::detect_format`]) and delegates to
//! the adapter that owns it. It is intentionally a plain dispatcher: no limits,
//! progress, report or cancellation policy is duplicated here, so the adapters
//! remain the single place each format's behaviour is defined.
//!
//! `ImportRequest::xref` is DWG-only. OBJ, STL, PLY, OFF, STEP, IFC, RVT and
//! SKP imports ignore it; those formats have no CAD external references, and
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
use cad_import_skp::SkpImporter;
use cad_import_step::StepImporter;
use cad_import_stl::StlImporter;

// Re-exported so call sites name one module (`cad_app::import`) instead of
// mixing the neutral contract crate with the dispatcher.
pub use cad_import_core::{ImportLimits, ImportPhase, ImportProgress, ImportRequest, Importer};

/// The single user-facing refusal for bytes no supported reader claims.
pub const UNSUPPORTED_INPUT: &str = "input is not a supported drawing: expected a DWG \
     signature, an ASCII/binary DXF section stream, an ISO 10303-21 STEP or IFC file, an \
     OLE compound file from Autodesk Revit (.rvt/.rte; .rfa/.rft families carry no geometry), \
     a SketchUp model (.skp), a Wavefront OBJ file, an STL file, a PLY file, or an OFF/COFF file";

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
    skp: SkpImporter,
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
            // The `.skp` header tells the containers apart but not the release;
            // the SKP adapter maps the release to a status ceiling, refuses
            // edge-only files by name, and is always `Partial`/`Unsupported` for
            // measurement.
            DrawingFormat::Skp => self.skp.import_with_progress(request, cancelled, progress),
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
            &request(fixture_at(
                "rvt",
                "upstream-metadata-scaffold-2024-no-geometry.rvt",
            )),
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

    /// The fixed `.skp` header the dispatcher must route on: the document type
    /// record, a version record and the 16-byte model GUID.
    fn skp_header(version: &str) -> Vec<u8> {
        fn record(text: &str) -> Vec<u8> {
            let mut bytes = vec![0xFF, 0xFE, 0xFF, text.encode_utf16().count() as u8];
            for unit in text.encode_utf16() {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
            bytes
        }
        let mut bytes = record("SketchUp Model");
        bytes.extend_from_slice(&record(version));
        bytes.extend_from_slice(&[0u8; 16]);
        bytes
    }

    #[test]
    fn sketchup_is_routed_to_the_skp_importer_and_fails_honestly() {
        // A valid `.skp` header followed by garbage is the SKP lane's problem:
        // the parser must answer `import.skp.parse_failed`, not the generic
        // `UNSUPPORTED_INPUT` (which would prove detection never reached it).
        let mut bytes = skp_header("{17.3.116}");
        bytes.extend_from_slice(b"this is not an MFC CArchive object stream");
        assert_eq!(detect_format(&bytes), DrawingFormat::Skp);
        let error = match FormatImporter::new().import(&request(bytes), &|| false) {
            Ok(_) => panic!("a header plus garbage is not an importable model"),
            Err(error) => error,
        };
        match error {
            CadError::CorruptData(message) => {
                assert!(
                    message.contains(cad_import_skp::PARSE_FAILED),
                    "the SKP lane must answer: {message}"
                );
                assert_ne!(message, UNSUPPORTED_INPUT);
            }
            other => panic!("expected CorruptData, got {other:?}"),
        }
    }

    #[test]
    fn sketchup_bytes_are_not_mistaken_for_another_format() {
        // A 32-byte-valid header must not be read as OLE, AC10xx, DXF, STL, PLY,
        // OFF or OBJ: the SKP lane owns these bytes, so its own refusal (not a
        // neighbouring importer's) is what a caller sees.
        for version in ["{3.0.0}", "{17.3.116}", "{26.0.0}"] {
            let bytes = skp_header(version);
            assert_eq!(
                detect_format(&bytes),
                DrawingFormat::Skp,
                "version {version} must be detected as SKP"
            );
            let error = match FormatImporter::new().import(&request(bytes), &|| false) {
                Ok(_) => panic!("a header-only model has no geometry"),
                Err(error) => error,
            };
            let message = match error {
                CadError::CorruptData(message) => message,
                CadError::Unsupported(message) => message,
                other => panic!("expected a format-specific refusal, got {other:?}"),
            };
            assert!(
                message.starts_with("import.skp."),
                "version {version} must be answered by the SKP lane: {message}"
            );
            assert!(
                !message.contains("import.rvt.")
                    && !message.contains("import.step.")
                    && !message.contains("import.ifc.")
                    && !message.contains("import.obj.")
                    && !message.contains("import.stl.")
                    && !message.contains("import.ply.")
                    && !message.contains("import.off."),
                "version {version} leaked to a neighbouring lane: {message}"
            );
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
