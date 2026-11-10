//! PLY/OFF contract tests: encodings, property order, losses, limits and
//! explicit refusals.

use std::sync::Arc;

use super::*;
use cad_domain::{CadError, Completeness, DatabaseId, DocumentId, Mesh, SemanticGeometry, Unit};
use cad_import_core::{ImportRequest, Importer};

const PLY_ASCII_FIXTURE: &[u8] = include_bytes!("../../../fixtures/ply/synthetic-attrs-ascii.ply");
const PLY_LE_FIXTURE: &[u8] = include_bytes!("../../../fixtures/ply/synthetic-binary-le.ply");
const PLY_BE_FIXTURE: &[u8] = include_bytes!("../../../fixtures/ply/synthetic-binary-be.ply");
const OFF_FIXTURE: &[u8] = include_bytes!("../../../fixtures/off/synthetic-quad-triangle.off");
const COFF_FIXTURE: &[u8] = include_bytes!("../../../fixtures/off/synthetic-colors.coff");

const TRI_PLY: &str = "ply\n\
format ascii 1.0\n\
element vertex 3\n\
property float x\n\
property float y\n\
property float z\n\
element face 1\n\
property list uchar int vertex_indices\n\
end_header\n\
0 0 0\n\
1 0 0\n\
0 1 0\n\
3 0 1 2\n";

const OFF_TRI: &str = "OFF\n3 1 0\n0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n";

fn request(bytes: &[u8]) -> ImportRequest {
    ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(bytes.to_vec().into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref: None,
    }
}

fn meshes(drawing: &ImportedDrawing) -> Vec<&Mesh> {
    drawing
        .database
        .entities()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Mesh(mesh) => Some(mesh),
            _ => None,
        })
        .inspect(|mesh| {
            assert!(
                mesh.normals.is_empty() || mesh.normals.len() == mesh.vertices.len(),
                "mesh normals must be empty or one per vertex"
            );
            assert!(
                mesh.colors.is_empty() || mesh.colors.len() == mesh.vertices.len(),
                "mesh colors must be empty or one per vertex"
            );
            for triangle in &mesh.triangles {
                assert!(triangle
                    .iter()
                    .all(|index| (*index as usize) < mesh.vertices.len()));
            }
            for vertex in &mesh.vertices {
                assert!(vertex.x.is_finite() && vertex.y.is_finite() && vertex.z.is_finite());
            }
        })
        .collect()
}

fn one_mesh(drawing: &ImportedDrawing) -> &Mesh {
    let meshes = meshes(drawing);
    assert_eq!(meshes.len(), 1, "exactly one mesh entity");
    assert_eq!(drawing.database.entities().count(), 1, "one DbEntity");
    meshes[0]
}

