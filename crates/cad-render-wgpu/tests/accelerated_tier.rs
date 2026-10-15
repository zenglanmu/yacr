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
use cad_scene::{RenderBatch, RenderConic, RenderTopology, SceneDelta};

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
    // Three vertex-stage storage bindings is the true requirement (the pulled
    // merged-mesh pipeline binds vertices, indices and the chunk table).
    limits.max_storage_buffers_per_shader_stage >= 3
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
        conics: Vec::new(),
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
        conics: Vec::new(),
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

/// A horizontal band of `rows` segments at `y`, spanning most of the target, so
/// a scanline is guaranteed to rasterize (a single zero-width line through a
/// pixel centre is not reliably covered on every rasterizer).
fn line_band(y: f32, rows: u32, entity: u128) -> RenderBatch {
    let mut vertices = Vec::new();
    for row in 0..rows {
        let offset = y + row as f32 * 0.02;
        vertices.push([-0.9, offset, 0.0]);
        vertices.push([0.9, offset, 0.0]);
    }
    let mut batch = lines_batch(vertices);
    batch.sources = vec![source(entity)];
    batch
}

/// The first **core** pixel the colour pass drew within `y0..y1`, scanning
/// row-major. Used to pick a pixel the accelerated colour pass covered at high
/// (pickable) coverage: with anti-aliased line strokes the outermost fringe can
/// carry a few units of ink but sits below the id pass's 0.5-coverage pick
/// threshold, so a `> 8` scan would pick an unpickable fringe pixel.
fn drawn_pixel_in_rows(
    pixels: &[u8],
    width: u32,
    height: u32,
    y0: u32,
    y1: u32,
) -> Option<(u32, u32)> {
    for y in y0..y1.min(height) {
        for x in 0..width {
            let i = (y as usize * width as usize + x as usize) * 4;
            if pixels[i] > 128 || pixels[i + 1] > 128 || pixels[i + 2] > 128 {
                return Some((x, y));
            }
        }
    }
    None
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
    // Every mesh vertex is resident in the pulled storage (3x3 quads -> 36).
    assert_eq!(
        renderer.pulled_mesh_vertex_count(),
        36,
        "the pulled mesh storage must cover every resident merged vertex"
    );
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

/// B1 fence: on the accelerated tier every stream in a storage segment shares
/// one buffer, so a line run's global instance index is `base + offset`. Two
/// streams in one segment with distinct entities must each pick their own
/// entity; reading `offset` alone (stream-local) would resolve both to the first
/// stream's key.
#[test]
fn accelerated_multi_stream_line_pick_reads_the_right_instance() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(96, 96);
    // Two bands at distinct y, three segments each so a scanline is reliably
    // covered. `set_max_instances_per_stream(1)` makes every segment its own
    // stream (six streams), and the whole scene fits one storage segment, so the
    // second band's runs have a non-zero `base`.
    let batches = vec![line_band(0.5, 3, 0xA1), line_band(-0.5, 3, 0xB2)];
    let mut renderer = init_tier(&gpu, true);
    renderer.set_max_instances_per_stream(1);
    let (pixels, stats) = upload_and_render(&mut renderer, &target, &batches, camera_2d());
    assert!(
        stats.accelerated,
        "the accelerated per-class draw must be live for this fence"
    );
    assert_eq!(
        renderer.instance_stream_count(),
        6,
        "one stream per segment"
    );
    assert_eq!(
        renderer.pulled_line_segment_count(),
        1,
        "all six streams must share ONE storage segment for base re-basing to matter"
    );

    let top = drawn_pixel_in_rows(&pixels, 96, 96, 0, 48).expect("the top band must rasterize");
    let bottom =
        drawn_pixel_in_rows(&pixels, 96, 96, 48, 96).expect("the bottom band must rasterize");
    // A band's first inked pixel can be an anti-aliased fringe below the id pass's
    // 0.5-coverage pick threshold, so scan the band for a **pickable** pixel: the
    // colour pass covered it and the id pass resolves it (the ±0.5 px AA-fringe
    // pick tolerance, matching `pick_arc`).
    let pickable = |renderer: &mut Renderer, y0: u32, y1: u32| -> Option<(u32, u32, u32)> {
        for y in y0..y1 {
            for x in 0..96u32 {
                let i = (y as usize * 96 + x as usize) * 4;
                if pixels[i] <= 8 && pixels[i + 1] <= 8 && pixels[i + 2] <= 8 {
                    continue;
                }
                if let Some(hit) = renderer.pick_at(x, y).expect("pick") {
                    return Some((x, y, hit.entity_key));
                }
            }
        }
        None
    };
    let (top_x, top_y, _) = pickable(&mut renderer, 0, 48).expect("the top band must be picked");
    let (bottom_x, bottom_y, _) =
        pickable(&mut renderer, 48, 96).expect("the bottom band must be picked");
    let top_hit = renderer
        .pick_at(top_x, top_y)
        .expect("pick")
        .expect("the top band must be picked");
    let bottom_hit = renderer
        .pick_at(bottom_x, bottom_y)
        .expect("pick")
        .expect("the bottom band must be picked");
    let _ = (top, bottom);
    assert_eq!(
        top_hit.source.expect("exact top").entity,
        EntityId(0xA1),
        "the top band must resolve to its own entity"
    );
    assert_eq!(
        bottom_hit.source.expect("exact bottom").entity,
        EntityId(0xB2),
        "the bottom band must resolve to its own entity (base + offset, not offset)"
    );
}

