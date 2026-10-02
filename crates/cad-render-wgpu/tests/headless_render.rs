//! Headless software-Vulkan integration tests (native only).
//!
//! These tests create a real wgpu device on whatever adapter is visible (on the
//! project's CI/agent machine that is Mesa **lavapipe**, forced with
//! `VK_ICD_FILENAMES=.../lvp_icd.json`), render small scenes offscreen, read the
//! target back and assert that geometry actually rasterized. When no adapter is
//! available they skip with a clear message rather than pretending to pass.

use cad_domain::{DocumentId, EntityId, InstancePath, Point3, SelectionRef, TaskStamp};
use cad_render_wgpu::headless::{create_headless_gpu, encode_png, HeadlessGpu, RgbaImage};
use cad_render_wgpu::{BackendPreference, Camera2d, Camera3d, RenderError, RenderTarget, Renderer};
use cad_scene::{RenderBatch, RenderTopology, SceneDelta};

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
        local_origin: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        topology: RenderTopology::Lines,
        vertices,
        normals: Vec::new(),
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

/// A triangle wound clockwise in world space. The 2D projection flips Y, so it
/// becomes counter-clockwise in framebuffer space and survives the `Ccw`
/// front-face / back-face culling used by the mesh pipeline.
fn triangle_mesh() -> RenderBatch {
    RenderBatch {
        local_origin: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        topology: RenderTopology::Mesh,
        vertices: vec![[-0.9, -0.9, 0.0], [0.0, 0.9, 0.0], [0.9, -0.9, 0.0]],
        normals: vec![[0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]],
        indices: vec![[0, 1, 2]],
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
    }
}

fn init(gpu: HeadlessGpu) -> Renderer {
    let HeadlessGpu {
        device,
        queue,
        adapter: _,
    } = gpu;
    let mut renderer = Renderer::new(BackendPreference::Auto);
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

#[test]
fn headless_adapter_is_reported_as_software_vulkan() {
    let Some(gpu) = gpu() else {
        return;
    };
    assert!(
        !gpu.adapter.backend.is_empty() && !gpu.adapter.name.is_empty(),
        "adapter must report backend and name: {:?}",
        gpu.adapter
    );
    // Forcing the lavapipe ICD must yield the Vulkan backend on a CPU device.
    if std::env::var("VK_ICD_FILENAMES")
        .map(|v| v.contains("lvp"))
        .unwrap_or(false)
    {
        assert_eq!(gpu.adapter.backend, "vulkan");
        assert_eq!(gpu.adapter.device_type, "cpu");
        assert_eq!(gpu.adapter.driver, "llvmpipe");
    }
}

#[test]
fn lines_render_onto_target() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    renderer
        .upload(&scene(vec![lines_batch(rectangle_lines())]))
        .expect("upload line batch");

    let target = RenderTarget::new(64, 64);
    let stats = renderer
        .render(camera_2d(), &target)
        .expect("render line frame");
    assert!(stats.draw_calls > 0, "frame submitted no draw calls");
    assert_eq!(stats.vertices, rectangle_lines().len());

    let image = renderer.read_target_rgba().expect("read back frame");
    assert_eq!((image.width, image.height), (64, 64));
    assert_eq!(image.pixels.len(), 64 * 64 * 4);
    let background = image.pixel(0, 0);
    let differing = image.count_differing_from(background, 8);
    assert!(
        differing > 0,
        "line geometry rasterized nothing (background {background:?})"
    );
    assert!(
        differing < 64 * 64,
        "the whole target differs from the corner; camera fit is wrong"
    );
    // The center of the rectangle outline is empty, so the exact middle pixel
    // must remain background — a sanity check that we are not painting a blob.
    let center = image.pixel(32, 32);
    assert_eq!(
        center, background,
        "rectangle interior should be background, got {center:?}"
    );
}

