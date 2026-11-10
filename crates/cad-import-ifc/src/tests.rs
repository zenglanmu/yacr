//! IFC contract tests: both schemas, placement, opening subtraction, units,
//! refusal channels, limits, cancellation, mesh invariants and detection.

use std::cell::Cell;
use std::sync::Arc;

use super::*;
use cad_domain::{
    CadError, Completeness, DatabaseId, DocumentId, EntityCapability, SemanticGeometry,
    SupportStatus,
};
use cad_import_core::{detect_format, DrawingFormat, ImportPhase, IFC_SCHEMA_REFUSED};
use cad_import_step::StepImporter;

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
        .join("../../fixtures/ifc")
        .join(name);
    std::fs::read(path).expect("fixture is committed")
}

fn import_fixture(name: &str) -> cad_import_core::ImportedDrawing {
    IfcImporter::new()
        .import(&request_bytes(fixture(name)), &|| false)
        .unwrap_or_else(|error| panic!("{name} must import: {error:?}"))
}

fn meshes(drawing: &cad_import_core::ImportedDrawing) -> Vec<&Mesh> {
    drawing
        .database
        .entities()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Mesh(mesh) => Some(mesh),
            _ => None,
        })
        .inspect(|mesh| assert_mesh_invariants(mesh))
        .collect()
}

