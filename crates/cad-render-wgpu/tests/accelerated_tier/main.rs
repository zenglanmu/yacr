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

mod tier_basics;
mod tier_edges;
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
