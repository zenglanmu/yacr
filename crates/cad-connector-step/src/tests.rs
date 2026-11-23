//! STEP contract tests: real fixtures, placement, refusal, limits and units.

use std::sync::{Arc, Mutex};

use super::*;
use cad_domain::{Completeness, DatabaseId, DocumentId, EntityCapability, SemanticGeometry};
use cad_import_core::ImportPhase;

fn request_bytes(bytes: Vec<u8>) -> ImportRequest {
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
        .join("../../fixtures/step")
        .join(name);
    std::fs::read(path).expect("fixture is committed")
}

fn fixture_text(name: &str) -> String {
    String::from_utf8(fixture(name)).expect("fixture is ASCII Part 21")
}

fn meshes(drawing: &cad_import_core::ImportedDrawing) -> Vec<&Mesh> {
    drawing
        .database
        .entities()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Mesh(mesh) => Some(mesh),
            _ => None,
        })
        .inspect(|mesh| {
            // The domain invariant every consumer relies on for STEP output.
            assert!(
                mesh.normals.is_empty() || mesh.normals.len() == mesh.vertices.len(),
                "mesh normals must be empty or one per vertex"
            );
            assert!(
                mesh.colors.is_empty() || mesh.colors.len() == mesh.vertices.len(),
                "mesh colours must be empty or one per vertex"
            );
        })
        .collect()
}

fn diagnostic_codes(drawing: &cad_import_core::ImportedDrawing) -> Vec<&str> {
    drawing
        .report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

fn triangle_total(meshes: &[&Mesh]) -> usize {
    meshes.iter().map(|mesh| mesh.triangles.len()).sum()
}

#[derive(Clone, Copy)]
struct Bounds {
    min_x: f64,
    max_x: f64,
}

fn bounds(mesh: &Mesh) -> Bounds {
    let mut bounds = Bounds {
        min_x: f64::INFINITY,
        max_x: f64::NEG_INFINITY,
    };
    for vertex in &mesh.vertices {
        bounds.min_x = bounds.min_x.min(vertex.x);
        bounds.max_x = bounds.max_x.max(vertex.x);
    }
    bounds
}

#[test]
fn ap214_sheet_tessellates_into_one_mesh() {
    let drawing = StepImporter::new()
        .import(&request_bytes(fixture("ap214-sheet.p21")), &|| false)
        .expect("AP214 sheet imports");
    let meshes = meshes(&drawing);
    assert_eq!(meshes.len(), 1, "one B-rep body becomes one entity");
    assert_eq!(triangle_total(&meshes), 1, "the sheet is one triangle");
    assert_eq!(drawing.units.display, Unit::Millimeter);
    assert_eq!(drawing.units.source, Unit::Millimeter);
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
    assert!(diagnostic_codes(&drawing).contains(&DIAG_UNITS_CANONICALISED));
    let capability: &EntityCapability = &drawing.report.capabilities[0];
    assert_eq!(capability.read, cad_domain::SupportStatus::Verified);
    assert_eq!(capability.render, cad_domain::SupportStatus::Verified);
    assert_eq!(capability.pick, cad_domain::SupportStatus::Verified);
    assert_eq!(capability.measure, cad_domain::SupportStatus::Unsupported);
    assert!(drawing.vendor_records.is_empty());
}

#[test]
fn a_refused_face_is_partial_with_a_diagnostic_and_keeps_the_rest() {
    let drawing = StepImporter::new()
        .import(
            &request_bytes(fixture("synthetic-partial-refusal.p21")),
            &|| false,
        )
        .expect("one good face survives one refusal");
    let meshes = meshes(&drawing);
    assert_eq!(meshes.len(), 1);
    assert_eq!(triangle_total(&meshes), 1, "the accepted face is kept");
    assert!(
        matches!(drawing.report.completeness, Completeness::Partial(_)),
        "a refusal must make the import Partial"
    );
    let codes = diagnostic_codes(&drawing);
    assert!(
        codes.contains(&DIAG_FACE_REFUSED),
        "refusals are first-class diagnostics, got {codes:?}"
    );
    let refusal = drawing
        .report
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == DIAG_FACE_REFUSED)
        .expect("refusal diagnostic");
    assert!(
        refusal.message.contains("no parameter range"),
        "the refusal text is preserved: {}",
        refusal.message
    );
}

