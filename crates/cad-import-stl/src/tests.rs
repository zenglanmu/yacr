//! STL contract tests: ASCII/binary parity, truncation, limits and losses.

use std::sync::Arc;

use super::*;
use cad_domain::{CadError, Completeness, DatabaseId, DocumentId, SemanticGeometry};
use cad_import_core::ImportRequest;

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

fn request(bytes: &[u8]) -> ImportRequest {
    request_bytes(bytes.to_vec())
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

const ASCII_QUAD: &str = "solid quad\n\
facet normal 0 0 1\n\
  outer loop\n\
    vertex 0 0 0\n\
    vertex 1 0 0\n\
    vertex 1 1 0\n\
  endloop\n\
endfacet\n\
facet normal 0 0 1\n\
  outer loop\n\
    vertex 0 0 0\n\
    vertex 1 1 0\n\
    vertex 0 1 0\n\
  endloop\n\
endfacet\n\
endsolid quad\n";

fn binary_triangle(normal: [f32; 3], vertices: [[f32; 3]; 3]) -> Vec<u8> {
    let mut bytes = vec![0u8; 80];
    bytes.extend_from_slice(&1u32.to_le_bytes());
    for value in normal {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for vertex in vertices {
        for value in vertex {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes
}

fn binary_triangles(triangles: &[([f32; 3], [[f32; 3]; 3])]) -> Vec<u8> {
    let mut bytes = vec![0u8; 80];
    bytes.extend_from_slice(&(triangles.len() as u32).to_le_bytes());
    for (normal, vertices) in triangles {
        for value in *normal {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for vertex in vertices {
            for value in vertex {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        bytes.extend_from_slice(&0u16.to_le_bytes());
    }
    bytes
}

#[test]
fn ascii_stl_imports_as_flat_triangles() {
    let drawing = StlImporter::new()
        .import(&request(ASCII_QUAD.as_bytes()), &|| false)
        .expect("ASCII STL imports");
    let meshes = meshes(&drawing);
    assert_eq!(meshes.len(), 1);
    let mesh = meshes[0];
    assert_eq!(mesh.triangles.len(), 2);
    assert_eq!(mesh.vertices.len(), 6, "flat shading expands per facet");
    assert_eq!(mesh.normals.len(), mesh.vertices.len());
    assert!(mesh.colors.is_empty());
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Complete
    ));
    assert_eq!(drawing.units.source, cad_domain::Unit::DrawingUnits);
}

#[test]
fn binary_stl_imports_with_the_same_topology() {
    let bytes = binary_triangle(
        [0.0, 0.0, 1.0],
        [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    );
    assert_eq!(detect_variant(&bytes), Some(StlVariant::Binary));
    let drawing = StlImporter::new()
        .import(&request(&bytes), &|| false)
        .expect("binary STL imports");
    let mesh = meshes(&drawing)[0];
    assert_eq!(mesh.triangles.len(), 1);
    assert_eq!(mesh.vertices.len(), 3);
    assert_eq!(mesh.normals[0].z, 1.0);
}

#[test]
fn truncated_binary_count_is_corrupt_not_an_empty_success() {
    let mut bytes = binary_triangle(
        [0.0, 0.0, 1.0],
        [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    );
    // Claim two facets while only one is present.
    bytes[80..84].copy_from_slice(&2u32.to_le_bytes());
    let error = StlImporter::new()
        .import(&request(&bytes), &|| false)
        .map(|_| ())
        .expect_err("a truncated facet count must fail");
    assert!(matches!(error, CadError::CorruptData(_)), "{error:?}");
}

#[test]
fn max_mesh_faces_is_an_invalid_input_naming_the_counts() {
    let mut req = request(ASCII_QUAD.as_bytes());
    req.limits.max_mesh_faces = Some(1);
    let error = StlImporter::new()
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
    let mut req = request(ASCII_QUAD.as_bytes());
    req.limits.max_mesh_face_indices = Some(5);
    let error = StlImporter::new()
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
fn cancellation_is_honoured() {
    assert!(matches!(
        StlImporter::new().import(&request(ASCII_QUAD.as_bytes()), &|| true),
        Err(CadError::Cancelled)
    ));
}

#[test]
fn degenerate_facet_is_dropped_and_reported() {
    let bytes = binary_triangles(&[
        (
            [0.0, 0.0, 1.0],
            [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        ),
        (
            [0.0, 0.0, 1.0],
            [[2.0, 0.0, 0.0], [2.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
        ),
    ]);
    let drawing = StlImporter::new()
        .import(&request(&bytes), &|| false)
        .expect("the valid facet still imports");
    let mesh = meshes(&drawing)[0];
    assert_eq!(mesh.triangles.len(), 1);
    assert!(diag_codes(&drawing).contains(&DIAG_DEGENERATE_TRIANGLE));
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
}

#[test]
fn zero_stored_normal_is_derived_and_reported() {
    let bytes = binary_triangle(
        [0.0, 0.0, 0.0],
        [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    );
    let drawing = StlImporter::new()
        .import(&request(&bytes), &|| false)
        .expect("binary STL imports");
    let mesh = meshes(&drawing)[0];
    assert_eq!(mesh.normals[0].z, 1.0, "normal derived from the triangle");
    assert!(diag_codes(&drawing).contains(&DIAG_NORMAL_RECOMPUTED));
    assert!(
        matches!(drawing.report.completeness, Completeness::Complete),
        "a recovered normal is not lost geometry"
    );
}

#[test]
fn variant_detection_is_conservative() {
    assert_eq!(
        detect_variant(ASCII_QUAD.as_bytes()),
        Some(StlVariant::Ascii)
    );
    let binary = binary_triangle(
        [0.0, 0.0, 1.0],
        [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    );
    assert_eq!(detect_variant(&binary), Some(StlVariant::Binary));
    assert_eq!(detect_variant(b"not an stl"), None);
}

#[test]
fn committed_ascii_fixture_imports() {
    let bytes = include_bytes!("../../../fixtures/stl/synthetic-quad.stl");
    let drawing = StlImporter::new()
        .import(&request(bytes), &|| false)
        .expect("ASCII fixture imports");
    let mesh = meshes(&drawing)[0];
    assert_eq!(mesh.triangles.len(), 2);
}

#[test]
fn committed_binary_fixture_imports() {
    let bytes = include_bytes!("../../../fixtures/stl/synthetic-quad-binary.stl");
    assert_eq!(detect_variant(bytes), Some(StlVariant::Binary));
    let drawing = StlImporter::new()
        .import(&request(bytes), &|| false)
        .expect("binary fixture imports");
    let mesh = meshes(&drawing)[0];
    assert_eq!(mesh.triangles.len(), 2);
}