fn codes(drawing: &ImportedDrawing) -> Vec<&str> {
    drawing
        .report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

fn ply_error(bytes: &[u8]) -> CadError {
    PlyImporter::new()
        .import(&request(bytes), &|| false)
        .map(|_| ())
        .expect_err("PLY import must fail")
}

fn off_error(bytes: &[u8]) -> CadError {
    OffImporter::new()
        .import(&request(bytes), &|| false)
        .map(|_| ())
        .expect_err("OFF import must fail")
}

fn corrupt_with(error: &CadError, code: &str) {
    match error {
        CadError::CorruptData(message) => assert!(
            message.contains(code),
            "expected corruption code {code}, got: {message}"
        ),
        other => panic!("expected CorruptData with {code}, got {other:?}"),
    }
}

fn invalid_with(error: &CadError, fragments: &[&str]) {
    match error {
        CadError::InvalidInput(message) => {
            for fragment in fragments {
                assert!(
                    message.contains(fragment),
                    "expected InvalidInput to contain {fragment}, got: {message}"
                );
            }
        }
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// PLY
// ---------------------------------------------------------------------------

#[test]
fn committed_ascii_ply_imports_one_entity_with_normals_colours_and_ngon_note() {
    let drawing = PlyImporter::new()
        .import(&request(PLY_ASCII_FIXTURE), &|| false)
        .expect("ASCII PLY imports");
    let mesh = one_mesh(&drawing);
    assert_eq!(mesh.vertices.len(), 5);
    assert_eq!(mesh.triangles.len(), 3, "quad fan (2) + triangle (1)");
    assert_eq!(mesh.normals.len(), 5);
    assert_eq!(mesh.normals[0].z, 1.0);
    assert_eq!(mesh.colors.len(), 5);
    assert_eq!(mesh.colors[0], [255, 0, 0]);
    assert_eq!(mesh.colors[4], [255, 0, 255]);
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
    assert!(codes(&drawing).contains(&ply::DIAG_NGON_FAN));
    assert_eq!(drawing.units.source, Unit::DrawingUnits);
    let capability = &drawing.report.capabilities[0];
    assert_eq!(capability.measure, cad_domain::SupportStatus::Unsupported);
}

#[test]
fn ascii_little_endian_and_big_endian_ply_are_equivalent() {
    let ascii = PlyImporter::new()
        .import(&request(PLY_ASCII_FIXTURE), &|| false)
        .expect("ASCII imports");
    let little = PlyImporter::new()
        .import(&request(PLY_LE_FIXTURE), &|| false)
        .expect("little-endian imports");
    let big = PlyImporter::new()
        .import(&request(PLY_BE_FIXTURE), &|| false)
        .expect("big-endian imports");

    let ascii = one_mesh(&ascii);
    let little = one_mesh(&little);
    let big = one_mesh(&big);

    assert_eq!(ascii.vertices, little.vertices);
    assert_eq!(ascii.vertices, big.vertices);
    assert_eq!(ascii.triangles, little.triangles);
    assert_eq!(ascii.triangles, big.triangles);
    assert_eq!(ascii.normals, little.normals);
    assert_eq!(ascii.normals, big.normals);
    assert_eq!(ascii.colors, little.colors);
    assert_eq!(ascii.colors, big.colors);
}

#[test]
fn property_order_permutation_does_not_shift_values() {
    // x/y/z in three different positions; an extra unknown scalar is inserted
    // before every coordinate so a reader that takes the first three properties
    // would corrupt the mesh.
    let first = "ply\nformat ascii 1.0\nelement vertex 3\n\
property float x\nproperty float y\nproperty float z\n\
property ushort confidence\nelement face 1\n\
property list uchar int vertex_indices\nend_header\n\
0 0 0 7\n1 0 0 8\n0 1 0 9\n3 0 1 2\n";
    let second = "ply\nformat ascii 1.0\nelement vertex 3\n\
property float y\nproperty ushort confidence\nproperty float z\nproperty float x\n\
element face 1\nproperty list uchar int vertex_indices\nend_header\n\
0 7 0 0\n0 8 0 1\n1 9 0 0\n3 0 1 2\n";
    let a = PlyImporter::new()
        .import(&request(first.as_bytes()), &|| false)
        .expect("first order");
    let b = PlyImporter::new()
        .import(&request(second.as_bytes()), &|| false)
        .expect("second order");
    assert_eq!(one_mesh(&a).vertices, one_mesh(&b).vertices);
}

#[test]
fn float_colour_channels_are_mapped_to_u8() {
    let bytes = "ply\nformat ascii 1.0\nelement vertex 3\n\
property float x\nproperty float y\nproperty float z\n\
property float red\nproperty float green\nproperty float blue\n\
element face 1\nproperty list uchar int vertex_indices\nend_header\n\
0 0 0 1.0 0.5 0.0\n\
1 0 0 0.0 0.25 0.75\n\
0 1 0 0.0 0.0 1.0\n\
3 0 1 2\n";
    let drawing = PlyImporter::new()
        .import(&request(bytes.as_bytes()), &|| false)
        .expect("float colours import");
    let mesh = one_mesh(&drawing);
    assert_eq!(mesh.colors[0], [255, 128, 0]);
    assert_eq!(mesh.colors[1], [0, 64, 191]);
    assert!(!codes(&drawing).contains(&ply::DIAG_COLORS_DROPPED));
}

#[test]
fn unsupported_colour_type_is_dropped_with_a_diagnostic() {
    let bytes = "ply\nformat ascii 1.0\nelement vertex 3\n\
property float x\nproperty float y\nproperty float z\n\
property ushort red\nproperty ushort green\nproperty ushort blue\n\
element face 1\nproperty list uchar int vertex_indices\nend_header\n\
0 0 0 255 0 0\n\
1 0 0 0 255 0\n\
0 1 0 0 0 255\n\
3 0 1 2\n";
    let drawing = PlyImporter::new()
        .import(&request(bytes.as_bytes()), &|| false)
        .expect("geometry still imports");
    assert!(one_mesh(&drawing).colors.is_empty());
    assert!(codes(&drawing).contains(&ply::DIAG_COLORS_DROPPED));
}

#[test]
fn truncated_ascii_payload_is_corrupt_naming_the_shortfall() {
    let bytes = "ply\nformat ascii 1.0\nelement vertex 3\n\
property float x\nproperty float y\nproperty float z\n\
element face 2\nproperty list uchar int vertex_indices\nend_header\n\
0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n";
    corrupt_with(&ply_error(bytes.as_bytes()), ply::ERR_TRUNCATED);
}

#[test]
fn truncated_binary_payload_is_corrupt_not_an_empty_success() {
    let mut bytes = PLY_LE_FIXTURE.to_vec();
    bytes.truncate(bytes.len() - 4);
    corrupt_with(&ply_error(&bytes), ply::ERR_TRUNCATED);
}

#[test]
fn trailing_bytes_after_the_declared_elements_are_corrupt() {
    let mut bytes = PLY_ASCII_FIXTURE.to_vec();
    bytes.extend_from_slice(b"garbage\n");
    corrupt_with(&ply_error(&bytes), ply::ERR_TRAILING_BYTES);
}

#[test]
fn non_finite_vertex_coordinate_is_corrupt() {
    let bytes = TRI_PLY.replace("0 0 0\n1 0 0", "nan 0 0\n1 0 0");
    corrupt_with(&ply_error(bytes.as_bytes()), ply::ERR_NON_FINITE);
}

#[test]
fn out_of_range_face_index_is_corrupt() {
    let bytes = TRI_PLY.replace("3 0 1 2", "3 0 1 3");
    corrupt_with(&ply_error(bytes.as_bytes()), ply::ERR_INDEX_RANGE);
}

#[test]
fn unsupported_format_and_property_type_are_explicit_refusals() {
    let bad_version = TRI_PLY.replace("format ascii 1.0", "format ascii 1.1");
    corrupt_with(
        &ply_error(bad_version.as_bytes()),
        ply::ERR_UNSUPPORTED_FORMAT,
    );
    let bad_encoding = TRI_PLY.replace("format ascii 1.0", "format binary_middle_endian 1.0");
    corrupt_with(
        &ply_error(bad_encoding.as_bytes()),
        ply::ERR_UNSUPPORTED_FORMAT,
    );
    let bad_type = TRI_PLY.replace("property float x", "property float128 x");
    corrupt_with(
        &ply_error(bad_type.as_bytes()),
        ply::ERR_UNSUPPORTED_PROPERTY_TYPE,
    );
}

#[test]
fn short_face_is_reported_partial_and_skipped() {
    let bytes = "ply\nformat ascii 1.0\nelement vertex 3\n\
property float x\nproperty float y\nproperty float z\n\
element face 2\nproperty list uchar int vertex_indices\nend_header\n\
0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n2 0 1\n";
    let drawing = PlyImporter::new()
        .import(&request(bytes.as_bytes()), &|| false)
        .expect("the valid face still imports");
    assert_eq!(one_mesh(&drawing).triangles.len(), 1);
    assert!(codes(&drawing).contains(&ply::DIAG_FACE_TOO_FEW_VERTICES));
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
}

#[test]
fn degenerate_triangle_is_dropped_and_reported() {
    let bytes = "ply\nformat ascii 1.0\nelement vertex 3\n\
property float x\nproperty float y\nproperty float z\n\
element face 2\nproperty list uchar int vertex_indices\nend_header\n\
0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n3 0 0 1\n";
    let drawing = PlyImporter::new()
        .import(&request(bytes.as_bytes()), &|| false)
        .expect("the valid face imports");
    assert_eq!(one_mesh(&drawing).triangles.len(), 1);
    assert!(codes(&drawing).contains(&ply::DIAG_DEGENERATE_TRIANGLE));
}

#[test]
fn ply_limits_are_invalid_input_with_real_counts() {
    let mut by_faces = request(PLY_ASCII_FIXTURE);
    by_faces.limits.max_mesh_faces = Some(2);
    invalid_with(
        &PlyImporter::new()
            .import(&by_faces, &|| false)
            .map(|_| ())
            .expect_err("face limit must reject"),
        &["3", "2"],
    );

    let mut by_indices = request(PLY_ASCII_FIXTURE);
    by_indices.limits.max_mesh_face_indices = Some(8);
    invalid_with(
        &PlyImporter::new()
            .import(&by_indices, &|| false)
            .map(|_| ())
            .expect_err("index limit must reject"),
        &["9", "8"],
    );
}

#[test]
fn ply_cancellation_is_honoured() {
    assert!(matches!(
        PlyImporter::new().import(&request(TRI_PLY.as_bytes()), &|| true),
        Err(CadError::Cancelled)
    ));
}

// ---------------------------------------------------------------------------
// OFF
// ---------------------------------------------------------------------------

#[test]
fn committed_off_imports_one_entity_with_three_triangles() {
    let drawing = OffImporter::new()
        .import(&request(OFF_FIXTURE), &|| false)
        .expect("OFF imports");
    let mesh = one_mesh(&drawing);
    assert_eq!(mesh.vertices.len(), 5);
    assert_eq!(mesh.triangles.len(), 3);
    assert!(mesh.normals.is_empty());
    assert!(mesh.colors.is_empty());
    assert!(codes(&drawing).contains(&off::DIAG_NGON_FAN));
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
    assert_eq!(drawing.units.source, Unit::DrawingUnits);
}

#[test]
fn committed_coff_imports_colours_and_two_triangles() {
    let drawing = OffImporter::new()
        .import(&request(COFF_FIXTURE), &|| false)
        .expect("COFF imports");
    let mesh = one_mesh(&drawing);
    assert_eq!(mesh.vertices.len(), 4);
    assert_eq!(mesh.triangles.len(), 2);
    assert_eq!(mesh.colors.len(), 4);
    assert_eq!(mesh.colors[0], [255, 0, 0]);
    assert_eq!(mesh.colors[3], [255, 255, 0]);
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Complete
    ));
    assert_eq!(
        drawing.report.capabilities[0].measure,
        cad_domain::SupportStatus::Unsupported
    );
}

#[test]
fn coff_float_colours_are_handled() {
    let bytes = "COFF\n3 1 0\n\
0 0 0 0.5 0.25 0.75\n\
1 0 0 0.9 0.1 0.2\n\
0 1 0 0.0 1.0 0.5\n\
3 0 1 2\n";
    let drawing = OffImporter::new()
        .import(&request(bytes.as_bytes()), &|| false)
        .expect("float COFF imports");
    assert_eq!(one_mesh(&drawing).colors[0], [128, 64, 191]);
}

#[test]
fn ambiguous_off_colours_are_dropped_with_a_diagnostic() {
    let bytes = "COFF\n3 1 0\n\
0 0 0 0 0 1\n\
1 0 0 0 1 0\n\
0 1 0 1 0 0\n\
3 0 1 2\n";
    let drawing = OffImporter::new()
        .import(&request(bytes.as_bytes()), &|| false)
        .expect("geometry still imports");
    assert!(one_mesh(&drawing).colors.is_empty());
    assert!(codes(&drawing).contains(&off::DIAG_COLORS_DROPPED));
}

#[test]
fn partial_vertex_colours_are_dropped_with_a_diagnostic() {
    // The first vertex has no colour, the rest do: the mesh cannot carry a
    // partial per-vertex list, so all colours are dropped and reported.
    let bytes = "COFF\n3 1 0\n\
0 0 0\n\
1 0 0 255 0 0\n\
0 1 0 0 255 0\n\
3 0 1 2\n";
    let drawing = OffImporter::new()
        .import(&request(bytes.as_bytes()), &|| false)
        .expect("geometry still imports");
    assert!(one_mesh(&drawing).colors.is_empty());
    assert!(codes(&drawing).contains(&off::DIAG_COLORS_DROPPED));
}

#[test]
fn partial_colour_triple_is_rejected() {
    // Four tokens is `x y z <one colour channel>`: not a valid vertex line.
    let bytes = "OFF\n3 1 0\n0 0 0\n1 0 0\n0 1 0 255\n3 0 1 2\n";
    corrupt_with(&off_error(bytes.as_bytes()), off::ERR_MALFORMED_HEADER);
}

#[test]
fn off_alpha_and_face_colour_are_reported_as_dropped() {
    let bytes = "COFF\n3 1 0\n\
0 0 0 255 0 0 128\n\
1 0 0 0 255 0 128\n\
0 1 0 0 0 255 128\n\
3 0 1 2 7\n";
    let drawing = OffImporter::new()
        .import(&request(bytes.as_bytes()), &|| false)
        .expect("COFF imports");
    let codes = codes(&drawing);
    assert!(codes.contains(&off::DIAG_ALPHA_DROPPED));
    assert!(codes.contains(&off::DIAG_FACE_COLOR_NOT_IMPORTED));
    assert_eq!(one_mesh(&drawing).colors.len(), 3);
}

#[test]
fn unknown_off_variant_is_refused_by_stable_code() {
    let bytes = OFF_TRI.replacen("OFF", "NOFF", 1);
    // Detection routes the known family to the OFF adapter, where it refuses.
    assert_eq!(
        cad_import_core::detect_format(bytes.as_bytes()),
        cad_import_core::DrawingFormat::Off
    );
    corrupt_with(&off_error(bytes.as_bytes()), off::ERR_VARIANT_REFUSED);
    for variant in ["STOFF", "4OFF", "CNOFF"] {
        let bytes = format!("{variant}\n3 1 0\n0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n");
        corrupt_with(&off_error(bytes.as_bytes()), off::ERR_VARIANT_REFUSED);
    }
}

#[test]
fn off_truncated_file_is_corrupt_naming_the_shortfall() {
    let bytes = "OFF\n5 1 0\n0 0 0\n1 0 0\n";
    corrupt_with(&off_error(bytes.as_bytes()), off::ERR_TRUNCATED);
}

#[test]
fn off_trailing_data_is_corrupt() {
    let bytes = format!("{OFF_TRI}garbage\n");
    corrupt_with(&off_error(bytes.as_bytes()), off::ERR_TRAILING_DATA);
}

#[test]
fn off_non_finite_coordinate_is_corrupt() {
    let bytes = OFF_TRI.replace("0 0 0", "nan 0 0");
    corrupt_with(&off_error(bytes.as_bytes()), off::ERR_NON_FINITE);
}

#[test]
fn off_out_of_range_index_is_corrupt() {
    let bytes = OFF_TRI.replace("3 0 1 2", "3 0 1 3");
    corrupt_with(&off_error(bytes.as_bytes()), off::ERR_INDEX_RANGE);
}

#[test]
fn off_short_face_is_reported_partial() {
    let bytes = "OFF\n3 2 0\n0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n2 0 1\n";
    let drawing = OffImporter::new()
        .import(&request(bytes.as_bytes()), &|| false)
        .expect("the valid face still imports");
    assert_eq!(one_mesh(&drawing).triangles.len(), 1);
    assert!(codes(&drawing).contains(&off::DIAG_FACE_TOO_FEW_VERTICES));
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
}

#[test]
fn off_limits_are_invalid_input_with_real_counts() {
    let mut by_faces = request(OFF_FIXTURE);
    by_faces.limits.max_mesh_faces = Some(2);
    invalid_with(
        &OffImporter::new()
            .import(&by_faces, &|| false)
            .map(|_| ())
            .expect_err("face limit must reject"),
        &["3", "2"],
    );

    let mut by_indices = request(OFF_FIXTURE);
    by_indices.limits.max_mesh_face_indices = Some(8);
    invalid_with(
        &OffImporter::new()
            .import(&by_indices, &|| false)
            .map(|_| ())
            .expect_err("index limit must reject"),
        &["9", "8"],
    );
}

#[test]
fn off_cancellation_is_honoured() {
    assert!(matches!(
        OffImporter::new().import(&request(OFF_TRI.as_bytes()), &|| true),
        Err(CadError::Cancelled)
    ));
}
