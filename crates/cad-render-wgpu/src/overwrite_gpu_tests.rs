//! Native GPU contracts for the in-place resident-batch overwrite (P4).
//!
//! These tests live inside `cad-render-wgpu` (children of the crate root), so
//! they can read [`Renderer`]'s private resident state (the `GpuBatch` fields,
//! the instance-stream CPU mirrors and the pulled segments) and sample the
//! rendered pixels. They bind a real headless device and auto-skip when none is
//! available: a skip prints `SKIP ... NOT a pass` and is never reported as
//! evidence.
//!
//! What is contracted:
//!
//! 1. A transform-only overwrite of a **legacy** line batch moves the drawn line
//!    (pixels), keeps the batch slot, the pick key, the selection flag, the batch
//!    count and the cached draw plan, and does not re-upload any buffer.
//! 2. The same for an **instanced** line batch on the universal tier (the moved
//!    geometry is read back from the rendered pixels; the tier keeps no CPU
//!    mirror), and for the accelerated tier (mirror + pulled records kept in
//!    lockstep, segment count unchanged).
//! 3. Every refusal gate refuses **by name**: topology, count, run list, stream
//!    origin, merged mesh, conic, unresolvable pulled mapping, out of range, and a
//!    renderer without a device.
//!
//! Run serially (these bind a real device):
//!
//! ```text
//! cargo test -p cad-render-wgpu --lib --locked -- --test-threads=1 overwrite_gpu_tests
//! ```
use crate::headless::{create_headless_gpu, HeadlessGpu};
use crate::{BackendPreference, Camera2d, RenderTarget, RenderTier, Renderer};
use cad_domain::{DocumentId, EntityId, InstancePath, Point3, SelectionRef};
use cad_scene::{RenderBatch, RenderConic, RenderTopology};
use std::time::Duration;

/// Smallest geometry tolerance: the fixture uses `world_per_px = 2/64`.
const WORLD_PER_PX: f64 = 2.0 / 64.0;
const SIZE: u32 = 64;

