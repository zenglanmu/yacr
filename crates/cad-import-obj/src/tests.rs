//! OBJ contract tests: mesh mapping, limits, honest losses and cancellation.

use std::sync::{Arc, Mutex};

use super::*;
use cad_domain::{
    CadError, Completeness, DatabaseId, DocumentId, EntityCapability, SemanticGeometry,
};
use cad_import_core::{ImportPhase, ImportProgress};

fn request(bytes: &str) -> ImportRequest {
    request_bytes(bytes.as_bytes().to_vec())
}

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

fn meshes(drawing: &cad_import_core::ImportedDrawing) -> Vec<&Mesh> {
    drawing
        .database
        .entities()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Mesh(mesh) => Some(mesh),
            _ => None,
        })
        .inspect(|mesh| {
            // The domain invariant every consumer relies on: normals are either
            // absent or exactly one per vertex. A short list would be indexed
            // out of bounds by the scene/render path.
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

fn diag_codes(drawing: &cad_import_core::ImportedDrawing) -> Vec<&str> {
    drawing
        .report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

fn quad() -> &'static str {
    "o quad\n\
     v 0 0 0\n\
     v 1 0 0\n\
     v 1 1 0\n\
     v 0 1 0\n\
     f 1 2 3 4\n"
}

fn triangle() -> &'static str {
    "o tri\n\
     v 0 0 0\n\
     v 1 0 0\n\
     v 0 1 0\n\
     f 1 2 3\n"
}

#[test]
fn quad_becomes_two_fan_triangles_and_is_partial() {
    let drawing = ObjImporter::new()
        .import(&request(quad()), &|| false)
        .expect("quad imports");
    let meshes = meshes(&drawing);
    assert_eq!(meshes.len(), 1);
    let mesh = meshes[0];
    assert_eq!(mesh.vertices.len(), 4);
    assert_eq!(mesh.triangles, vec![[0, 1, 2], [0, 2, 3]]);
    assert!(mesh.normals.is_empty(), "no vn means no normals");
    assert!(mesh.face_sources.is_empty());
    assert!(mesh.colors.is_empty());
    assert!(
        matches!(drawing.report.completeness, Completeness::Partial(_)),
        "a fan-triangulated quad must not claim Complete"
    );
    assert!(diag_codes(&drawing).contains(&DIAG_NGON_FAN));
}

#[test]
fn triangle_without_normals_is_complete_and_has_empty_normals() {
    let drawing = ObjImporter::new()
        .import(&request(triangle()), &|| false)
        .expect("triangle imports");
    let meshes = meshes(&drawing);
    assert_eq!(meshes.len(), 1);
    assert_eq!(meshes[0].triangles.len(), 1);
    assert!(meshes[0].normals.is_empty());
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Complete
    ));
}

#[test]
fn supplied_normals_are_one_per_vertex() {
    let bytes = "o tri\n\
                 v 0 0 0\n\
                 v 1 0 0\n\
                 v 0 1 0\n\
                 vn 0 0 1\n\
                 f 1//1 2//1 3//1\n";
    let drawing = ObjImporter::new()
        .import(&request(bytes), &|| false)
        .expect("normals import");
    let mesh = meshes(&drawing)[0];
    assert_eq!(mesh.normals.len(), mesh.vertices.len());
    assert!(mesh.normals.iter().all(|n| n.z == 1.0));
}

#[test]
fn multiple_objects_become_multiple_entities() {
    let bytes = "o a\n\
                 v 0 0 0\n\
                 v 1 0 0\n\
                 v 0 1 0\n\
                 f 1 2 3\n\
                 o b\n\
                 v 2 0 0\n\
                 v 3 0 0\n\
                 v 2 1 0\n\
                 f 4 5 6\n";
    let drawing = ObjImporter::new()
        .import(&request(bytes), &|| false)
        .expect("two objects import");
    let meshes = meshes(&drawing);
    assert_eq!(meshes.len(), 2);
    assert_eq!(meshes[0].triangles.len(), 1);
    assert_eq!(meshes[1].triangles.len(), 1);
    assert_eq!(drawing.database.entity_count(), 2);
}

#[test]
fn duplicate_object_names_merge_into_one_entity() {
    // `tobj` emits one Model per `o`/`g`/`usemtl` boundary. Two records with the
    // same name (and no material) are the same OBJ object, so they must merge
    // rather than produce a duplicate entity.
    let bytes = "o same\n\
                 v 0 0 0\n\
                 v 1 0 0\n\
                 v 0 1 0\n\
                 f 1 2 3\n\
                 o same\n\
                 v 2 0 0\n\
                 v 3 0 0\n\
                 v 2 1 0\n\
                 f 4 5 6\n";
    let drawing = ObjImporter::new()
        .import(&request(bytes), &|| false)
        .expect("duplicate names import");
    let meshes = meshes(&drawing);
    assert_eq!(meshes.len(), 1, "same name/material is one entity");
    assert_eq!(meshes[0].triangles.len(), 2);
    assert_eq!(drawing.database.entity_count(), 1);
}

#[test]
fn a_five_corner_face_is_partial_with_the_ngon_code() {
    let bytes = "o pent\n\
                 v 0 0 0\n\
                 v 1 0 0\n\
                 v 1 1 0\n\
                 v 0.5 1.5 0\n\
                 v 0 1 0\n\
                 f 1 2 3 4 5\n";
    let drawing = ObjImporter::new()
        .import(&request(bytes), &|| false)
        .expect("pentagon imports");
    let mesh = meshes(&drawing)[0];
    assert_eq!(mesh.triangles.len(), 3, "a 5-gon fans into 3 triangles");
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
    assert!(diag_codes(&drawing).contains(&DIAG_NGON_FAN));
}

