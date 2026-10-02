//! GPU integration tests for *rendering behaviour* beyond the non-blank checks
//! in `tests/headless_render.rs` (native only).
//!
//! Each test creates a real software-Vulkan device (Mesa **lavapipe**, forced
//! with `VK_ICD_FILENAMES=.../lvp_icd.json`) and skips with a clear message when
//! no adapter exists, mirroring the headless setup. Assertions are about
//! deterministic, observable outcomes (statistics, relative pixel relationships,
//! byte-for-byte determinism) rather than exact driver colours.

use cad_domain::{DocumentId, EntityId, InstancePath, Point3, SelectionRef, TaskStamp};
use cad_render_wgpu::headless::{create_headless_gpu, encode_png, HeadlessGpu};
use cad_render_wgpu::{BackendPreference, Camera2d, RenderError, RenderTarget, Renderer};
use cad_scene::{FrameBudget, RenderBatch, RenderTopology, SceneDelta};

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
/// Wound clockwise in world space; the 2D projection flips Y, so the winding
/// becomes counter-clockwise in framebuffer space and survives the `Ccw`
/// front-face / back-face culling used by the mesh pipeline.
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
        local_origin: origin,
        topology: RenderTopology::Mesh,
        vertices: v,
        normals: vec![normal; 4],
        colors: Vec::new(),
        indices: vec![[0, 1, 2], [0, 2, 3]],
        edges: Vec::new(),
        mirrored: false,
        alpha,
        color: cad_scene::DEFAULT_BATCH_COLOR,
        color_unresolved: true,
        lineweight: 0.0,
        lineweight_unresolved: true,
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

#[test]
fn transparent_batch_composites_over_opaque() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);

    // Scene 1: overlay alpha = 0.5 (transparent pass).
    renderer
        .upload(&transparency_scene(0.5))
        .expect("upload transparent scene");
    let half_stats = renderer.render(camera_2d(), &target).expect("render half");
    assert_eq!(
        half_stats.transparent_batches, 1,
        "the 0.5-alpha overlay belongs to the transparent pass"
    );
    assert!(
        half_stats.opaque_batches >= 1,
        "the full-cover square is an opaque batch"
    );
    assert_eq!(
        half_stats.invisible_batches, 1,
        "the alpha = 0.0 ghost must be reported as invisible"
    );
    let half_image = renderer.read_target_rgba().expect("read half frame");

    // Scene 2: the exact same scene with the overlay alpha = 1.0 (opaque pass).
    renderer.clear_batches();
    renderer
        .upload(&transparency_scene(1.0))
        .expect("upload opaque scene");
    let opaque_stats = renderer
        .render(camera_2d(), &target)
        .expect("render opaque");
    assert_eq!(
        opaque_stats.transparent_batches, 0,
        "alpha = 1.0 must not use the transparent pass"
    );
    assert_eq!(opaque_stats.invisible_batches, 1);
    let opaque_image = renderer.read_target_rgba().expect("read opaque frame");

    // Probe pixels: one covered only by the full-cover square, one inside the
    // overlay. Neither lies on a quad diagonal.
    let cover = opaque_image.pixel(8, 9);
    let overlay = opaque_image.pixel(36, 28);
    let blended = half_image.pixel(36, 28);

    assert_ne!(
        overlay, cover,
        "the alpha = 1.0 overlay must repaint the covered pixel"
    );
    assert_ne!(
        blended, overlay,
        "alpha = 0.5 must differ from alpha = 1.0: blending ran"
    );
    assert_ne!(
        blended, cover,
        "the blended pixel must not equal the backdrop"
    );

    // A linear blend encoded to sRGB stays numerically between its endpoints,
    // channel by channel.
    for channel in 0..4 {
        let lo = cover[channel].min(overlay[channel]);
        let hi = cover[channel].max(overlay[channel]);
        assert!(
            blended[channel] >= lo.saturating_sub(3) && blended[channel] <= hi.saturating_add(3),
            "blended channel {channel} = {} not between {lo} and {hi} (cover {cover:?}, overlay {overlay:?}, blended {blended:?})",
            blended[channel]
        );
    }

    // Scene 3: only the alpha = 0.0 ghost; it must be counted and not drawn.
    renderer.clear_batches();
    renderer
        .upload(&scene(vec![ghost_quad()]))
        .expect("upload ghost scene");
    let ghost_stats = renderer.render(camera_2d(), &target).expect("render ghost");
    assert_eq!(ghost_stats.invisible_batches, 1);
    assert_eq!(ghost_stats.opaque_batches, 0);
    assert_eq!(ghost_stats.transparent_batches, 0);
    assert_eq!(
        ghost_stats.draw_calls, 0,
        "nothing submitted for an invisible batch"
    );
    let ghost_image = renderer.read_target_rgba().expect("read ghost frame");
    let background = ghost_image.pixel(0, 0);
    assert_eq!(
        ghost_image.count_differing_from(background, 0),
        0,
        "an alpha = 0.0 batch must leave the frame untouched"
    );
}