#[test]
fn mesh_render_uses_triangle_pipeline() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    renderer
        .upload(&scene(vec![triangle_mesh()]))
        .expect("upload mesh batch");

    let target = RenderTarget::new(64, 64);
    let stats = renderer
        .render(camera_2d(), &target)
        .expect("render mesh frame");
    assert!(stats.triangles >= 1, "mesh batch reported no triangles");
    assert_eq!(stats.opaque_batches, 1);

    let image = renderer.read_target_rgba().expect("read back mesh frame");
    let background = image.pixel(0, 0);
    let differing = image.count_differing_from(background, 8);
    assert!(
        differing > 0,
        "mesh geometry rasterized nothing (background {background:?})"
    );
}

/// A translucent mesh batch must composite over the background differently from
/// an opaque one, proving `RenderBatch::alpha` reaches the blend state. This is
/// a real GPU test (software Vulkan/lavapipe under `VK_ICD_FILENAMES`), not a
/// plan-level assertion.
#[test]
fn translucent_batch_composites_differently_from_opaque() {
    let opaque_pixel = {
        let Some(gpu) = gpu() else {
            return;
        };
        let mut renderer = init(gpu);
        let mut batch = triangle_mesh();
        batch.alpha = 1.0;
        renderer
            .upload(&scene(vec![batch]))
            .expect("upload opaque mesh");
        let target = RenderTarget::new(64, 64);
        let stats = renderer
            .render(camera_2d(), &target)
            .expect("render opaque mesh");
        assert_eq!(stats.opaque_batches, 1);
        assert_eq!(stats.transparent_batches, 0);
        renderer
            .read_target_rgba()
            .expect("read opaque frame")
            .pixel(32, 32)
    };

    // A fresh renderer (new device) so the opaque batch is not still resident.
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let mut batch = triangle_mesh();
    batch.alpha = 0.5;
    renderer
        .upload(&scene(vec![batch]))
        .expect("upload translucent mesh");
    let target = RenderTarget::new(64, 64);
    let stats = renderer
        .render(camera_2d(), &target)
        .expect("render translucent mesh");
    assert_eq!(
        stats.opaque_batches, 0,
        "0.5 alpha must not be drawn in the opaque pass"
    );
    assert_eq!(stats.transparent_batches, 1);
    let image = renderer.read_target_rgba().expect("read translucent frame");
    let translucent_pixel = image.pixel(32, 32);
    let background = image.pixel(0, 0);

    assert_ne!(
        translucent_pixel, background,
        "the translucent triangle did not rasterize the center pixel"
    );
    assert_ne!(
        opaque_pixel, translucent_pixel,
        "alpha=0.5 must composite differently from an opaque batch (opaque {opaque_pixel:?}, \
         translucent {translucent_pixel:?})"
    );
}

#[test]
fn render_3d_is_not_blank() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    renderer
        .upload(&scene(vec![lines_batch(rectangle_lines())]))
        .expect("upload 3d batch");

    let camera = Camera3d {
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
    };
    let target = RenderTarget::new(64, 64);
    renderer
        .render_3d(camera, &target)
        .expect("render 3d frame");

    let image = renderer.read_target_rgba().expect("read back 3d frame");
    let background = image.pixel(0, 0);
    assert!(
        image.count_differing_from(background, 8) > 0,
        "3D frame is blank (background {background:?})"
    );
}

#[test]
fn png_round_trips_dimensions_and_signature() {
    let image = RgbaImage {
        width: 3,
        height: 2,
        pixels: (0..(3 * 2 * 4)).map(|i| (i * 7) as u8).collect(),
    };
    let png = encode_png(&image).expect("encode png");
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "PNG signature");
    // First chunk is IHDR: 4-byte length, "IHDR", width u32, height u32.
    assert_eq!(&png[12..16], b"IHDR");
    let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
    assert_eq!((width, height), (3, 2));

    let wrong = RgbaImage {
        width: 3,
        height: 2,
        pixels: vec![0; 7],
    };
    assert!(encode_png(&wrong).is_err(), "short buffer must be rejected");
}

#[test]
fn readback_before_render_is_not_initialized() {
    let renderer = Renderer::new(BackendPreference::Auto);
    assert!(matches!(
        renderer.read_target_rgba(),
        Err(RenderError::NotInitialized(_))
    ));
}