#[test]
fn max_mesh_faces_is_an_invalid_input_naming_the_counts() {
    let mut req = request(quad());
    req.limits.max_mesh_faces = Some(1);
    let error = ObjImporter::new()
        .import(&req, &|| false)
        .map(|_| ())
        .expect_err("face limit must reject");
    match error {
        CadError::InvalidInput(message) => {
            assert!(
                message.contains('2'),
                "message names the real count: {message}"
            );
            assert!(message.contains('1'), "message names the limit: {message}");
        }
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn max_mesh_face_indices_is_an_invalid_input_naming_the_counts() {
    let mut req = request(quad());
    req.limits.max_mesh_face_indices = Some(5);
    let error = ObjImporter::new()
        .import(&req, &|| false)
        .map(|_| ())
        .expect_err("index limit must reject");
    match error {
        CadError::InvalidInput(message) => {
            assert!(
                message.contains('6'),
                "message names the real count: {message}"
            );
            assert!(message.contains('5'), "message names the limit: {message}");
        }
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn empty_and_garbage_inputs_fail_instead_of_succeeding_empty() {
    for bytes in ["", "not an obj directive\n", "v 0 0 0\n"] {
        assert!(
            ObjImporter::new()
                .import(&request(bytes), &|| false)
                .is_err(),
            "input {bytes:?} must not import as an empty success"
        );
    }
}

#[test]
fn cancellation_is_honoured() {
    let result = ObjImporter::new().import(&request(quad()), &|| true);
    assert!(matches!(result, Err(CadError::Cancelled)));
}

#[test]
fn per_vertex_colours_are_mapped_when_present() {
    let bytes = "o tri\n\
                 v 0 0 0 1 0 0\n\
                 v 1 0 0 0 1 0\n\
                 v 0 1 0 0 0 1\n\
                 f 1 2 3\n";
    let drawing = ObjImporter::new()
        .import(&request(bytes), &|| false)
        .expect("vertex colour import");
    let mesh = meshes(&drawing)[0];
    assert_eq!(mesh.colors.len(), mesh.vertices.len());
    assert_eq!(mesh.colors[0], [255, 0, 0]);
}

#[test]
fn material_diffuse_maps_to_explicit_colour_and_missing_is_by_layer() {
    let material = Material {
        name: "red".into(),
        diffuse: Some([1.0, 0.5, 0.0]),
        ..Default::default()
    };
    assert_eq!(
        material_color(Some(0), std::slice::from_ref(&material)),
        cad_db::EntityColor::Explicit([255, 128, 0])
    );
    assert_eq!(
        material_color(None, std::slice::from_ref(&material)),
        cad_db::EntityColor::ByLayer
    );
    assert_eq!(
        material_color(Some(9), std::slice::from_ref(&material)),
        cad_db::EntityColor::ByLayer
    );
}

#[test]
fn a_referenced_mtl_is_reported_not_silently_dropped() {
    let bytes = "mtllib missing.mtl\n\
                 o tri\n\
                 v 0 0 0\n\
                 v 1 0 0\n\
                 v 0 1 0\n\
                 f 1 2 3\n";
    let drawing = ObjImporter::new()
        .import(&request(bytes), &|| false)
        .expect("geometry still imports without its MTL");
    assert_eq!(meshes(&drawing).len(), 1);
    assert!(diag_codes(&drawing).contains(&DIAG_MTL_NOT_LOADED));
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
}

#[test]
fn progress_reports_reading_parsing_entities_finishing() {
    let ticks = Mutex::new(Vec::new());
    ObjImporter::new()
        .import_with_progress(
            &request(triangle()),
            &|| false,
            &|progress: ImportProgress| ticks.lock().unwrap().push(progress.phase),
        )
        .expect("triangle imports");
    let phases = ticks.into_inner().unwrap();
    assert_eq!(
        phases,
        vec![
            ImportPhase::Reading,
            ImportPhase::Parsing,
            ImportPhase::Entities,
            ImportPhase::Finishing
        ]
    );
}

#[test]
fn capability_rows_say_mesh_is_not_measurable() {
    let drawing = ObjImporter::new()
        .import(&request(triangle()), &|| false)
        .expect("triangle imports");
    let row: &EntityCapability = drawing
        .report
        .capabilities
        .iter()
        .find(|row| row.type_key == OBJ_MESH_TYPE_KEY)
        .expect("OBJ capability row");
    assert_eq!(row.measure, cad_domain::SupportStatus::Unsupported);
    assert_eq!(row.render, cad_domain::SupportStatus::Verified);
    assert_eq!(row.pick, cad_domain::SupportStatus::Verified);
}

#[test]
fn committed_quad_fixture_imports_with_two_triangles() {
    let bytes = include_str!("../../../fixtures/obj/synthetic-quad.obj");
    let drawing = ObjImporter::new()
        .import(&request(bytes), &|| false)
        .expect("fixture imports");
    let meshes = meshes(&drawing);
    assert!(!meshes.is_empty(), "fixture must produce at least one mesh");
    let total: usize = meshes.iter().map(|mesh| mesh.triangles.len()).sum();
    assert_eq!(
        total, 2,
        "the committed quad fan-triangulates to two triangles"
    );
}
