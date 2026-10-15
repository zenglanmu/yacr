//! Accelerated-tier integration fences (native only).
//!
//! The accelerated tier collapses the converted opaque classes to **one draw per
//! class** by pulling primitives from read-only vertex-stage storage buffers
//! (see `docs/adr/0007-entity-as-data-renderer.md` and
//! `docs/picking-gpu-id-pass.md`). It is capability-gated and chosen once per
//! device; the universal per-run path stays the default fallback.
//!
//! These fences run the **same scene on both tiers** and assert byte-identical
//! pixels wherever the arithmetic is identical, plus the draw collapse, the
//! automatic downgrade and the static-stream no-reupload invariant. The
//! universal-path fences live in `tests/render_effects.rs` / `tests/pick_id.rs`
//! (which pin the universal tier explicitly).
//!
//! Each test creates a real wgpu device on the adapter `create_headless_gpu`
//! selects (default Vulkan ICD → the project's Quadro P620; Mesa lavapipe on a
//! GPU-less machine). The adapter is printed, never assumed. Tests skip with a
//! clear message when no adapter exists or the device lacks vertex-stage storage
//! buffers.

use cad_domain::{DocumentId, EntityId, InstancePath, Point3, SelectionRef, TaskStamp};
use cad_render_wgpu::headless::{create_headless_gpu, HeadlessGpu};
use cad_render_wgpu::{
    BackendPreference, Camera2d, Camera3d, FrameStats, RenderTarget, RenderTier, Renderer,
    FLAG_HIDDEN, FLAG_SELECTED,
};
use cad_scene::{RenderBatch, RenderTopology, SceneDelta};