/// S4 fence: force the line class across several storage segments and assert the
/// per-segment accelerated draws are byte-identical to the universal per-run
/// frame.
#[test]
fn accelerated_forced_multi_segment_lines_match_universal_pixels() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let batches = grid_line_batches(6, 6); // 36 non-overlapping segments

    let mut accelerated = init_tier(&gpu, true);
    // 36 streams, split into segments of two: eighteen storage segments, each
    // drawn once, still covering every instance.
    accelerated.set_max_instances_per_stream(1);
    accelerated.set_pulled_line_segment_elems_override(Some(2));
    let (accel_px, accel_stats) =
        upload_and_render(&mut accelerated, &target, &batches, camera_2d());
    assert!(accel_stats.accelerated);
    assert!(
        accelerated.pulled_line_segment_count() > 1,
        "the override must force several storage segments, got {}",
        accelerated.pulled_line_segment_count()
    );
    assert_eq!(
        accelerated.pulled_line_instance_count() as usize,
        36,
        "every instance must still be resident across the segments"
    );
    assert_eq!(
        accel_stats.instanced_draws as usize,
        accelerated.pulled_line_segment_count(),
        "the accelerated frame must issue one draw per storage segment"
    );

    let mut universal = init_tier(&gpu, false);
    universal.set_max_instances_per_stream(1);
    let (univ_px, univ_stats) = upload_and_render(&mut universal, &target, &batches, camera_2d());
    assert!(
        univ_stats.instanced_draws > 1,
        "the universal path uses per-run draws"
    );
    assert_eq!(
        accel_px, univ_px,
        "the multi-segment accelerated frame must match the universal frame"
    );
}

/// B2 fence: an oversized line stream at reduced limits must not emit an invalid
/// storage binding. The class downgrades to the universal per-run path, whose
/// per-stream buffers must hold the real geometry (not the prepare-time
/// placeholder), and the frame is pixel-identical to the universal renderer.
#[test]
fn accelerated_oversized_line_stream_downgrades_without_validation_error() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    // One stream of 2048 instances, far past a 64 KiB binding's 1024 records.
    let mut vertices = Vec::new();
    for i in 0..2048u32 {
        let x = -0.9 + (i / 64) as f32 * 0.028;
        let y = -0.9 + (i % 64) as f32 * 0.028;
        vertices.push([x, y, 0.0]);
        vertices.push([x + 0.02, y + 0.02, 0.0]);
    }
    let batches = vec![lines_batch(vertices)];

    let mut renderer = init_tier(&gpu, true);
    renderer.set_pulled_binding_limit_override(Some(64 * 1024));
    let (pixels, stats) = upload_and_render(&mut renderer, &target, &batches, camera_2d());
    assert_eq!(
        renderer.render_tier(),
        RenderTier::Accelerated,
        "the device is still accelerated; only this class falls back"
    );
    assert!(
        !stats.accelerated,
        "an oversized stream must fall back to the universal per-run path, not accelerate"
    );
    assert_eq!(
        renderer.pulled_line_segment_count(),
        0,
        "no storage segment may be built for an oversized stream"
    );
    assert_eq!(
        renderer.instance_stream_count(),
        1,
        "one stream for the batch"
    );
    // The fallback must draw real geometry, not the prepare-time placeholder.
    let background = [pixels[0], pixels[1], pixels[2], pixels[3]];
    assert!(
        pixels.chunks_exact(4).any(|px| px != background.as_slice()),
        "the fallback frame is blank; the per-stream buffers held no real geometry"
    );

    let mut universal = init_tier(&gpu, false);
    let (univ_px, _) = upload_and_render(&mut universal, &target, &batches, camera_2d());
    assert_eq!(
        pixels, univ_px,
        "the reduced-limit fallback must draw the same geometry as the universal path"
    );
}

