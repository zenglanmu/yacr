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
fn progressive_pages_match_full_scene_without_raising_per_frame_budget() {
    let Some(gpu) = gpu() else {
        return;
    };
    eprintln!("progressive contract adapter: {:?}", gpu.adapter);
    let mut renderer = init(gpu);
    let mut batches: Vec<_> = rectangle_lines()
        .chunks_exact(2)
        .map(|points| lines_batch(points.to_vec()))
        .collect();
    let mut transparent = triangle_mesh();
    transparent.alpha = 0.5;
    transparent.draw_order = -10;
    batches.push(transparent);
    batches.push(triangle_mesh());
    let delta = scene(batches);
    let mut staged = renderer.prepare_upload_batches(&delta.added[..2]).unwrap();
    staged
        .append(renderer.prepare_upload_batches(&delta.added[2..]).unwrap())
        .unwrap();
    assert_eq!(
        renderer.batch_count(),
        0,
        "staging must not publish a prefix"
    );
    renderer.commit_upload(staged, 0).unwrap();
    let target = RenderTarget::new(64, 64);
    renderer.render(camera_2d(), &target).unwrap();
    let reference = renderer.read_target_rgba().unwrap();
    renderer.frame_budget.max_vertices = 3;
    renderer.frame_budget.max_triangles = 1;
    renderer.set_progressive_rendering(true);
    let mut pages = 0;
    loop {
        let stats = renderer.render(camera_2d(), &target).unwrap();
        assert!(stats.vertices <= 3);
        assert!(stats.triangles <= 1);
        pages += 1;
        assert!(pages <= delta.added.len());
        if !renderer.frame_pending() {
            break;
        }
    }
    assert!(pages > 1);
    assert_eq!(
        renderer.frame_progress(),
        Some((delta.added.len(), delta.added.len()))
    );
    assert_eq!(
        renderer.read_target_rgba().unwrap().pixels,
        reference.pixels
    );
    let mut camera = camera_2d();
    camera.center.x = 0.1;
    renderer.render(camera, &target).unwrap();
    assert!(
        renderer.frame_pending(),
        "navigation must restart accumulation, not retain stale pixels"
    );
    renderer.clear_batches();
    assert!(!renderer.frame_pending());
}

#[test]
fn progressive_oversized_batch_is_an_explicit_error_not_empty_completion() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    renderer
        .upload(&scene(vec![lines_batch(rectangle_lines())]))
        .unwrap();
    renderer.frame_budget.max_vertices = 1;
    renderer.set_progressive_rendering(true);
    assert!(renderer
        .render(camera_2d(), &RenderTarget::new(64, 64))
        .is_err());
    assert_eq!(renderer.frame_progress(), Some((0, 1)));
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
fn positive_world_y_renders_above_center_after_pan() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    // Asymmetric geometry, nonzero camera and batch origins: symmetric smoke
    // scenes cannot detect an upside-down projection or a translation sign bug.
    let mut batch = lines_batch(vec![[-0.5, 0.5, 0.0], [0.5, 0.5, 0.0]]);
    batch.local_origin = Point3 {
        x: 10.0,
        y: 20.0,
        z: 0.0,
    };
    renderer.upload(&scene(vec![batch])).unwrap();
    let mut camera = camera_2d();
    camera.center = Point3 {
        x: 10.0,
        y: 20.0,
        z: 0.0,
    };
    let target = RenderTarget::new(64, 64);
    renderer.render(camera, &target).unwrap();
    let image = renderer.read_target_rgba().unwrap();
    let background = image.pixel(0, 0);
    let mut rows = Vec::new();
    for y in 0..64 {
        for x in 0..64 {
            if image.pixel(x, y) != background {
                rows.push(y);
            }
        }
    }
    assert!(!rows.is_empty());
    assert!(rows.iter().all(|y| (15..=16).contains(y)), "rows: {rows:?}");
    // Pan upward by 0.5 world units: the same line now reaches screen center.
    camera.center.y += 0.5;
    renderer.render(camera, &target).unwrap();
    let image = renderer.read_target_rgba().unwrap();
    assert!((31..=32).any(|y| image.pixel(32, y) != background));
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