fn source(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn camera() -> Camera2d {
    Camera2d {
        center: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        world_per_px: WORLD_PER_PX,
        z_plane: 0.0,
    }
}

/// An opaque 2-vertex `Lines` batch at `y`, spanning `x` in `[-0.9, 0.9]`.
fn line_batch(entity: u128, y: f32) -> RenderBatch {
    RenderBatch {
        conics: Vec::new(),
        local_origin: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        topology: RenderTopology::Lines,
        vertices: vec![[-0.9, y, 0.0], [0.9, y, 0.0]],
        normals: Vec::new(),
        colors: Vec::new(),
        indices: Vec::new(),
        edges: Vec::new(),
        mirrored: false,
        alpha: 1.0,
        color: cad_scene::DEFAULT_BATCH_COLOR,
        color_unresolved: true,
        lineweight: 0.0,
        lineweight_unresolved: true,
        linetype: cad_scene::LinetypePattern::continuous(),
        linetype_unresolved: true,
        sources: vec![source(entity)],
        draw_order: entity as i64,
    }
}

/// An opaque mergeable triangle quad.
fn quad_mesh(entity: u128, half: f32) -> RenderBatch {
    RenderBatch {
        conics: Vec::new(),
        local_origin: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
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
        indices: vec![[0, 2, 1], [0, 3, 2]],
        edges: Vec::new(),
        mirrored: false,
        alpha: 1.0,
        color: cad_scene::DEFAULT_BATCH_COLOR,
        color_unresolved: true,
        lineweight: 0.0,
        lineweight_unresolved: true,
        linetype: cad_scene::LinetypePattern::continuous(),
        linetype_unresolved: true,
        sources: vec![source(entity)],
        draw_order: entity as i64,
    }
}

/// An opaque full circle at the origin (SDF conic class).
fn circle_batch(entity: u128) -> RenderBatch {
    let mut batch = line_batch(entity, 0.0);
    batch.topology = RenderTopology::Conics;
    batch.vertices.clear();
    batch.conics = vec![RenderConic {
        center: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        major: Point3 {
            x: 0.2,
            y: 0.0,
            z: 0.0,
        },
        minor: Point3 {
            x: 0.0,
            y: 0.2,
            z: 0.0,
        },
        start: 0.0,
        sweep: std::f64::consts::TAU,
        full: true,
    }];
    batch
}

fn gpu_or_skip(what: &str) -> Option<HeadlessGpu> {
    match create_headless_gpu(BackendPreference::Auto) {
        Ok(gpu) => {
            eprintln!(
                "[overwrite-gpu] {what}: adapter backend={} name={} device_type={} driver={}",
                gpu.adapter.backend, gpu.adapter.name, gpu.adapter.device_type, gpu.adapter.driver
            );
            Some(gpu)
        }
        Err(error) => {
            eprintln!("[overwrite-gpu] {what}: SKIP — no headless adapter ({error}); NOT a pass");
            None
        }
    }
}

fn init(gpu: &HeadlessGpu, accelerated: bool, what: &str) -> Renderer {
    let mut renderer = Renderer::new(BackendPreference::Auto);
    if accelerated {
        renderer.set_storage_buffer_support_override(Some(true));
    }
    renderer.set_poll_timeout(Duration::from_secs(30));
    renderer
        .initialize_with_device(gpu.device.clone(), gpu.queue.clone())
        .expect("initialize the renderer");
    if accelerated {
        assert_eq!(
            renderer.render_tier(),
            RenderTier::Accelerated,
            "{what}: the forced tier must be accelerated, otherwise the contract is vacuous"
        );
    }
    renderer
}

fn commit(renderer: &mut Renderer, batches: &[RenderBatch]) {
    let staged = renderer
        .prepare_upload_batches(batches)
        .expect("stage upload");
    renderer.commit_upload(staged, 0).expect("commit upload");
}

fn render(renderer: &mut Renderer) {
    let target = RenderTarget::new(SIZE, SIZE);
    renderer.render(camera(), &target).expect("render");
}

/// Brightest pixel value in a 3x3 neighbourhood: the anti-aliased line core is
/// one pixel wide, so a single sample could land on the ramp.
fn max_channel(renderer: &Renderer, x: i32, y: i32) -> u8 {
    let image = renderer
        .read_target_rgba()
        .expect("read the presented frame");
    let mut best = 0u8;
    for dy in -1..=1i32 {
        for dx in -1..=1i32 {
            let px = (x + dx).clamp(0, SIZE as i32 - 1) as usize;
            let py = (y + dy).clamp(0, SIZE as i32 - 1) as usize;
            let at = (py * SIZE as usize + px) * 4;
            best = best.max(
                image.pixels[at]
                    .max(image.pixels[at + 1])
                    .max(image.pixels[at + 2]),
            );
        }
    }
    best
}

/// The pixel row of a world `y` (the canvas maps world `-1..1` to 64 px).
fn row_of(world_y: f64) -> i32 {
    (SIZE as f64 * 0.5 - world_y / WORLD_PER_PX) as i32
}

#[test]
fn legacy_line_overwrite_moves_ink_and_keeps_every_identity() {
    let Some(gpu) = gpu_or_skip("legacy line overwrite") else {
        return;
    };
    let mut renderer = init(&gpu, false, "legacy line overwrite");
    // Force the legacy per-batch path so the batch owns its own quad buffer.
    renderer.set_line_instancing(false);
    renderer.set_mesh_merging(false);
    commit(&mut renderer, &[line_batch(1, -0.5), line_batch(2, 0.25)]);
    render(&mut renderer);

    let key = renderer.pick_key_for_batch(0).expect("resident batch");
    renderer
        .set_pick_key_flags(key, cad_scene::FLAG_SELECTED)
        .expect("select the batch");
    let plan_before = renderer
        .draw_plan
        .as_ref()
        .map(|plan| plan.opaque().to_vec())
        .expect("a render built the draw plan");
    let plan_rebuilds = renderer.draw_plan_rebuilds();
    let pick_full_rebuilds = renderer.pick_table_full_rebuilds();
    let batch_count = renderer.batch_count();
    assert!(
        max_channel(&renderer, 32, row_of(-0.5)) > 40,
        "the old line draws"
    );
    assert_eq!(
        max_channel(&renderer, 32, row_of(0.5)),
        0,
        "nothing draws at the target row yet"
    );

    renderer
        .overwrite_batch(0, &line_batch(1, 0.5))
        .expect("a same-shape line must fit the legacy buffers");
    render(&mut renderer);

    assert_eq!(
        renderer.batch_count(),
        batch_count,
        "no batch is added/removed"
    );
    assert_eq!(
        renderer.pick_key_for_batch(0),
        Some(key),
        "the pick key is unchanged"
    );
    assert_eq!(
        renderer.pick_key_flags(key) & cad_scene::FLAG_SELECTED,
        cad_scene::FLAG_SELECTED,
        "the selection flag still resolves to the same pick key"
    );
    assert_eq!(
        renderer.draw_plan_rebuilds(),
        plan_rebuilds,
        "the cached draw plan is not rebuilt"
    );
    assert_eq!(
        renderer
            .draw_plan
            .as_ref()
            .map(|plan| plan.opaque().to_vec()),
        Some(plan_before),
        "the frame's draw plan / batch order is unchanged"
    );
    assert_eq!(
        renderer.pick_table_full_rebuilds(),
        pick_full_rebuilds,
        "the pick table is untouched"
    );
    assert_eq!(renderer.batch_overwrites(), 1);
    assert_eq!(renderer.instance_stream_uploads(), 0);
    assert_eq!(renderer.mesh_chunk_uploads(), 0);
    assert!(
        max_channel(&renderer, 32, row_of(-0.5)) < 40,
        "the old line no longer draws"
    );
    assert!(
        max_channel(&renderer, 32, row_of(0.5)) > 40,
        "the moved line draws at the new position"
    );
}

#[test]
fn instanced_line_overwrite_moves_ink_without_reuploading_a_stream() {
    let Some(gpu) = gpu_or_skip("instanced line overwrite") else {
        return;
    };
    let mut renderer = init(&gpu, false, "instanced line overwrite");
    renderer.set_mesh_merging(false);
    commit(&mut renderer, &[line_batch(1, -0.5), line_batch(2, 0.25)]);
    render(&mut renderer);
    assert!(
        !renderer.batches[0].instanced.is_empty(),
        "an opaque line batch converts to an instance stream"
    );
    let uploads = renderer.instance_stream_uploads();
    let key = renderer.pick_key_for_batch(0).unwrap();
    renderer
        .set_pick_key_flags(key, cad_scene::FLAG_SELECTED)
        .expect("select the batch");
    let plan_before = renderer
        .draw_plan
        .as_ref()
        .map(|plan| plan.opaque().to_vec())
        .expect("a render built the draw plan");
    let plan_rebuilds = renderer.draw_plan_rebuilds();
    let segments =
        renderer.instance_streams[renderer.batches[0].instanced[0].stream as usize].instance_count;

    renderer
        .overwrite_batch(0, &line_batch(1, 0.5))
        .expect("the same segment count repacks against the stored origin");
    render(&mut renderer);

    assert_eq!(
        renderer.instance_stream_uploads(),
        uploads,
        "an overwrite never allocates/uploads a stream"
    );
    assert_eq!(
        renderer.draw_plan_rebuilds(),
        plan_rebuilds,
        "the cached draw plan is not rebuilt"
    );
    assert_eq!(
        renderer
            .draw_plan
            .as_ref()
            .map(|plan| plan.opaque().to_vec()),
        Some(plan_before),
        "the frame's draw plan / batch order is unchanged"
    );
    assert_eq!(
        renderer.pick_key_flags(key) & cad_scene::FLAG_SELECTED,
        cad_scene::FLAG_SELECTED,
        "the selection flag still resolves to the same pick key"
    );
    assert_eq!(renderer.instance_streams.len(), 1, "no stream is added");
    assert_eq!(
        renderer.instance_streams[renderer.batches[0].instanced[0].stream as usize].instance_count,
        segments,
        "the stream's instance count is unchanged"
    );
    assert_eq!(renderer.pick_key_for_batch(0), Some(key));
    assert!(
        max_channel(&renderer, 32, row_of(-0.5)) < 40,
        "the old line no longer draws"
    );
    assert!(
        max_channel(&renderer, 32, row_of(0.5)) > 40,
        "the moved line draws at the new position"
    );
}

#[test]
fn accelerated_overwrite_updates_the_mirror_and_the_pulled_records_together() {
    let Some(gpu) = gpu_or_skip("accelerated overwrite") else {
        return;
    };
    let mut renderer = init(&gpu, true, "accelerated overwrite");
    renderer.set_mesh_merging(false);
    commit(&mut renderer, &[line_batch(1, -0.5), line_batch(2, 0.25)]);
    render(&mut renderer);
    assert!(
        renderer.pulled_lines.is_some(),
        "the accelerated tier must build the pulled line storage"
    );
    let record_count = renderer.pulled_line_instance_count();
    let key = renderer.pick_key_for_batch(0).unwrap();
    let run = renderer.batches[0].instanced[0];
    assert_eq!(
        renderer.pulled_lines.as_ref().unwrap().segments[0].instance_count,
        record_count,
        "one pulled segment covers the whole scene"
    );

    renderer
        .overwrite_batch(0, &line_batch(1, 0.5))
        .expect("a same-shape accelerated batch overwrites in place");

    // The CPU mirror is what a later pushed rebuild re-encodes.
    let stream = &renderer.instance_streams[run.stream as usize];
    let first = (stream.base + run.offset) as usize;
    assert_eq!(
        stream.pulled_instances[first].p0_width[1], 0.5,
        "the stream mirror holds the moved y"
    );
    assert_eq!(
        stream.pulled_instances[first].id_flags[0], key,
        "the mirrored instance keeps the pick key"
    );
    assert_eq!(
        renderer.pulled_line_instance_count(),
        record_count,
        "the pulled record count is unchanged"
    );
    render(&mut renderer);
    assert!(
        max_channel(&renderer, 32, row_of(0.5)) > 40,
        "the accelerated draw shows the moved line"
    );
}

#[test]
fn accelerated_overwrite_resolves_the_record_range_in_a_later_pulled_segment() {
    let Some(gpu) = gpu_or_skip("multi-segment pulled overwrite") else {
        return;
    };
    let mut renderer = init(&gpu, true, "multi-segment pulled overwrite");
    // One stream per pulled segment, and two spatially separate streams, so the
    // second batch's records start at whole-scene record index 1.
    renderer.set_pulled_line_segment_elems_override(Some(1));
    let mut far = line_batch(2, 0.0);
    // Beyond 2x `MAX_STREAM_RADIUS`, so the two batches cannot share a chunk
    // (a union box would exceed the radius) and each gets its own stream.
    far.local_origin = Point3 {
        x: 0.0,
        y: 20_000.0,
        z: 0.0,
    };
    commit(&mut renderer, &[line_batch(1, -0.5), far.clone()]);
    render(&mut renderer);
    assert!(renderer.pulled_lines.is_some(), "the pulled class is live");
    assert_eq!(
        renderer.pulled_line_segment_count(),
        2,
        "one stream per segment makes the second batch live in a later segment"
    );
    let run = renderer.batches[1].instanced[0];
    assert_eq!(run.stream, 1, "the far batch is the second stream");

    let mut moved = far;
    moved.vertices = vec![[-0.9, 0.25, 0.0], [0.9, 0.25, 0.0]];
    renderer
        .overwrite_batch(1, &moved)
        .expect("a far batch re-packs against its own stream origin");
    assert_eq!(
        renderer.instance_streams[1].pulled_instances[0].p0_width[1], 0.25,
        "the second stream's mirror holds the moved y"
    );
    render(&mut renderer);
    // If the record index resolved to 0 (the first segment) the first batch's
    // record would have been clobbered with this batch's 0.25. Neither may happen.
    assert!(
        max_channel(&renderer, 32, row_of(-0.5)) > 40,
        "the first segment's geometry is untouched"
    );
    assert_eq!(
        max_channel(&renderer, 32, row_of(0.25)),
        0,
        "nothing was written into the first segment's record"
    );
}

#[test]
fn every_refusal_gate_names_its_reason_and_writes_nothing() {
    let Some(gpu) = gpu_or_skip("overwrite refusals") else {
        return;
    };
    // No device at all.
    let mut detached = Renderer::default();
    assert_eq!(
        detached.overwrite_batch(0, &line_batch(1, 0.0)),
        Err(crate::BatchOverwriteRefusal::NotInitialized)
    );

    // Legacy line batch for the shape/count gates.
    let mut renderer = init(&gpu, false, "overwrite refusals");
    renderer.set_line_instancing(false);
    renderer.set_mesh_merging(false);
    commit(&mut renderer, &[line_batch(1, -0.5)]);
    assert_eq!(
        renderer.overwrite_batch(9, &line_batch(1, 0.0)),
        Err(crate::BatchOverwriteRefusal::OutOfRange {
            index: 9,
            batches: 1
        }),
        "an out-of-range index is a refusal, not an append"
    );
    assert_eq!(
        renderer.overwrite_batch(0, &quad_mesh(1, 0.25)),
        Err(crate::BatchOverwriteRefusal::TopologyChanged)
    );
    let mut four_vertices = line_batch(1, 0.0);
    four_vertices.vertices = vec![
        [-0.9, 0.0, 0.0],
        [0.9, 0.0, 0.0],
        [-0.9, 0.2, 0.0],
        [0.9, 0.2, 0.0],
    ];
    assert_eq!(
        renderer.overwrite_batch(0, &four_vertices),
        Err(crate::BatchOverwriteRefusal::VertexCountChanged {
            existing: 2,
            rebuilt: 4
        })
    );
    let mut transparent = line_batch(1, 0.0);
    transparent.alpha = 0.5;
    assert_eq!(
        renderer.overwrite_batch(0, &transparent),
        Err(crate::BatchOverwriteRefusal::AlphaClassChanged)
    );
    let mut reordered = line_batch(1, 0.0);
    reordered.draw_order = 99;
    assert_eq!(
        renderer.overwrite_batch(0, &reordered),
        Err(crate::BatchOverwriteRefusal::DrawOrderChanged)
    );
    let mut mirrored = line_batch(1, 0.0);
    mirrored.mirrored = true;
    assert_eq!(
        renderer.overwrite_batch(0, &mirrored),
        Err(crate::BatchOverwriteRefusal::MirroredChanged)
    );
    assert_eq!(
        renderer.batch_overwrites(),
        0,
        "no refused call may count as an overwrite"
    );

    // Instanced batch for the run-list and origin gates.
    let mut instanced = init(&gpu, false, "overwrite refusals");
    commit(&mut instanced, &[line_batch(1, -0.5), line_batch(2, 0.25)]);
    instanced.batches[0].instanced[0].count += 1;
    assert_eq!(
        instanced.overwrite_batch(0, &line_batch(1, 0.5)),
        Err(crate::BatchOverwriteRefusal::RunListChanged),
        "a run list that no longer covers the batch is refused"
    );
    let mut far = init(&gpu, false, "overwrite refusals");
    commit(&mut far, &[line_batch(1, -0.5), line_batch(2, 0.25)]);
    let mut far_away = line_batch(1, 0.5);
    far_away.vertices = vec![[-0.9, 9000.0, 0.0], [0.9, 9000.0, 0.0]];
    assert_eq!(
        far.overwrite_batch(0, &far_away),
        Err(crate::BatchOverwriteRefusal::StreamOriginChanged),
        "a move past the stream radius would need a different origin"
    );

    // Merged mesh.
    let mut merged = init(&gpu, false, "overwrite refusals");
    merged.set_mesh_merging(true);
    commit(&mut merged, &[quad_mesh(1, 0.25)]);
    assert!(
        !merged.batches[0].mesh_runs.is_empty(),
        "the quad merged into a chunk"
    );
    assert_eq!(
        merged.overwrite_batch(0, &quad_mesh(1, 0.2)),
        Err(crate::BatchOverwriteRefusal::MeshNotSupported)
    );

    // SDF conic stream.
    let mut conic = init(&gpu, false, "overwrite refusals");
    commit(&mut conic, &[circle_batch(1)]);
    assert!(
        !conic.batches[0].arc_runs.is_empty(),
        "the circle converted to the conic stream"
    );
    assert_eq!(
        conic.overwrite_batch(0, &circle_batch(1)),
        Err(crate::BatchOverwriteRefusal::ConicNotUpdatable)
    );
}

#[test]
fn an_unresolvable_pulled_mapping_is_refused_rather_than_written() {
    let Some(gpu) = gpu_or_skip("pulled mapping refusal") else {
        return;
    };
    let mut renderer = init(&gpu, true, "pulled mapping refusal");
    commit(&mut renderer, &[line_batch(1, -0.5), line_batch(2, 0.25)]);
    let pulled = renderer.pulled_lines.as_mut().expect("pulled lines");
    // Simulate an incoherent retained segment count (the state the mapping walk
    // must never guess about): it no longer equals the streams it covers.
    pulled.segments[0].instance_count += 1;
    assert_eq!(
        renderer.overwrite_batch(0, &line_batch(1, 0.5)),
        Err(crate::BatchOverwriteRefusal::PulledNotUpdatable),
        "an unresolvable pulled range must never be written partially"
    );
    assert_eq!(renderer.batch_overwrites(), 0);
}
