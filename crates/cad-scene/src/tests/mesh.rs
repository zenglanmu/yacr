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
    // Two triangles => six directed edges => twelve edge vertices.
    assert_eq!(batch.edges.len(), 12);
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