fn source(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn gpu() -> Option<HeadlessGpu> {
    match create_headless_gpu(BackendPreference::Auto) {
        Ok(gpu) => Some(gpu),
        Err(error) => {
            eprintln!("skipping accelerated-tier test: {error}");
            None
        }
    }
}

/// Whether this device can host the accelerated tier (vertex-stage storage
/// buffers). A device that cannot is skipped, not failed: the downgrade itself
/// is fenced by `capability_fallback_*` and the pure `tier_for` unit tests.
fn supports_storage(gpu: &HeadlessGpu) -> bool {
    let limits = gpu.device.limits();
    limits.max_storage_buffers_per_shader_stage >= 1
        && limits.max_storage_buffer_binding_size >= 64 * 1024
        && gpu.adapter.backend != "webgl2"
}

/// Initialize a renderer on a forced tier (so the test is deterministic on any
/// adapter), printing the adapter once.
fn init_tier(gpu: &HeadlessGpu, accelerated: bool) -> Renderer {
    let mut renderer = Renderer::new(BackendPreference::Auto);
    renderer.set_storage_buffer_support_override(Some(accelerated));
    renderer
        .initialize_with_device(gpu.device.clone(), gpu.queue.clone())
        .expect("initialize renderer");
    renderer
}

fn scene(batches: Vec<RenderBatch>) -> SceneDelta {
    SceneDelta {
        stamp: TaskStamp::new(DocumentId(1), 0),
        added: batches,
        removed_chunks: Vec::new(),
        images: Vec::new(),
    }
}

fn camera_2d() -> Camera2d {
    Camera2d {
        center: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        world_per_px: 2.0 / 64.0,
        z_plane: 0.0,
    }
}

fn camera_2d_at(cx: f64, cy: f64, world_per_px: f64) -> Camera2d {
    Camera2d {
        center: Point3 {
            x: cx,
            y: cy,
            z: 0.0,
        },
        world_per_px,
        z_plane: 0.0,
    }
}

fn camera_3d() -> Camera3d {
    Camera3d {
        eye: Point3 {
            x: 0.0,
            y: 0.0,
            z: 4.0,
        },
        target: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        up: Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
        fov_y: std::f64::consts::FRAC_PI_4,
        near: 0.1,
        far: 100.0,
    }
}

fn lines_batch(vertices: Vec<[f32; 3]>) -> RenderBatch {
    RenderBatch {
        local_origin: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        topology: RenderTopology::Lines,
        vertices,
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
        sources: vec![source(1)],
        draw_order: 0,
    }
}

/// A `Lines` batch at an explicit `f64` origin (for the large-coordinate fence).
fn lines_batch_at(origin: [f64; 3], vertices: Vec<[f32; 3]>) -> RenderBatch {
    let mut batch = lines_batch(vertices);
    batch.local_origin = Point3 {
        x: origin[0],
        y: origin[1],
        z: origin[2],
    };
    batch
}

fn quad_mesh(
    half: f32,
    origin: Point3,
    alpha: f32,
    normal: [f32; 3],
    draw_order: i64,
    mirrored: bool,
) -> RenderBatch {
    RenderBatch {
        local_origin: origin,
        topology: RenderTopology::Mesh,
        vertices: vec![
            [-half, -half, 0.0],
            [-half, half, 0.0],
            [half, half, 0.0],
            [half, -half, 0.0],
        ],
        normals: vec![normal; 4],
        colors: Vec::new(),
        indices: vec![[0, 2, 1], [0, 3, 2]],
        edges: Vec::new(),
        mirrored,
        alpha,
        color: cad_scene::DEFAULT_BATCH_COLOR,
        color_unresolved: true,
        lineweight: 0.0,
        lineweight_unresolved: true,
        linetype: cad_scene::LinetypePattern::continuous(),
        linetype_unresolved: true,
        sources: vec![source(1)],
        draw_order,
    }
}

fn mesh_at(
    half: f32,
    x: f64,
    y: f64,
    mirrored: bool,
    entity: u128,
    draw_order: i64,
) -> RenderBatch {
    let mut batch = quad_mesh(
        half,
        Point3 { x, y, z: 0.0 },
        1.0,
        [0.0, 0.0, 1.0],
        draw_order,
        mirrored,
    );
    batch.sources = vec![source(entity)];
    batch
}

/// Non-overlapping 2-vertex segments, one batch each.
fn grid_line_batches(rows: usize, cols: usize) -> Vec<RenderBatch> {
    let mut batches = Vec::with_capacity(rows * cols);
    for row in 0..rows {
        for col in 0..cols {
            let x = -0.9 + col as f32 * 0.22;
            let y = -0.9 + row as f32 * 0.22;
            batches.push(lines_batch(vec![[x, y, 0.0], [x + 0.15, y + 0.15, 0.0]]));
        }
    }
    batches
}

/// Non-overlapping quads on a grid, one batch each.
fn quad_grid(rows: usize, cols: usize, mirrored: bool) -> Vec<RenderBatch> {
    let mut batches = Vec::new();
    for row in 0..rows {
        for col in 0..cols {
            let x = -0.9 + col as f64 * 0.3;
            let y = -0.9 + row as f64 * 0.3;
            batches.push(mesh_at(
                0.08,
                x,
                y,
                mirrored,
                (row * cols + col + 1) as u128,
                0,
            ));
        }
    }
    batches
}

fn upload_and_render(
    renderer: &mut Renderer,
    target: &RenderTarget,
    batches: &[RenderBatch],
    camera: Camera2d,
) -> (Vec<u8>, FrameStats) {
    renderer.clear_batches();
    renderer
        .upload(&scene(batches.to_vec()))
        .expect("upload scene");
    let stats = renderer.render(camera, target).expect("render frame");
    let pixels = renderer.read_target_rgba().expect("read frame").pixels;
    (pixels, stats)
}

/// Render `batches` on both tiers and assert the frames are byte-identical,
/// returning the accelerated frame's stats.
fn assert_tiers_identical(
    gpu: &HeadlessGpu,
    target: &RenderTarget,
    batches: &[RenderBatch],
    camera: Camera2d,
    what: &str,
) -> FrameStats {
    let mut accelerated = init_tier(gpu, true);
    let mut universal = init_tier(gpu, false);
    let (accel_px, accel_stats) = upload_and_render(&mut accelerated, target, batches, camera);
    let (univ_px, univ_stats) = upload_and_render(&mut universal, target, batches, camera);
    assert_eq!(accel_stats.tier, RenderTier::Accelerated);
    assert_eq!(univ_stats.tier, RenderTier::Universal);
    assert!(
        accel_stats.accelerated,
        "{what}: the accelerated frame must actually use the per-class draws"
    );
    assert!(
        accel_px
            .chunks_exact(4)
            .any(|px| px != [accel_px[0], accel_px[1], accel_px[2], accel_px[3]]),
        "{what}: the accelerated frame is blank; the comparison would be vacuous"
    );
    assert_eq!(
        accel_px, univ_px,
        "{what}: accelerated and universal frames must be byte-identical"
    );
    accel_stats
}

#[test]
fn default_tier_is_accelerated_on_a_storage_capable_device() {
    let Some(gpu) = gpu() else {
        return;
    };
    eprintln!(
        "accelerated-tier adapter: {} ({}, {})",
        gpu.adapter.name, gpu.adapter.backend, gpu.adapter.device_type
    );
    let mut renderer = Renderer::new(BackendPreference::Auto);
    renderer
        .initialize_with_device(gpu.device.clone(), gpu.queue.clone())
        .expect("initialize renderer");
    if supports_storage(&gpu) {
        assert_eq!(
            renderer.render_tier(),
            RenderTier::Accelerated,
            "a storage-buffer-capable device must select the accelerated tier automatically"
        );
    } else {
        assert_eq!(
            renderer.render_tier(),
            RenderTier::Universal,
            "a device without storage buffers must downgrade automatically"
        );
    }
}

#[test]
fn capability_fallback_downgrades_and_renders_identically() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        eprintln!("skipping: device has no vertex-stage storage buffers");
        return;
    }
    let target = RenderTarget::new(64, 64);
    let batches = vec![
        lines_batch(vec![[-0.9, -0.5, 0.0], [0.9, -0.5, 0.0]]),
        lines_batch(vec![[-0.9, 0.5, 0.0], [0.9, 0.5, 0.0]]),
        mesh_at(0.2, 0.0, 0.0, false, 7, 0),
    ];
    // The downgrade is silent (no error) and labelled by the tier accessor.
    let mut downgraded = init_tier(&gpu, false);
    let (downgraded_px, downgraded_stats) =
        upload_and_render(&mut downgraded, &target, &batches, camera_2d());
    assert_eq!(downgraded.render_tier(), RenderTier::Universal);
    assert!(!downgraded_stats.accelerated);
    assert_eq!(downgraded_stats.accelerated_draws, 0);

    let mut accelerated = init_tier(&gpu, true);
    let (accelerated_px, accelerated_stats) =
        upload_and_render(&mut accelerated, &target, &batches, camera_2d());
    assert_eq!(accelerated.render_tier(), RenderTier::Accelerated);
    assert!(accelerated_stats.accelerated);
    assert_eq!(
        downgraded_px, accelerated_px,
        "the automatic downgrade must not change a single pixel"
    );
}

