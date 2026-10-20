//! GPU id-readback picking: id-pass correctness per draw class, non-interference
//! with the color pass, and the selection-flags machinery.
//!
//! These tests create a real wgpu device on whatever adapter is visible (on this
//! project's machine a real Vulkan GPU; the report line is the run's own adapter)
//! and assert exact identities, not just "something happened". When no adapter is
//! available they skip with a clear message rather than pretending to pass.

use cad_domain::{
    DocumentId, EntityId, InstancePath, Point3, Revision, SelectionRef, SubElementId, TaskStamp,
    Transform3,
};

use cad_render_wgpu::headless::{create_headless_gpu, HeadlessGpu};

use cad_render_wgpu::{
    BackendPreference, Camera2d, IdTargetFormat, RenderTarget, Renderer, FLAG_HIDDEN, FLAG_HOVERED,
    FLAG_SELECTED,
};

use cad_resources::{DecodedImage, DecodedImageCache, ResourceKey, ResourceLimits};

use cad_scene::{ImageBatch, RenderBatch, RenderTopology, SceneDelta};

mod entity_flags;
mod picking;
fn source(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

/// A source reached through an `INSERT` instance chain.
fn source_instance(entity: u128, instance: &[u128]) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath(instance.iter().copied().map(EntityId).collect()),
        sub_element: None,
    }
}

fn gpu() -> Option<HeadlessGpu> {
    match create_headless_gpu(BackendPreference::Auto) {
        Ok(gpu) => Some(gpu),
        Err(error) => {
            eprintln!("skipping headless GPU test: {error}");
            None
        }
    }
}

fn init(gpu: HeadlessGpu, format: Option<IdTargetFormat>) -> Renderer {
    let HeadlessGpu {
        device,
        queue,
        adapter,
    } = gpu;
    eprintln!(
        "id-pass test adapter: {} ({}, {})",
        adapter.name, adapter.backend, adapter.device_type
    );
    let mut renderer = Renderer::new(BackendPreference::Auto);
    renderer.set_id_format_override(format);
    // The id-pass fences compare against the universal color path's exact
    // pixels and draw counts; pin the universal tier here. The accelerated tier
    // has its own fences in `tests/accelerated_tier.rs`.
    renderer.set_storage_buffer_support_override(Some(false));
    renderer
        .initialize_with_device(device, queue)
        .expect("initialize renderer");
    renderer.set_poll_timeout(std::time::Duration::from_secs(30));
    renderer
}

fn scene(batches: Vec<RenderBatch>) -> SceneDelta {
    SceneDelta {
        stamp: TaskStamp::new(DocumentId(1), 0),
        added: batches,
        removed_chunks: Vec::new(),
        images: Vec::new(),
        masks: Vec::new(),
    }
}

fn camera_2d() -> Camera2d {
    Camera2d {
        center: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        world_per_px: 1.0,
        z_plane: 0.0,
    }
}

/// A solid-colour `width`x`height` RGBA8 image.
fn solid_image(width: u32, height: u32, rgba: [u8; 4]) -> DecodedImage {
    let pixels: Vec<u8> = rgba
        .iter()
        .copied()
        .cycle()
        .take((width * height * 4) as usize)
        .collect();
    DecodedImage {
        width,
        height,
        rgba: pixels.into(),
    }
}

/// An image batch mapping the unit square to a 16x16 world square centred on
/// `center` (so at `world_per_px == 1` it covers ~16 px, well clear of geometry
/// placed elsewhere in the target).
fn image_batch_at(resource: &str, entity: u128, center: [f64; 2]) -> ImageBatch {
    ImageBatch {
        resource: ResourceKey(resource.to_string()),
        transform: Transform3::from_basis(
            Point3 {
                x: 16.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 0.0,
                y: 16.0,
                z: 0.0,
            },
            Point3 {
                x: center[0] - 8.0,
                y: center[1] - 8.0,
                z: 0.0,
            },
        ),
        clip: None,
        alpha: 1.0,
        draw_order: 0,
        sources: vec![source(entity)],
    }
}

/// The origin-centred image batch used by the id-pass image tests.
fn image_batch(resource: &str, entity: u128) -> ImageBatch {
    image_batch_at(resource, entity, [0.0, 0.0])
}

