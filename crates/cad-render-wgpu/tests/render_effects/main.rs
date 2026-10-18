//! GPU integration tests for *rendering behaviour* beyond the non-blank checks
//! in `tests/headless_render.rs` (native only).
//!
//! Each test creates a real wgpu device on whatever adapter `create_headless_gpu`
//! (see [`gpu`], `BackendPreference::Auto`) selects — the default Vulkan ICD, so
//! on the project's machine that is the real NVIDIA Quadro P620, and on a
//! GPU-less machine the Mesa **lavapipe** software backend. Tests skip with a
//! clear message when no adapter exists. Evidence is labelled by the adapter the
//! run actually reported (`gpu.adapter`), never assumed. Assertions are about
//! deterministic, observable outcomes (statistics, relative pixel relationships,
//! byte-for-byte determinism) rather than exact driver colours.

use cad_domain::{DocumentId, EntityId, InstancePath, Point3, SelectionRef, TaskStamp};

use cad_render_wgpu::headless::{create_headless_gpu, encode_png, HeadlessGpu};

use cad_render_wgpu::{BackendPreference, Camera2d, Camera3d, RenderError, RenderTarget, Renderer};

use cad_scene::{FrameBudget, RenderBatch, RenderTopology, SceneDelta};

mod core;
mod instanced_lines;
mod merged_meshes;
fn source() -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(1),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

/// Create a headless device, or `None` (skipping the test) when no adapter is
/// visible. Mirrors `tests/headless_render.rs`.
fn gpu() -> Option<HeadlessGpu> {
    match create_headless_gpu(BackendPreference::Auto) {
        Ok(gpu) => Some(gpu),
        Err(error) => {
            eprintln!("skipping headless GPU test: {error}");
            None
        }
    }
}

fn init(gpu: HeadlessGpu) -> Renderer {
    let HeadlessGpu {
        device,
        queue,
        adapter: _,
    } = gpu;
    let mut renderer = Renderer::new(BackendPreference::Auto);
    // This suite's fences pin the **universal** per-run draw organization
    // (coalesced instanced/merged runs and their exact draw counts), so the
    // automatic accelerated-tier selection is disabled here. The accelerated
    // tier has its own fences in `tests/accelerated_tier.rs`.
    renderer.set_storage_buffer_support_override(Some(false));
    renderer
        .initialize_with_device(device, queue)
        .expect("initialize headless renderer");
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
    camera_2d_at(0.0, 0.0)
}

/// A 2D camera whose world→pixel scale is such that `[-1, 1]` spans the whole
/// 64×64 target, centred on `(cx, cy)`.
fn camera_2d_at(cx: f64, cy: f64) -> Camera2d {
    Camera2d {
        center: Point3 {
            x: cx,
            y: cy,
            z: 0.0,
        },
        world_per_px: 2.0 / 64.0,
        z_plane: 0.0,
    }
}

/// A 3D camera looking down `-Z` at the origin from `z = 4`, so a scene in the
/// `[-1, 1]²` plane fills the 64×64 target under a 45° vertical FOV.
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
        sources: vec![source()],
        draw_order: 0,
    }
}

/// A rectangle outline in world units, as `LineList` pairs.
fn rectangle_lines() -> Vec<[f32; 3]> {
    let (a, b) = (-0.8f32, 0.8f32);
    vec![
        [-a, -b, 0.0],
        [a, -b, 0.0],
        [a, -b, 0.0],
        [a, b, 0.0],
        [a, b, 0.0],
        [-a, b, 0.0],
        [-a, b, 0.0],
        [-a, -b, 0.0],
    ]
}

/// An axis-aligned square mesh, centred on `origin`, `half` world units wide.
///
/// Wound counter-clockwise in world/clip space, facing +Z.
fn quad_mesh(
    half: f32,
    origin: Point3,
    alpha: f32,
    normal: [f32; 3],
    draw_order: i64,
) -> RenderBatch {
    let v = vec![
        [-half, -half, 0.0],
        [-half, half, 0.0],
        [half, half, 0.0],
        [half, -half, 0.0],
    ];
    RenderBatch {
        conics: Vec::new(),
        local_origin: origin,
        topology: RenderTopology::Mesh,
        vertices: v,
        normals: vec![normal; 4],
        colors: Vec::new(),
        indices: vec![[0, 2, 1], [0, 3, 2]],
        edges: Vec::new(),
        mirrored: false,
        alpha,
        color: cad_scene::DEFAULT_BATCH_COLOR,
        color_unresolved: true,
        lineweight: 0.0,
        lineweight_unresolved: true,
        linetype: cad_scene::LinetypePattern::continuous(),
        linetype_unresolved: true,
        sources: vec![source()],
        draw_order,
    }
}

