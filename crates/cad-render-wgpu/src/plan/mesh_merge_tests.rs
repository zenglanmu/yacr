use super::*;
use cad_domain::{EntityId, SelectionRef};

fn source(entity: u128) -> SelectionRef {
    SelectionRef {
        document: cad_domain::DocumentId(1),
        entity: EntityId(entity),
        instance: Default::default(),
        sub_element: None,
    }
}

/// The renderer pick keys these tests use: `1 + batch ordinal`, exactly what
/// `prepare_upload_batches` assigns (see `PickTable`).
fn keys(batches: &[cad_scene::RenderBatch]) -> Vec<u32> {
    (0..batches.len() as u32).map(|index| index + 1).collect()
}

/// A quad mesh (4 vertices, 2 triangles) at `origin`, tint `color`.
fn quad(origin: [f64; 3], color: [f32; 3], entity: u128) -> cad_scene::RenderBatch {
    let half = 1.0f32;
    cad_scene::RenderBatch {
        conics: Vec::new(),
        local_origin: cad_domain::Point3 {
            x: origin[0],
            y: origin[1],
            z: origin[2],
        },
        topology: RenderTopology::Mesh,
        vertices: vec![
            [-half, -half, 0.0],
            [-half, half, 0.0],
            [half, half, 0.0],
            [half, -half, 0.0],
        ],
        normals: vec![[0.0, 0.0, 1.0]; 4],
        colors: Vec::new(),
        indices: vec![[0, 1, 2], [0, 2, 3]],
        edges: Vec::new(),
        mirrored: false,
        alpha: 1.0,
        color,
        color_unresolved: false,
        lineweight: 0.0,
        lineweight_unresolved: true,
        linetype: cad_scene::LinetypePattern::continuous(),
        linetype_unresolved: true,
        sources: vec![source(entity)],
        draw_order: 0,
    }
}

#[test]
fn mesh_vertex_stride_matches_the_packed_layout() {
    assert_eq!(
        MESH_VERTEX_STRIDE as usize,
        std::mem::size_of::<MeshVertex>()
    );
    let vertex = MeshVertex {
        position: [0.0; 3],
        normal: [0.0; 3],
        color: [0.0; 3],
        entity_flags: [0, 0],
    };
    assert_eq!(
        mesh_vertex_bytes(std::slice::from_ref(&vertex)).len(),
        MESH_VERTEX_STRIDE as usize
    );
}

#[test]
fn mesh_merge_gate_accepts_only_opaque_triangle_meshes() {
    assert!(mesh_batch_is_mergeable(&quad([0.0; 3], [1.0; 3], 1)));
    // Transparent stays legacy (camera-dependent order).
    let mut transparent = quad([0.0; 3], [1.0; 3], 1);
    transparent.alpha = 0.5;
    assert!(!mesh_batch_is_mergeable(&transparent));
    // A mesh with no triangle list has nothing to merge.
    let mut empty = quad([0.0; 3], [1.0; 3], 1);
    empty.indices.clear();
    assert!(!mesh_batch_is_mergeable(&empty));
    // A line batch is not a mesh.
    let mut line = quad([0.0; 3], [1.0; 3], 1);
    line.topology = RenderTopology::Lines;
    assert!(!mesh_batch_is_mergeable(&line));
}