#[test]
fn accelerated_collapses_many_line_streams_to_one_draw() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let batches = grid_line_batches(6, 6); // 36 non-overlapping segments

    // Force one segment per stream so the universal path needs many runs.
    let mut accelerated = init_tier(&gpu, true);
    accelerated.set_max_instances_per_stream(1);
    let (accel_px, accel_stats) =
        upload_and_render(&mut accelerated, &target, &batches, camera_2d());
    assert!(
        accelerated.instance_stream_count() > 1,
        "the scene must be split into several chunks to make the collapse meaningful"
    );
    assert_eq!(accel_stats.tier, RenderTier::Accelerated);
    assert!(accel_stats.accelerated);
    assert_eq!(
        accel_stats.instanced_draws, 1,
        "every line chunk must collapse into ONE accelerated draw"
    );
    assert_eq!(accel_stats.accelerated_draws, 1);
    assert_eq!(accel_stats.draw_calls, 1, "36 batches -> one class draw");
    assert_eq!(
        accelerated.pulled_line_segment_count(),
        1,
        "a small scene fits one storage segment"
    );
    assert_eq!(
        accelerated.pulled_line_instance_count() as usize,
        36,
        "the shared segment holds every instance"
    );

    let mut universal = init_tier(&gpu, false);
    universal.set_max_instances_per_stream(1);
    let (univ_px, univ_stats) = upload_and_render(&mut universal, &target, &batches, camera_2d());
    assert!(
        univ_stats.instanced_draws > 1,
        "the universal path draws one run per chunk ({} runs)",
        univ_stats.instanced_draws
    );
    assert_eq!(
        accel_px, univ_px,
        "the collapsed line draw must be byte-identical to the per-run draws"
    );
}