#[test]
fn frame_budget_over_budget_is_reported_not_silent() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    // Two line batches of eight vertices each: the first fits, the second
    // crosses the tiny vertex limit.
    renderer.frame_budget = FrameBudget {
        max_vertices: 10,
        max_triangles: 1_000_000,
    };
    renderer
        .upload(&scene(vec![
            lines_batch(rectangle_lines()),
            lines_batch(rectangle_lines()),
        ]))
        .expect("upload two batches");

    let target = RenderTarget::new(64, 64);
    let stats = renderer
        .render(camera_2d(), &target)
        .expect("over-budget frame must still render the batches that fit");

    let over = stats
        .over_budget
        .expect("crossing the frame budget must be reported, never silent");
    assert_eq!(over.report.category, "vertices");
    assert_eq!(over.report.limit, 10);
    assert!(
        over.report.skipped_batches >= 1,
        "at least the offending batch must be reported skipped: {over:?}"
    );
    assert!(
        stats.vertices <= 10,
        "only accepted vertices may be submitted, got {}",
        stats.vertices
    );
    assert_eq!(
        stats.draw_calls, 1,
        "exactly the one batch that fit should be drawn"
    );
}

#[test]
fn device_loss_is_classified_after_note() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    renderer.note_device_lost("test reset");
    assert!(renderer.is_device_lost(), "renderer must latch the loss");

    let target = RenderTarget::new(32, 32);
    let error = renderer
        .render(camera_2d(), &target)
        .err()
        .expect("rendering after a device loss must fail");
    assert!(
        matches!(error, RenderError::DeviceLost(_)),
        "device loss must not be reported as a bad frame: {error:?}"
    );
    assert!(error.is_device_loss());
}

#[test]
fn same_scene_is_deterministic() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    renderer
        .upload(&scene(vec![
            lines_batch(rectangle_lines()),
            quad_mesh(
                0.4,
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                1.0,
                FULL_COVER_NORMAL,
                1,
            ),
        ]))
        .expect("upload deterministic scene");

    let target = RenderTarget::new(64, 64);
    renderer.render(camera_2d(), &target).expect("render first");
    let first = renderer.read_target_rgba().expect("read first frame");
    renderer
        .render(camera_2d(), &target)
        .expect("render second");
    let second = renderer.read_target_rgba().expect("read second frame");

    assert_eq!(
        first.pixels, second.pixels,
        "rendering the same scene twice must be byte-identical"
    );
}

#[test]
fn large_coordinate_batch_keeps_precision() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    // A small local square placed at a very large world origin. The renderer
    // must subtract the camera centre in f64 before narrowing, so centring the
    // camera on the batch still rasterizes it.
    let origin = Point3 {
        x: 1.0e7,
        y: 1.0e7,
        z: 0.0,
    };
    renderer
        .upload(&scene(vec![quad_mesh(
            0.5,
            origin,
            1.0,
            FULL_COVER_NORMAL,
            0,
        )]))
        .expect("upload large-coordinate batch");

    let target = RenderTarget::new(64, 64);
    renderer
        .render(camera_2d_at(1.0e7, 1.0e7), &target)
        .expect("render large-coordinate frame");

    let image = renderer
        .read_target_rgba()
        .expect("read large-coordinate frame");
    let background = image.pixel(0, 0);
    assert!(
        image.count_differing_from(background, 8) > 0,
        "a batch at 1e7 centred under the camera must still rasterize"
    );
}

#[test]
fn frame_png_is_written_and_valid() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    renderer
        .upload(&scene(vec![lines_batch(rectangle_lines())]))
        .expect("upload png scene");

    let target = RenderTarget::new(64, 64);
    renderer
        .render(camera_2d(), &target)
        .expect("render png frame");
    let image = renderer.read_target_rgba().expect("read png frame");
    let png = encode_png(&image).expect("encode frame png");

    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "PNG signature");
    assert_eq!(&png[12..16], b"IHDR", "first chunk must be IHDR");
    let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
    assert_eq!((width, height), (64, 64));

    // Lossless round-trip through the `png` crate must reproduce the readback
    // exactly (`encode_png` performs no colour conversion).
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().expect("png read_info");
    let required = reader.output_buffer_size().expect("png output size");
    let mut decoded = vec![0u8; required];
    let info = reader.next_frame(&mut decoded).expect("png next_frame");
    assert_eq!((info.width, info.height), (64, 64));
    assert_eq!(info.color_type, png::ColorType::Rgba);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    assert_eq!(
        &decoded[..required],
        &image.pixels[..],
        "PNG round-trip must be byte-identical to the readback"
    );
}

