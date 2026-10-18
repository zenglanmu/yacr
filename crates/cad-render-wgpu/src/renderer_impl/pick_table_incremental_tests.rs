//! Pure contracts for the incremental pick-table update (no GPU required):
//! an append/suffix commit must extend the table, never rebuild it, and the
//! result must equal a full rebuild.

use super::*;

fn source(entity: u128) -> SelectionRef {
    SelectionRef {
        document: cad_domain::DocumentId(1),
        entity: cad_domain::EntityId(entity),
        instance: Default::default(),
        sub_element: None,
    }
}

/// A `GpuBatch` carrying only the fields the pick table reads; the GPU
/// buffers are `None` (the table is pure CPU data).
fn gpu_batch(pick_key: u32, sources: Vec<SelectionRef>) -> GpuBatch {
    GpuBatch {
        vertices: None,
        vertex_count: 0,
        normals: None,
        colors: None,
        topology: cad_scene::RenderTopology::Lines,
        indices: None,
        index_count: 0,
        line_quads: None,
        line_quad_count: 0,
        mirrored: false,
        alpha: 1.0,
        color: [1.0, 1.0, 1.0],
        lineweight: 0.0,
        draw_order: 0,
        centroid: [0.0; 3],
        origin: [0.0; 3],
        instanced: Vec::new(),
        mesh_runs: Vec::new(),
        arc_runs: Vec::new(),
        upload_bytes: 0,
        pick_key,
        sources,
    }
}

fn fresh_table(renderer: &Renderer) -> PickTable {
    let entries: Vec<(u32, Vec<SelectionRef>)> = renderer
        .batches
        .iter()
        .map(|batch| (batch.pick_key, batch.sources.clone()))
        .chain(
            renderer
                .images
                .iter()
                .map(|image| (image.pick_key, image.sources.clone())),
        )
        .collect();
    let mut table = PickTable::build(entries, false).expect("resident keys are unique");
    table.ensure_source_index();
    table
}

#[test]
fn an_append_only_commit_extends_the_pick_table_without_a_full_rebuild() {
    let mut renderer = Renderer::new(BackendPreference::Auto);
    renderer.batches = (1..=3)
        .map(|key| gpu_batch(key, vec![source(key as u128)]))
        .collect();
    renderer.rebuild_pick_table(None).expect("initial build");
    assert_eq!(renderer.pick_table_full_rebuilds(), 1);
    assert_eq!(renderer.pick_table_incremental_extends(), 0);

    // Append two batches (keys 4, 5): a commit that keeps the whole prefix.
    renderer.batches.push(gpu_batch(4, vec![source(40)]));
    renderer.batches.push(gpu_batch(5, vec![source(50)]));
    renderer
        .rebuild_pick_table(Some((3, Vec::new())))
        .expect("append");
    assert_eq!(
        renderer.pick_table_full_rebuilds(),
        1,
        "an append must not rebuild the whole table"
    );
    assert_eq!(renderer.pick_table_incremental_extends(), 1);
    assert_eq!(renderer.pick_table_len(), 5);
    assert_eq!(renderer.pick_key_for_batch(4), Some(5));

    let mut incremental = renderer.pick_table.clone();
    incremental.ensure_source_index();
    assert_eq!(incremental, fresh_table(&renderer));
}

#[test]
fn a_camera_settle_suffix_commit_stays_cheap_on_a_large_batch_set() {
    // A large synthetic base set, then repeated camera-settle-style overlay
    // suffix re-prepares: each keeps the base prefix and replaces the single
    // overlay batch. None of them may trigger a full rebuild.
    let mut renderer = Renderer::new(BackendPreference::Auto);
    renderer.batches = (1..=4096)
        .map(|key| gpu_batch(key, vec![source(key as u128)]))
        .collect();
    renderer.rebuild_pick_table(None).expect("base build");
    let full_rebuilds = renderer.pick_table_full_rebuilds();

    // The first overlay suffix (key 4097), then a settle re-prepare that
    // retires it and appends key 4098.
    renderer.batches.push(gpu_batch(4097, vec![source(9001)]));
    renderer
        .rebuild_pick_table(Some((4096, Vec::new())))
        .expect("first suffix");
    renderer.batches.truncate(4096);
    renderer.batches.push(gpu_batch(4098, vec![source(9002)]));
    renderer
        .rebuild_pick_table(Some((4096, vec![4097])))
        .expect("settle re-prepare");

    assert_eq!(
        renderer.pick_table_full_rebuilds(),
        full_rebuilds,
        "a camera-settle suffix commit must not rebuild the whole table"
    );
    assert_eq!(renderer.pick_table_incremental_extends(), 2);
    assert_eq!(renderer.pick_table_len(), 4097);
    assert_eq!(renderer.pick_key_for_batch(4096), Some(4098));
    assert!(
        renderer.pick_table.resolve(4097).is_none(),
        "the retired suffix key is a gap"
    );

    let mut incremental = renderer.pick_table.clone();
    incremental.ensure_source_index();
    assert_eq!(incremental, fresh_table(&renderer));
}

#[test]
fn a_base_change_falls_back_to_a_full_rebuild() {
    let mut renderer = Renderer::new(BackendPreference::Auto);
    renderer.batches = (1..=3)
        .map(|key| gpu_batch(key, vec![source(key as u128)]))
        .collect();
    renderer.rebuild_pick_table(None).expect("initial build");

    // A base change (keep_prefix = 0) retires every key: a full rebuild.
    renderer.batches = (10..=12)
        .map(|key| gpu_batch(key, vec![source(key as u128)]))
        .collect();
    renderer
        .rebuild_pick_table(Some((0, vec![1, 2, 3])))
        .expect("base change");
    assert_eq!(renderer.pick_table_full_rebuilds(), 2);
    assert_eq!(renderer.pick_table_incremental_extends(), 0);
    assert_eq!(renderer.pick_table_len(), 3);
    assert!(renderer.pick_table.resolve(1).is_none());
    assert_eq!(renderer.pick_key_for_batch(0), Some(10));

    // A prefix the table does not cover (a lifecycle hole) must also fall
    // back to a full rebuild rather than extend a stale table.
    renderer.batches = (20..=22)
        .map(|key| gpu_batch(key, vec![source(key as u128)]))
        .collect();
    renderer
        .rebuild_pick_table(Some((5, Vec::new())))
        .expect("uncovered prefix");
    assert_eq!(renderer.pick_table_full_rebuilds(), 3);
    assert_eq!(renderer.pick_table_incremental_extends(), 0);
    assert_eq!(renderer.pick_table_len(), 3);
    assert_eq!(renderer.pick_key_for_batch(0), Some(20));
}
