use super::*;
use cad_domain::SelectionRef;

fn source() -> SelectionRef {
    SelectionRef {
        document: cad_domain::DocumentId(1),
        entity: cad_domain::EntityId(42),
        instance: Default::default(),
        sub_element: None,
    }
}

fn batch(topology: RenderTopology, vertices: Vec<[f32; 3]>, alpha: f32) -> cad_scene::RenderBatch {
    cad_scene::RenderBatch {
        conics: Vec::new(),
        local_origin: cad_domain::Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        topology,
        vertices,
        normals: Vec::new(),
        colors: Vec::new(),
        indices: Vec::new(),
        edges: Vec::new(),
        mirrored: false,
        alpha,
        color: [0.25, 0.5, 0.75],
        color_unresolved: false,
        lineweight: 0.0,
        lineweight_unresolved: false,
        linetype: cad_scene::LinetypePattern::continuous(),
        linetype_unresolved: false,
        sources: vec![source()],
        draw_order: 0,
    }
}

/// Convert every `Lines` batch (the only class this module exercises) the
/// way [`convert_line_batches`] would, with `line_instancing` on.
fn converted(batches: &[cad_scene::RenderBatch]) -> Vec<ConvertedLine> {
    batches
        .iter()
        .enumerate()
        .filter(|(_, batch)| line_batch_is_instanced(batch))
        .map(|(index, batch)| ConvertedLine {
            batch: index,
            segments: (batch.vertices.len() / 2) as u32,
            indices: None,
        })
        .collect()
}

/// The renderer pick keys these tests use: `1 + batch ordinal`, exactly what
/// `prepare_upload_batches` assigns (see `PickTable`).
fn keys(batches: &[cad_scene::RenderBatch]) -> Vec<u32> {
    (0..batches.len() as u32).map(|index| index + 1).collect()
}

#[test]
fn instance_stride_matches_the_packed_layout() {
    assert_eq!(
        LINE_INSTANCE_STRIDE as usize,
        std::mem::size_of::<LineInstance>()
    );
    let instance = LineInstance {
        p0_width: [0.0; 4],
        p1_pad: [0.0; 4],
        rgba: [0.0; 4],
        id_flags: [0, 0],
    };
    assert_eq!(
        line_instance_bytes(std::slice::from_ref(&instance)).len(),
        LINE_INSTANCE_STRIDE as usize
    );
}