#[test]
fn build_mesh_chunks_concatenates_adjacent_batches_in_order() {
    let batches = vec![
        quad([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 11),
        quad([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 22),
    ];
    let (chunks, runs) = build_mesh_chunks(
        &batches,
        &[0, 1],
        &keys(&batches),
        MAX_VERTICES_PER_MESH_STREAM,
        MAX_STREAM_RADIUS,
    );
    assert_eq!(chunks.len(), 1, "adjacent same-pipeline meshes merge");
    assert_eq!(chunks[0].vertices.len(), 8, "4 + 4 vertices");
    assert_eq!(chunks[0].indices.len(), 12, "6 + 6 triangle indices");
    // Runs are contiguous in the chunk, so the frame coalesces them.
    assert_eq!(
        runs[0].as_ref().unwrap()[0],
        MeshRun {
            chunk: 0,
            index_offset: 0,
            index_count: 6,
            vertex_count: 4,
        }
    );
    assert_eq!(
        runs[1].as_ref().unwrap()[0],
        MeshRun {
            chunk: 0,
            index_offset: 6,
            index_count: 6,
            vertex_count: 4,
        }
    );
    // The second batch's indices are rebased by its vertex base (4).
    assert_eq!(&chunks[0].indices[6..], &[4, 5, 6, 4, 6, 7]);
}

#[test]
fn build_mesh_chunks_splits_a_batch_beyond_the_vertex_cap() {
    // Two quads (4 vertices each) with a cap of 4: each piece is its own
    // chunk, no triangle lost and no empty chunk.
    let batches = vec![quad([0.0; 3], [1.0; 3], 1), quad([0.0; 3], [1.0; 3], 2)];
    let (chunks, runs) =
        build_mesh_chunks(&batches, &[0, 1], &keys(&batches), 4, MAX_STREAM_RADIUS);
    assert_eq!(chunks.len(), 2, "a full chunk starts a new one");
    assert!(chunks.iter().all(|chunk| !chunk.vertices.is_empty()));
    assert_eq!(
        chunks.iter().map(|c| c.indices.len()).sum::<usize>(),
        12,
        "no triangle lost"
    );
    assert_eq!(runs[0].as_ref().unwrap()[0].chunk, 0);
    assert_eq!(runs[1].as_ref().unwrap()[0].chunk, 1);
}

#[test]
fn build_mesh_chunks_splits_one_oversized_batch_without_loss() {
    // A single batch of four triangles (6 distinct vertices) with a cap of 4:
    // greedy splitting keeps every triangle, in order, across pieces.
    let mut batch = quad([0.0; 3], [1.0; 3], 1);
    batch.vertices.push([2.0, 0.0, 0.0]);
    batch.vertices.push([2.0, 1.0, 0.0]);
    batch.normals = vec![[0.0, 0.0, 1.0]; 6];
    batch.indices = vec![[0, 1, 2], [0, 2, 3], [3, 2, 4], [2, 5, 4]];
    let batches = vec![batch];
    let (chunks, runs) = build_mesh_chunks(&batches, &[0], &keys(&batches), 4, MAX_STREAM_RADIUS);
    assert!(chunks.len() >= 2, "an oversized batch must split");
    assert!(chunks.iter().all(|chunk| !chunk.vertices.is_empty()));
    assert_eq!(
        chunks.iter().map(|c| c.indices.len()).sum::<usize>(),
        12,
        "all four triangles survive the split"
    );
    let runs = runs[0].as_ref().expect("batch converted");
    assert_eq!(runs.iter().map(|r| r.index_count).sum::<u32>(), 12);
    assert_eq!(
        runs.iter().map(|r| r.chunk).collect::<Vec<_>>(),
        (0..runs.len() as u32).collect::<Vec<_>>(),
        "one run per piece, one piece per chunk"
    );
}

#[test]
fn build_mesh_chunks_is_a_mirrored_boundary() {
    let a = quad([0.0; 3], [1.0; 3], 1);
    let mut b = quad([0.0; 3], [1.0; 3], 2);
    b.mirrored = true;
    let batches = vec![a, b];
    let (chunks, _) = build_mesh_chunks(
        &batches,
        &[0, 1],
        &keys(&batches),
        MAX_VERTICES_PER_MESH_STREAM,
        MAX_STREAM_RADIUS,
    );
    assert_eq!(chunks.len(), 2, "a mirrored batch is a pipeline boundary");
    assert!(!chunks[0].mirrored);
    assert!(chunks[1].mirrored);
}

#[test]
fn push_mesh_vertices_bakes_the_tint_and_carries_the_pick_key() {
    let mut batch = quad([100.0, 200.0, 0.0], [0.5, 0.25, 0.75], 42);
    // A per-vertex gradient: the shader multiplies the batch tint by this.
    batch.colors = vec![
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 1.0, 1.0],
    ];
    let piece = &mesh_pieces(&batch, 0, 64)[0];
    let origin = [
        batch.local_origin.x,
        batch.local_origin.y,
        batch.local_origin.z,
    ];
    let mut out = Vec::new();
    push_mesh_vertices(&mut out, &batch, piece, origin, 9);
    assert_eq!(out.len(), 4);
    // Baked colour = sanitized tint * sanitized vertex colour.
    assert_eq!(out[0].color, [0.5, 0.0, 0.0]);
    assert_eq!(out[1].color, [0.0, 0.25, 0.0]);
    assert_eq!(out[2].color, [0.0, 0.0, 0.75]);
    assert_eq!(out[3].color, [0.5, 0.25, 0.75]);
    // The renderer pick key/flags ride per vertex (the legacy per-batch draw
    // carried no explicit pick key at all).
    for vertex in &out {
        assert_eq!(vertex.entity_flags[0], 9);
        assert_eq!(
            vertex.entity_flags[1],
            INSTANCE_FLAG_OPAQUE | INSTANCE_FLAG_HAS_SOURCE
        );
    }
}

#[test]
fn merged_chunk_carries_the_right_entity_id_per_vertex() {
    // "Pick one triangle per class and read its id": two batches with
    // different source entities merge into one chunk, and every vertex keeps
    // its own batch's id, at the chunk vertex offsets the run list reports.
    let batches = vec![
        quad([0.0, 0.0, 0.0], [1.0; 3], 111),
        quad([3.0, 0.0, 0.0], [1.0; 3], 222),
    ];
    let (chunks, runs) = build_mesh_chunks(
        &batches,
        &[0, 1],
        &keys(&batches),
        MAX_VERTICES_PER_MESH_STREAM,
        MAX_STREAM_RADIUS,
    );
    assert_eq!(chunks.len(), 1);
    let first = runs[0].as_ref().unwrap()[0];
    let second = runs[1].as_ref().unwrap()[0];
    assert_eq!(first.vertex_count, 4);
    assert_eq!(second.vertex_count, 4);
    // The run's vertex range is implicit (runs are packed in order); walk the
    // chunk vertices and attribute each to its batch by that ordering.
    for (index, vertex) in chunks[0].vertices.iter().enumerate() {
        let expected = if index < first.vertex_count as usize {
            1
        } else {
            2
        };
        assert_eq!(
            vertex.entity_flags[0], expected,
            "vertex {index} must carry its own batch's pick key"
        );
        assert_eq!(
            vertex.entity_flags[1],
            INSTANCE_FLAG_OPAQUE | INSTANCE_FLAG_HAS_SOURCE
        );
    }
}

#[test]
fn push_mesh_vertices_stores_shared_origin_positions_verbatim() {
    // With a shared origin the stored position is the batch-local f32 value,
    // so the merged draw reproduces the legacy per-batch matrix exactly.
    let vertices = vec![
        [0.1f32, -0.2, 3.0e-3],
        [1.5e-4, 12345.5, -0.75],
        [2.0, 0.0, 0.0],
    ];
    let mut batch = quad([1.0e7, -5.0e6, 0.5], [1.0; 3], 1);
    batch.vertices = vertices.clone();
    batch.normals = vec![[0.0, 0.0, 1.0]; 3];
    batch.indices = vec![[0, 1, 2]];
    let piece = &mesh_pieces(&batch, 0, 64)[0];
    let origin = [
        batch.local_origin.x,
        batch.local_origin.y,
        batch.local_origin.z,
    ];
    let mut out = Vec::new();
    push_mesh_vertices(&mut out, &batch, piece, origin, 1);
    assert_eq!(out[0].position, vertices[0]);
    assert_eq!(out[1].position, vertices[1]);
}

#[test]
fn overlay_is_instanced_only_for_opaque_meshes_with_edges() {
    let mut with_edges = quad([0.0; 3], [1.0; 3], 1);
    assert!(overlay_is_instanced(&with_edges).is_none());
    with_edges.edges = vec![[0.0; 3], [1.0, 0.0, 0.0]];
    // `mesh_edge_indices` sorts/de-duplicates the triangle edges.
    assert_eq!(
        overlay_is_instanced(&with_edges),
        Some(vec![0, 1, 0, 2, 0, 3, 1, 2, 2, 3])
    );
    // Transparent overlays stay legacy.
    let mut transparent = with_edges.clone();
    transparent.alpha = 0.5;
    assert!(overlay_is_instanced(&transparent).is_none());
}

#[test]
fn convert_line_batches_instances_overlays_only_when_merging() {
    let mut mesh = quad([0.0; 3], [1.0; 3], 1);
    mesh.edges = vec![[0.0; 3], [1.0, 0.0, 0.0]];
    let batches = vec![mesh];
    // Mesh merging off: nothing converts (the overlay stays a legacy draw).
    assert!(convert_line_batches(&batches, true, false).is_empty());
    // Mesh merging on: the overlay becomes an index-pair line source.
    let converted = convert_line_batches(&batches, true, true);
    assert_eq!(converted.len(), 1);
    assert_eq!(converted[0].batch, 0);
    assert_eq!(converted[0].segments, 5);
    assert_eq!(
        converted[0].indices.as_deref(),
        Some(&[0, 1, 0, 2, 0, 3, 1, 2, 2, 3][..])
    );
}

#[test]
fn push_line_instances_expands_index_pairs_from_the_overlay() {
    // An overlay is an index list over the batch vertices; each consecutive
    // index pair must expand to one 2-vertex instance.
    let mut batch = quad([0.0; 3], [0.25, 0.5, 0.75], 7);
    batch.edges = vec![[0.0; 3], [1.0, 0.0, 0.0]];
    let indices = overlay_is_instanced(&batch).expect("overlay");
    let origin = [0.0, 0.0, 0.0];
    let mut out = Vec::new();
    push_line_instances(&mut out, &batch, Some(&indices), origin, 0, 5, 7);
    assert_eq!(out.len(), 5);
    // First edge (0,1): vertices[0] -> vertices[1].
    assert_eq!(
        [out[0].p0_width[0], out[0].p0_width[1], out[0].p0_width[2]],
        batch.vertices[0]
    );
    assert_eq!(
        [out[0].p1_pad[0], out[0].p1_pad[1], out[0].p1_pad[2]],
        batch.vertices[1]
    );
    // Colour comes from the batch (as the legacy line uniform did); the pick
    // key is new per-instance metadata the legacy draw did not carry.
    assert_eq!(out[0].rgba, [0.25, 0.5, 0.75, 1.0]);
    assert_eq!(out[0].id_flags[0], 7);
}