#[test]
fn a_file_whose_every_face_is_refused_is_an_error_not_an_empty_success() {
    let error = StepImporter::new()
        .import(&request_bytes(fixture("ap203-sheet.p21")), &|| false)
        .err()
        .expect("all faces refused");
    match error {
        CadError::Unsupported(message) => {
            assert!(message.contains(NO_TESSELLATED_FACES), "got {message}");
            assert!(message.contains("refused"), "got {message}");
        }
        other => panic!("expected an explicit Unsupported refusal, got {other:?}"),
    }
}

#[test]
fn multi_body_assembly_bodies_are_placed_apart_not_stacked_at_the_origin() {
    let drawing = StepImporter::new()
        .import(
            &request_bytes(fixture("synthetic-two-part-assembly.p21")),
            &|| false,
        )
        .expect("two-body assembly imports");
    let meshes = meshes(&drawing);
    assert_eq!(meshes.len(), 2, "one DbEntity per body");
    assert_eq!(triangle_total(&meshes), 2);

    let mut boxes = meshes.iter().map(|mesh| bounds(mesh)).collect::<Vec<_>>();
    boxes.sort_by(|left, right| left.min_x.total_cmp(&right.min_x));
    assert!(
        boxes[0].max_x < boxes[1].min_x,
        "placed bodies must not overlap: {:?} / {:?}",
        boxes[0].min_x..boxes[0].max_x,
        boxes[1].min_x..boxes[1].max_x
    );
    let overall = meshes.iter().map(|mesh| bounds(mesh)).fold(
        Bounds {
            min_x: f64::INFINITY,
            max_x: f64::NEG_INFINITY,
        },
        |acc, value| Bounds {
            min_x: acc.min_x.min(value.min_x),
            max_x: acc.max_x.max(value.max_x),
        },
    );
    assert!(
        overall.min_x.abs() < 1e-9 && overall.max_x > 100.0,
        "the second part's occurrence translation (+100 mm) is applied, overall x = {:?}",
        overall.min_x..overall.max_x
    );
}

