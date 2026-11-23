//! Contract tests for real SketchUp edges carried as display geometry
//! (ADR-0049). Every fixture is synthetic; no vendor product was involved.

use super::*;

// ---- Real edges as body display data (ADR-0049) ---------------------------
//
// Every fixture here is synthetic (`skpwright::Model` built in memory, or the
// pinned writer's own bytes). None of it is vendor acceptance: the writer and
// reader are the same crate.

fn square() -> Vec<[f64; 3]> {
    vec![
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ]
}

fn painted_face(outer: Vec<u32>, material: Option<u16>) -> skpwright::MeshFace {
    skpwright::MeshFace {
        outer,
        normal: [0.0, 0.0, 1.0],
        front_material: material,
        ..Default::default()
    }
}

fn mesh_edge(v0: u32, v1: u32) -> skpwright::MeshEdge {
    skpwright::MeshEdge {
        v0,
        v1,
        ..Default::default()
    }
}

fn scene_model(
    vertices: Vec<[f64; 3]>,
    faces: Vec<skpwright::MeshFace>,
    edges: Vec<skpwright::MeshEdge>,
) -> skpwright::Model {
    skpwright::Model {
        geometry: vec![skpwright::GeometryRun {
            def_index: None,
            mesh: skpwright::Mesh {
                vertices,
                faces,
                edges,
                ..Default::default()
            },
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn read(model: &skpwright::Model) -> (crate::SkpScene, MeshImport) {
    let mut import =
        MeshImport::new(&request(Vec::new()), crate::SKP_MESH_TYPE_KEY).expect("import");
    let scene = read_scene(model, &ImportLimits::default(), &|| false, &mut import)
        .expect("the synthetic model walks");
    (scene, import)
}

/// The endpoints of every stored segment, as unordered world pairs.
fn segment_pairs(edges: &[Vec<cad_domain::Point3>]) -> Vec<((u64, u64), (u64, u64))> {
    let key = |p: &cad_domain::Point3| (p.x.to_bits(), p.y.to_bits());
    edges
        .iter()
        .map(|segment| {
            let a = key(&segment[0]);
            let b = key(&segment[1]);
            if a <= b {
                (a, b)
            } else {
                (b, a)
            }
        })
        .collect()
}

#[test]
fn a_quad_with_its_four_ring_edges_becomes_a_body_with_four_edges() {
    let model = scene_model(
        square(),
        vec![painted_face(vec![0, 1, 2, 3], None)],
        vec![
            mesh_edge(0, 1),
            mesh_edge(1, 2),
            mesh_edge(2, 3),
            mesh_edge(3, 0),
        ],
    );
    let (scene, _) = read(&model);

    assert_eq!(scene.placed.len(), 1);
    assert!(
        scene.edges.is_empty(),
        "a face-backed edge is not a standalone wire"
    );
    let placed = &scene.placed[0];
    assert_eq!(
        placed.mesh.triangles.len(),
        2,
        "the quad still triangulates"
    );
    assert_eq!(placed.edges.len(), 4, "one stored segment per ring edge");
    // The stored set is the ring's own sides. A triangulation diagonal would be
    // sqrt(2) long, and none is present.
    for segment in &placed.edges {
        let dx = segment[1].x - segment[0].x;
        let dy = segment[1].y - segment[0].y;
        let length = (dx * dx + dy * dy).sqrt();
        assert!((length - 1.0).abs() < 1e-9, "unexpected segment {length} m");
    }
    assert_eq!(scene.stats.edges.records, 4);
    assert_eq!(scene.stats.edges.boundary, 4);
    assert_eq!(scene.stats.edges.wire, 0);
    assert_eq!(
        scene.stats.edges.covered_by_body, 4,
        "the group's ring records are stored on the body"
    );
    assert_eq!(
        scene.stats.edges.covered_by_mesh, 0,
        "a body group's records must not be counted as mesh-derived coverage"
    );
    assert_eq!(scene.entity_count(), 1);
}

#[test]
fn a_ring_segment_without_a_record_keeps_the_mesh_carrier_and_is_reported() {
    let model = scene_model(
        square(),
        vec![painted_face(vec![0, 1, 2, 3], None)],
        vec![mesh_edge(0, 1), mesh_edge(1, 2), mesh_edge(2, 3)],
    );
    let (scene, _) = read(&model);

    assert_eq!(scene.placed.len(), 1);
    assert!(
        scene.placed[0].edges.is_empty(),
        "an incomplete record set must not produce a body"
    );
    assert_eq!(scene.placed[0].mesh.triangles.len(), 2, "the surface stays");
    assert_eq!(scene.stats.edges.groups_without_records, 1);
    assert_eq!(scene.stats.edges.unrecorded_boundary_segments, 1);
    assert_eq!(
        scene.stats.edges.covered_by_mesh, 3,
        "the mesh group's derived boundary accounts for its three real records"
    );
    assert!(
        scene.edges.is_empty(),
        "the mesh group's triangulated boundary covers its own ring, so no wire duplicates it"
    );

    // The named code is reported once per document, not silently.
    let mut import =
        MeshImport::new(&request(Vec::new()), crate::SKP_MESH_TYPE_KEY).expect("import");
    crate::record_edges(&mut import, &scene.stats);
    let outcome = import.finish().expect("finish");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == crate::DIAG_GROUPS_WITHOUT_EDGE_RECORDS));
}

#[test]
fn only_the_visible_standalone_edge_becomes_a_wire_body() {
    let mut hidden = mesh_edge(1, 2);
    hidden.hidden = true;
    let mut on_invisible_tag = mesh_edge(2, 3);
    on_invisible_tag.layer = 9;
    let mut model = scene_model(
        square(),
        Vec::new(),
        vec![mesh_edge(0, 1), hidden, on_invisible_tag],
    );
    model.layers = vec![skpwright::Layer {
        name: "Hidden".into(),
        visible: false,
        rgba: [0, 0, 0, 255],
    }];
    model.layer_links = vec![(9, 0)];
    let (scene, _) = read(&model);

    assert!(scene.placed.is_empty(), "no faces, no mesh entities");
    assert_eq!(scene.edges.len(), 1, "one wire group");
    assert_eq!(scene.edges[0].edges.len(), 1);
    assert_eq!(scene.edges[0].material, 0);
    assert_eq!(scene.stats.edges.hidden_skipped, 1);
    assert_eq!(scene.stats.edges.invisible_layer_skipped, 1);
    assert_eq!(scene.stats.edges.wire, 1);
    assert_eq!(scene.entity_count(), 1);
}

#[test]
fn a_curve_edge_is_stored_as_its_chord_and_counted() {
    let mut curved = mesh_edge(0, 1);
    curved.curve = Some(0);
    let model = scene_model(square(), Vec::new(), vec![curved]);
    let (scene, _) = read(&model);

    let segment = &scene.edges[0].edges[0];
    assert_eq!(
        segment.len(),
        2,
        "a curve is flattened to its two endpoints"
    );
    assert_eq!((segment[0].x, segment[0].y), (0.0, 0.0));
    assert_eq!((segment[1].x, segment[1].y), (1.0, 0.0));
    assert_eq!(scene.stats.edges.curves_flattened, 1);

    let mut import =
        MeshImport::new(&request(Vec::new()), crate::SKP_MESH_TYPE_KEY).expect("import");
    crate::record_edges(&mut import, &scene.stats);
    let outcome = import.finish().expect("finish");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == crate::DIAG_EDGE_CURVES_FLATTENED));
}

