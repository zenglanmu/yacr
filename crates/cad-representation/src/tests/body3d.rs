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
        uvs: Vec::new(),
        texture: None,
        pbr: None,
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
    expanded_body_with(entity, &context())
}

fn expanded_body_with(entity: DbEntity, context: &RepresentationContext) -> DisplayRepresentation {
    let id = entity.id;
    let db = geometry_db(entity);
    ProviderRegistry::with_default_provider()
        .build_expanded(&db, db.entity(id).expect("inserted body"), context)
        .unwrap()
}

#[test]
fn exact_sheet_body_mesh_keeps_kernel_source_and_body_precision() {
    let display = BodyDisplay {
        mesh: plane_mesh(),
        // One stored 2-point polyline = one independent endpoint pair. It is the
        // body's wireframe source, so the fragment carries it beside the mesh.
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
        DisplayPrimitive::MeshWithStoredEdges { mesh, edges, .. } => {
            assert_eq!(mesh.triangles.len(), 2);
            assert_eq!(edges.len(), 2, "one stored pair, not per-triangle edges");
            assert_eq!(edges[0], p(0.0, 0.0));
            assert_eq!(edges[1], p(1.0, 0.0));
        }
        _ => panic!("a body with a display mesh and stored edges carries both"),
    }
}

#[test]
fn body_stored_edges_are_flattened_to_independent_pairs() {
    let display = BodyDisplay {
        mesh: plane_mesh(),
        edges: vec![
            vec![p(0.0, 0.0), p(1.0, 0.0)],
            vec![p3(0.0, 0.0, 1.0), p3(1.0, 0.0, 1.0), p3(1.0, 1.0, 1.0)],
        ],
        tolerance: None,
        precision: Precision::Analytic,
        completeness: Completeness::Complete,
    };
    let rep = expanded_body(body_entity(
        94,
        BodyKind::Sheet,
        BodyExact::Exact(Box::new(square_brep(false))),
        display,
    ));
    assert_eq!(rep.fragments.len(), 1);
    match &rep.fragments[0].primitive {
        DisplayPrimitive::MeshWithStoredEdges { edges, .. } => {
            // 1 segment + 2 segments = 3 independent pairs = 6 endpoints.
            assert_eq!(edges.len(), 6);
            assert_eq!(edges[0], p(0.0, 0.0));
            assert_eq!(edges[1], p(1.0, 0.0));
            assert_eq!(edges[2], p3(0.0, 0.0, 1.0));
            assert_eq!(edges[3], p3(1.0, 0.0, 1.0));
            assert_eq!(edges[4], p3(1.0, 0.0, 1.0));
            assert_eq!(edges[5], p3(1.0, 1.0, 1.0));
        }
        _ => panic!("stored edges must travel with the mesh"),
    }
}

#[test]
fn body_mesh_without_stored_edges_keeps_the_plain_mesh_primitive() {
    let display = BodyDisplay {
        mesh: plane_mesh(),
        edges: Vec::new(),
        tolerance: None,
        precision: Precision::Analytic,
        completeness: Completeness::Complete,
    };
    let rep = expanded_body(body_entity(
        95,
        BodyKind::Sheet,
        BodyExact::Exact(Box::new(square_brep(false))),
        display,
    ));
    assert_eq!(rep.fragments.len(), 1);
    assert!(
        matches!(&rep.fragments[0].primitive, DisplayPrimitive::Mesh(_)),
        "no stored edges keeps the legacy plain Mesh (triangle-derived overlay)"
    );
}

#[test]
fn body_wireframe_emits_the_stored_edges_as_segments_only() {
    // `VSCURRENT WIREFRAME`: a body with stored edges draws exactly those pairs;
    // no mesh fragment is emitted at all.
    let display = BodyDisplay {
        mesh: plane_mesh(),
        edges: vec![
            vec![p(0.0, 0.0), p(1.0, 0.0)],
            vec![p3(0.0, 0.0, 1.0), p3(1.0, 0.0, 1.0), p3(1.0, 1.0, 1.0)],
        ],
        tolerance: None,
        precision: Precision::Analytic,
        completeness: Completeness::Complete,
    };
    let rep = expanded_body_with(
        body_entity(
            98,
            BodyKind::Sheet,
            BodyExact::Exact(Box::new(square_brep(false))),
            display,
        ),
        &context().with_body_wireframe(true),
    );
    assert_eq!(rep.fragments.len(), 1);
    assert_eq!(rep.completeness, Completeness::Complete);
    assert_eq!(
        rep.fragments[0].geometry_source,
        GeometrySource::KernelMesh,
        "wireframe does not change the geometry source"
    );
    match &rep.fragments[0].primitive {
        DisplayPrimitive::LineSegments(points) => {
            assert_eq!(points.len(), 6, "1 + 2 stored segments");
            assert_eq!(points[0], p(0.0, 0.0));
            assert_eq!(points[1], p(1.0, 0.0));
            assert_eq!(points[2], p3(0.0, 0.0, 1.0));
            assert_eq!(points[3], p3(1.0, 0.0, 1.0));
            assert_eq!(points[4], p3(1.0, 0.0, 1.0));
            assert_eq!(points[5], p3(1.0, 1.0, 1.0));
        }
        _ => panic!("WIREFRAME must emit LineSegments"),
    }
    assert!(
        !rep.fragments
            .iter()
            .any(|f| matches!(&f.primitive, DisplayPrimitive::Mesh(_))),
        "no mesh fragment travels with a wireframe body"
    );
}