fn base_batch(topology: RenderTopology, vertices: Vec<[f32; 3]>, entity: u128) -> RenderBatch {
    RenderBatch {
        conics: Vec::new(),
        local_origin: Point3 {
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
        alpha: 1.0,
        color: cad_scene::DEFAULT_BATCH_COLOR,
        color_unresolved: true,
        lineweight: 0.0,
        lineweight_unresolved: true,
        linetype: cad_scene::LinetypePattern::continuous(),
        linetype_unresolved: true,
        sources: vec![source(entity)],
        draw_order: 0,
    }
}

/// A `LineList` batch: a horizontal band of `rows` segments at `y`, spanning
/// `x0..x1`, so a scanline is guaranteed to rasterize (a single zero-width line
/// through a pixel centre is not reliably covered on every rasterizer).
fn line_band(y: f32, rows: u32, entity: u128) -> RenderBatch {
    let mut vertices = Vec::new();
    for row in 0..rows {
        let offset = y + row as f32 * 0.5;
        vertices.push([-20.0, offset, 0.0]);
        vertices.push([20.0, offset, 0.0]);
    }
    base_batch(RenderTopology::Lines, vertices, entity)
}

/// A quad mesh (two triangles) centred at `(cx, cy)` with half-size `half`.
/// `edges` adds the wireframe overlay (which the merged path instances).
fn quad(cx: f32, cy: f32, half: f32, entity: u128, edges: bool) -> RenderBatch {
    let mut batch = base_batch(
        RenderTopology::Mesh,
        vec![
            [cx - half, cy - half, 0.0],
            [cx - half, cy + half, 0.0],
            [cx + half, cy + half, 0.0],
            [cx + half, cy - half, 0.0],
        ],
        entity,
    );
    batch.normals = vec![[0.0, 0.0, 1.0]; 4];
    // The vertex order is clockwise in a Y-up world, so the index order flips it
    // to counter-clockwise and the `front_face = Ccw` culling keeps it visible
    // (the same convention the existing merged-mesh tests use).
    batch.indices = vec![[0, 2, 1], [0, 3, 2]];
    if edges {
        batch.edges = vec![[cx - half, cy - half, 0.0], [cx + half, cy - half, 0.0]];
    }
    batch
}

/// The pixel containing world point `(wx, wy)` for `camera`, clamped to the
/// target. Mirrors the renderer's own world→clip mapping.
fn world_to_pixel(camera: &Camera2d, width: u32, height: u32, world: [f64; 2]) -> (u32, u32) {
    let sx = 2.0 / (f64::from(width) * camera.world_per_px);
    let sy = 2.0 / (f64::from(height) * camera.world_per_px);
    let clip_x = sx * (world[0] - camera.center.x);
    let clip_y = sy * (world[1] - camera.center.y);
    let px = ((clip_x + 1.0) * 0.5 * f64::from(width)).floor() as i64;
    let py = ((1.0 - clip_y) * 0.5 * f64::from(height)).floor() as i64;
    (
        px.clamp(0, i64::from(width) - 1) as u32,
        py.clamp(0, i64::from(height) - 1) as u32,
    )
}

fn render(renderer: &mut Renderer, target: &RenderTarget) -> Vec<u8> {
    renderer
        .render(camera_2d(), target)
        .expect("render color frame");
    renderer.read_target_rgba().expect("read frame").pixels
}

/// The first pixel the color pass actually drew (not the clear colour), scanning
/// row-major. Using a pixel the color pass covered makes the id-pass assertion
/// about *the same visible geometry* rather than a guess about rasterization.
fn first_drawn_pixel(pixels: &[u8], width: u32, height: u32) -> Option<(u32, u32)> {
    for y in 0..height {
        for x in 0..width {
            let i = (y as usize * width as usize + x as usize) * 4;
            let drawn = pixels[i] > 8 || pixels[i + 1] > 8 || pixels[i + 2] > 8;
            if drawn {
                return Some((x, y));
            }
        }
    }
    None
}

/// The first pixel in `y0..y1` the colour pass drew that the id pass also
/// resolves. Anti-aliased line strokes put a sub-unit-coverage fringe outside the
/// id pass's 0.5-coverage pick threshold (±0.5 px, matching `pick_arc`), so a
/// plain "first inked pixel" scan can return an unpickable fringe pixel.
fn first_pickable_pixel(
    renderer: &mut Renderer,
    pixels: &[u8],
    width: u32,
    height: u32,
    y0: u32,
    y1: u32,
) -> Option<(u32, u32)> {
    for y in y0..y1.min(height) {
        for x in 0..width {
            let i = (y as usize * width as usize + x as usize) * 4;
            let drawn = pixels[i] > 8 || pixels[i + 1] > 8 || pixels[i + 2] > 8;
            if !drawn {
                continue;
            }
            if renderer
                .pick_at(x, y)
                .expect("pick while scanning")
                .is_some()
            {
                return Some((x, y));
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Id-pass correctness per class
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Multi-source and collision reporting
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Non-interference with the color pass
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Selection-flags machinery
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Id-target format tier
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Raster images through the id pass
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Key-space lifecycle: a reset must not alias resident image keys
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Selection flags across an incremental commit (P3a-2 contract)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Flags-disabled pixel identity per draw class
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Entity-level flags API (P3a-2)
// ---------------------------------------------------------------------------
