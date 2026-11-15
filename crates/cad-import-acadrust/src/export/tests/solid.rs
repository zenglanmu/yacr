//! `SOLID` export: a created 3/4-corner face as a real DXF SOLID record.
//!
//! Writer + our reader agreement is **internal consistency**, not third-party
//! compatibility: nothing here claims AutoCAD or another consumer fills the same face.

use super::*;

use super::entities::export_one_as;

/// A `SOLID` as the domain command creates it: the four corners in **boundary**
/// order plus the implied two-triangle fan (`cad-app`'s draw2 `solid_mesh`).
///
/// Every corner has a distinct `(x, y)`, so the written group order is visible
/// in the bytes without relying on the reader.
fn created_solid_mesh() -> Mesh {
    let vertices = vec![
        p(0.0, 0.0, 0.0),
        p(4.0, 0.0, 0.0),
        p(4.0, 3.0, 0.0),
        p(0.0, 3.0, 0.0),
    ];
    let mut mesh = Mesh {
        vertices,
        triangles: vec![[0, 1, 2], [0, 2, 3]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
    };
    mesh.normals = cad_geometry::compute_vertex_normals(&mesh);
    mesh
}

#[test]
fn a_created_solid_is_written_as_a_solid_record_in_dxf_group_order() {
    let (bytes, report) = export_one_as("AcDbSolid", SemanticGeometry::Mesh(created_solid_mesh()));

    // Nothing is dropped or approximated: the record carries the same 3/4
    // corners the domain mesh stores.
    assert_eq!(report.counts.exact, 1, "{:?}", report.entries);
    assert_eq!(report.counts.converted, 0, "{:?}", report.entries);
    assert!(report.entries.is_empty(), "{:?}", report.entries);

    // Byte level (no reader in the loop): the record type is SOLID, not
    // POLYFACEMESH/TRACE, and groups 10/11/12/13 hold the stored order whose
    // *visible* boundary is 10 -> 11 -> 13 -> 12. The third stored corner
    // (group 12) is therefore boundary[3] = (0, 3), and the fourth stored corner
    // (group 13) is boundary[2] = (4, 3); the naive order would swap them and
    // emit a bow-tie.
    let corners = record_corner_points(&bytes, "SOLID");
    assert_eq!(
        corners,
        vec![
            (10, [0.0, 0.0, 0.0]),
            (11, [4.0, 0.0, 0.0]),
            (12, [0.0, 3.0, 0.0]),
            (13, [4.0, 3.0, 0.0]),
        ]
    );
    let text = String::from_utf8_lossy(&bytes);
    let record_types: Vec<&str> = text.lines().map(str::trim).collect();
    assert!(
        !record_types.contains(&"POLYFACE"),
        "a SOLID must not be a polyface mesh"
    );
    assert!(
        !record_types.contains(&"TRACE"),
        "a SOLID must not be written as TRACE"
    );

    // The reader's own boundary view is the domain order again.
    match &read_entities(&bytes)[0] {
        EntityType::Solid(solid) => {
            assert!(!solid.is_trace);
            assert_eq!(
                solid.boundary_corners(),
                vec![
                    Vector3::new(0.0, 0.0, 0.0),
                    Vector3::new(4.0, 0.0, 0.0),
                    Vector3::new(4.0, 3.0, 0.0),
                    Vector3::new(0.0, 3.0, 0.0),
                ]
            );
        }
        other => panic!("expected SOLID, got {other:?}"),
    }
}

#[test]
fn a_written_solid_reimports_as_the_same_boundary_mesh() {
    // Export -> our importer: the boundary order and the fan survive, so a
    // SAVE->OPEN cycle does not turn the face into a bow-tie or a polyface.
    let original = created_solid_mesh();
    let (bytes, _report) = export_one_as("AcDbSolid", SemanticGeometry::Mesh(original.clone()));
    let reopened = import_bytes(&bytes);
    let entity = reopened.entities().next().expect("the SOLID reimports");
    let SemanticGeometry::Mesh(mesh) = &entity.geometry else {
        panic!("a SOLID reimports as a mesh, got {:?}", entity.geometry);
    };
    assert_eq!(mesh.vertices.len(), original.vertices.len());
    for (actual, expected) in mesh.vertices.iter().zip(original.vertices.iter()) {
        assert_point(*actual, *expected);
    }
    assert_eq!(
        mesh.triangles, original.triangles,
        "the written corner order re-imports as the same boundary fan"
    );
}

#[test]
fn a_three_corner_solid_writes_a_triangle_with_a_repeated_fourth_corner() {
    let mut mesh = created_solid_mesh();
    mesh.vertices.truncate(3);
    mesh.triangles = vec![[0, 1, 2]];
    let (bytes, report) = export_one_as("AcDbSolid", SemanticGeometry::Mesh(mesh));
    assert_eq!(report.counts.exact, 1, "{:?}", report.entries);
    assert_eq!(
        record_corner_points(&bytes, "SOLID"),
        vec![
            (10, [0.0, 0.0, 0.0]),
            (11, [4.0, 0.0, 0.0]),
            (12, [4.0, 3.0, 0.0]),
            (13, [4.0, 3.0, 0.0]),
        ],
        "DXF encodes a triangular SOLID by repeating the third corner in group 13"
    );
    let reopened = import_bytes(&bytes);
    let entity = reopened.entities().next().expect("the triangle reimports");
    let SemanticGeometry::Mesh(mesh) = &entity.geometry else {
        panic!("a SOLID reimports as a mesh");
    };
    // The importer keeps the degenerate record as a four-corner `quad_mesh` with
    // its last two corners coincident and a single triangle, so the filled
    // triangle (not just its corner count) survives.
    assert_eq!(mesh.vertices.len(), 4);
    assert_point(mesh.vertices[2], mesh.vertices[3]);
    assert_eq!(mesh.triangles, vec![[0, 1, 2]]);
}

#[test]
fn a_solid_that_cannot_be_a_solid_record_keeps_the_mesh_path_and_is_declared() {
    // More than four corners: no DXF SOLID record can hold it, so the polyface
    // path stays — and must say so, never silently change the entity type.
    let mut mesh = created_solid_mesh();
    mesh.vertices.push(p(10.0, 10.0, 0.0));
    let (bytes, report) = export_one_as("AcDbSolid", SemanticGeometry::Mesh(mesh));
    assert!(matches!(
        &read_entities(&bytes)[0],
        EntityType::PolyfaceMesh(_)
    ));
    assert_eq!(report.counts.converted, 1);
    assert_eq!(report.counts.exact, 0);
    let entry = report
        .entries
        .iter()
        .find(|entry| entry.code == "export.converted.mesh")
        .expect("the fallback must be declared");
    assert!(entry.message.contains("SOLID"), "{}", entry.message);
    assert!(
        entry.message.contains("this mesh has 5"),
        "the reason must name the corner count: {}",
        entry.message
    );
}

#[test]
fn a_solid_whose_triangulation_is_not_the_record_fan_keeps_the_mesh_path() {
    // A SOLID record has no triangle list: its four corners *are* the
    // `[0,1,2]`/`[0,2,3]` fan. A mesh filled across the other diagonal would
    // change shape for a concave quad, so it keeps the polyface path and is
    // declared instead of being flattened into corners.
    let mut mesh = created_solid_mesh();
    mesh.triangles = vec![[1, 2, 3], [1, 3, 0]];
    let (bytes, report) = export_one_as("AcDbSolid", SemanticGeometry::Mesh(mesh));
    assert!(matches!(
        &read_entities(&bytes)[0],
        EntityType::PolyfaceMesh(_)
    ));
    let entry = report
        .entries
        .iter()
        .find(|entry| entry.code == "export.converted.mesh")
        .expect("the fallback must be declared");
    assert!(entry.message.contains("triangles"), "{}", entry.message);
}

// ---------------------------------------------------------------------------
// Class provenance: an imported SOLID/TRACE must come back as the same class.
// ---------------------------------------------------------------------------

/// A hand-written minimal DXF with one `SOLID` and one `TRACE`.
///
/// Hand-written (no HEADER/TABLES) so the importer is exercised against plain
/// DXF rather than our own writer. Both are rectangles in DXF's **stored**
/// corner order (`10/11/12/13`), whose visible boundary is `10 → 11 → 13 → 12`;
/// the trace is offset in x so a mixed-up record is visible in the corners.
const SOLID_AND_TRACE_DXF: &str = "\
0\nSECTION\n2\nENTITIES\n\
0\nSOLID\n5\n20\n8\n0\n\
10\n0.0\n20\n0.0\n30\n0.0\n\
11\n4.0\n21\n0.0\n31\n0.0\n\
12\n0.0\n22\n3.0\n32\n0.0\n\
13\n4.0\n23\n3.0\n33\n0.0\n\
0\nTRACE\n5\n21\n8\n0\n\
10\n10.0\n20\n0.0\n30\n0.0\n\
11\n14.0\n21\n0.0\n31\n0.0\n\
12\n10.0\n22\n3.0\n32\n0.0\n\
13\n14.0\n23\n3.0\n33\n0.0\n\
0\nENDSEC\n0\nEOF\n";

/// The boundary-order rectangle a `SOLID_AND_TRACE_DXF` record imports as.
fn solid_and_trace_boundary(x: f64) -> Vec<Point3> {
    vec![
        p(x, 0.0, 0.0),
        p(x + 4.0, 0.0, 0.0),
        p(x + 4.0, 3.0, 0.0),
        p(x, 3.0, 0.0),
    ]
}

/// The classes of the imported entities, looked up by their stored type key.
fn imported_face_vertices(db: &DrawingDatabase, type_key: &str) -> Vec<Point3> {
    let entity = db
        .entities()
        .find(|entity| entity.object.type_key == type_key)
        .unwrap_or_else(|| panic!("no imported entity with type key {type_key}"));
    match &entity.geometry {
        SemanticGeometry::Mesh(mesh) => mesh.vertices.clone(),
        other => panic!("{type_key} must import as a mesh, got {other:?}"),
    }
}

#[test]
fn an_imported_solid_and_trace_keep_their_class_through_a_save() {
    let imported = import_bytes(SOLID_AND_TRACE_DXF.as_bytes());

    // Provenance: the importer keeps the two classes apart, so a save can write
    // each one back as its own record.
    assert_eq!(
        imported_face_vertices(&imported, "AcDbSolid"),
        solid_and_trace_boundary(0.0),
        "a SOLID imports in boundary order under its own class"
    );
    assert_eq!(
        imported_face_vertices(&imported, "AcDbTrace"),
        solid_and_trace_boundary(10.0),
        "a TRACE imports in boundary order under its own class"
    );

    let (bytes, report) = export_db(&imported);
    assert_eq!(
        report.counts.exact, 2,
        "both faces are written faithfully: {:?}",
        report.entries
    );
    assert!(report.entries.is_empty(), "{:?}", report.entries);

    // One record per class, each with the `AcDbTrace` subclass and the stored
    // corner order that reproduces the source boundary (`10 -> 11 -> 13 -> 12`).
    let pairs = dxf_pairs(&bytes);
    let record_types = |name: &str| {
        pairs
            .iter()
            .filter(|(code, value)| *code == 0 && value == name)
            .count()
    };
    assert_eq!(record_types("SOLID"), 1, "the SOLID keeps its class");
    assert_eq!(record_types("TRACE"), 1, "the TRACE keeps its class");

    let solid_record = record_pairs_of(&bytes, "SOLID");
    assert!(
        solid_record.contains(&(100, "AcDbTrace".to_string())),
        "a SOLID is written under the AcDbTrace subclass: {solid_record:?}"
    );
    assert_eq!(
        record_corner_points(&bytes, "SOLID"),
        vec![
            (10, [0.0, 0.0, 0.0]),
            (11, [4.0, 0.0, 0.0]),
            (12, [0.0, 3.0, 0.0]),
            (13, [4.0, 3.0, 0.0]),
        ]
    );

    let trace_record = record_pairs_of(&bytes, "TRACE");
    assert!(
        trace_record.contains(&(100, "AcDbTrace".to_string())),
        "a TRACE is written under the AcDbTrace subclass: {trace_record:?}"
    );
    assert_eq!(
        record_corner_points(&bytes, "TRACE"),
        vec![
            (10, [10.0, 0.0, 0.0]),
            (11, [14.0, 0.0, 0.0]),
            (12, [10.0, 3.0, 0.0]),
            (13, [14.0, 3.0, 0.0]),
        ]
    );

    // Second round trip: the classes and the boundary order still survive.
    let reopened = import_bytes(&bytes);
    assert_eq!(
        imported_face_vertices(&reopened, "AcDbSolid"),
        solid_and_trace_boundary(0.0)
    );
    assert_eq!(
        imported_face_vertices(&reopened, "AcDbTrace"),
        solid_and_trace_boundary(10.0)
    );
    // A TRACE must never be re-labelled as the SOLID class.
    assert!(
        reopened
            .entities()
            .filter(|entity| entity.object.type_key == "AcDbSolid")
            .count()
            == 1,
        "exactly one SOLID survives the second round trip"
    );
}

#[test]
fn an_imported_triangular_solid_still_writes_a_solid_record() {
    // A triangular DXF SOLID repeats its third corner in group 13; the importer
    // keeps that as a degenerate four-corner mesh, which must still be a SOLID
    // record (the repeated corner is the DXF encoding, not a data loss).
    let dxf = "\
0\nSECTION\n2\nENTITIES\n\
0\nSOLID\n8\n0\n\
10\n0.0\n20\n0.0\n30\n0.0\n\
11\n4.0\n21\n0.0\n31\n0.0\n\
12\n4.0\n22\n3.0\n32\n0.0\n\
13\n4.0\n23\n3.0\n33\n0.0\n\
0\nENDSEC\n0\nEOF\n";
    let imported = import_bytes(dxf.as_bytes());
    assert_eq!(
        imported_face_vertices(&imported, "AcDbSolid"),
        vec![
            p(0.0, 0.0, 0.0),
            p(4.0, 0.0, 0.0),
            p(4.0, 3.0, 0.0),
            p(4.0, 3.0, 0.0),
        ]
    );

    let (bytes, report) = export_db(&imported);
    assert_eq!(report.counts.exact, 1, "{:?}", report.entries);
    let pairs = dxf_pairs(&bytes);
    assert!(
        !pairs
            .iter()
            .any(|(code, value)| *code == 0 && value == "POLYFACE"),
        "a triangular SOLID must stay a SOLID record"
    );
    assert_eq!(
        record_corner_points(&bytes, "SOLID"),
        vec![
            (10, [0.0, 0.0, 0.0]),
            (11, [4.0, 0.0, 0.0]),
            (12, [4.0, 3.0, 0.0]),
            (13, [4.0, 3.0, 0.0]),
        ]
    );
}

#[test]
fn a_trace_that_cannot_be_a_record_keeps_the_mesh_path_and_names_its_class() {
    // More than four corners: no TRACE record can hold it, so the polyface path
    // stays and the report must name the class it wanted — never silently
    // relabel the TRACE as a SOLID.
    let mut mesh = created_solid_mesh();
    mesh.vertices.push(p(10.0, 10.0, 0.0));
    let (bytes, report) = export_one_as("AcDbTrace", SemanticGeometry::Mesh(mesh));
    assert!(matches!(
        &read_entities(&bytes)[0],
        EntityType::PolyfaceMesh(_)
    ));
    let entry = report
        .entries
        .iter()
        .find(|entry| entry.code == "export.converted.mesh")
        .expect("the fallback must be declared");
    assert!(entry.message.contains("TRACE"), "{}", entry.message);
    assert!(
        !entry.message.contains("SOLID"),
        "a TRACE must not be relabelled as a SOLID: {}",
        entry.message
    );
    assert!(
        entry.message.contains("this mesh has 5"),
        "the reason must name the corner count: {}",
        entry.message
    );
}

#[test]
fn a_corrupt_solid_corner_keeps_its_class_on_the_placeholder_and_is_declared() {
    // A non-finite corner cannot be a filled-face record or even a mesh. The
    // importer keeps it as an opaque placeholder, which must still carry the
    // SOLID class (not the combined `AcDbTrace` label) and must be declared when
    // exported, rather than silently disappearing.
    let dxf = "\
0\nSECTION\n2\nENTITIES\n\
0\nSOLID\n8\n0\n\
10\n1e999\n20\n0.0\n30\n0.0\n\
11\n4.0\n21\n0.0\n31\n0.0\n\
12\n0.0\n22\n3.0\n32\n0.0\n\
13\n4.0\n23\n3.0\n33\n0.0\n\
0\nENDSEC\n0\nEOF\n";
    let imported = import_bytes(dxf.as_bytes());
    let entity = imported
        .entities()
        .next()
        .expect("the corrupt SOLID imports");
    assert_eq!(entity.object.type_key, "AcDbSolid");
    match &entity.geometry {
        SemanticGeometry::Opaque { type_key, .. } => {
            assert_eq!(
                type_key, "AcDbSolid",
                "the placeholder must not relabel the class"
            );
        }
        other => panic!("a non-finite SOLID imports as an opaque placeholder, got {other:?}"),
    }

    let (bytes, report) = export_db(&imported);
    assert_eq!(report.counts.dropped, 1);
    let entry = report
        .entries
        .iter()
        .find(|entry| entry.code == "export.dropped.opaque")
        .expect("the placeholder must be declared");
    assert!(
        entry.message.contains("AcDbSolid"),
        "the drop reason must name the SOLID class: {}",
        entry.message
    );
    assert!(
        !String::from_utf8_lossy(&bytes).contains("1e999"),
        "a non-finite corner must never reach the file"
    );
}

#[test]
fn an_imported_solid_and_trace_keep_their_class_through_the_dwg_path() {
    // The same reverse-mapped document feeds upstream `DwgWriter`, which picks
    // `OBJ_SOLID`/`OBJ_TRACE` from the same `is_trace` flag
    // (`io/dwg/dwg_stream_writers/object_writer/entities.rs:1087`). This is a
    // synthetic self-roundtrip (internal consistency); it is not vendor evidence
    // and the DWG branch stays experimental.
    let imported = import_bytes(SOLID_AND_TRACE_DXF.as_bytes());
    let (bytes, report) = export_db_format(&imported, ExportFormat::Dwg);
    assert!(report
        .notes
        .iter()
        .any(|note| note.code == "export.note.dwg_experimental"));

    let read = DwgReader::from_stream(Cursor::new(bytes)).read().unwrap();
    let mut faces: Vec<(bool, Vec<acadrust::types::Vector3>)> = read
        .model_space_entities()
        .map(|entity| match entity {
            EntityType::Solid(solid) => (solid.is_trace, solid.boundary_corners()),
            other => panic!("unexpected entity in the DWG roundtrip: {other:?}"),
        })
        .collect();
    faces.sort_by(|a, b| a.1[0].x.total_cmp(&b.1[0].x));
    assert_eq!(faces.len(), 2, "one SOLID and one TRACE survive");
    assert!(!faces[0].0, "the SOLID is not a TRACE");
    assert!(faces[1].0, "the TRACE is not a SOLID");
    let expected = [
        (solid_and_trace_boundary(0.0), false),
        (solid_and_trace_boundary(10.0), true),
    ];
    for ((is_trace, corners), (boundary, want_trace)) in faces.iter().zip(expected) {
        assert_eq!(*is_trace, want_trace);
        assert_eq!(corners.len(), boundary.len());
        for (actual, expected) in corners.iter().zip(boundary.iter()) {
            assert!(
                (actual.x - expected.x).abs() < 1e-9
                    && (actual.y - expected.y).abs() < 1e-9
                    && (actual.z - expected.z).abs() < 1e-9,
                "{actual:?} != {expected:?}"
            );
        }
    }
}