#[test]
fn body_wireframe_without_stored_edges_derives_unique_triangle_segments() {
    // A sphere/torus full primitive has triangles but no stored boundary edges.
    // WIREFRAME derives the triangle edges, de-duplicated by unordered pair, so
    // the body is neither invisible nor drawn twice on its shared diagonal.
    let display = BodyDisplay {
        mesh: plane_mesh(),
        edges: Vec::new(),
        tolerance: None,
        precision: Precision::Analytic,
        completeness: Completeness::Complete,
    };
    let rep = expanded_body_with(
        body_entity(
            99,
            BodyKind::Sheet,
            BodyExact::Exact(Box::new(square_brep(false))),
            display,
        ),
        &context().with_body_wireframe(true),
    );
    assert_eq!(rep.fragments.len(), 1);
    let points = match &rep.fragments[0].primitive {
        DisplayPrimitive::LineSegments(points) => points.to_vec(),
        _ => panic!("WIREFRAME must emit LineSegments"),
    };
    // A quad: four boundary edges + one loaded diagonal = five unique segments.
    // The second triangle's (0,2) edge is the first triangle's (2,0) edge.
    assert_eq!(points.len(), 10, "five unique unordered pairs");
    let mut pairs = std::collections::HashSet::new();
    for pair in points.as_chunks::<2>().0 {
        let key = |point: Point3| [point.x.to_bits(), point.y.to_bits(), point.z.to_bits()];
        let (a, b) = (key(pair[0]), key(pair[1]));
        let unordered = if a <= b { [a, b] } else { [b, a] };
        assert!(pairs.insert(unordered), "duplicate segment {pair:?}");
    }
    assert_eq!(pairs.len(), 5);
}

#[test]
fn body_wireframe_off_keeps_the_mesh_with_stored_edges() {
    // The default (`SHADED`) path is untouched: the same cache still carries its
    // filled mesh plus the stored-edge overlay.
    let display = BodyDisplay {
        mesh: plane_mesh(),
        edges: vec![vec![p(0.0, 0.0), p(1.0, 0.0)]],
        tolerance: None,
        precision: Precision::Analytic,
        completeness: Completeness::Complete,
    };
    let rep = expanded_body_with(
        body_entity(
            100,
            BodyKind::Sheet,
            BodyExact::Exact(Box::new(square_brep(false))),
            display,
        ),
        &context().with_body_wireframe(false),
    );
    assert_eq!(rep.fragments.len(), 1);
    match &rep.fragments[0].primitive {
        DisplayPrimitive::MeshWithStoredEdges { mesh, edges, .. } => {
            assert_eq!(mesh.triangles.len(), 2);
            assert_eq!(edges.len(), 2);
        }
        _ => panic!("SHADED keeps MeshWithStoredEdges"),
    }
}

#[test]
fn body_wireframe_leaves_a_wire_body_as_its_line_segments() {
    // A wire body (no mesh) already draws as `LineSegments`; the visual style
    // must not change that or invent a mesh.
    let display = BodyDisplay {
        mesh: Mesh::default(),
        edges: vec![vec![p(0.0, 0.0), p(1.0, 0.0)]],
        tolerance: None,
        precision: Precision::Analytic,
        completeness: Completeness::Partial(vec!["wire body has no exact brep".into()]),
    };
    let rep = expanded_body_with(
        body_entity(
            101,
            BodyKind::Wire,
            BodyExact::Unavailable {
                reason: "wire".into(),
            },
            display,
        ),
        &context().with_body_wireframe(true),
    );
    assert_eq!(rep.fragments.len(), 1);
    assert_eq!(
        rep.completeness,
        Completeness::Partial(vec!["wire body has no exact brep".into()]),
        "completeness is still the body's own"
    );
    match &rep.fragments[0].primitive {
        DisplayPrimitive::LineSegments(points) => assert_eq!(points.len(), 2),
        _ => panic!("a wire body stays LineSegments"),
    }
}