#[test]
fn a_boundary_shared_by_two_material_groups_is_stored_by_both() {
    // Two quads sharing the (1, 2) edge, each painted with its own material.
    let vertices = vec![
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [2.0, 0.0, 0.0],
        [2.0, 1.0, 0.0],
    ];
    let model = scene_model(
        vertices,
        vec![
            painted_face(vec![0, 1, 2, 3], Some(5)),
            painted_face(vec![1, 4, 5, 2], Some(6)),
        ],
        vec![
            mesh_edge(0, 1),
            mesh_edge(1, 2),
            mesh_edge(2, 3),
            mesh_edge(3, 0),
            mesh_edge(1, 4),
            mesh_edge(4, 5),
            mesh_edge(5, 2),
        ],
    );
    let (scene, _) = read(&model);

    assert_eq!(scene.placed.len(), 2, "one group per material");
    assert_eq!(scene.stats.edges.boundary, 8, "four segments per group");
    let shared = (
        (1.0f64.to_bits(), 0.0f64.to_bits()),
        (1.0f64.to_bits(), 1.0f64.to_bits()),
    );
    for placed in &scene.placed {
        assert_eq!(placed.edges.len(), 4);
        assert!(
            segment_pairs(&placed.edges).contains(&shared),
            "the shared boundary is stored by each group (display data, no cross-group identity)"
        );
    }
    assert!(
        scene.edges.is_empty(),
        "every record is a face boundary, so nothing is a wire"
    );
}