/// S2 fence: the lifetime `uploaded_bytes` counter must charge the accelerated
/// pulled geometry exactly once. A 2-segment line batch plus one merged quad
/// charges, at prepare, line `2*64` + merged `4*44 + 6*4` + pulled
/// `4*64 + 6*4`; the commit then adds only the two 16-byte chunk tables — not
/// the segment buffers a second time.
#[test]
fn accelerated_uploaded_bytes_charge_pulled_geometry_once() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let batches = vec![
        lines_batch(vec![
            [-0.9, -0.5, 0.0],
            [0.9, -0.5, 0.0],
            [-0.9, 0.5, 0.0],
            [0.9, 0.5, 0.0],
        ]),
        mesh_at(0.3, 0.0, 0.0, false, 3, 0),
    ];
    let mut renderer = init_tier(&gpu, true);
    let (_, stats) = upload_and_render(&mut renderer, &target, &batches, camera_2d());
    assert!(stats.accelerated);
    let line = 2 * 64;
    let merged = 4 * 44 + 6 * 4;
    let pulled = 4 * 64 + 6 * 4;
    let tables = 2 * 16; // one line storage segment + one mesh storage segment
    assert_eq!(
        stats.uploaded_bytes,
        (line + merged + pulled + tables) as u64,
        "pulled geometry must be charged once, not again on every rebuild"
    );
}

/// A one-conic SDF batch (opaque white by default).
fn conic_batch(conics: Vec<RenderConic>, entity: u128) -> RenderBatch {
    let origin = conics.first().map(|c| c.center).unwrap_or(Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    });
    RenderBatch {
        local_origin: origin,
        topology: RenderTopology::Conics,
        vertices: Vec::new(),
        normals: Vec::new(),
        colors: Vec::new(),
        indices: Vec::new(),
        edges: Vec::new(),
        conics,
        mirrored: false,
        alpha: 1.0,
        color: [1.0, 0.0, 0.0],
        color_unresolved: false,
        lineweight: 0.0,
        lineweight_unresolved: false,
        linetype: cad_scene::LinetypePattern::continuous(),
        linetype_unresolved: false,
        sources: vec![source(entity)],
        draw_order: 0,
    }
}

fn circle(center: Point3, radius: f64) -> RenderConic {
    RenderConic {
        center,
        major: Point3 {
            x: radius,
            y: 0.0,
            z: 0.0,
        },
        minor: Point3 {
            x: 0.0,
            y: radius,
            z: 0.0,
        },
        start: 0.0,
        sweep: std::f64::consts::TAU,
        full: true,
    }
}