#[test]
fn transforming_a_body_primitive_moves_its_stored_edges() {
    let primitive = DisplayPrimitive::MeshWithStoredEdges {
        mesh: std::sync::Arc::new(plane_mesh()),
        edges: std::sync::Arc::from(vec![p(0.0, 0.0), p(1.0, 0.0)].into_boxed_slice()),
        derive_when_empty: true,
    };
    let moved = primitive.transformed(&Transform3::translation(p(10.0, 5.0)));
    match moved {
        DisplayPrimitive::MeshWithStoredEdges { mesh, edges, .. } => {
            assert_eq!(edges[0], p(10.0, 5.0));
            assert_eq!(edges[1], p(11.0, 5.0));
            assert_eq!(mesh.vertices[0], p(10.0, 5.0));
        }
        _ => panic!("transforming a body primitive keeps the variant"),
    }
}

/// Expand a model INSERT of block 1 whose single member is a body carrying
/// `display`, under the given block clip.
fn clipped_body_rep(id: u128, clip: BlockClip, display: BodyDisplay) -> DisplayRepresentation {
    let member = body_entity(
        id,
        BodyKind::Sheet,
        BodyExact::Exact(Box::new(square_brep(false))),
        display,
    );
    let member_id = member.id;
    let mut b = empty_db();
    b.insert_block(BlockDefinition {
        id: BlockId(1),
        entities: vec![member_id],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(member).unwrap();
    let mut insert = insert_entity(1, SpaceId::Model, 1, 0.0);
    insert.geometry = SemanticGeometry::Insert {
        block: BlockId(1),
        transform: Transform3::identity(),
        attributes: Vec::new(),
    };
    b.insert_entity(insert).unwrap();
    b.set_entity_render_attributes(
        EntityId(1),
        EntityRenderAttributes {
            clip: Some(clip),
            ..EntityRenderAttributes::default()
        },
    )
    .unwrap();
    let db = b.finish().unwrap();
    ProviderRegistry::with_default_provider()
        .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap()
}

fn body_with_edges(edges: Vec<Vec<Point3>>) -> BodyDisplay {
    BodyDisplay {
        mesh: plane_mesh(),
        edges,
        tolerance: None,
        precision: Precision::Analytic,
        completeness: Completeness::Complete,
    }
}

#[test]
fn fully_inside_clipped_body_keeps_its_stored_edges_and_reports_nothing() {
    // The clip region strictly contains the body's mesh and every stored edge
    // endpoint, so no cut can change it: the variant (and its stored edges) must
    // survive with no degradation diagnostic.
    let rep = clipped_body_rep(
        97,
        BlockClip::new(vec![[-1.0, -1.0], [2.0, 2.0]], true, true),
        body_with_edges(vec![vec![p(0.0, 0.0), p(1.0, 0.0)]]),
    );
    assert_eq!(rep.fragments.len(), 1);
    match &rep.fragments[0].primitive {
        DisplayPrimitive::MeshWithStoredEdges { edges, .. } => {
            assert_eq!(edges.len(), 2, "the stored edge pair survives uncut");
        }
        _ => panic!("a body wholly inside the clip keeps MeshWithStoredEdges"),
    }
    assert!(
        !rep.diagnostics
            .iter()
            .any(|d| d.code == "representation.xclip_body_edges_derived"),
        "an unchanged body must not report a derived-overlay degradation, got {:?}",
        rep.diagnostics
            .iter()
            .map(|d| d.code.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn clipped_body_mesh_falls_back_to_triangle_edges_with_a_named_diagnostic() {
    // A body straddling a clipped block reference: the block clip cuts the mesh
    // only, so the surviving piece is a plain `Mesh` (triangle-derived overlay)
    // and one `representation.xclip_body_edges_derived` diagnostic records the
    // loss.
    let rep = clipped_body_rep(
        96,
        BlockClip::new(vec![[0.0, 0.0], [0.5, 1.0]], true, true),
        body_with_edges(vec![vec![p(0.0, 0.0), p(1.0, 0.0)]]),
    );
    assert!(
        rep.fragments
            .iter()
            .all(|f| matches!(&f.primitive, DisplayPrimitive::Mesh(_))),
        "a clipped body piece is a plain Mesh"
    );
    assert!(
        !rep.fragments.is_empty(),
        "the half of the square inside the clip survives"
    );
    assert_eq!(
        rep.diagnostics
            .iter()
            .filter(|d| d.code == "representation.xclip_body_edges_derived")
            .count(),
        1,
        "one degradation diagnostic per clipped body, got {:?}",
        rep.diagnostics
            .iter()
            .map(|d| d.code.as_str())
            .collect::<Vec<_>>()
    );
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