#[test]
fn accelerated_storage_is_one_segment_for_a_small_scene() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let mut renderer = init_tier(&gpu, true);
    let mut batches = grid_line_batches(4, 4);
    batches.extend(quad_grid(3, 3, false));
    renderer.clear_batches();
    renderer.upload(&scene(batches)).unwrap();
    // The line class shares one `VERTEX | STORAGE` segment with the per-run
    // instance buffer (no duplicate geometry); the mesh class has its own pulled
    // storage segment.
    assert_eq!(renderer.pulled_line_segment_count(), 1);
    assert_eq!(renderer.pulled_mesh_segment_count(), 1);
    let stats = renderer.render(camera_2d(), &target).unwrap();
    assert!(stats.accelerated);
    assert_eq!(
        stats.accelerated_draws,
        stats.instanced_draws + stats.mesh_draws,
        "every accelerated draw comes from a storage segment"
    );
}

#[test]
fn accelerated_lines_match_universal_pixels() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let batches = vec![
        lines_batch(vec![[-0.9, -0.5, 0.0], [0.9, -0.5, 0.0]]),
        lines_batch(vec![[-0.9, 0.5, 0.0], [0.9, 0.5, 0.0]]),
        lines_batch(vec![[0.0, -0.9, 0.0], [0.0, 0.9, 0.0]]),
    ];
    let stats = assert_tiers_identical(&gpu, &target, &batches, camera_2d(), "instanced lines");
    assert_eq!(stats.instanced_draws, 1);
}

#[test]
fn accelerated_merged_meshes_match_universal_pixels() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let batches = quad_grid(6, 6, false);
    let stats = assert_tiers_identical(&gpu, &target, &batches, camera_2d(), "merged meshes");
    assert_eq!(
        stats.mesh_draws, 1,
        "every merged chunk collapses to one draw"
    );
    assert_eq!(stats.accelerated_draws, 1);
}

#[test]
fn accelerated_mirrored_meshes_match_universal_pixels() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    // Non-overlapping, so the mirrored/non-mirrored group reorder cannot change a
    // tie (the reorder caveat is documented in the ADR).
    let mut batches = quad_grid(3, 3, false);
    batches.extend(quad_grid(3, 3, true));
    let stats = assert_tiers_identical(
        &gpu,
        &target,
        &batches,
        camera_2d(),
        "mirrored merged meshes",
    );
    assert!(
        stats.mesh_draws <= 2,
        "the mirrored and non-mirrored classes each collapse to one draw, got {}",
        stats.mesh_draws
    );
}

#[test]
fn accelerated_mixed_lines_and_meshes_match_universal_pixels() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let batches = vec![
        lines_batch(vec![[-0.9, 0.9, 0.0], [0.9, 0.9, 0.0]]),
        mesh_at(0.2, 0.0, 0.0, false, 3, 0),
        lines_batch(vec![[-0.9, -0.9, 0.0], [0.9, -0.9, 0.0]]),
    ];
    let stats = assert_tiers_identical(&gpu, &target, &batches, camera_2d(), "mixed classes");
    // One line class draw + one mesh class draw.
    assert_eq!(stats.draw_calls, 2);
}

#[test]
fn accelerated_lines_keep_precision_at_large_coordinates() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    // A drawing far from the world origin, drawn through a camera centred there.
    let origin = [1.0e7, 1.0e7, 0.0];
    let batches = vec![
        lines_batch_at(origin, vec![[-0.9, -0.5, 0.0], [0.9, -0.5, 0.0]]),
        lines_batch_at(origin, vec![[-0.9, 0.5, 0.0], [0.9, 0.5, 0.0]]),
    ];
    let camera = camera_2d_at(1.0e7, 1.0e7, 2.0 / 64.0);
    let stats = assert_tiers_identical(&gpu, &target, &batches, camera, "large-coordinate lines");
    assert_eq!(stats.instanced_draws, 1);
}