/// Batch origins for the transparency scenario. The `Mesh` shader is a fixed
/// headlight: normal `+Z` gives the full diffuse term, normal `+X` gives only
/// the ambient floor, so the overlay is a visibly different colour from the
/// background even though both use the same pipeline.
const FULL_COVER_NORMAL: [f32; 3] = [0.0, 0.0, 1.0];

const OVERLAY_NORMAL: [f32; 3] = [1.0, 0.0, 0.0];

/// Opaque full-cover square (farther, `z = +0.5`), a smaller overlapping
/// overlay (`z = 0`, nearer) with the requested alpha, and an `alpha = 0.0`
/// ghost that must be reported but never drawn.
fn transparency_scene(overlay_alpha: f32) -> SceneDelta {
    let cover = quad_mesh(
        0.95,
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.5,
        },
        1.0,
        FULL_COVER_NORMAL,
        0,
    );
    let overlay = quad_mesh(
        0.3,
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        overlay_alpha,
        OVERLAY_NORMAL,
        1,
    );
    let ghost = ghost_quad();
    scene(vec![cover, overlay, ghost])
}

fn ghost_quad() -> RenderBatch {
    quad_mesh(
        0.3,
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        0.0,
        FULL_COVER_NORMAL,
        2,
    )
}

/// A line batch with an explicit colour, for the per-colour frame test.
fn colored_line_batch(color: [f32; 3]) -> RenderBatch {
    let mut batch = lines_batch(rectangle_lines());
    batch.color = color;
    batch.color_unresolved = false;
    batch
}

/// A dashed horizontal line, exactly as `cad-representation` emits it: each
/// dash is its own sub-polyline (and so its own batch), and the renderer only
/// ever sees ordinary two-vertex `LineList` segments.
fn dashed_horizontal_batches() -> Vec<RenderBatch> {
    vec![
        lines_batch(vec![[-0.9, 0.0, 0.0], [-0.3, 0.0, 0.0]]),
        lines_batch(vec![[0.3, 0.0, 0.0], [0.9, 0.0, 0.0]]),
    ]
}

/// A solid horizontal line over the same span.
fn solid_horizontal_lines() -> Vec<[f32; 3]> {
    vec![[-0.9, 0.0, 0.0], [0.9, 0.0, 0.0]]
}

// ---------------------------------------------------------------------------
// Instanced line streams ("entity = data", first slice)
// ---------------------------------------------------------------------------

/// Upload `batches` on the requested path, render one frame and read it back.
fn render_pixels(
    renderer: &mut Renderer,
    target: &RenderTarget,
    batches: &[RenderBatch],
    instanced: bool,
    camera: Camera2d,
) -> (Vec<u8>, cad_render_wgpu::FrameStats) {
    renderer.set_line_instancing(instanced);
    // `instanced = false` is the fully legacy per-batch baseline, so the mesh
    // flat-merge path is disabled with it (a mesh keeps its per-batch draw).
    renderer.set_mesh_merging(instanced);
    renderer.clear_batches();
    renderer
        .upload(&scene(batches.to_vec()))
        .expect("upload scene");
    let stats = renderer.render(camera, target).expect("render frame");
    let pixels = renderer.read_target_rgba().expect("read frame").pixels;
    (pixels, stats)
}

fn opaque_black(pixels: &[u8]) -> [u8; 4] {
    [pixels[0], pixels[1], pixels[2], pixels[3]]
}

/// Non-overlapping 2-vertex segments on an 8-column grid, one batch each. The
/// segments never share a pixel, so a single coalesced instanced draw has the
/// same fragment outcome as the same segments drawn individually.
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

// ---------------------------------------------------------------------------
// Instanced line streams — review-mandated fences (per-chunk origins, cap,
// tie-order, non-monotonic order, incremental commit/GC, perspective parity)
// ---------------------------------------------------------------------------

/// Number of pixels whose RGBA differs on any channel by more than `tolerance`.
fn differing_pixels(a: &[u8], b: &[u8], tolerance: u8) -> usize {
    assert_eq!(a.len(), b.len(), "frames must have the same size");
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(pa, pb)| {
            pa.iter()
                .zip(pb.iter())
                .any(|(x, y)| x.abs_diff(*y) > tolerance)
        })
        .count()
}

/// A one-segment *vertical* batch whose `local_origin` is its first vertex, as
/// the real import path produces (`origin = points.first()`); the segment is
/// 100 mm tall at world `x = base`. A vertical segment makes a horizontal
/// position error visible: it moves to a different pixel column instead of
/// sliding unnoticed along its own length.
fn world_segment(base: f64) -> RenderBatch {
    let mut batch = lines_batch(vec![[0.0, -50.0, 0.0], [0.0, 50.0, 0.0]]);
    batch.local_origin = Point3 {
        x: base,
        y: 0.0,
        z: 0.0,
    };
    batch
}

