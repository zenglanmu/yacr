//! Headless software-Vulkan integration tests (native only).
//!
//! These tests create a real wgpu device on whatever adapter is visible (on the
//! project's CI/agent machine that is Mesa **lavapipe**, forced with
//! `VK_ICD_FILENAMES=.../lvp_icd.json`), render small scenes offscreen, read the
//! target back and assert that geometry actually rasterized. When no adapter is
//! available they skip with a clear message rather than pretending to pass.

use cad_domain::{DocumentId, EntityId, InstancePath, Point3, SelectionRef, TaskStamp, Transform3};

use cad_render_wgpu::headless::{create_headless_gpu, encode_png, HeadlessGpu, RgbaImage};

use cad_render_wgpu::{BackendPreference, Camera2d, Camera3d, RenderError, RenderTarget, Renderer};

use cad_representation::ImageVertex;

use cad_resources::{DecodedImage, DecodedImageCache, ResourceKey, ResourceLimits};

use cad_scene::{ImageBatch, RenderBatch, RenderTopology, SceneDelta};

use std::sync::Arc;

mod images_and_culling;
mod masks;
mod render_basics;
fn source() -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(1),
        instance: InstancePath::default(),
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

/// A triangle wound counter-clockwise in world/clip space, facing +Z.
fn triangle_mesh() -> RenderBatch {
    RenderBatch {
        conics: Vec::new(),
        local_origin: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        topology: RenderTopology::Mesh,
        vertices: vec![[-0.9, -0.9, 0.0], [0.0, 0.9, 0.0], [0.9, -0.9, 0.0]],
        normals: vec![[0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]],
        colors: Vec::new(),
        indices: vec![[0, 2, 1]],
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

fn scene(batches: Vec<RenderBatch>) -> SceneDelta {
    SceneDelta {
        stamp: TaskStamp::new(DocumentId(1), 0),
        added: batches,
        removed_chunks: Vec::new(),
        images: Vec::new(),
        masks: Vec::new(),
    }
}

fn init(gpu: HeadlessGpu) -> Renderer {
    let HeadlessGpu {
        device,
        queue,
        adapter: _,
    } = gpu;
    let mut renderer = Renderer::new(BackendPreference::Auto);
    // This suite pins the universal per-run path (progressive pages, non-blank
    // checks); the accelerated tier is fenced in `tests/accelerated_tier.rs`.
    renderer.set_storage_buffer_support_override(Some(false));
    renderer
        .initialize_with_device(device, queue)
        .expect("initialize headless renderer");
    renderer
}

fn camera_2d() -> Camera2d {
    Camera2d {
        center: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        // 64 px span 2 world units, so [-1, 1] exactly fills the target.
        world_per_px: 2.0 / 64.0,
        z_plane: 0.0,
    }
}

/// A `2`-triangle quad covering the centre, facing +Z (same winding convention
/// as [`triangle_mesh`]). Charges 4 vertices / 2 triangles, one more vertex than
/// a minimum triangle, so a tiny paced budget can be shown to reject it.
fn quad_mesh() -> RenderBatch {
    RenderBatch {
        conics: Vec::new(),
        local_origin: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        topology: RenderTopology::Mesh,
        vertices: vec![
            [-0.5, -0.5, 0.0],
            [0.5, -0.5, 0.0],
            [0.5, 0.5, 0.0],
            [-0.5, 0.5, 0.0],
        ],
        normals: vec![[0.0, 0.0, 1.0]; 4],
        colors: Vec::new(),
        indices: vec![[0, 1, 2], [0, 2, 3]],
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

/// `n` independent one-segment line batches (each charges 6 vertices / 2
/// triangles), all in the same upload order.
fn line_batches(n: usize) -> Vec<RenderBatch> {
    (0..n)
        .map(|i| {
            let y = -0.8 + i as f32 * 0.1;
            lines_batch(vec![[-0.8, y, 0.0], [0.8, y, 0.0]])
        })
        .collect()
}

/// A quad mesh (world-counter-clockwise so it survives back-face culling) with an
/// explicit per-vertex colour ramp: `left` at the two x = -0.8 vertices and
/// `right` at the two x = +0.8 vertices.
fn gradient_quad(left: [f32; 3], right: [f32; 3]) -> RenderBatch {
    RenderBatch {
        conics: Vec::new(),
        local_origin: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        topology: RenderTopology::Mesh,
        vertices: vec![
            [-0.8, -0.8, 0.0],
            [-0.8, 0.8, 0.0],
            [0.8, 0.8, 0.0],
            [0.8, -0.8, 0.0],
        ],
        normals: vec![
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
        ],
        colors: vec![left, left, right, right],
        indices: vec![[0, 2, 1], [0, 3, 2]],
        edges: Vec::new(),
        mirrored: false,
        alpha: 1.0,
        color: cad_scene::DEFAULT_BATCH_COLOR,
        color_unresolved: true,
        lineweight: 0.0,
        lineweight_unresolved: true,
        linetype: cad_scene::LinetypePattern::continuous(),
        linetype_unresolved: false,
        sources: vec![source()],
        draw_order: 0,
    }
}

/// Render one gradient quad and return `(left_probe, right_probe)` from the
/// readback. The probes sit inside the quad at world x = -0.5 and +0.5, y = 0.
fn render_gradient_probes(left: [f32; 3], right: [f32; 3]) -> ([u8; 4], [u8; 4]) {
    let Some(gpu) = gpu() else {
        return ([0, 0, 0, 0], [0, 0, 0, 0]);
    };
    let mut renderer = init(gpu);
    renderer
        .upload(&scene(vec![gradient_quad(left, right)]))
        .expect("upload gradient quad");
    let target = RenderTarget::new(64, 64);
    let stats = renderer
        .render(camera_2d(), &target)
        .expect("render gradient frame");
    assert_eq!(stats.triangles, 2, "gradient quad should be two triangles");
    let image = renderer.read_target_rgba().expect("read gradient frame");
    // x = -0.5 -> pixel 16; x = +0.5 -> pixel 48; y = 0 -> pixel 32.
    (image.pixel(16, 32), image.pixel(48, 32))
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

/// A unit-square image batch mapping the texture to `[-0.8, 0.8]` in world
/// space, centred on the camera.
fn image_batch(resource: &str) -> ImageBatch {
    ImageBatch {
        resource: ResourceKey(resource.to_string()),
        transform: Transform3::from_basis(
            Point3 {
                x: 1.6,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 0.0,
                y: 1.6,
                z: 0.0,
            },
            Point3 {
                x: -0.8,
                y: -0.8,
                z: 0.0,
            },
        ),
        clip: None,
        alpha: 1.0,
        draw_order: 0,
        sources: vec![source()],
    }
}

/// A 2x2 image whose top row is red and bottom row is blue, used to prove the
/// orientation of a rendered quad.
fn two_tone_image() -> DecodedImage {
    DecodedImage {
        width: 2,
        height: 2,
        rgba: Arc::from(vec![
            255, 0, 0, 255, 255, 0, 0, 255, // top row: red
            0, 0, 255, 255, 0, 0, 255, 255, // bottom row: blue
        ]),
    }
}

// ---------------------------------------------------------------------------
// Chunk-level view culling (backlog §P1)
// ---------------------------------------------------------------------------

/// A single-segment line batch whose chunk origin is `(origin_x, 0, 0)`. With
/// [`Renderer::set_max_instances_per_stream`] set to `1` every batch is its own
/// chunk, so an off-screen batch is an off-screen chunk.
fn line_at(origin_x: f32, y: f32) -> RenderBatch {
    let mut batch = lines_batch(vec![[-0.5, y - 0.2, 0.0], [0.5, y + 0.2, 0.0]]);
    batch.local_origin = Point3 {
        x: f64::from(origin_x),
        y: 0.0,
        z: 0.0,
    };
    batch
}

/// Three on-screen chunks and two far off-screen chunks (each its own chunk).
fn cull_scene() -> Vec<RenderBatch> {
    vec![
        line_at(0.0, -0.5),
        line_at(0.0, 0.0),
        line_at(0.0, 0.5),
        line_at(1000.0, 0.0),
        line_at(-1000.0, 0.0),
    ]
}

/// Measurement-only GPU frame-time contracts (ADR 0013 class 1).
///
/// These are the fences for the explicit, documented opt-in: the product default
/// requests no feature and reports `gpu_ms = None`; the enabled path measures a
/// real bracket **without** changing pixels; the resources are created once and
/// reused across frames; and a missing feature is named, never silently ignored.
mod gpu_time_contracts {
    use super::*;
    use cad_render_wgpu::gpu_time::{GpuTimeConfig, GpuTimeState, GPU_TIME_FEATURES};
    use cad_render_wgpu::headless::create_headless_gpu_with_gpu_time;
    use cad_render_wgpu::wgpu;
    use cad_render_wgpu::GpuSelection;

    /// Fence 1: with the flag off the device requests no measurement feature and
    /// `gpu_ms` stays `None`.
    #[test]
    fn disabled_requests_no_feature_and_reports_none() {
        let Some(gpu) = gpu() else {
            return;
        };
        assert!(
            !gpu.device.features().contains(GPU_TIME_FEATURES),
            "the product-default device must not have the timestamp feature set"
        );
        let mut renderer = init(gpu);
        assert_eq!(
            renderer.set_gpu_time_measurement(GpuTimeConfig::disabled()),
            GpuTimeState::Disabled
        );
        assert_eq!(
            renderer.gpu_time_resource_creations(),
            0,
            "the disabled path allocates no measurement resources"
        );
        renderer
            .upload(&scene(vec![lines_batch(rectangle_lines())]))
            .unwrap();
        let stats = renderer
            .render(camera_2d(), &RenderTarget::new(64, 64))
            .unwrap();
        assert_eq!(
            stats.gpu_ms, None,
            "gpu_ms stays honestly absent when measurement is off"
        );
        assert_eq!(stats.gpu_time, GpuTimeState::Disabled);
    }

    /// Fence 3: the opt-in is on but the feature is absent — the reported state
    /// names the missing feature. Simulated through the capability-injection hook
    /// (no fake device), so the `Unsupported` branch is real code under test.
    #[test]
    fn unsupported_state_names_the_missing_feature() {
        let Some(gpu) = gpu() else {
            return;
        };
        let mut renderer = init(gpu);
        // Simulate an adapter whose granted set lacks the timestamp feature.
        renderer.set_gpu_time_feature_override(Some(wgpu::Features::empty()));
        let state = renderer.set_gpu_time_measurement(GpuTimeConfig::enabled());
        assert_eq!(state, GpuTimeState::Unsupported("TIMESTAMP_QUERY"));
        assert_eq!(state.as_token(), "unsupported(TIMESTAMP_QUERY)");
        assert_eq!(
            renderer.gpu_time_resource_creations(),
            0,
            "an unsupported request must not create measurement resources"
        );
        // Only the encoder feature missing: it is the one named.
        renderer.set_gpu_time_feature_override(Some(wgpu::Features::TIMESTAMP_QUERY));
        assert_eq!(
            renderer.set_gpu_time_measurement(GpuTimeConfig::enabled()),
            GpuTimeState::Unsupported("TIMESTAMP_QUERY_INSIDE_ENCODERS")
        );
    }

    /// Fences 4 + 5: enabling measurement reports a real `gpu_ms` and leaves the
    /// rendered output **byte-identical** to the disabled device, and the
    /// measurement resources are created once and reused across frames.
    #[test]
    fn enabled_measures_without_changing_pixels_and_does_not_grow() {
        let (gpu_on, report) = match create_headless_gpu_with_gpu_time(
            BackendPreference::Auto,
            GpuSelection::Auto,
            GpuTimeConfig::enabled(),
        ) {
            Ok(value) => value,
            Err(error) => {
                eprintln!("skipping: no headless GPU: {error}");
                return;
            }
        };
        if report.state != GpuTimeState::Enabled {
            eprintln!(
                "SKIP: this adapter cannot measure ({}); NOT a pass",
                report.state.as_token()
            );
            return;
        }
        let Some(gpu_off) = gpu() else {
            return;
        };

        let mut renderer_on = init(gpu_on);
        let mut renderer_off = init(gpu_off);
        let batches = || vec![lines_batch(rectangle_lines()), triangle_mesh()];
        renderer_on.upload(&scene(batches())).unwrap();
        renderer_off.upload(&scene(batches())).unwrap();
        renderer_off.set_gpu_time_measurement(GpuTimeConfig::disabled());
        assert_eq!(
            renderer_on.set_gpu_time_measurement(GpuTimeConfig::enabled()),
            GpuTimeState::Enabled
        );

        let target = RenderTarget::new(64, 64);
        let stats_on = renderer_on.render(camera_2d(), &target).unwrap();
        let pixels_on = renderer_on.read_target_rgba().unwrap();
        let stats_off = renderer_off.render(camera_2d(), &target).unwrap();
        let pixels_off = renderer_off.read_target_rgba().unwrap();

        assert_eq!(stats_on.gpu_time, GpuTimeState::Enabled);
        assert!(
            stats_on.gpu_ms.is_some(),
            "an enabled measurement must report a real gpu_ms"
        );
        assert_eq!(stats_off.gpu_ms, None);
        assert_eq!(
            pixels_on.pixels, pixels_off.pixels,
            "enabling measurement must not change rendered output"
        );

        // Fence 5: no per-frame allocation of the measurement resources.
        assert_eq!(renderer_on.gpu_time_resource_creations(), 1);
        for _ in 0..4 {
            renderer_on.render(camera_2d(), &target).unwrap();
        }
        assert_eq!(
            renderer_on.gpu_time_resource_creations(),
            1,
            "the query set / buffers must be reused across frames, not re-allocated"
        );
    }
}