/// S2: the accelerated tier's **residual** SDF conic path is exercised, and an
/// arc-vs-legacy-batch equal-depth tie still resolves by plan order (the S1
/// flush), so the accelerated frame matches the universal frame.
///
/// The ADR's "arc-vs-pulled-class tie exception" is about *pulled* classes; this
/// fence pins the residual path and a legacy-batch tie, which the S1 fix
/// (`flush_pending_arc` before the legacy `draw_batch`) makes order-correct.
#[test]
fn accelerated_sdf_arc_residual_path_and_legacy_tie_match_universal() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    // The circle (red, SDF) and a horizontal legacy `Lines` batch (default
    // colour) cross at the ring's two axis points at equal depth.
    let arc = conic_batch(
        vec![circle(
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            0.5,
        )],
        1,
    );
    // Three vertices -> not an instanced pair, so it stays a legacy per-batch
    // draw on the accelerated tier (the residual path's neighbour).
    let mut legacy = lines_batch(vec![[-0.9, 0.0, 0.0], [0.9, 0.0, 0.0], [0.9, 0.01, 0.0]]);
    legacy.color = [0.0, 0.0, 1.0];
    legacy.color_unresolved = false;
    // A converted (pulled) line class elsewhere, so the accelerated tier actually
    // engages its per-class pulled path (which the arc residual path sits beside).
    let pulled_line = lines_batch(vec![[-0.9, 0.8, 0.0], [0.9, 0.8, 0.0]]);

    let stats = assert_tiers_identical(
        &gpu,
        &target,
        &[arc, legacy, pulled_line],
        camera_2d(),
        "residual SDF arc + legacy tie",
    );
    assert!(
        stats.arc_draws > 0,
        "the accelerated frame must draw the SDF arc through the residual path"
    );
    assert_eq!(
        stats.arc_batches, 1,
        "one conic batch must go through the residual path"
    );
}

/// S2: the accelerated tier's documented **arc-vs-pulled-class** tie exception.
///
/// Pulled classes draw before the residual SDF conic class, so on the accelerated
/// tier an arc-vs-pulled-line equal-depth tie is always won by the arc (drawn
/// after, `LessEqual`), independent of plan order — whereas the universal tier
/// resolves the tie by plan order. This fence pins that difference (and therefore
/// the honesty of the ADR claim).
#[test]
fn accelerated_sdf_arc_vs_pulled_class_tie_is_the_documented_exception() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let wpp = 2.0 / 64.0;
    // World coordinates that are integer multiples of `wpp` land on pixel centres
    // (the ring's right point and the line's row are both centred).
    let cam = Camera2d {
        center: Point3 {
            x: 0.5 * wpp,
            y: 0.5 * wpp,
            z: 0.0,
        },
        world_per_px: wpp,
        z_plane: 0.0,
    };
    let origin = Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    let mut arc = conic_batch(vec![circle(origin, 0.5)], 1);
    arc.color = [1.0, 0.0, 0.0];
    let mut line = lines_batch(vec![[-0.9, 0.0, 0.0], [0.9, 0.0, 0.0]]);
    line.color = [0.0, 0.0, 1.0];
    line.color_unresolved = false;

    let sample = |renderer: &mut Renderer, batches: &[RenderBatch]| -> [u8; 4] {
        renderer.clear_batches();
        renderer
            .upload(&scene(batches.to_vec()))
            .expect("upload tie scene");
        renderer.render(cam, &target).expect("render tie scene");
        renderer.read_target_rgba().unwrap().pixel(47, 32)
    };

    let mut accel = init_tier(&gpu, true);
    let mut univ = init_tier(&gpu, false);
    let accel_arc_first = sample(&mut accel, &[arc.clone(), line.clone()]);
    let accel_line_first = sample(&mut accel, &[line.clone(), arc.clone()]);
    let univ_arc_first = sample(&mut univ, &[arc.clone(), line.clone()]);
    let univ_line_first = sample(&mut univ, &[line.clone(), arc.clone()]);

    // Accelerated: the residual arc is drawn after the pulled line, so it always
    // wins the tie (red), whatever the plan order.
    for (name, px) in [
        ("arc-first", accel_arc_first),
        ("line-first", accel_line_first),
    ] {
        assert!(
            px[0] > px[2].saturating_add(40),
            "accelerated ({name}): the residual arc must win the pulled-line tie (red), got {px:?}"
        );
    }
    // Universal: plan order decides (the later draw wins).
    assert!(
        univ_arc_first[2] > univ_arc_first[0].saturating_add(40),
        "universal (arc-first): the later line must win the tie (blue), got {univ_arc_first:?}"
    );
    assert!(
        univ_line_first[0] > univ_line_first[2].saturating_add(40),
        "universal (line-first): the later arc must win the tie (red), got {univ_line_first:?}"
    );
    // The exception: with the arc first in plan order the two tiers disagree.
    assert_ne!(
        accel_arc_first, univ_arc_first,
        "the accelerated tier's pulled-before-residual order is the documented tie exception"
    );
}