#[test]
fn an_edge_record_on_a_failed_face_becomes_a_wire_group() {
    // The face is degenerate (collinear ring) so it emits no triangles; its real
    // edges are not part of any visible face boundary and stay visible.
    let vertices = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
    let model = scene_model(
        vertices,
        vec![painted_face(vec![0, 1, 2], None)],
        vec![mesh_edge(0, 1), mesh_edge(1, 2), mesh_edge(2, 0)],
    );
    let (scene, _) = read(&model);

    assert!(scene.placed.is_empty(), "the degenerate face emits no mesh");
    assert_eq!(scene.edges.len(), 1, "its edges are still visible geometry");
    assert_eq!(scene.edges[0].edges.len(), 3);
    assert_eq!(scene.stats.edges.wire, 3);
}

#[test]
fn a_wire_group_is_coloured_by_its_own_edge_material() {
    let mut painted = mesh_edge(0, 1);
    painted.material = Some(5);
    let mut model = scene_model(square(), Vec::new(), vec![painted, mesh_edge(2, 3)]);
    model.materials = vec![skpwright::Material::Solid {
        name: "Red".into(),
        rgba: [255, 0, 0, 255],
        opacity: 1.0,
        colorize_type: 0,
    }];
    model.material_links = vec![(5, 0)];
    let (scene, _) = read(&model);

    assert_eq!(
        scene.edges.len(),
        2,
        "one wire group per effective material"
    );
    let red = scene
        .edges
        .iter()
        .find(|group| group.material == 5)
        .expect("the painted group");
    assert_eq!(red.color, Some([255, 0, 0]));
    let by_layer = scene
        .edges
        .iter()
        .find(|group| group.material == 0)
        .expect("the unpainted group");
    assert_eq!(by_layer.color, None, "no material, no invented colour");
}

/// The 2026 writer needs an edge per face-ring segment, so a written quad always
/// satisfies ADR-0049's carrier rule; the read-back is a display body.
#[test]
fn a_written_quad_imports_as_a_display_body_with_its_four_edges() {
    let model = written_model(
        skpwright::Material::Solid {
            name: "Plain".into(),
            rgba: [30, 60, 90, 255],
            opacity: 1.0,
            colorize_type: 0,
        },
        None,
    );
    let bytes = write_and_reread(&model);
    let drawing = SkpImporter::new()
        .import(&request(bytes), &|| false)
        .expect("the written quad imports");

    assert_eq!(drawing.database.entities().count(), 1);
    let entity = drawing.database.entities().next().expect("the entity");
    assert_eq!(entity.object.type_key, crate::SKP_MESH_TYPE_KEY);
    let SemanticGeometry::Body3d(body) = &entity.geometry else {
        panic!("a complete record set makes the group a display body");
    };
    assert_eq!(body.kind, cad_domain::BodyKind::Unknown);
    assert!(matches!(
        body.exact,
        cad_domain::BodyExact::Unavailable { .. }
    ));
    assert_eq!(body.display.precision, cad_domain::Precision::Unknown);
    assert_eq!(body.display.tolerance, None);
    assert!(matches!(
        body.display.completeness,
        Completeness::Partial(_)
    ));
    assert_eq!(body.display.mesh.triangles.len(), 2);
    assert_eq!(body.display.edges.len(), 4, "the quad's four real edges");
    assert!(drawing
        .report
        .capabilities
        .iter()
        .any(|capability| capability.type_key == crate::SKP_MESH_TYPE_KEY));
}