#[test]
fn line_batch_gate_accepts_only_opaque_even_line_lists() {
    // Opaque 2-vertex segment: the exact case.
    assert!(line_batch_is_instanced(&batch(
        RenderTopology::Lines,
        vec![[0.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
        1.0
    )));
    // Opaque multi-segment LineList (even): each pair is independent.
    assert!(line_batch_is_instanced(&batch(
        RenderTopology::Lines,
        vec![
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [2.0, 0.0, 0.0],
            [3.0, 1.0, 0.0]
        ],
        1.0
    )));
    // Transparent stays legacy (camera-dependent ordering).
    assert!(!line_batch_is_instanced(&batch(
        RenderTopology::Lines,
        vec![[0.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
        0.5
    )));
    // Invisible stays legacy (reported, never silently instanced).
    assert!(!line_batch_is_instanced(&batch(
        RenderTopology::Lines,
        vec![[0.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
        0.0
    )));
    // An odd trailing vertex would change the LineList pairing rule.
    assert!(!line_batch_is_instanced(&batch(
        RenderTopology::Lines,
        vec![[0.0, 0.0, 0.0], [1.0, 1.0, 0.0], [2.0, 0.0, 0.0]],
        1.0
    )));
    // MeshEdges is now converted too: its legacy draw used to be a no-op
    // (the gate's doc explains the G3 fix), so it is instanced as line
    // geometry like `Lines`.
    assert!(line_batch_is_instanced(&batch(
        RenderTopology::MeshEdges,
        vec![[0.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
        1.0
    )));
    // A single vertex is not a segment.
    assert!(!line_batch_is_instanced(&batch(
        RenderTopology::Lines,
        vec![[0.0, 0.0, 0.0]],
        1.0
    )));
}

#[test]
fn shared_origin_is_kept_verbatim() {
    let mut a = batch(
        RenderTopology::Lines,
        vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
        1.0,
    );
    let mut b = batch(
        RenderTopology::Lines,
        vec![[2.0, 0.0, 0.0], [3.0, 0.0, 0.0]],
        1.0,
    );
    a.local_origin = cad_domain::Point3 {
        x: 1.0e7,
        y: -2.0e7,
        z: 5.0,
    };
    b.local_origin = a.local_origin;
    assert_eq!(
        choose_instance_origin(&[&a, &b]),
        [1.0e7, -2.0e7, 5.0],
        "a shared origin is used verbatim so the legacy matrix is reproduced"
    );
}

#[test]
fn mixed_origin_uses_the_world_aabb_midpoint() {
    let mut a = batch(
        RenderTopology::Lines,
        vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
        1.0,
    );
    let mut b = batch(
        RenderTopology::Lines,
        vec![[0.0, 0.0, 0.0], [4.0, 2.0, 0.0]],
        1.0,
    );
    a.local_origin = cad_domain::Point3 {
        x: 1.0e7,
        y: 0.0,
        z: 0.0,
    };
    b.local_origin = cad_domain::Point3 {
        x: 1.0e7 + 8.0,
        y: 0.0,
        z: 0.0,
    };
    // World x spans [1e7, 1e7 + 12]; world y spans [0, 2].
    assert_eq!(choose_instance_origin(&[&a, &b]), [1.0e7 + 6.0, 1.0, 0.0]);
}

#[test]
fn push_line_instances_packs_positions_colour_width_and_pick_key() {
    let mut b = batch(
        RenderTopology::Lines,
        vec![[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]],
        1.0,
    );
    b.lineweight = 0.35;
    let origin = [10.0, 20.0, 30.0];
    let mut out = Vec::new();
    push_line_instances(&mut out, &b, None, origin, 0, 1, 9);
    assert_eq!(out.len(), 1);
    let instance = out[0];
    // (origin-relative) positions
    assert_eq!(instance.p0_width, [-9.0, -18.0, -27.0, 0.35]);
    assert_eq!(instance.p1_pad, [-6.0, -15.0, -24.0, 0.0]);
    // sanitized colour + clamped alpha
    assert_eq!(instance.rgba, [0.25, 0.5, 0.75, 1.0]);
    // the renderer pick key (not a truncated EntityId) + the descriptive flags
    assert_eq!(instance.id_flags[0], 9);
    assert_eq!(
        instance.id_flags[1],
        INSTANCE_FLAG_OPAQUE | INSTANCE_FLAG_HAS_SOURCE | INSTANCE_FLAG_LINEWEIGHT
    );
}

#[test]
fn shared_origin_positions_are_exactly_the_batch_vertices() {
    // The byte-identity contract: with a shared origin, `(o + v - o) as f32`
    // must equal `v` exactly for arbitrary f32 vertices.
    let vertices = vec![[0.1f32, -0.2, 3.0e-3], [1.5e-4, 12345.5, -0.75]];
    let mut b = batch(RenderTopology::Lines, vertices.clone(), 1.0);
    b.local_origin = cad_domain::Point3 {
        x: 1.0e7,
        y: -5.0e6,
        z: 0.5,
    };
    let origin = [b.local_origin.x, b.local_origin.y, b.local_origin.z];
    let mut out = Vec::new();
    push_line_instances(&mut out, &b, None, origin, 0, 1, 1);
    assert_eq!(
        [out[0].p0_width[0], out[0].p0_width[1], out[0].p0_width[2]],
        vertices[0]
    );
    assert_eq!(
        [out[0].p1_pad[0], out[0].p1_pad[1], out[0].p1_pad[2]],
        vertices[1]
    );
}

fn origin(x: f64, y: f64, z: f64) -> cad_domain::Point3 {
    cad_domain::Point3 { x, y, z }
}

fn segment(x: f32) -> Vec<[f32; 3]> {
    vec![[x, 0.0, 0.0], [x + 0.5, 1.0, 0.0]]
}

/// A batch with `count` disjoint segments, all relative to `local_origin`.
fn multi_segment_batch(count: usize, local: cad_domain::Point3) -> cad_scene::RenderBatch {
    let vertices = (0..count).flat_map(|i| segment(i as f32)).collect();
    let mut b = batch(RenderTopology::Lines, vertices, 1.0);
    b.local_origin = local;
    b
}

#[test]
fn build_instance_chunks_keeps_a_shared_origin_verbatim() {
    let a = multi_segment_batch(2, origin(0.0, 0.0, 0.0));
    let b = multi_segment_batch(3, origin(0.0, 0.0, 0.0));
    let batches = vec![a, b];
    let (chunks, runs) = build_instance_chunks(
        &batches,
        &converted(&batches),
        &keys(&batches),
        MAX_INSTANCES_PER_STREAM,
        MAX_STREAM_RADIUS,
    );
    assert_eq!(chunks.len(), 1, "a shared origin stays in one stream");
    assert_eq!(chunks[0].origin, [0.0, 0.0, 0.0]);
    assert_eq!(chunks[0].instances.len(), 5, "2 + 3 segments");
    // Contiguous runs in one stream, so the frame path coalesces them.
    assert_eq!(
        runs[0].as_ref().unwrap()[0],
        InstanceRun {
            stream: 0,
            offset: 0,
            count: 2
        }
    );
    assert_eq!(
        runs[1].as_ref().unwrap()[0],
        InstanceRun {
            stream: 0,
            offset: 2,
            count: 3
        }
    );
}

#[test]
fn build_instance_chunks_splits_a_batch_beyond_the_cap() {
    // 5 segments in ONE batch with a cap of 2: split into 2 + 2 + 1 with no
    // segment lost, no empty chunk and no oversized buffer.
    let batches = vec![multi_segment_batch(5, origin(0.0, 0.0, 0.0))];
    let (chunks, runs) = build_instance_chunks(
        &batches,
        &converted(&batches),
        &keys(&batches),
        2,
        MAX_STREAM_RADIUS,
    );
    assert_eq!(chunks.len(), 3, "ceil(5 / 2) chunks");
    assert!(
        chunks
            .iter()
            .all(|chunk| !chunk.instances.is_empty() && chunk.instances.len() <= 2),
        "no empty and no oversized chunk: {:?}",
        chunks.iter().map(|c| c.instances.len()).collect::<Vec<_>>()
    );
    assert_eq!(
        chunks.iter().map(|c| c.instances.len()).sum::<usize>(),
        5,
        "no segment lost"
    );
    let runs = runs[0].as_ref().expect("batch converted");
    assert_eq!(runs.len(), 3, "one run per chunk piece");
    assert_eq!(runs.iter().map(|r| r.count).sum::<u32>(), 5);
    assert_eq!(
        runs.iter().map(|r| r.stream).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    // Order is preserved: segment `i` of the batch lands in slice `i / 2`.
    assert_eq!(
        chunks[0].instances[0].p0_width[0], 0.0,
        "first chunk starts at the batch's first segment"
    );
    assert_eq!(
        chunks[2].instances[0].p0_width[0], 4.0,
        "last chunk starts at segment 4"
    );
}

#[test]
fn build_instance_chunks_splits_distant_origins_into_separate_streams() {
    // Two batches whose origins are far beyond the radius must not share a
    // stream (their midpoint would blow up every stored position).
    let mut a = multi_segment_batch(1, origin(0.0, 0.0, 0.0));
    a.local_origin = origin(0.0, 0.0, 0.0);
    let mut b = multi_segment_batch(1, origin(0.0, 0.0, 0.0));
    b.local_origin = origin(1.0e6, 0.0, 0.0);
    let batches = vec![a, b];
    let (chunks, runs) = build_instance_chunks(
        &batches,
        &converted(&batches),
        &keys(&batches),
        MAX_INSTANCES_PER_STREAM,
        MAX_STREAM_RADIUS,
    );
    assert_eq!(chunks.len(), 2, "a distant origin starts a new stream");
    assert_eq!(runs[0].as_ref().unwrap()[0].stream, 0);
    assert_eq!(runs[1].as_ref().unwrap()[0].stream, 1);
    for chunk in &chunks {
        for instance in &chunk.instances {
            for axis in 0..3 {
                assert!(
                    instance.p0_width[axis].abs() <= 1.0,
                    "stream positions must stay small, got {:?}",
                    instance.p0_width
                );
            }
        }
    }
}

#[test]
fn build_instance_chunks_merges_nearby_mixed_origins_at_the_midpoint() {
    // Near origins still coalesce into one stream, with a midpoint origin
    // and positions bounded by the chunk radius.
    let mut a = multi_segment_batch(1, origin(0.0, 0.0, 0.0));
    a.vertices = vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
    a.local_origin = origin(0.0, 0.0, 0.0);
    let mut b = multi_segment_batch(1, origin(0.0, 0.0, 0.0));
    b.vertices = vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
    b.local_origin = origin(2.0, 0.0, 0.0);
    let batches = vec![a, b];
    let (chunks, runs) = build_instance_chunks(
        &batches,
        &converted(&batches),
        &keys(&batches),
        MAX_INSTANCES_PER_STREAM,
        MAX_STREAM_RADIUS,
    );
    assert_eq!(chunks.len(), 1, "near origins merge");
    assert_eq!(chunks[0].origin, [2.0, 0.0, 0.0], "AABB midpoint");
    assert_eq!(chunks[0].instances.len(), 2);
    assert_eq!(runs[0].as_ref().unwrap()[0].offset, 0);
    assert_eq!(runs[1].as_ref().unwrap()[0].offset, 1);
}

#[test]
fn build_instance_chunks_never_emits_an_empty_chunk() {
    // A batch that does not fit the current chunk starts a new one; the
    // chunk it leaves behind is never emitted empty.
    let a = multi_segment_batch(2, origin(0.0, 0.0, 0.0));
    let b = multi_segment_batch(2, origin(1.0e6, 0.0, 0.0));
    let batches = vec![a, b];
    let (chunks, _) = build_instance_chunks(
        &batches,
        &converted(&batches),
        &keys(&batches),
        MAX_INSTANCES_PER_STREAM,
        MAX_STREAM_RADIUS,
    );
    assert!(chunks.iter().all(|chunk| !chunk.instances.is_empty()));
    assert_eq!(chunks.len(), 2);
}
