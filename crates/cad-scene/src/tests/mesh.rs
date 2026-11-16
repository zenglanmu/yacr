//! Mesh and line batch construction and topology.

use super::*;

#[test]
fn mesh_batch_is_indexed_and_carries_normals_and_edges() {
    let mut cache = SceneCache::default();
    let delta = cache
        .build(&mesh_representation(1, quad()), stamp())
        .unwrap();
    assert_eq!(delta.added.len(), 1);
    let batch = &delta.added[0];
    assert_eq!(batch.topology, RenderTopology::Mesh);
    // Indexed: one vertex per mesh vertex, not per triangle corner.
    assert_eq!(batch.vertices.len(), 4);
    assert_eq!(batch.normals.len(), 4);
    assert_eq!(batch.indices, vec![[0, 1, 2], [0, 2, 3]]);
    assert_eq!(batch.triangle_count(), 2);
    // Two triangles => six directed edges, de-duplicated to the five unique
    // undirected segments of a quad (the diagonal is shared) => ten positions.
    assert_eq!(batch.edges.len(), 10);
}

#[test]
fn filled_mesh_can_disable_its_edge_overlay_without_changing_triangle_topology() {
    let mut representation = mesh_representation(2, quad());
    representation.fragments[0].primitive =
        DisplayPrimitive::MeshWithoutEdgeOverlay(std::sync::Arc::new(quad()));
    let delta = SceneCache::default()
        .build(&representation, stamp())
        .unwrap();
    let batch = &delta.added[0];
    assert_eq!(batch.indices, vec![[0, 1, 2], [0, 2, 3]]);
    assert!(
        batch.edges.is_empty(),
        "disabled mesh overlay must not allocate edge geometry"
    );
    assert_eq!(batch.upload_size_bytes(), 4 * 12 * 3 + 2 * 12);
}

#[test]
fn text_glyph_class_is_an_indexed_mesh_without_edge_overlay() {
    // The glyph primitive (no per-edge overlay) is the text display class.
    let mut glyph_rep = mesh_representation(3, quad());
    glyph_rep.fragments[0].primitive = DisplayPrimitive::MeshWithoutEdgeOverlay(Arc::new(quad()));
    let glyph_delta = SceneCache::default().build(&glyph_rep, stamp()).unwrap();
    let glyph = &glyph_delta.added[0];
    assert!(
        glyph.is_text_glyph(),
        "indexed mesh with no edges is a glyph"
    );

    // A normal mesh carries the wireframe overlay: not text.
    let normal_delta = SceneCache::default()
        .build(&mesh_representation(4, quad()), stamp())
        .unwrap();
    let normal = &normal_delta.added[0];
    assert!(!normal.edges.is_empty());
    assert!(
        !normal.is_text_glyph(),
        "a mesh with edge overlay is not text"
    );

    // A zero-triangle mesh has no indices (and no edges): not text.
    let mut empty_mesh = quad();
    empty_mesh.triangles.clear();
    let empty_delta = SceneCache::default()
        .build(&mesh_representation(5, empty_mesh), stamp())
        .unwrap();
    let empty = &empty_delta.added[0];
    assert!(empty.indices.is_empty());
    assert!(!empty.is_text_glyph(), "a zero-triangle mesh is not text");
}

#[test]
fn body_mesh_batch_uses_its_stored_edges_not_the_triangle_derived_ones() {
    // A `Body3d` with a display mesh and stored edges arrives as
    // `DisplayPrimitive::MeshWithStoredEdges`. Its batch must carry the stored
    // endpoint pairs, never the twelve triangle-derived positions of the quad.
    let mesh = quad();
    let stored = vec![
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
    ];
    let mut representation = mesh_representation(20, mesh.clone());
    representation.fragments[0].primitive = DisplayPrimitive::MeshWithStoredEdges {
        mesh: Arc::new(mesh),
        edges: Arc::from(stored.into_boxed_slice()),
    };
    let delta = SceneCache::default()
        .build(&representation, stamp())
        .unwrap();
    let batch = &delta.added[0];
    assert_eq!(batch.topology, RenderTopology::Mesh);
    assert_eq!(batch.indices, vec![[0, 1, 2], [0, 2, 3]]);
    assert_eq!(
        batch.edges,
        vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
        "one stored segment, relative to the batch origin; not 12 triangle edges"
    );
    assert!(!batch.is_text_glyph());
}

#[test]
fn body_stored_edges_are_deduplicated_and_degenerate_segments_dropped() {
    let mesh = quad();
    let a = Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    let b = Point3 {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    };
    let c = Point3 {
        x: 1.0,
        y: 1.0,
        z: 0.0,
    };
    let non_finite = Point3 {
        x: f64::NAN,
        y: 0.0,
        z: 0.0,
    };
    let stored = vec![
        a, b, // first occurrence, kept
        b, a, // same unordered pair in the other direction: dropped
        c, c, // zero length: dropped
        non_finite, b, // non-finite endpoint: dropped
        b, c, // a distinct segment, kept
    ];
    let mut representation = mesh_representation(21, mesh.clone());
    representation.fragments[0].primitive = DisplayPrimitive::MeshWithStoredEdges {
        mesh: Arc::new(mesh),
        edges: Arc::from(stored.into_boxed_slice()),
    };
    let delta = SceneCache::default()
        .build(&representation, stamp())
        .unwrap();
    let batch = &delta.added[0];
    assert_eq!(
        batch.edges,
        vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0]
        ],
        "first occurrence of each unordered segment, in input order"
    );
}