fn written_edges_only() -> skpwright::Model {
    skpwright::Model {
        geometry: vec![skpwright::GeometryRun {
            def_index: None,
            mesh: skpwright::Mesh {
                vertex_pids: vec![0; 4],
                vertices: square(),
                faces: Vec::new(),
                edges: vec![mesh_edge(0, 1), mesh_edge(2, 3)],
                vertices_in: Vec::new(),
            },
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn an_edge_only_written_model_imports_as_wire_bodies() {
    let bytes = write_and_reread(&written_edges_only());
    let drawing = SkpImporter::new()
        .import(&request(bytes), &|| false)
        .expect("an edge-only file imports as wire bodies, not a refusal");

    let mut types = Vec::new();
    let mut wire_bodies = 0;
    for entity in drawing.database.entities() {
        types.push(entity.object.type_key.clone());
        let SemanticGeometry::Body3d(body) = &entity.geometry else {
            panic!("an edge-only import yields display bodies");
        };
        assert_eq!(body.kind, cad_domain::BodyKind::Wire);
        assert!(body.display.mesh.triangles.is_empty());
        assert!(!body.display.edges.is_empty());
        wire_bodies += 1;
    }
    assert_eq!(wire_bodies, 1, "both loose edges share material slot 0");
    assert_eq!(types, vec![crate::SKP_EDGE_TYPE_KEY.to_string()]);
    // An edge-only import inserts no face group, so it must not report the mesh
    // carrier it never used: exactly one capability row, the wire body's.
    let capability_keys: Vec<&str> = drawing
        .report
        .capabilities
        .iter()
        .map(|capability| capability.type_key.as_str())
        .collect();
    assert_eq!(
        capability_keys,
        vec![crate::SKP_EDGE_TYPE_KEY],
        "an edge-only SKP reports only the edge carrier"
    );
    for capability in &drawing.report.capabilities {
        assert_eq!(
            capability.measure,
            SupportStatus::Partial,
            "stored visible edges are an endpoint/midpoint MEASURE source"
        );
    }

    let codes = codes_of(&drawing);
    assert!(
        codes.contains(&crate::DIAG_EDGES_ONLY),
        "an edge-only file is reported as such; got {codes:?}"
    );
    assert!(!codes.contains(&crate::NO_GEOMETRY));
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
}

fn request_with_limits(bytes: Vec<u8>, limits: ImportLimits) -> cad_connector_core::ImportRequest {
    cad_connector_core::ImportRequest {
        limits,
        ..request(bytes)
    }
}

#[test]
fn the_edge_segment_budget_refuses_by_name_and_writes_nothing() {
    let bytes = write_and_reread(&written_edges_only());
    let error = SkpImporter::new()
        .import(
            &request_with_limits(
                bytes,
                ImportLimits {
                    max_mesh_edges: Some(1),
                    ..ImportLimits::default()
                },
            ),
            &|| false,
        )
        .err()
        .expect("two segments over a one-segment budget is a refusal");
    match error {
        cad_domain::CadError::InvalidInput(message) => {
            assert!(
                message.contains(crate::LIMITS_EXCEEDED),
                "the refusal carries the stable code: {message}"
            );
            assert!(
                message.contains('2'),
                "the refusal carries the real count: {message}"
            );
        }
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

/// Complete records whose every boundary edge was pruned must not become an
/// edge-less body: the group keeps the mesh carrier, and the fallback is named.
#[test]
fn a_group_whose_boundary_records_are_all_hidden_falls_back_to_a_mesh_and_is_reported() {
    let hidden = |v0: u32, v1: u32| {
        let mut edge = mesh_edge(v0, v1);
        edge.hidden = true;
        edge
    };
    let model = scene_model(
        square(),
        vec![painted_face(vec![0, 1, 2, 3], None)],
        vec![hidden(0, 1), hidden(1, 2), hidden(2, 3), hidden(3, 0)],
    );
    let (scene, mut import) = read(&model);

    assert_eq!(scene.placed.len(), 1);
    assert!(
        scene.placed[0].edges.is_empty(),
        "an all-hidden boundary must not produce a body edge set"
    );
    assert_eq!(scene.stats.edges.groups_empty_boundary, 1);
    assert_eq!(scene.stats.edges.hidden_skipped, 4);
    assert_eq!(
        scene.stats.edges.covered_by_body, 0,
        "nothing was stored on a body"
    );
    assert_eq!(
        scene.stats.edges.covered_by_mesh, 0,
        "the rings are not counted as mesh-derived coverage either"
    );

    crate::record_edges(&mut import, &scene.stats);
    let object = import.reserve_object().expect("reserve an object");
    let placed = scene.placed.into_iter().next().expect("the group");
    import
        .push_mesh(object, placed.mesh, cad_db::EntityColor::ByLayer)
        .expect("a mesh carrier is inserted");
    let outcome = import.finish().expect("finish");

    let entity = outcome.database.entities().next().expect("the group row");
    assert!(
        matches!(entity.geometry, SemanticGeometry::Mesh(_)),
        "the fallback carrier is the plain mesh, never an edge-less body"
    );
    let diagnostic = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == crate::DIAG_EDGES_HIDDEN_FALLBACK)
        .expect("the fallback is named, not silent");
    assert!(diagnostic.message.contains('1'), "{}", diagnostic.message);
    assert!(matches!(
        outcome.completeness,
        Completeness::Partial(ref reasons)
            if reasons.iter().any(|reason| reason.contains("fell back"))
    ));
}

/// The flags note is about flags that actually occurred, not about records
/// existing: a plain quad must not claim unmodelled soft/smooth/shadow semantics.
#[test]
fn the_edge_flag_note_is_absent_for_plain_records_and_present_when_a_flag_occurred() {
    let plain = scene_model(square(), Vec::new(), vec![mesh_edge(0, 1)]);
    let (scene, mut import) = read(&plain);
    assert_eq!(scene.stats.edges.flag_records, 0);
    crate::record_edges(&mut import, &scene.stats);
    let outcome = import.finish().expect("finish");
    assert!(
        outcome
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != crate::DIAG_EDGE_FLAGS_NOT_MODELED),
        "no flag occurred, so no flag note"
    );

    let mut soft = mesh_edge(0, 1);
    soft.soft = true;
    let flagged = scene_model(square(), Vec::new(), vec![soft]);
    let (scene, mut import) = read(&flagged);
    assert_eq!(scene.stats.edges.flag_records, 1);
    crate::record_edges(&mut import, &scene.stats);
    let outcome = import.finish().expect("finish");
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == crate::DIAG_EDGE_FLAGS_NOT_MODELED),
        "a real soft flag is reported"
    );
}

/// A group with one good face and one face whose triangulation fails still has a
/// real body: the failed face's own ring records stay on that body, so they are
/// neither duplicated as a `SkpPreviewEdges` wire group nor dropped.
#[test]
fn a_body_keeps_a_failed_faces_real_ring_records_without_a_wire_group() {
    // The quad is the successful face; the collinear triple is a face that
    // cannot triangulate. Their vertices are disjoint so no ring segment is
    // shared between the two faces.
    let vertices = vec![
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [3.0, 0.0, 0.0],
        [4.0, 0.0, 0.0],
        [5.0, 0.0, 0.0],
    ];
    let model = scene_model(
        vertices,
        vec![
            painted_face(vec![0, 1, 2, 3], None),
            painted_face(vec![4, 5, 6], None),
        ],
        vec![
            mesh_edge(0, 1),
            mesh_edge(1, 2),
            mesh_edge(2, 3),
            mesh_edge(3, 0),
            mesh_edge(4, 5),
            mesh_edge(5, 6),
            mesh_edge(6, 4),
        ],
    );
    let (scene, _) = read(&model);

    assert_eq!(
        scene.stats.faces_untriangulable, 1,
        "the collinear face really failed to triangulate"
    );
    assert_eq!(scene.placed.len(), 1, "both faces share material slot 0");
    let placed = &scene.placed[0];
    assert_eq!(
        placed.mesh.triangles.len(),
        2,
        "only the quad has triangles"
    );
    assert_eq!(
        placed.edges.len(),
        7,
        "the body stores both faces' complete ring records"
    );
    assert!(!placed.edges.is_empty(), "the body carrier is non-empty");

    let world_pair = |a: [f64; 3], b: [f64; 3]| {
        let key = |p: [f64; 3]| (p[0].to_bits(), p[1].to_bits());
        let (a, b) = (key(a), key(b));
        if a <= b {
            (a, b)
        } else {
            (b, a)
        }
    };
    let stored = segment_pairs(&placed.edges);
    let expected = [
        // The good face's four ring segments.
        world_pair([0.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
        world_pair([1.0, 0.0, 0.0], [1.0, 1.0, 0.0]),
        world_pair([1.0, 1.0, 0.0], [0.0, 1.0, 0.0]),
        world_pair([0.0, 1.0, 0.0], [0.0, 0.0, 0.0]),
        // The failed face's three real ring segments.
        world_pair([3.0, 0.0, 0.0], [4.0, 0.0, 0.0]),
        world_pair([4.0, 0.0, 0.0], [5.0, 0.0, 0.0]),
        world_pair([5.0, 0.0, 0.0], [3.0, 0.0, 0.0]),
    ];
    for pair in expected {
        assert!(
            stored.contains(&pair),
            "the stored boundary must keep segment {pair:?}; got {stored:?}"
        );
    }

    assert!(
        scene.edges.is_empty(),
        "the failed face's records stayed on the body, so no SkpPreviewEdges group"
    );
    assert_eq!(scene.stats.edges.records, 7);
    assert_eq!(
        scene.stats.edges.boundary, 7,
        "every distinct ring segment is stored as a body boundary segment"
    );
    assert_eq!(
        scene.stats.edges.wire, 0,
        "nothing became a standalone wire"
    );
    assert_eq!(scene.stats.edges.covered_by_body, 7);
    assert_eq!(scene.entity_count(), 1);
}

/// The note is not only about flags: a record dropped as malformed (zero-length
/// or an unresolvable endpoint index) is named here too, so it is not only
/// summarized in the document totals.
#[test]
fn the_edge_flag_note_also_covers_malformed_records() {
    // Same index twice: the record names a zero-length edge with no flag set.
    let (scene, mut import) = read(&scene_model(square(), Vec::new(), vec![mesh_edge(0, 0)]));
    assert_eq!(scene.stats.edges.records, 1);
    assert_eq!(scene.stats.edges.flag_records, 0, "no flag on this record");
    assert_eq!(scene.stats.edges.zero_length_skipped, 1);
    crate::record_edges(&mut import, &scene.stats);
    let outcome = import.finish().expect("finish");
    let diagnostic = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == crate::DIAG_EDGE_FLAGS_NOT_MODELED)
        .expect("a zero-length record is reported, not silently summarized");
    assert!(
        diagnostic
            .message
            .contains("1 zero-length or collapsed and 0 unresolved-index"),
        "the note names the malformed counts: {}",
        diagnostic.message
    );

    // An endpoint index that names no vertex: also malformed, also no flag.
    let (scene, mut import) = read(&scene_model(square(), Vec::new(), vec![mesh_edge(0, 99)]));
    assert_eq!(scene.stats.edges.records, 1);
    assert_eq!(scene.stats.edges.flag_records, 0);
    assert_eq!(scene.stats.edges.index_skipped, 1);
    crate::record_edges(&mut import, &scene.stats);
    let outcome = import.finish().expect("finish");
    let diagnostic = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == crate::DIAG_EDGE_FLAGS_NOT_MODELED)
        .expect("an unresolved-index record is reported");
    assert!(
        diagnostic
            .message
            .contains("0 zero-length or collapsed and 1 unresolved-index"),
        "the note names the malformed counts: {}",
        diagnostic.message
    );

    // A plain visible record still says nothing.
    let (scene, mut import) = read(&scene_model(square(), Vec::new(), vec![mesh_edge(0, 1)]));
    crate::record_edges(&mut import, &scene.stats);
    let outcome = import.finish().expect("finish");
    assert!(
        outcome
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != crate::DIAG_EDGE_FLAGS_NOT_MODELED),
        "a plain record is not malformed and carries no flag"
    );
}

/// A face group plus one loose stored record: the face keeps its own carrier and
/// the uncovered record becomes a `SkpPreviewEdges` wire body, so both type keys
/// are real insertions and both get a capability row.
#[test]
fn a_mixed_face_and_wire_import_reports_both_carriers() {
    let mut run = written_quad(None, None);
    // The quad's ring edges already cover (0,1), (1,2), (2,3), (3,0); this
    // diagonal is not a ring segment, so it stays visible and uncovered.
    run.mesh.edges.push(mesh_edge(0, 2));
    let model = skpwright::Model {
        geometry: vec![run],
        ..Default::default()
    };
    let bytes = write_and_reread(&model);
    let drawing = SkpImporter::new()
        .import(&request(bytes), &|| false)
        .expect("a face plus one loose edge imports");

    let mut types: Vec<String> = drawing
        .database
        .entities()
        .map(|entity| entity.object.type_key.clone())
        .collect();
    types.sort();
    assert_eq!(
        types,
        vec![
            crate::SKP_EDGE_TYPE_KEY.to_string(),
            crate::SKP_MESH_TYPE_KEY.to_string()
        ],
        "one face carrier and one wire carrier"
    );
    let mut capability_keys: Vec<&str> = drawing
        .report
        .capabilities
        .iter()
        .map(|capability| capability.type_key.as_str())
        .collect();
    capability_keys.sort_unstable();
    assert_eq!(
        capability_keys,
        vec![crate::SKP_EDGE_TYPE_KEY, crate::SKP_MESH_TYPE_KEY],
        "both inserted carriers get exactly one row"
    );
    for capability in &drawing.report.capabilities {
        assert_eq!(
            capability.measure,
            SupportStatus::Partial,
            "both carriers stored visible edges: {}",
            capability.type_key
        );
    }
}

// ------------------------------------------ measure claim / release ceiling ---

use cad_domain::{BodyKind, Mesh, Point3};

fn unit_triangle() -> Mesh {
    Mesh {
        vertices: vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
        ],
        triangles: vec![[0, 1, 2]],
        ..Mesh::default()
    }
}

