//! Format dispatcher for the shared import contract.
//!
//! `cad-app` and every carrier open bytes through one [`Importer`]. This module
//! is a thin wrapper over [`crate::ConnectorRegistry::builtin`], which owns
//! the ordered probe table (SKP, DGN V7/V8, OLE/RVT, DWG, DXF, IFC, STEP, STL,
//! PLY, OFF, OBJ) and the per-format descriptors. No limits, progress,
//! report or cancellation policy is duplicated here, so the adapters remain the
//! single place each format's behaviour is defined.
//!
//! `ImportRequest::xref` is DWG-only. OBJ, STL, PLY, OFF, STEP, IFC, RVT and
//! SKP imports ignore it; those formats have no CAD external references, and
//! reporting otherwise would invent a resolution that never happened.

use cad_domain::CadResult;
use cad_import_core::{ImportProgressSink, ImportedDrawing, NoopProgress};

use crate::connector::ConnectorRegistry;

// Re-exported so call sites name one module (`cad_app::import`) instead of
// mixing the neutral contract crate with the dispatcher.
pub use cad_import_core::{ImportLimits, ImportPhase, ImportProgress, ImportRequest, Importer};

/// The single user-facing refusal for bytes no supported reader claims.
pub const UNSUPPORTED_INPUT: &str = "input is not a supported drawing: expected a DWG \
     signature, an ASCII/binary DXF section stream, an ISO 10303-21 STEP or IFC file, a Bentley \
     DGN V7 design file or V8 compound file, an OLE compound file from Autodesk Revit \
     (.rvt/.rte; .rfa/.rft families carry no geometry), a SketchUp model (.skp), a Wavefront OBJ \
     file, an STL file, a PLY file, or an OFF/COFF file";

/// Sniff-and-delegate importer used by every carrier.
///
/// Kept as the public entry point (and [`Importer`] implementation) so every
/// call site keeps working; the dispatch itself is
/// [`crate::ConnectorRegistry::builtin`], which owns the ordered probe table and the
/// per-format descriptors. This type adds no second dispatch path.
pub struct FormatImporter {
    registry: ConnectorRegistry,
}

impl FormatImporter {
    pub fn new() -> Self {
        FormatImporter {
            registry: ConnectorRegistry::builtin(),
        }
    }

    /// The registry this dispatcher delegates to.
    ///
    /// Exposed so a host can list descriptors (`descriptors()`) or drive an
    /// export through the same table it imports with.
    pub fn registry(&self) -> &ConnectorRegistry {
        &self.registry
    }
}

impl Default for FormatImporter {
    fn default() -> Self {
        Self::new()
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
        let mut imported = self.registry.import(request, cancelled, progress)?;
        // Attribution for the persisted report/provenance rows (ADR-0037 D11):
        // an adapter cannot know its registry key, so the dispatcher stamps it.
        // `route` is a pure sniff with no import limit; the bytes already passed
        // `import`, so re-sniffing is safe and cannot change the decision. A
        // direct adapter call (tests, CLI probes) keeps `None`.
        if imported.report.connector.is_none() {
            imported.report.connector = self.registry.route(&request.bytes).map(|d| d.id);
        }
        Ok(imported)
    }
}

#[cfg(test)]
mod tests {
    // Most cases below are gated on their format feature: a slim build
    // (`--no-default-features`, the DWG/DXF-only `dom-canvas` edge) compiles
    // those lanes out, so their helpers and imports go unused there instead of
    // failing the build.
    #![allow(unused_imports, dead_code)]

    use super::*;
    use cad_domain::{CadError, DatabaseId, DocumentId};
    use cad_import_core::{detect_format, DrawingFormat};
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

    #[cfg(feature = "formats-ifc")]
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

    #[cfg(feature = "formats-ifc")]
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

    #[cfg(feature = "formats-rvt")]
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

    #[cfg(feature = "formats-skp")]
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

    #[cfg(feature = "formats-skp")]
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
                    && !message.contains("import.gltf.")
                    && !message.contains("import.off."),
                "version {version} leaked to a neighbouring lane: {message}"
            );
        }
    }

    #[cfg(feature = "formats-all")]
    #[test]
    fn a_truncated_glb_header_is_not_a_drawing() {
        // glTF is a user format (the registry owns both lanes), but the GLB
        // magic alone is not a container: a header shorter than the 12 fixed
        // bytes is `Unknown` and gets the single unsupported-input message.
        let error = match FormatImporter::new()
            .import(&request(b"glTF\x02\x00\x00\x00".to_vec()), &|| false)
        {
            Ok(_) => panic!("a truncated header is not an importable drawing"),
            Err(error) => error,
        };
        match error {
            CadError::CorruptData(message) => assert_eq!(message, UNSUPPORTED_INPUT),
            other => panic!("expected CorruptData, got {other:?}"),
        }
    }

    #[cfg(feature = "formats-dgn")]
    #[test]
    fn dgn_v7_is_routed_to_the_dgn_importer() {
        let drawing = FormatImporter::new()
            .import(&request(fixture_at("dgn", "smalltest.dgn")), &|| false)
            .expect("the dispatcher imports DGN V7");
        let mut layers: Vec<&str> = drawing
            .database
            .layers()
            .map(|layer| layer.name.as_str())
            .collect();
        layers.sort();
        assert_eq!(layers, ["Level 1", "Level 2"]);
        assert_eq!(drawing.database.entity_count(), 4);
    }

    #[cfg(feature = "formats-dgn")]
    #[test]
    fn dgn_v8_is_routed_to_the_dgn_importer_not_rvt() {
        let drawing = FormatImporter::new()
            .import(&request(fixture_at("dgn", "test_dgnv8.dgn")), &|| false)
            .expect("the dispatcher imports DGN V8 through the OLE probe");
        assert!(
            drawing
                .database
                .layers()
                .any(|layer| layer.name == "Default"),
            "the DGN level table reached the DGN lane"
        );
        assert!(
            drawing
                .report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == cad_connector_dgn::UNKNOWN_ELEMENT),
            "the DGN lane's own diagnostics are present"
        );
        assert!(
            drawing
                .report
                .diagnostics
                .iter()
                .all(|diagnostic| { !diagnostic.code.starts_with("import.rvt.") }),
            "the V8 fixture must never reach the RVT lane"
        );
    }

    #[cfg(feature = "formats-dgn")]
    #[test]
    fn ole_sniff_happens_after_the_file_size_limit() {
        let mut request = request(fixture_at("dgn", "test_dgnv8.dgn"));
        request.limits.max_file_bytes = 8;
        let error = FormatImporter::new()
            .import(&request, &|| false)
            .err()
            .expect("the OLE container is over the limit");
        assert!(
            matches!(error, CadError::InvalidInput(_)),
            "the size check must precede the CFB directory sniff: {error:?}"
        );
    }

    #[test]
    fn unsupported_input_names_dgn() {
        assert!(UNSUPPORTED_INPUT.contains("DGN"));
    }

    #[cfg(feature = "formats-mesh")]
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

    #[cfg(feature = "formats-mesh")]
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