#[test]
fn body_stored_edges_are_relative_to_the_batch_local_origin() {
    let mut mesh = quad();
    // Shift every vertex so the batch origin is the first vertex, not (0,0,0).
    for vertex in &mut mesh.vertices {
        vertex.x += 10.0;
        vertex.y += 20.0;
        vertex.z += 30.0;
    }
    let origin = mesh.vertices[0];
    let stored = vec![
        origin,
        Point3 {
            x: origin.x + 2.0,
            y: origin.y,
            z: origin.z,
        },
    ];
    let mut representation = mesh_representation(22, mesh.clone());
    representation.fragments[0].primitive = DisplayPrimitive::MeshWithStoredEdges {
        mesh: Arc::new(mesh),
        edges: Arc::from(stored.into_boxed_slice()),
    };
    let delta = SceneCache::default()
        .build(&representation, stamp())
        .unwrap();
    let batch = &delta.added[0];
    assert_eq!(batch.local_origin, origin);
    assert_eq!(batch.edges, vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0]]);
}

#[test]
fn mesh_with_stored_edges_but_no_usable_segment_keeps_the_triangle_overlay() {
    // Defensive only: the provider never emits an empty stored-edge list, but if
    // one reaches the scene the legacy triangle-derived overlay is kept (and the
    // batch is still not a text glyph).
    let mesh = quad();
    let mut representation = mesh_representation(23, mesh.clone());
    representation.fragments[0].primitive = DisplayPrimitive::MeshWithStoredEdges {
        mesh: Arc::new(mesh),
        edges: Arc::from(Vec::<Point3>::new().into_boxed_slice()),
    };
    let delta = SceneCache::default()
        .build(&representation, stamp())
        .unwrap();
    let batch = &delta.added[0];
    assert_eq!(batch.edges.len(), 10);
    assert!(!batch.is_text_glyph());
}

#[test]
fn body_edges_only_fragment_becomes_a_line_batch_with_no_triangles() {
    // A zero-triangle body with non-empty edges arrives as `LineSegments`
    // (consecutive segment endpoint pairs), which builds as a line batch. It
    // never reaches the index-less `RenderTopology::Mesh` path whose
    // `triangle_count` fallback would read `vertices.len() / 3` as invented
    // triangles.
    let mut representation = line_representation(21, unit_points());
    let mut edges = unit_points();
    edges.extend([
        Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
        Point3 {
            x: 1.0,
            y: 1.0,
            z: 0.0,
        },
    ]);
    representation.fragments[0].primitive =
        DisplayPrimitive::LineSegments(Arc::from(edges.into_boxed_slice()));
    let delta = SceneCache::default()
        .build(&representation, stamp())
        .unwrap();
    assert_eq!(delta.added.len(), 1);
    let batch = &delta.added[0];
    assert_eq!(batch.topology, RenderTopology::Lines);
    assert!(batch.indices.is_empty());
    assert_eq!(batch.triangle_count(), 0);
    // Two endpoint pairs are four vertices, and none are joined into a polyline.
    assert_eq!(batch.vertices.len(), 4);
}

#[test]
fn body_wireframe_edges_only_fragment_is_a_line_batch_with_no_triangles() {
    // The `VSCURRENT WIREFRAME` body path emits `LineSegments` (stored edges or
    // unique triangle-derived edges). Whatever the source, the scene must build
    // a line batch: no indices, no triangles, and one vertex per endpoint.
    let segments = vec![
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
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 1.0,
            y: 1.0,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 1.0,
            y: 1.0,
            z: 0.0,
        },
    ];
    let mut representation = line_representation(24, unit_points());
    representation.fragments[0].primitive =
        DisplayPrimitive::LineSegments(Arc::from(segments.into_boxed_slice()));
    let delta = SceneCache::default()
        .build(&representation, stamp())
        .unwrap();
    assert_eq!(delta.added.len(), 1);
    let batch = &delta.added[0];
    assert_eq!(batch.topology, RenderTopology::Lines);
    assert!(batch.indices.is_empty());
    assert_eq!(batch.triangle_count(), 0);
    assert_eq!(
        batch.vertices.len(),
        6,
        "three endpoint pairs, not a polyline"
    );
}

#[test]
fn lines_batch_has_no_triangles_or_normals() {
    let mut cache = SceneCache::default();
    let rep = line_representation(
        1,
        vec![
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
        ],
    );
    let delta = cache.build(&rep, stamp()).unwrap();
    let batch = &delta.added[0];
    assert_eq!(batch.topology, RenderTopology::Lines);
    assert!(batch.normals.is_empty());
    assert!(batch.indices.is_empty());
    assert_eq!(batch.triangle_count(), 0);
}

#[test]
fn line_topologies_do_not_consume_the_triangle_budget() {
    let mut cache = SceneCache::default();
    let mut points = unit_points();
    points.extend(unit_points());
    points.extend(unit_points());
    let delta = cache
        .build(&line_representation(1, points), stamp())
        .unwrap();
    let mut batch = delta.added[0].clone();
    let budget = FrameBudget {
        max_vertices: 6,
        max_triangles: 0,
        max_bytes: usize::MAX,
    };
    for topology in [RenderTopology::Lines, RenderTopology::MeshEdges] {
        batch.topology = topology;
        assert_eq!(batch.triangle_count(), 0);
        let mut usage = FrameUsage::default();
        budget
            .charge(&mut usage, batch.vertices.len(), batch.triangle_count())
            .unwrap();
        assert_eq!(usage.vertices, 6);
        assert_eq!(usage.triangles, 0);
    }
    // Non-indexed mesh batches retain their triangle-list fallback.
    batch.topology = RenderTopology::Mesh;
    assert_eq!(batch.triangle_count(), 2);
}