#[test]
fn a_triangle_only_skp_carrier_stays_unsupported_and_a_ceiling_cannot_force_it() {
    let mut import =
        MeshImport::new(&request(Vec::new()), crate::SKP_MESH_TYPE_KEY).expect("import");
    import.declare_edge_snap_measure(true);
    let object = import.reserve_object().unwrap();
    import
        .push_mesh(object, unit_triangle(), cad_db::EntityColor::ByLayer)
        .unwrap();
    let mut capabilities = import.finish().unwrap().capabilities;
    assert_eq!(capabilities.len(), 1);
    assert_eq!(capabilities[0].measure, SupportStatus::Unsupported);

    // The ceiling narrows, never forces: an Unsupported claim stays Unsupported
    // even when the release is fully verified.
    crate::apply_ceiling(&mut capabilities, SupportStatus::Verified);
    assert_eq!(capabilities[0].measure, SupportStatus::Unsupported);
    assert_eq!(capabilities[0].read, SupportStatus::Verified);
}

#[test]
fn the_release_ceiling_narrows_a_partial_measure_claim() {
    let mut import =
        MeshImport::new(&request(Vec::new()), crate::SKP_EDGE_TYPE_KEY).expect("import");
    import.declare_edge_snap_measure(true);
    let object = import.reserve_object().unwrap();
    import
        .push_body(
            object,
            crate::preview_body(
                Mesh::default(),
                vec![vec![
                    Point3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    Point3 {
                        x: 1.0,
                        y: 0.0,
                        z: 0.0,
                    },
                ]],
                BodyKind::Wire,
            ),
            crate::SKP_EDGE_TYPE_KEY,
        )
        .unwrap();
    let mut capabilities = import.finish().unwrap().capabilities;
    assert_eq!(capabilities[0].measure, SupportStatus::Partial);

    // 2021–2025 releases are read by container equivalence only: the Partial
    // measure claim narrows to Unverified rather than being forced either way.
    crate::apply_ceiling(&mut capabilities, SupportStatus::Unverified);
    assert_eq!(capabilities[0].measure, SupportStatus::Unverified);
    assert_eq!(capabilities[0].read, SupportStatus::Unverified);
}