#[test]
fn accelerated_deep_zoom_mixed_origin_stays_within_the_bound() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(128, 128);
    // Two line batches with distinct origins inside one chunk (mixed origin),
    // zoomed in hard.
    let batches = vec![
        lines_batch_at(
            [1000.0, 0.0, 0.0],
            vec![[-0.5, -0.2, 0.0], [0.5, -0.2, 0.0]],
        ),
        lines_batch_at([1000.5, 0.0, 0.0], vec![[-0.5, 0.2, 0.0], [0.5, 0.2, 0.0]]),
    ];
    let camera = camera_2d_at(1000.0, 0.0, 4.0 / 128.0);
    assert_tiers_identical(&gpu, &target, &batches, camera, "deep-zoom mixed origin");
}

#[test]
fn accelerated_matches_universal_in_perspective() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let batches = vec![
        mesh_at(0.3, 0.0, 0.0, false, 1, 0),
        lines_batch(vec![[-0.9, -0.5, 0.0], [0.9, -0.5, 0.0]]),
    ];
    let mut accelerated = init_tier(&gpu, true);
    let mut universal = init_tier(&gpu, false);
    accelerated.clear_batches();
    accelerated.upload(&scene(batches.clone())).unwrap();
    universal.clear_batches();
    universal.upload(&scene(batches.clone())).unwrap();
    let accel = accelerated.render_3d(camera_3d(), &target).unwrap();
    let accel_px = accelerated.read_target_rgba().unwrap().pixels;
    let univ = universal.render_3d(camera_3d(), &target).unwrap();
    let univ_px = universal.read_target_rgba().unwrap().pixels;
    assert!(accel.accelerated);
    assert!(!univ.accelerated);
    assert_eq!(
        accel_px, univ_px,
        "the 3D perspective path must render byte-identically on both tiers"
    );
}

#[test]
fn accelerated_overlapping_ties_match_universal_pixels() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    // Coplanar overlapping quads and a line crossing them at equal depth: the
    // cross-class tie must resolve identically on both tiers.
    let batches = vec![
        mesh_at(0.6, 0.0, 0.0, false, 1, 0),
        mesh_at(0.4, 0.0, 0.0, false, 2, 0),
        lines_batch(vec![[-0.8, 0.0, 0.0], [0.8, 0.0, 0.0]]),
    ];
    assert_tiers_identical(&gpu, &target, &batches, camera_2d(), "overlapping ties");
}

#[test]
fn accelerated_selection_flags_tint_identically() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let batches = vec![
        lines_batch(vec![[-0.9, -0.5, 0.0], [0.9, -0.5, 0.0]]),
        mesh_at(0.4, 0.0, 0.3, false, 5, 0),
    ];
    let mut accelerated = init_tier(&gpu, true);
    let mut universal = init_tier(&gpu, false);
    accelerated.clear_batches();
    accelerated.upload(&scene(batches.clone())).unwrap();
    universal.clear_batches();
    universal.upload(&scene(batches.clone())).unwrap();
    // Flag the first resident key (the line batch) as selected on both.
    accelerated
        .set_entity_flags(&[(source(1), FLAG_SELECTED)])
        .expect("flag entity");
    universal
        .set_entity_flags(&[(source(1), FLAG_SELECTED)])
        .expect("flag entity");
    accelerated.set_selection_flags_enabled(true);
    universal.set_selection_flags_enabled(true);

    let _ = accelerated.render(camera_2d(), &target).unwrap();
    let accel_px = accelerated.read_target_rgba().unwrap().pixels;
    let _ = universal.render(camera_2d(), &target).unwrap();
    let univ_px = universal.read_target_rgba().unwrap().pixels;
    assert_eq!(
        accel_px, univ_px,
        "FLAG_SELECTED must tint identically on the pulled and universal shaders"
    );
    // The tint must be observable (a non-vacuous fence).
    let unselected = {
        let mut plain = init_tier(&gpu, true);
        let (px, _) = upload_and_render(&mut plain, &target, &batches, camera_2d());
        px
    };
    assert_ne!(accel_px, unselected, "the selected tint must be visible");
}