/// A line batch with an explicit colour, for the per-colour frame test.
fn colored_line_batch(color: [f32; 3]) -> RenderBatch {
    let mut batch = lines_batch(rectangle_lines());
    batch.color = color;
    batch.color_unresolved = false;
    batch
}

/// Two entities with different colours must render differently. `RenderBatch`'s
/// per-batch colour reaches the shader uniform; this is a real software-Vulkan
/// (lavapipe) frame, not a plan-level assertion.
#[test]
fn batches_with_different_colors_render_different_frames() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);

    renderer
        .upload(&scene(vec![colored_line_batch([1.0, 0.0, 0.0])]))
        .expect("upload red batch");
    renderer.render(camera_2d(), &target).expect("render red");
    let red = renderer.read_target_rgba().expect("read red frame");

    renderer.clear_batches();
    renderer
        .upload(&scene(vec![colored_line_batch([0.0, 0.0, 1.0])]))
        .expect("upload blue batch");
    renderer.render(camera_2d(), &target).expect("render blue");
    let blue = renderer.read_target_rgba().expect("read blue frame");

    assert_ne!(
        red.pixels, blue.pixels,
        "the same geometry with different batch colours must produce different frames"
    );
    // The red frame must be redder than the blue frame on some pixel, and vice
    // versa, rather than merely differing by antialiasing noise.
    let redder = (0..64)
        .flat_map(|y| (0..64).map(move |x| (x, y)))
        .any(|(x, y)| red.pixel(x, y)[0] > blue.pixel(x, y)[0] + 8);
    let bluer = (0..64)
        .flat_map(|y| (0..64).map(move |x| (x, y)))
        .any(|(x, y)| blue.pixel(x, y)[2] > red.pixel(x, y)[2] + 8);
    assert!(
        redder && bluer,
        "each coloured frame must dominate its own channel"
    );
}

/// ByLayer resolves to the layer colour upstream; at the renderer the important
/// contract is that the resolved value travels into the uniform, so two batches
/// that differ only in colour (one red, one the default white) are not equal.
#[test]
fn resolved_layer_color_reaches_the_uniform() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);

    let mut default_batch = lines_batch(rectangle_lines());
    default_batch.color = cad_scene::DEFAULT_BATCH_COLOR;
    renderer
        .upload(&scene(vec![default_batch]))
        .expect("upload default batch");
    renderer
        .render(camera_2d(), &target)
        .expect("render default");
    let default_frame = renderer.read_target_rgba().expect("read default");

    renderer.clear_batches();
    renderer
        .upload(&scene(vec![colored_line_batch([1.0, 0.0, 0.0])]))
        .expect("upload layer-colour batch");
    renderer
        .render(camera_2d(), &target)
        .expect("render layer-colour");
    let layer_frame = renderer
        .read_target_rgba()
        .expect("read layer-colour frame");

    assert_ne!(
        default_frame.pixels, layer_frame.pixels,
        "a batch whose colour came from the layer must render differently from \
         the default colour"
    );
}

/// Lineweight is carried but not drawn: the frame must report it explicitly.
#[test]
fn lineweight_is_reported_not_drawn() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let mut batch = lines_batch(rectangle_lines());
    batch.lineweight = 0.5;
    batch.lineweight_unresolved = false;
    renderer
        .upload(&scene(vec![batch]))
        .expect("upload weighted batch");

    let target = RenderTarget::new(64, 64);
    let stats = renderer
        .render(camera_2d(), &target)
        .expect("render weighted");

    assert_eq!(
        stats.lineweight_not_drawn.len(),
        1,
        "a non-zero lineweight must be reported as not drawn"
    );
    assert!((stats.lineweight_not_drawn[0].millimeters - 0.5).abs() < 1e-6);
    let reason = stats
        .lineweight_reason
        .as_ref()
        .expect("an explicit diagnostic reason must accompany the gap");
    assert_eq!(
        reason.code,
        cad_diagnostics::codes::RENDER_LINEWEIGHT_NOT_DRAWN
    );

    // A batch that asks for no weight reports nothing.
    renderer.clear_batches();
    renderer
        .upload(&scene(vec![lines_batch(rectangle_lines())]))
        .expect("upload hairline batch");
    let stats = renderer
        .render(camera_2d(), &target)
        .expect("render hairline");
    assert!(
        stats.lineweight_not_drawn.is_empty(),
        "a zero lineweight must not be reported, got {:?}",
        stats.lineweight_not_drawn
    );
    assert!(stats.lineweight_reason.is_none());
}