/// Eight clusters of six vertical objects, each cluster's objects carrying
/// distinct `local_origin`s (so a cluster forms a mixed-origin chunk) and the
/// clusters spread across a ~1.4e7 mm (14 km) drawing.
fn large_extent_clusters() -> Vec<RenderBatch> {
    let mut batches = Vec::new();
    for cluster in 0..8 {
        let base_x = -8.0e6 + cluster as f64 * 2.0e6;
        for k in 0..6 {
            batches.push(world_segment(base_x + k as f64 * 1000.0));
        }
    }
    batches
}

// ---------------------------------------------------------------------------
// Flat-merged mesh runs (Task A) + instanced edge overlays (Task B)
// ---------------------------------------------------------------------------

/// Upload `batches`, render one frame and read it back, with both conversion
/// paths toggled explicitly. `line_instancing = mesh_merging = false` is the
/// fully legacy per-batch baseline.
fn render_pixels_full(
    renderer: &mut Renderer,
    target: &RenderTarget,
    batches: &[RenderBatch],
    line_instancing: bool,
    mesh_merging: bool,
    camera: Camera2d,
) -> (Vec<u8>, cad_render_wgpu::FrameStats) {
    renderer.set_line_instancing(line_instancing);
    renderer.set_mesh_merging(mesh_merging);
    renderer.clear_batches();
    renderer
        .upload(&scene(batches.to_vec()))
        .expect("upload scene");
    let stats = renderer.render(camera, target).expect("render frame");
    let pixels = renderer.read_target_rgba().expect("read frame").pixels;
    (pixels, stats)
}

/// An axis-aligned quad mesh at `origin`, uniform per-vertex `color`, optional
/// wireframe overlay, with a distinct source entity id.
fn merged_quad(
    half: f32,
    origin: [f64; 3],
    color: [f32; 3],
    entity: u128,
    edges: bool,
    draw_order: i64,
) -> RenderBatch {
    let edge_positions = if edges {
        vec![[0.0, 0.0, 0.0], [0.0, 0.0, 0.0]]
    } else {
        Vec::new()
    };
    RenderBatch {
        conics: Vec::new(),
        local_origin: Point3 {
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
        colors: vec![color; 4],
        indices: vec![[0, 2, 1], [0, 3, 2]],
        edges: edge_positions,
        mirrored: false,
        alpha: 1.0,
        color: cad_scene::DEFAULT_BATCH_COLOR,
        color_unresolved: true,
        lineweight: 0.0,
        lineweight_unresolved: true,
        linetype: cad_scene::LinetypePattern::continuous(),
        linetype_unresolved: true,
        sources: vec![SelectionRef {
            document: DocumentId(1),
            entity: EntityId(entity),
            instance: InstancePath::default(),
            sub_element: None,
        }],
        draw_order,
    }
}

/// Non-overlapping quads on a grid, one batch each, so no two triangles share a
/// rasterized pixel. This fences the merged-vs-legacy *path* equality (baked
/// tint, per-vertex colour, upload order) without depending on coplanar fragment
/// order; the overlapping-tie and 3D perspective fences below cover the hard
/// cases this grid deliberately avoids.
fn quad_grid_batches(rows: usize, cols: usize, edges: bool) -> Vec<RenderBatch> {
    let mut batches = Vec::with_capacity(rows * cols);
    for row in 0..rows {
        for col in 0..cols {
            let x = -0.9 + col as f64 * 0.3;
            let y = -0.9 + row as f64 * 0.3;
            let color = [
                (col as f32 + 1.0) / (cols as f32 + 1.0),
                (row as f32 + 1.0) / (rows as f32 + 1.0),
                0.5,
            ];
            batches.push(merged_quad(
                0.08,
                [x, y, 0.0],
                color,
                (row * cols + col + 1) as u128,
                edges,
                0,
            ));
        }
    }
    batches
}

/// A clockwise-wound quad marked `mirrored`, which the `front_face = Cw` merged
/// pipeline must still cull correctly. (A CCW quad would be culled as a back
/// face, exactly as on the legacy mirrored mesh pipeline.)
fn mirrored_quad(origin: [f64; 3], color: [f32; 3], entity: u128) -> RenderBatch {
    let mut batch = merged_quad(0.2, origin, color, entity, false, 0);
    batch.mirrored = true;
    // Clockwise winding in world space, so `Cw` is the front face.
    batch.indices = vec![[0, 1, 2], [0, 2, 3]];
    batch
}
