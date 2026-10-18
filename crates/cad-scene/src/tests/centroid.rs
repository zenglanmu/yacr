//! Batch centroid computation.

use super::*;

#[test]
fn centroid_is_origin_plus_mean_vertex() {
    let mut cache = SceneCache::default();
    let delta = cache
        .build(&mesh_representation(1, quad()), stamp())
        .unwrap();
    let batch = &delta.added[0];
    // Quad spans (0,0)-(1,1) with origin (0,0,0): centroid is (0.5, 0.5, 0).
    let c = batch.centroid();
    assert!((c[0] - 0.5).abs() < 1e-6, "got {c:?}");
    assert!((c[1] - 0.5).abs() < 1e-6, "got {c:?}");
    assert!(c[2].abs() < 1e-6, "got {c:?}");
}

#[test]
fn centroid_keeps_precision_at_large_coordinates() {
    let mut cache = SceneCache::default();
    let rep = line_representation(
        1,
        vec![
            Point3 {
                x: 1_000_000.0,
                y: 2_000_000.0,
                z: 0.0,
            },
            Point3 {
                x: 1_000_010.0,
                y: 2_000_000.0,
                z: 0.0,
            },
        ],
    );
    let delta = cache.build(&rep, stamp()).unwrap();
    let c = delta.added[0].centroid();
    assert!((c[0] - 1_000_005.0).abs() < 1e-3, "got {c:?}");
    assert!((c[1] - 2_000_000.0).abs() < 1e-3, "got {c:?}");
}

#[test]
fn empty_batch_centroid_falls_back_to_local_origin() {
    let batch = RenderBatch {
        conics: Vec::new(),
        local_origin: Point3 {
            x: 3.0,
            y: 4.0,
            z: 5.0,
        },
        topology: RenderTopology::Lines,
        vertices: Vec::new(),
        normals: Vec::new(),
        colors: Vec::new(),
        indices: Vec::new(),
        edges: Vec::new(),
        mirrored: false,
        alpha: 1.0,
        color: DEFAULT_BATCH_COLOR,
        color_unresolved: true,
        lineweight: DEFAULT_BATCH_LINEWEIGHT_MM,
        lineweight_unresolved: true,
        linetype: cad_db::LinetypePattern::continuous(),
        linetype_unresolved: true,
        sources: Vec::new(),
        draw_order: 0,
    };
    assert_eq!(batch.centroid(), [3.0, 4.0, 5.0]);
}