/// A quad mesh (world-counter-clockwise so it survives back-face culling) with an
/// explicit per-vertex colour ramp: `left` at the two x = -0.8 vertices and
/// `right` at the two x = +0.8 vertices.
fn gradient_quad(left: [f32; 3], right: [f32; 3]) -> RenderBatch {
    RenderBatch {
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

/// A per-vertex gradient must reach the fragment shader: the two ends of a
/// red→blue ramp rasterize as different, channel-dominant colours. This is a
/// real software-Vulkan (lavapipe) frame.
#[test]
fn gradient_mesh_renders_a_visible_ramp() {
    let Some(_gpu) = gpu() else {
        return;
    };
    let (left, right) = render_gradient_probes([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
    assert_ne!(left, right, "gradient endpoints rendered identically");
    assert!(
        left[0] > left[2],
        "left end should be red-dominant, got {left:?}"
    );
    assert!(
        right[2] > right[0],
        "right end should be blue-dominant, got {right:?}"
    );
}

/// Two gradients with the same stops but swapped directions must produce
/// different frames, proving the vertex colours (not just a constant tint)
/// drive the raster.
#[test]
fn gradient_direction_changes_the_frame() {
    let Some(_gpu) = gpu() else {
        return;
    };
    let forward = render_gradient_probes([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
    let reversed = render_gradient_probes([0.0, 0.0, 1.0], [1.0, 0.0, 0.0]);
    assert_ne!(
        forward, reversed,
        "reversing the gradient must change the rendered frame"
    );
    // The first probe swaps dominance.
    assert!(forward.0[0] > forward.0[2] && reversed.0[2] > reversed.0[0]);
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

/// A resident image must actually rasterize its texture colour. This is a real
/// software-Vulkan (lavapipe) frame, not a plan-level assertion.
#[test]
fn uploaded_image_renders_its_texture_colour() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let mut cache = DecodedImageCache::new();
    cache
        .insert(
            ResourceKey("red".into()),
            solid_image(2, 2, [255, 0, 0, 255]),
            &ResourceLimits::default(),
        )
        .expect("insert red image");

    let report = renderer
        .upload_images(&[image_batch("red")], &cache)
        .expect("upload image");
    assert_eq!(report.uploaded, 1);
    assert!(report.unresolved.is_empty());
    assert!(
        report.diagnostic().is_none(),
        "a resolved image must not report a gap"
    );

    let target = RenderTarget::new(64, 64);
    let stats = renderer
        .render(camera_2d(), &target)
        .expect("render image frame");
    assert_eq!(stats.image_batches, 1, "the resident image must be drawn");
    assert!(stats.draw_calls >= 1);

    let image = renderer.read_target_rgba().expect("read image frame");
    let center = image.pixel(32, 32);
    // The source is opaque sRGB red and the target is `Rgba8UnormSrgb`, so the
    // encoded channel value round-trips.
    assert!(
        center[0] > 240 && center[1] < 15 && center[2] < 15,
        "centre pixel should be opaque red, got {center:?}"
    );
    // Outside the quad stays the clear colour (a bluish background), so the red
    // channel is strictly dominated by the green/blue channels there.
    let corner = image.pixel(0, 0);
    assert!(
        corner[0] < center[0].saturating_sub(100) && corner[2] >= corner[0],
        "outside the quad must stay the bluish clear colour, got {corner:?}"
    );
}

/// An image whose resource is absent from the cache is skipped and reported,
/// never drawn as a placeholder. This is a real software-Vulkan (lavapipe) frame.
#[test]
fn missing_image_resource_draws_nothing_and_is_reported() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    // Empty cache: "absent" cannot be resolved.
    let cache = DecodedImageCache::new();
    let report = renderer
        .upload_images(&[image_batch("absent")], &cache)
        .expect("a missing resource must not panic");
    assert_eq!(report.uploaded, 0);
    assert_eq!(report.unresolved, vec![ResourceKey("absent".into())]);
    let reason = report
        .diagnostic()
        .expect("a missing image must be reported, never faked");
    assert_eq!(reason.code, cad_diagnostics::codes::RESOURCE_MISSING);

    let target = RenderTarget::new(64, 64);
    let stats = renderer
        .render(camera_2d(), &target)
        .expect("render after a skipped image");
    assert_eq!(stats.image_batches, 0, "no image quad may be drawn");
    assert_eq!(stats.draw_calls, 0, "nothing was submitted");
    let image = renderer.read_target_rgba().expect("read frame");
    let background = image.pixel(0, 0);
    assert_eq!(
        image.count_differing_from(background, 0),
        0,
        "a skipped image must leave the frame untouched"
    );
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

/// The texture must not be drawn vertically mirrored: with decoded rows stored
/// top-down and the CAD/DXF origin at the lower-left, the top of the quad shows
/// the image's top row. This is a real software-Vulkan (lavapipe) frame; without
/// the `uv.y = 1 - v` flip the halves swap and this test fails.
#[test]
fn uploaded_image_is_not_vertically_flipped() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let mut cache = DecodedImageCache::new();
    cache
        .insert(
            ResourceKey("two-tone".into()),
            two_tone_image(),
            &ResourceLimits::default(),
        )
        .expect("insert two-tone image");
    renderer
        .upload_images(&[image_batch("two-tone")], &cache)
        .expect("upload two-tone image");

    let target = RenderTarget::new(64, 64);
    renderer
        .render(camera_2d(), &target)
        .expect("render two-tone frame");
    let image = renderer.read_target_rgba().expect("read two-tone frame");
    // The quad spans world y in [-0.8, 0.8]; the target's row 0 is the top, so
    // row 8 samples near the top of the quad and row 56 near the bottom.
    let top = image.pixel(32, 8);
    let bottom = image.pixel(32, 56);
    assert!(
        top[0] > top[2].saturating_add(40) && top[1] < 60,
        "the top of the quad must sample the image's top (red) row, got {top:?}"
    );
    assert!(
        bottom[2] > bottom[0].saturating_add(40) && bottom[1] < 60,
        "the bottom of the quad must sample the image's bottom (blue) row, got {bottom:?}"
    );
}

/// A `clip` polygon must be consumed: the rendered footprint is the polygon, not
/// the whole unit square. Only the left half is clipped in, so the right half of
/// the quad must stay the clear colour. Real software-Vulkan (lavapipe) frame.
#[test]
fn clipped_image_renders_only_the_clip_polygon() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let mut cache = DecodedImageCache::new();
    cache
        .insert(
            ResourceKey("red".into()),
            solid_image(2, 2, [255, 0, 0, 255]),
            &ResourceLimits::default(),
        )
        .expect("insert red image");
    // World-space left half of the [-0.8, 0.8] square with matching UVs.
    let clip: Arc<[ImageVertex]> = Arc::from(
        vec![
            ImageVertex {
                position: Point3 {
                    x: -0.8,
                    y: -0.8,
                    z: 0.0,
                },
                uv: [0.0, 0.0],
            },
            ImageVertex {
                position: Point3 {
                    x: 0.0,
                    y: -0.8,
                    z: 0.0,
                },
                uv: [0.5, 0.0],
            },
            ImageVertex {
                position: Point3 {
                    x: 0.0,
                    y: 0.8,
                    z: 0.0,
                },
                uv: [0.5, 1.0],
            },
            ImageVertex {
                position: Point3 {
                    x: -0.8,
                    y: 0.8,
                    z: 0.0,
                },
                uv: [0.0, 1.0],
            },
        ]
        .into_boxed_slice(),
    );
    let mut batch = image_batch("red");
    batch.clip = Some(clip);
    renderer
        .upload_images(&[batch], &cache)
        .expect("upload clipped image");

    let target = RenderTarget::new(64, 64);
    let stats = renderer
        .render(camera_2d(), &target)
        .expect("render clipped frame");
    assert_eq!(stats.image_batches, 1, "the clipped image must still draw");

    let image = renderer.read_target_rgba().expect("read clipped frame");
    let left = image.pixel(16, 32);
    let right = image.pixel(48, 32);
    let background = image.pixel(0, 0);
    assert!(
        left[0] > 240 && left[1] < 15 && left[2] < 15,
        "the left (clipped-in) half must show the texture, got {left:?}"
    );
    assert_eq!(
        right, background,
        "the right (clipped-away) half must stay the clear colour, got {right:?}"
    );
}

/// Two image batches referencing the same `ResourceKey` must allocate a single
/// GPU texture (and bind group), not one per batch.
#[test]
fn image_batches_sharing_a_key_allocate_one_texture() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let mut cache = DecodedImageCache::new();
    cache
        .insert(
            ResourceKey("shared".into()),
            solid_image(2, 2, [255, 0, 0, 255]),
            &ResourceLimits::default(),
        )
        .expect("insert shared image");
    let first = image_batch("shared");
    let mut second = image_batch("shared");
    second.draw_order = 1;
    let report = renderer
        .upload_images(&[first, second], &cache)
        .expect("upload two batches sharing a key");

    assert_eq!(report.uploaded, 2, "both quads are resident");
    assert_eq!(
        renderer.image_texture_count(),
        1,
        "two batches with the same key must share one texture"
    );
    // The padded 2x2 texture (256-byte row alignment) is charged once; each quad
    // charges its own 4 vertices * 20 bytes + 6 indices * 4 bytes.
    let padded_texture = 256u64 * 2;
    let quad = (4 * 20 + 6 * 4) as u64;
    assert_eq!(
        report.bytes,
        padded_texture + 2 * quad,
        "texture bytes must be charged once, quad bytes per batch"
    );
}

/// The renderer persists across document opens, but each open builds a fresh
/// `DecodedImageCache`. A `ResourceKey` reused by a second drawing with different
/// bytes must rebuild the texture instead of silently sampling the first
/// drawing's image. Real software-Vulkan (lavapipe) frame; without the identity
/// check the second readback would still be red.
#[test]
fn new_cache_with_same_key_rebuilds_texture() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);

    // First document: "logo" names opaque red bytes.
    let mut red_cache = DecodedImageCache::new();
    red_cache
        .insert(
            ResourceKey("logo".into()),
            solid_image(2, 2, [255, 0, 0, 255]),
            &ResourceLimits::default(),
        )
        .expect("insert red logo");
    renderer
        .upload_images(&[image_batch("logo")], &red_cache)
        .expect("upload red logo");
    renderer
        .render(camera_2d(), &target)
        .expect("render red logo frame");
    let red = renderer.read_target_rgba().expect("read red frame");
    let red_center = red.pixel(32, 32);
    assert!(
        red_center[0] > 240 && red_center[1] < 15 && red_center[2] < 15,
        "the first document's texture must render red, got {red_center:?}"
    );

    // Second document: the same key now names opaque blue bytes in a new cache.
    let mut blue_cache = DecodedImageCache::new();
    blue_cache
        .insert(
            ResourceKey("logo".into()),
            solid_image(2, 2, [0, 0, 255, 255]),
            &ResourceLimits::default(),
        )
        .expect("insert blue logo");
    renderer
        .upload_images(&[image_batch("logo")], &blue_cache)
        .expect("upload blue logo");
    renderer
        .render(camera_2d(), &target)
        .expect("render blue logo frame");
    let blue = renderer.read_target_rgba().expect("read blue frame");
    let blue_center = blue.pixel(32, 32);
    assert!(
        blue_center[2] > 240 && blue_center[0] < 15 && blue_center[1] < 15,
        "the second document's bytes must rebuild the texture and render blue, got {blue_center:?}"
    );
}

/// Progressive rendering must composite images only on the page that clears the
/// target. A tiny vertex budget splits the geometry across two pages; the image
/// is charged and drawn on the first page only, never re-composited (which would
/// accumulate an `alpha < 1` image's opacity).
#[test]
fn progressive_images_draw_only_on_the_cleared_page() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    // Two 60-vertex line batches: a 100-vertex budget fits the first on page 0
    // (60) and pushes the second to a continuation page, while leaving room on
    // page 0 for the image's 4 vertices.
    let big_line = || {
        let mut vertices = Vec::new();
        for i in 0..30 {
            let x = -0.9 + i as f32 * 0.06;
            vertices.push([x, -0.5, 0.0]);
            vertices.push([x, 0.5, 0.0]);
        }
        lines_batch(vertices)
    };
    renderer
        .upload(&scene(vec![big_line(), big_line()]))
        .expect("upload two large line batches");
    let mut cache = DecodedImageCache::new();
    cache
        .insert(
            ResourceKey("red".into()),
            solid_image(2, 2, [255, 0, 0, 255]),
            &ResourceLimits::default(),
        )
        .expect("insert red image");
    renderer
        .upload_images(&[image_batch("red")], &cache)
        .expect("upload image");

    renderer.frame_budget.max_vertices = 100;
    renderer.set_progressive_rendering(true);

    let target = RenderTarget::new(64, 64);
    let mut image_draws = Vec::new();
    let mut pages = 0;
    loop {
        let stats = renderer.render(camera_2d(), &target).unwrap();
        image_draws.push(stats.image_batches);
        pages += 1;
        assert!(pages <= 4, "progressive frame did not terminate");
        if !renderer.frame_pending() {
            break;
        }
    }
    assert!(pages > 1, "the scene must span more than one page");
    assert_eq!(
        image_draws[0], 1,
        "the image must be drawn on the cleared first page"
    );
    assert!(
        image_draws[1..].iter().all(|&n| n == 0),
        "images must not re-composite on continuation pages, got {image_draws:?}"
    );
    assert_eq!(
        image_draws.iter().sum::<usize>(),
        1,
        "each image is drawn exactly once per frame"
    );
}