#[test]
fn ifc_schema_input_is_refused_by_name() {
    let ifc = b"ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\n\
                FILE_NAME('x','2026-11-10T00:00:00',(''),(''),'','','');\n\
                FILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n";
    let error = StepImporter::new()
        .import(&request_bytes(ifc.to_vec()), &|| false)
        .err()
        .expect("IFC is not STEP");
    match error {
        CadError::Unsupported(message) => {
            assert!(message.contains(IFC_SCHEMA_REFUSED), "got {message}");
            assert!(message.contains("IFC4"), "the schema is named: {message}");
        }
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

#[test]
fn garbage_step_shaped_input_is_corrupt_data() {
    let bytes =
        b"ISO-10303-21;\nHEADER;\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN'));\nDATA;\n#1=NOT_A_RECORD(";
    let error = StepImporter::new()
        .import(&request_bytes(bytes.to_vec()), &|| false)
        .err()
        .expect("truncated Part 21");
    assert!(
        matches!(error, CadError::CorruptData(_)),
        "a failed decode is CorruptData, got {error:?}"
    );
}

#[test]
fn oversized_input_is_rejected_before_decode() {
    let mut request = request_bytes(fixture("ap214-sheet.p21"));
    request.limits.max_file_bytes = 16;
    let error = StepImporter::new()
        .import(&request, &|| false)
        .err()
        .expect("file size limit");
    assert!(matches!(error, CadError::InvalidInput(_)));
}

#[test]
fn triangle_budget_is_an_invalid_input_naming_the_count() {
    let mut request = request_bytes(fixture("ap214-sheet.p21"));
    request.limits.max_mesh_faces = Some(0);
    let error = StepImporter::new()
        .import(&request, &|| false)
        .err()
        .expect("face limit");
    match error {
        CadError::InvalidInput(message) => {
            assert!(message.contains("0"), "got {message}");
            assert!(message.contains("triangle"), "got {message}");
        }
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn entity_budget_is_an_invalid_input() {
    let mut request = request_bytes(fixture("ap214-sheet.p21"));
    request.limits.max_entities = 0;
    let error = StepImporter::new()
        .import(&request, &|| false)
        .err()
        .expect("entity limit");
    assert!(matches!(error, CadError::InvalidInput(_)));
}

#[test]
fn cancellation_is_honoured() {
    let error = StepImporter::new()
        .import(&request_bytes(fixture("ap214-sheet.p21")), &|| true)
        .err()
        .expect("cancelled");
    assert!(matches!(error, CadError::Cancelled));
}

#[test]
fn unresolved_length_unit_is_reported_as_drawing_units_not_millimetres() {
    let mut text = fixture_text("ap214-sheet.p21");
    text = text.replace(
        "#1=(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.));\n",
        "",
    );
    text = text.replace(
        "#2=(GEOMETRIC_REPRESENTATION_CONTEXT(3) GLOBAL_UNIT_ASSIGNED_CONTEXT((#1)) REPRESENTATION_CONTEXT('model','3D'));",
        "#2=(GEOMETRIC_REPRESENTATION_CONTEXT(3) REPRESENTATION_CONTEXT('model','3D'));",
    );
    let drawing = StepImporter::new()
        .import(&request_bytes(text.into_bytes()), &|| false)
        .expect("geometry decodes without a resolved unit");
    assert_eq!(drawing.units.source, Unit::DrawingUnits);
    assert_eq!(drawing.units.display, Unit::DrawingUnits);
    assert!(diagnostic_codes(&drawing).contains(&DIAG_UNITS_UNKNOWN));
    assert!(
        matches!(drawing.report.completeness, Completeness::Partial(_)),
        "an unknown unit cannot be a complete import"
    );
}

#[test]
fn progress_reaches_every_phase_in_order() {
    let phases = Mutex::new(Vec::<ImportPhase>::new());
    let sink = |progress: ImportProgress| phases.lock().unwrap().push(progress.phase);
    StepImporter::new()
        .import_with_progress(&request_bytes(fixture("ap214-sheet.p21")), &|| false, &sink)
        .expect("AP214 sheet imports");
    let phases = phases.into_inner().unwrap();
    for phase in [
        ImportPhase::Reading,
        ImportPhase::Parsing,
        ImportPhase::Entities,
        ImportPhase::Finishing,
    ] {
        assert!(phases.contains(&phase), "missing {phase:?} in {phases:?}");
    }
    assert_eq!(phases.first(), Some(&ImportPhase::Reading));
    assert_eq!(phases.last(), Some(&ImportPhase::Finishing));
}

#[test]
fn placement_normal_matrix_handles_translation_and_scale() {
    let translate = [
        [1.0, 0.0, 0.0, 25.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
    ];
    assert_eq!(
        placement::transform_point(&translate, [1.0, 2.0, 3.0]),
        Some([26.0, 2.0, 3.0])
    );
    assert_eq!(
        placement::normal_matrix(&translate),
        Some([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0],])
    );
    let scale = [
        [2.0, 0.0, 0.0, 0.0],
        [0.0, 2.0, 0.0, 0.0],
        [0.0, 0.0, 2.0, 0.0],
    ];
    assert_eq!(
        placement::normal_matrix(&scale),
        Some([[0.5, 0.0, 0.0], [0.0, 0.5, 0.0], [0.0, 0.0, 0.5],])
    );
}