#[test]
fn accelerated_hidden_flag_discards_identically() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let batches = vec![mesh_at(0.5, 0.0, 0.0, false, 9, 0)];
    let mut accelerated = init_tier(&gpu, true);
    accelerated.clear_batches();
    accelerated.upload(&scene(batches.clone())).unwrap();
    accelerated
        .set_entity_flags(&[(source(9), FLAG_HIDDEN)])
        .expect("flag entity");
    accelerated.set_selection_flags_enabled(true);
    let stats = accelerated.render(camera_2d(), &target).unwrap();
    assert!(stats.accelerated);
    let pixels = accelerated.read_target_rgba().unwrap().pixels;
    let background = [pixels[0], pixels[1], pixels[2], pixels[3]];
    assert!(
        pixels.chunks_exact(4).all(|px| px == background.as_slice()),
        "a FLAG_HIDDEN mesh must be discarded by the pulled shader"
    );
}

#[test]
fn accelerated_camera_move_does_not_reupload_static_storage() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let batches = quad_grid(4, 4, false);
    let mut renderer = init_tier(&gpu, true);
    renderer.clear_batches();
    renderer.upload(&scene(batches)).unwrap();
    let (stream_uploads, chunk_uploads) = (
        renderer.instance_stream_uploads(),
        renderer.mesh_chunk_uploads(),
    );
    let _ = renderer.render(camera_2d(), &target).unwrap();
    for step in 0..10 {
        let camera = camera_2d_at(step as f64 * 0.1, step as f64 * 0.05, 2.0 / 64.0);
        let stats = renderer.render(camera, &target).unwrap();
        assert!(
            stats.accelerated,
            "the accelerated path must stay live across a pan"
        );
    }
    assert_eq!(
        renderer.instance_stream_uploads(),
        stream_uploads,
        "a camera move must not re-upload the static instance streams"
    );
    assert_eq!(
        renderer.mesh_chunk_uploads(),
        chunk_uploads,
        "a camera move must not re-upload the static merged chunks"
    );
}

#[test]
fn accelerated_pick_uses_the_universal_id_pass() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let batches = vec![mesh_at(0.9, 0.0, 0.0, false, 42, 0)];
    let mut renderer = init_tier(&gpu, true);
    renderer.clear_batches();
    renderer.upload(&scene(batches)).unwrap();
    let stats = renderer.render(camera_2d(), &target).unwrap();
    assert!(
        stats.accelerated,
        "the color frame uses the accelerated tier"
    );
    let hit = renderer
        .pick_at(32, 32)
        .expect("pick must not error")
        .expect("the centred mesh must be picked");
    assert_eq!(
        hit.source.as_ref().map(|s| s.entity),
        Some(EntityId(42)),
        "the id pass must resolve the entity the accelerated color pass drew"
    );
}

#[test]
fn accelerated_page_boundary_falls_back_to_universal_without_loss() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let batches = quad_grid(2, 4, false);
    let mut renderer = init_tier(&gpu, true);
    renderer.clear_batches();
    renderer.upload(&scene(batches.clone())).unwrap();

    // Reference: a full-budget frame uses the accelerated per-class draw.
    let reference_stats = renderer.render(camera_2d(), &target).unwrap();
    assert!(reference_stats.accelerated);
    assert_eq!(reference_stats.mesh_draws, 1);
    let reference = renderer.read_target_rgba().unwrap().pixels;

    // A page boundary inside the merged run forces the universal per-run
    // organization for correctness (the per-class draw cannot pull geometry the
    // budget held back); the accumulated frame must still equal the reference.
    // One accelerated merged quad charges its merged share (200 B) plus its
    // pulled share (64 B/vertex + 4 B/index = 280 B) = 480 B, so this budget
    // accepts exactly one batch per page.
    renderer.frame_budget = cad_scene::FrameBudget {
        max_vertices: usize::MAX,
        max_triangles: usize::MAX,
        max_bytes: 480,
    };
    renderer.set_progressive_rendering(true);
    let mut pages = 0;
    loop {
        let stats = renderer.render(camera_2d(), &target).unwrap();
        assert!(
            !stats.accelerated,
            "a paginated frame must fall back to the universal per-run path"
        );
        pages += 1;
        assert!(
            pages <= batches.len(),
            "progressive frame did not terminate"
        );
        if !renderer.frame_pending() {
            break;
        }
    }
    assert_eq!(pages, batches.len(), "one page per merged batch");
    assert_eq!(
        renderer.read_target_rgba().unwrap().pixels,
        reference,
        "the fallback pages must reconstruct the accelerated reference frame"
    );
}
