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
