//! `SemanticGeometry::Body3d` display mapping.
//!
//! A body is displayed from its derived cache only: a filled mesh when the
//! cache has triangles, otherwise its boundary/feature edges as independent
//! segments. Precision and completeness are the body's own; the geometry source
//! distinguishes an exact body (`KernelMesh`) from a display-only one
//! (`DirectMesh`).

use super::*;

/// A planar square cache: four vertices, two triangles, unit normals.
fn plane_mesh() -> Mesh {
    Mesh {
        vertices: vec![
            p3(0.0, 0.0, 0.0),
            p3(1.0, 0.0, 0.0),
            p3(1.0, 1.0, 0.0),
            p3(0.0, 1.0, 0.0),
        ],
        triangles: vec![[0, 1, 2], [0, 2, 3]],
        normals: vec![p3(0.0, 0.0, 1.0); 4],
        face_sources: Vec::new(),
        colors: Vec::new(),
    }
}

fn body_entity(id: u128, kind: BodyKind, exact: BodyExact, display: BodyDisplay) -> DbEntity {
    let mut entity = entity(
        id,
        SemanticGeometry::Body3d(Box::new(Body3d {
            kind,
            exact,
            display,
        })),
    );
    entity.object.type_key = "AcDb3dSolid".into();
    entity
}

/// Build through the real expansion path, so the entity's default
/// `geometry_source == Analytic` attribute is in play.
fn expanded_body(entity: DbEntity) -> DisplayRepresentation {
    let id = entity.id;
    let db = geometry_db(entity);
    ProviderRegistry::with_default_provider()
        .build_expanded(&db, db.entity(id).expect("inserted body"), &context())
        .unwrap()
}

#[test]
fn exact_sheet_body_mesh_keeps_kernel_source_and_body_precision() {
    let display = BodyDisplay {
        mesh: plane_mesh(),
        // Stored edges are deferred while a display mesh draws (P0.1) but they
        // must not change the filled draw or its source tag.
        edges: vec![vec![p(0.0, 0.0), p(1.0, 0.0)]],
        tolerance: Some(0.01),
        precision: Precision::Approximate {
            error_bound: Some(0.01),
        },
        completeness: Completeness::Complete,
    };
    let rep = expanded_body(body_entity(
        90,
        BodyKind::Sheet,
        BodyExact::Exact(Box::new(square_brep(false))),
        display,
    ));
    assert_eq!(rep.fragments.len(), 1);
    assert_eq!(
        rep.fragments[0].geometry_source,
        GeometrySource::KernelMesh,
        "an exact body's cache was derived from the kernel"
    );
    assert_eq!(
        rep.fragments[0].precision,
        Precision::Approximate {
            error_bound: Some(0.01)
        },
        "precision is the body display's own, not an assumed Analytic"
    );
    assert_eq!(rep.completeness, Completeness::Complete);
    match &rep.fragments[0].primitive {
        DisplayPrimitive::Mesh(mesh) => assert_eq!(mesh.triangles.len(), 2),
        _ => panic!("a body with a display mesh is a filled Mesh primitive"),
    }
}

#[test]
fn wire_body_draws_its_edges_as_segments_without_a_mesh() {
    let display = BodyDisplay {
        mesh: Mesh::default(),
        edges: vec![
            vec![p(0.0, 0.0), p(1.0, 0.0)],
            vec![p3(0.0, 0.0, 1.0), p3(1.0, 0.0, 1.0), p3(1.0, 1.0, 1.0)],
        ],
        tolerance: None,
        precision: Precision::Analytic,
        completeness: Completeness::Partial(vec!["wire body has no exact brep".into()]),
    };
    let rep = expanded_body(body_entity(
        91,
        BodyKind::Wire,
        BodyExact::Unavailable {
            reason: "wire".into(),
        },
        display,
    ));
    assert_eq!(rep.fragments.len(), 1);
    assert_eq!(rep.fragments[0].geometry_source, GeometrySource::DirectMesh);
    assert_eq!(
        rep.completeness,
        Completeness::Partial(vec!["wire body has no exact brep".into()]),
        "the body's own completeness is carried, not upgraded"
    );
    match &rep.fragments[0].primitive {
        DisplayPrimitive::LineSegments(points) => {
            // One segment (2 points) plus two segments (3 points): every pair is
            // an explicit endpoint pair, never a joined polyline.
            assert_eq!(points.len(), 6);
            assert_eq!(points[0], p(0.0, 0.0));
            assert_eq!(points[1], p(1.0, 0.0));
        }
        _ => panic!("an edge-only body draws as LineSegments"),
    }
    assert!(!rep
        .fragments
        .iter()
        .any(|f| matches!(&f.primitive, DisplayPrimitive::Mesh(_))));
}

#[test]
fn body_with_neither_mesh_nor_edges_is_missing_not_an_empty_success() {
    // Built through the provider directly: `cad-db` rejects a <2-point edge
    // polyline on write, but the provider still refuses to invent a vertex for
    // one it is handed.
    let entity = body_entity(
        92,
        BodyKind::Unknown,
        BodyExact::Unavailable {
            reason: "unparsed payload".into(),
        },
        BodyDisplay {
            mesh: Mesh::default(),
            edges: vec![vec![p(0.0, 0.0)]],
            tolerance: None,
            precision: Precision::Unknown,
            completeness: Completeness::Complete,
        },
    );
    let rep = DefaultRepresentationProvider
        .build(&entity, &context())
        .unwrap();
    assert!(rep.fragments.is_empty(), "no geometry is fabricated");
    assert!(
        matches!(rep.completeness, Completeness::Missing(_)),
        "an empty body cache is missing geometry, got {:?}",
        rep.completeness
    );
    assert!(rep
        .diagnostics
        .iter()
        .any(|d| d.code == "representation.body_empty"));
}

#[test]
fn unavailable_exact_body_reports_direct_mesh_and_keeps_its_completeness() {
    let display = BodyDisplay {
        mesh: plane_mesh(),
        edges: Vec::new(),
        tolerance: Some(0.05),
        precision: Precision::Approximate {
            error_bound: Some(0.05),
        },
        completeness: Completeness::Partial(vec!["kernel tessellation was degraded".into()]),
    };
    let rep = expanded_body(body_entity(
        93,
        BodyKind::Sheet,
        BodyExact::Unavailable {
            reason: "no kernel result".into(),
        },
        display,
    ));
    assert_eq!(rep.fragments.len(), 1);
    assert_eq!(
        rep.fragments[0].geometry_source,
        GeometrySource::DirectMesh,
        "a display-only body is not kernel-derived"
    );
    assert_eq!(
        rep.fragments[0].precision,
        Precision::Approximate {
            error_bound: Some(0.05)
        }
    );
    assert_eq!(
        rep.completeness,
        Completeness::Partial(vec!["kernel tessellation was degraded".into()]),
        "completeness is never upgraded to Complete"
    );
}
