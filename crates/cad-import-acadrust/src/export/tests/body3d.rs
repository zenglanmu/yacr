//! `SemanticGeometry::Body3d` export contract: only the display cache is
//! written, never a fabricated ACIS record.

use super::*;

fn triangle_mesh() -> Mesh {
    Mesh {
        vertices: vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(0.0, 1.0, 0.0)],
        triangles: vec![[0, 1, 2]],
        normals: vec![p(0.0, 0.0, 1.0); 3],
        face_sources: Vec::new(),
        colors: Vec::new(),
    }
}

fn body_geometry(
    kind: BodyKind,
    mesh: Mesh,
    edges: Vec<Vec<Point3>>,
    exact: BodyExact,
) -> SemanticGeometry {
    SemanticGeometry::Body3d(Box::new(Body3d {
        kind,
        exact,
        display: BodyDisplay {
            mesh,
            edges,
            tolerance: Some(0.01),
            precision: Precision::Approximate {
                error_bound: Some(0.01),
            },
            completeness: Completeness::Partial(vec!["display-only body".into()]),
        },
    }))
}

fn unavailable() -> BodyExact {
    BodyExact::Unavailable {
        reason: "test fixture without exact geometry".into(),
    }
}

/// A body that has a display mesh is written as a `POLYFACE` mesh — the same
/// record a standalone `Mesh` writes — and the report names the exact loss.
#[test]
fn body3d_display_mesh_exports_as_a_polyface_and_never_as_a_3dsolid() {
    let edge = vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)];
    let geometry = body_geometry(BodyKind::Sheet, triangle_mesh(), vec![edge], unavailable());
    let (bytes, report) = export_one(geometry);
    let entities = read_entities(&bytes);
    assert_eq!(entities.len(), 1);
    match &entities[0] {
        EntityType::PolyfaceMesh(poly) => {
            assert_eq!(poly.vertices.len(), 3);
            assert_eq!(poly.faces.len(), 1);
        }
        other => panic!("expected POLYFACEMESH, got {other:?}"),
    }
    assert!(
        !entities
            .iter()
            .any(|entity| matches!(entity, EntityType::Solid3D(_))),
        "a body must never fabricate a 3DSOLID record"
    );

    // The exact geometry was not written, so the report must not claim it.
    assert_eq!(report.counts.exact, 0);
    assert_eq!(report.counts.converted, 1);
    assert!(
        report
            .entries
            .iter()
            .any(|entry| entry.code == "export.converted.mesh"
                && entry.message.contains("not as a 3DSOLID")
                && entry.message.contains("original SAT/SAB source")),
        "{:?}",
        report.entries
    );
    // The display edges are not silently dropped with the mesh.
    assert!(
        report
            .entries
            .iter()
            .any(|entry| entry.code == "export.converted.mesh"
                && entry.message.contains("display edge polyline")),
        "{:?}",
        report.entries
    );
    assert_ne!(report.completeness, Completeness::Complete);
}

/// An edge-only (wire) body must not invent faces: its polylines become real
/// line records and the entity-type change is reported.
#[test]
fn edge_only_body_exports_as_lines_and_invents_no_faces() {
    let geometry = body_geometry(
        BodyKind::Wire,
        Mesh::default(),
        vec![
            vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)],
            vec![p(0.0, 0.0, 0.0), p(0.0, 1.0, 0.0), p(0.0, 0.0, 1.0)],
        ],
        unavailable(),
    );
    let (bytes, report) = export_one(geometry);
    let entities = read_entities(&bytes);
    assert_eq!(entities.len(), 2, "one record per boundary polyline");
    assert!(
        matches!(&entities[0], EntityType::Line(_)),
        "a two-point polyline is a LINE"
    );
    match &entities[1] {
        EntityType::Polyline3D(poly) => assert_eq!(poly.vertices.len(), 3),
        other => panic!("expected POLYLINE3D, got {other:?}"),
    }
    assert!(
        !entities
            .iter()
            .any(|entity| matches!(entity, EntityType::PolyfaceMesh(_))),
        "an edge-only body must not invent faces"
    );
    assert_eq!(report.counts.exact, 0);
    assert_eq!(report.counts.converted, 1);
    assert!(
        report
            .entries
            .iter()
            .any(|entry| entry.code == "export.converted.wire_body"
                && entry.message.contains("not as a 3DSOLID")),
        "{:?}",
        report.entries
    );
    assert!(
        report
            .entries
            .iter()
            .any(|entry| entry.code == "export.converted.wire_body"
                && entry.message.contains("entity type is not preserved")),
        "{:?}",
        report.entries
    );
}

/// A body with nothing drawable is dropped by name, never written as an empty
/// success.
#[test]
fn body_without_display_geometry_is_dropped_explicitly() {
    let geometry = body_geometry(BodyKind::Sheet, Mesh::default(), Vec::new(), unavailable());
    let (bytes, report) = export_one(geometry);
    assert!(read_entities(&bytes).is_empty());
    assert_eq!(report.counts.exact, 0);
    assert_eq!(report.counts.converted, 0);
    assert_eq!(report.counts.dropped, 1);
    assert!(
        report
            .entries
            .iter()
            .any(|entry| entry.code == "export.dropped.body3d"
                && entry
                    .message
                    .contains("no 3DSOLID/BODY/REGION record is fabricated")),
        "{:?}",
        report.entries
    );
}