/// The invariants `cad-db::validate_geometry` and every consumer rely on.
fn assert_mesh_invariants(mesh: &Mesh) {
    assert!(
        mesh.normals.is_empty() || mesh.normals.len() == mesh.vertices.len(),
        "mesh normals must be empty or one per vertex"
    );
    assert!(
        mesh.colors.is_empty() || mesh.colors.len() == mesh.vertices.len(),
        "mesh colours must be empty or one per vertex"
    );
    assert!(!mesh.vertices.is_empty(), "a stored mesh has vertices");
    assert!(!mesh.triangles.is_empty(), "a stored mesh has triangles");
    for vertex in &mesh.vertices {
        assert!(
            vertex.x.is_finite() && vertex.y.is_finite() && vertex.z.is_finite(),
            "mesh vertex is non-finite"
        );
    }
    for normal in &mesh.normals {
        assert!(
            normal.x.is_finite() && normal.y.is_finite() && normal.z.is_finite(),
            "mesh normal is non-finite"
        );
    }
    let count = mesh.vertices.len() as u32;
    for triangle in &mesh.triangles {
        assert!(
            triangle.iter().all(|index| *index < count),
            "mesh triangle references an unknown vertex"
        );
        assert!(
            triangle[0] != triangle[1] && triangle[1] != triangle[2] && triangle[0] != triangle[2],
            "mesh triangle is degenerate"
        );
    }
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

#[derive(Clone, Copy, Debug)]
struct Bounds {
    min: [f64; 3],
    max: [f64; 3],
}

fn bounds(mesh: &Mesh) -> Bounds {
    let mut bounds = Bounds {
        min: [f64::INFINITY; 3],
        max: [f64::NEG_INFINITY; 3],
    };
    for vertex in &mesh.vertices {
        for (axis, value) in [vertex.x, vertex.y, vertex.z].into_iter().enumerate() {
            bounds.min[axis] = bounds.min[axis].min(value);
            bounds.max[axis] = bounds.max[axis].max(value);
        }
    }
    bounds
}

fn extent(bounds: &Bounds, axis: usize) -> f64 {
    bounds.max[axis] - bounds.min[axis]
}

#[test]
fn ifc4_wall_imports_one_mesh_in_metres() {
    let drawing = import_fixture("synthetic-wall.ifc");
    let meshes = meshes(&drawing);
    assert_eq!(meshes.len(), 1, "one product becomes one DbEntity");
    // A 4000 × 200 × 3000 mm box extrusion is 12 triangles.
    assert_eq!(triangle_total(&meshes), 12);
    // The file declares millimetres; the router scales to metres, so a
    // 4000 × 200 × 3000 mm wall is 4 × 0.2 × 3 m in the database.
    let bounds = bounds(meshes[0]);
    assert!((extent(&bounds, 0) - 4.0).abs() < 1e-3, "{bounds:?}");
    assert!((extent(&bounds, 1) - 0.2).abs() < 1e-3, "{bounds:?}");
    assert!((extent(&bounds, 2) - 3.0).abs() < 1e-3, "{bounds:?}");
    // Stored metres, displayed in the authored millimetres.
    assert_eq!(drawing.units.source, Unit::Meter);
    assert_eq!(drawing.units.display, Unit::Millimeter);
    assert_eq!(drawing.units.display_per_source, Some(1000.0));
    assert_eq!(drawing.units.to_display(4.0), Some(4000.0));
    assert_eq!(
        drawing.report.completeness,
        Completeness::Complete,
        "diagnostics: {:?}",
        drawing.report.diagnostics
    );
    assert!(diagnostic_codes(&drawing).contains(&DIAG_UNITS_CANONICALISED));
    let capability: &EntityCapability = &drawing.report.capabilities[0];
    assert_eq!(capability.type_key, IFC_MESH_TYPE_KEY);
    assert_eq!(capability.read, SupportStatus::Verified);
    assert_eq!(capability.render, SupportStatus::Verified);
    assert_eq!(capability.pick, SupportStatus::Verified);
    assert_eq!(capability.measure, SupportStatus::Unsupported);
    assert!(
        drawing.vendor_records.is_empty(),
        "IFC has no vendor records"
    );
}

#[test]
fn ifc2x3_two_walls_keep_distinct_placed_bounds() {
    let drawing = import_fixture("synthetic-two-walls-ifc2x3.ifc");
    let meshes = meshes(&drawing);
    assert_eq!(meshes.len(), 2, "two products become two DbEntities");
    assert_eq!(
        triangle_total(&meshes),
        24,
        "each wall is a 12-triangle box"
    );
    let mut ordered: Vec<Bounds> = meshes.iter().map(|mesh| bounds(mesh)).collect();
    ordered.sort_by(|left, right| left.min[0].total_cmp(&right.min[0]));
    // Wall A: x 0..4000 mm, wall B placed at x=6000 mm. If placement were not
    // baked in, both would sit at the origin with identical bounds.
    assert!((ordered[0].min[0] - 0.0).abs() < 1e-3, "{ordered:?}");
    assert!((ordered[0].max[0] - 4.0).abs() < 1e-3, "{ordered:?}");
    assert!((ordered[1].min[0] - 6.0).abs() < 1e-3, "{ordered:?}");
    assert!((ordered[1].max[0] - 10.0).abs() < 1e-3, "{ordered:?}");
    assert!(
        ordered[0].max[0] <= ordered[1].min[0],
        "placed walls must not overlap: {ordered:?}"
    );
    assert_eq!(drawing.units.source, Unit::Meter);
}

#[test]
fn opening_subtraction_adds_triangles() {
    let uncut = import_fixture("synthetic-wall.ifc");
    let cut = import_fixture("synthetic-wall-opening.ifc");
    let uncut_meshes = meshes(&uncut);
    let cut_meshes = meshes(&cut);
    assert_eq!(uncut_meshes.len(), 1);
    assert_eq!(
        cut_meshes.len(),
        1,
        "the opening is a cutter, not a visible product"
    );
    let uncut_triangles = triangle_total(&uncut_meshes);
    let cut_triangles = triangle_total(&cut_meshes);
    assert_eq!(uncut_triangles, 12, "the uncut wall is a 12-triangle box");
    assert_eq!(
        cut_triangles, 32,
        "the opening cuts the box and adds reveal faces (12 -> 32)"
    );
    assert!(
        cut_triangles > uncut_triangles,
        "an opening must add reveal geometry: {uncut_triangles} -> {cut_triangles}"
    );
    // The cut must not move the wall: the outer bounds stay the same.
    let before = bounds(uncut_meshes[0]);
    let after = bounds(cut_meshes[0]);
    for axis in 0..3 {
        assert!(
            (before.min[axis] - after.min[axis]).abs() < 1e-3
                && (before.max[axis] - after.max[axis]).abs() < 1e-3,
            "opening moved the wall on axis {axis}: {before:?} -> {after:?}"
        );
    }
    assert!(
        !diagnostic_codes(&cut).contains(&DIAG_CSG_FAILURE),
        "the fixture's opening must cut cleanly: {:?}",
        cut.report.diagnostics
    );
}

#[test]
fn mesh_invariants_hold_for_every_imported_fixture() {
    for name in [
        "synthetic-wall.ifc",
        "synthetic-wall-opening.ifc",
        "synthetic-two-walls-ifc2x3.ifc",
    ] {
        let drawing = import_fixture(name);
        let meshes = meshes(&drawing);
        assert!(!meshes.is_empty(), "{name} imports at least one mesh");
        for mesh in meshes {
            assert_mesh_invariants(mesh);
        }
    }
}

#[test]
fn an_unresolved_length_unit_is_drawing_units_and_partial() {
    let drawing = import_fixture("synthetic-wall-no-units.ifc");
    let meshes = meshes(&drawing);
    assert_eq!(meshes.len(), 1);
    assert_eq!(drawing.units.source, Unit::DrawingUnits);
    assert_eq!(drawing.units.display, Unit::DrawingUnits);
    assert_eq!(
        drawing.units.display_per_source, None,
        "an unknown unit must not invent a scale"
    );
    // No declared unit means the router's scale stays 1.0: the raw file numbers
    // (4000, not 4 metres) are what is stored.
    assert!((extent(&bounds(meshes[0]), 0) - 4000.0).abs() < 1e-3);
    assert!(diagnostic_codes(&drawing).contains(&DIAG_UNITS_UNKNOWN));
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
}

#[test]
fn every_element_refused_is_an_error_not_an_empty_success() {
    let error = IfcImporter::new()
        .import(&request_bytes(fixture("synthetic-axis-only.ifc")), &|| {
            false
        })
        .err()
        .expect("an axis-only wall has no drawable mesh");
    match error {
        CadError::Unsupported(message) => assert!(
            message.contains(ALL_ELEMENTS_FAILED),
            "stable code missing: {message}"
        ),
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

#[test]
fn over_limit_file_size_is_invalid_input() {
    let mut request = request_bytes(fixture("synthetic-wall.ifc"));
    request.limits.max_file_bytes = 128;
    let error = IfcImporter::new()
        .import(&request, &|| false)
        .err()
        .expect("file exceeds the byte limit");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert!(error.to_string().contains("byte import limit"));
}

#[test]
fn over_limit_triangle_count_is_invalid_input() {
    let mut request = request_bytes(fixture("synthetic-wall.ifc"));
    request.limits.max_mesh_faces = Some(1);
    let error = IfcImporter::new()
        .import(&request, &|| false)
        .err()
        .expect("the wall exceeds one triangle");
    match error {
        CadError::InvalidInput(message) => assert!(
            message.contains("triangle") && message.contains('1'),
            "counts missing: {message}"
        ),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn garbage_bytes_are_corrupt_data() {
    let error = IfcImporter::new()
        .import(&request_bytes(b"hello world".to_vec()), &|| false)
        .err()
        .expect("not an IFC file");
    match error {
        CadError::CorruptData(message) => assert!(
            message.contains(NO_GEOMETRY_PRODUCTS),
            "stable code missing: {message}"
        ),
        other => panic!("expected CorruptData, got {other:?}"),
    }
}

#[test]
fn truncated_ifc_is_corrupt_data() {
    let truncated = b"ISO-10303-21;\nHEADER;\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
                      #1=IFCWALL('2O2Fr$t4X7Zf8NOew3FL"
        .to_vec();
    let error = IfcImporter::new()
        .import(&request_bytes(truncated), &|| false)
        .err()
        .expect("a truncated DATA section holds no product");
    match error {
        CadError::CorruptData(message) => assert!(
            message.contains(NO_GEOMETRY_PRODUCTS),
            "stable code missing: {message}"
        ),
        other => panic!("expected CorruptData, got {other:?}"),
    }
}

#[test]
fn cancellation_is_honoured() {
    let calls = Cell::new(0usize);
    let cancelled = || {
        calls.set(calls.get() + 1);
        calls.get() > 1
    };
    let error = IfcImporter::new()
        .import(&request_bytes(fixture("synthetic-wall.ifc")), &cancelled)
        .err()
        .expect("cancellation stops the import");
    assert!(matches!(error, CadError::Cancelled), "{error:?}");
}

#[test]
fn ifc_detection_differs_from_step_by_file_schema() {
    let ifc = fixture("synthetic-wall.ifc");
    assert_eq!(detect_format(&ifc), DrawingFormat::Ifc);
    assert!(ifc_file_schema(&ifc).is_some());
    let step = b"ISO-10303-21;\nHEADER;\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN'));\nENDSEC;\n";
    assert_eq!(detect_format(step), DrawingFormat::Step);
    assert!(ifc_file_schema(step).is_none());
}

#[test]
fn ifc_never_reaches_the_step_importer() {
    let request = request_bytes(fixture("synthetic-wall.ifc"));
    let error = StepImporter::new()
        .import(&request, &|| false)
        .err()
        .expect("the STEP adapter must refuse IFC bytes");
    match error {
        CadError::Unsupported(message) => assert!(
            message.contains(IFC_SCHEMA_REFUSED),
            "stable code missing: {message}"
        ),
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

#[test]
fn real_ifc_from_env_is_imported() {
    // Opt-in real-file check (`YACR_TEST_IFC`), same convention as
    // `YACR_TEST_DWG`. A skip is not a run and not evidence.
    let Ok(path) = std::env::var("YACR_TEST_IFC") else {
        return;
    };
    let bytes = std::fs::read(&path).expect("YACR_TEST_IFC must name a readable file");
    let drawing = IfcImporter::new()
        .import(&request_bytes(bytes), &|| false)
        .unwrap_or_else(|error| panic!("{path} must import: {error:?}"));
    assert!(
        meshes(&drawing)
            .iter()
            .any(|mesh| !mesh.triangles.is_empty()),
        "{path} produced no drawable mesh"
    );
}

#[test]
fn an_opening_without_a_host_is_reported_not_drawn() {
    let drawing = import_fixture("synthetic-orphan-opening.ifc");
    let meshes = meshes(&drawing);
    assert_eq!(meshes.len(), 1, "the wall is the only visible product");
    assert_eq!(triangle_total(&meshes), 12, "the wall stays uncut");
    assert!(diagnostic_codes(&drawing).contains(&DIAG_OPENING_WITHOUT_HOST));
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
}

#[test]
fn progress_reaches_every_phase_with_real_counts() {
    let phases = std::sync::Mutex::new(Vec::<ImportPhase>::new());
    let sink = |progress: ImportProgress| phases.lock().unwrap().push(progress.phase);
    IfcImporter::new()
        .import_with_progress(
            &request_bytes(fixture("synthetic-two-walls-ifc2x3.ifc")),
            &|| false,
            &sink,
        )
        .expect("two-wall IFC imports");
    let phases = phases.into_inner().unwrap();
    for expected in [
        ImportPhase::Reading,
        ImportPhase::Parsing,
        ImportPhase::Entities,
        ImportPhase::Finishing,
    ] {
        assert!(phases.contains(&expected), "missing phase {expected:?}");
    }
}
