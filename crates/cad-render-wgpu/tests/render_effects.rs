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
        images: Vec::new(),
    }
}

#[test]
fn staged_upload_failure_preserves_active_scene_and_overlay_prefix() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let base = lines_batch(rectangle_lines());
    renderer.upload(&scene(vec![base.clone()])).unwrap();
    let mut invalid = base.clone();
    invalid.vertices[0][0] = f32::NAN;
    assert!(renderer
        .prepare_upload(&scene(vec![base.clone(), invalid]))
        .is_err());
    assert_eq!(renderer.batch_count(), 1, "failed staging must retain base");
    let overlay = renderer.prepare_upload(&scene(vec![base])).unwrap();
    assert_eq!(renderer.batch_count(), 1, "prepare is not publication");
    renderer.commit_upload(overlay, 1).unwrap();
    assert_eq!(renderer.batch_count(), 2);
    let empty = renderer.prepare_upload(&scene(vec![])).unwrap();
    renderer.commit_upload(empty, 1).unwrap();
    assert_eq!(
        renderer.batch_count(),
        1,
        "clearing overlay retains base buffers"
    );
    let stale = renderer.prepare_upload(&scene(vec![])).unwrap();
    renderer.note_device_lost("test loss");
    assert!(renderer.commit_upload(stale, 0).is_err());
}

#[test]
fn texture_revision_changes_only_when_attachment_is_replaced() {
    let Some(gpu) = gpu() else {
        return;
    };
    let device = gpu.device.clone();
    let queue = gpu.queue.clone();
    let mut renderer = init(gpu);
    renderer
        .render(camera_2d(), &RenderTarget::new(64, 64))
        .unwrap();
    let first = renderer.texture_revision();
    renderer
        .render(camera_2d(), &RenderTarget::new(64, 64))
        .unwrap();
    assert_eq!(renderer.texture_revision(), first);
    renderer.initialize_with_device(device, queue).unwrap();
    renderer
        .render(camera_2d(), &RenderTarget::new(64, 64))
        .unwrap();
    assert!(
        renderer.texture_revision() > first,
        "same-size reattachment changes texture identity"
    );
    let reattached = renderer.texture_revision();
    renderer
        .render(camera_2d(), &RenderTarget::new(128, 64))
        .unwrap();
    assert!(renderer.texture_revision() > reattached);
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
        max_bytes: usize::MAX,
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
fn frame_byte_budget_is_reported_not_silent() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    // Two line batches of eight vertices each. Each converts to an instanced
    // stream of four segments, charged at the exact instance stride
    // (4 * 56 = 224 bytes), which is what the frame budget must account. The
    // first fits a 300-byte budget; the second crosses it on the byte category.
    renderer.frame_budget = FrameBudget {
        max_vertices: usize::MAX,
        max_triangles: usize::MAX,
        max_bytes: 300,
    };
    renderer
        .upload(&scene(vec![
            lines_batch(rectangle_lines()),
            lines_batch(rectangle_lines()),
        ]))
        .expect("upload two batches");
    // The upload phase records a real measured wall-clock time.
    let upload_ms = renderer
        .last_upload_ms()
        .expect("upload must record a measured time");
    assert!(upload_ms.is_finite() && upload_ms >= 0.0, "got {upload_ms}");

    let target = RenderTarget::new(64, 64);
    let stats = renderer
        .render(camera_2d(), &target)
        .expect("over-budget frame must still render the batches that fit");

    let over = stats
        .over_budget
        .expect("crossing the byte budget must be reported, never silent");
    assert_eq!(over.report.category, "bytes");
    assert_eq!(over.report.limit, 300);
    assert_eq!(over.report.requested, 448);
    assert_eq!(
        stats.draw_calls, 1,
        "the one accepted (converted) batch is a single instanced draw"
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
fn uniform_arena_places_batches_across_shared_buffers() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    // The per-batch uniform arena's page/offset selection is a property of the
    // legacy per-batch path: converted line batches share one stream uniform and
    // deliberately collapse to a single draw. Force the legacy path here so this
    // test keeps exercising the multi-page arena (see
    // `instanced_lines_match_legacy_pixels` for the instanced equivalent).
    renderer.set_line_instancing(false);
    // Many more batches than one WebGPU-default uniform page (256 slots), so the
    // last batch necessarily lands in a later shared buffer with a non-zero
    // dynamic offset. Every other batch sits far off-screen; only the last one is
    // a visible rectangle at the origin. A wrong page/offset selection would hand
    // the visible batch a neighbour's far-away transform and blank the frame.
    let total = 300usize;
    let mut batches: Vec<RenderBatch> = (0..total - 1)
        .map(|k| {
            let x = 10_000.0 + k as f32;
            lines_batch(vec![[x, 0.0, 0.0], [x + 0.5, 0.5, 0.0]])
        })
        .collect();
    batches.push(lines_batch(rectangle_lines()));
    renderer
        .upload(&scene(batches))
        .expect("upload many batches");

    let target = RenderTarget::new(64, 64);
    let stats = renderer
        .render(camera_2d(), &target)
        .expect("render many batches");
    assert_eq!(
        stats.draw_calls, total as u64,
        "consolidating uniforms must not change the draw call count"
    );
    assert!(
        stats.uniform_writes < total as u64,
        "uniforms must consolidate into shared pages, got {} writes for {total} batches",
        stats.uniform_writes
    );

    let image = renderer.read_target_rgba().expect("read many-batch frame");
    let background = image.pixel(0, 0);
    assert!(
        image.count_differing_from(background, 8) > 0,
        "a batch in a later uniform buffer must still rasterize at its own origin"
    );

    // Repeating the frame must be byte-identical (stable slot assignment).
    renderer
        .render(camera_2d(), &target)
        .expect("render many batches again");
    let again = renderer.read_target_rgba().expect("read again");
    assert_eq!(
        image.pixels, again.pixels,
        "shared uniform slot assignment must be deterministic"
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

/// Linetype dashes are visible: a pre-subdivided dashed line lights strictly
/// fewer pixels than the solid line over the same span, and the midpoint (a
/// gap) stays at the background colour.
#[test]
fn dashed_line_renders_a_visible_gap() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    let camera = camera_2d();

    renderer
        .upload(&scene(vec![lines_batch(solid_horizontal_lines())]))
        .expect("upload solid line");
    renderer.render(camera, &target).expect("render solid line");
    let solid = renderer.read_target_rgba().expect("read solid frame");
    let background = solid.pixel(0, 0);
    let solid_lit = solid.count_differing_from(background, 8);

    renderer.clear_batches();
    renderer
        .upload(&scene(dashed_horizontal_batches()))
        .expect("upload dashed line");
    renderer
        .render(camera, &target)
        .expect("render dashed line");
    let dashed = renderer.read_target_rgba().expect("read dashed frame");
    // A pixel in the middle of the gap (world x = 0 -> pixel x = 32) must be
    // background, while the solid frame is lit along its whole span.
    assert_eq!(
        dashed.pixel(32, 32),
        background,
        "the midpoint of a dashed line must be an unfilled gap"
    );
    let dashed_lit = dashed.count_differing_from(background, 8);
    assert!(
        dashed_lit < solid_lit,
        "dashed line ({dashed_lit} px) must light fewer pixels than solid ({solid_lit} px)"
    );
    assert!(dashed_lit > 0, "the dashes themselves must still render");
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

/// The exact case the slice claims: 2-vertex `LineList` segments rendered from an
/// instance stream must be byte-identical to the legacy per-batch path.
#[test]
fn instanced_lines_match_legacy_pixels() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    let batches = vec![
        lines_batch(vec![[-0.9, -0.5, 0.0], [0.9, -0.5, 0.0]]),
        lines_batch(vec![[-0.9, 0.5, 0.0], [0.9, 0.5, 0.0]]),
        lines_batch(vec![[0.0, -0.9, 0.0], [0.0, 0.9, 0.0]]),
    ];

    let (instanced, stats) = render_pixels(&mut renderer, &target, &batches, true, camera_2d());
    assert_eq!(
        renderer.instance_segment_count(),
        3,
        "the three segments must be resident in an instance stream"
    );
    assert_eq!(stats.instanced_draws, 1);
    assert_eq!(stats.instanced_batches, 3);
    assert_eq!(
        stats.draw_calls, 1,
        "three segments must collapse to one instanced draw"
    );
    assert!(
        instanced
            .chunks_exact(4)
            .any(|px| px != opaque_black(&instanced).as_slice()),
        "instanced frame rasterized nothing"
    );

    let (legacy, _) = render_pixels(&mut renderer, &target, &batches, false, camera_2d());
    assert_eq!(
        renderer.instance_stream_count(),
        0,
        "the legacy path holds no instance streams"
    );
    assert_eq!(
        instanced, legacy,
        "instanced 2-vertex segments must be byte-identical to the legacy path"
    );
}

/// Multi-vertex `LineList` batches (each consecutive vertex pair an independent
/// segment) must also match the legacy path byte-for-byte.
#[test]
fn multi_vertex_line_batch_matches_legacy_pixels() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    // One 8-vertex rectangle outline (four disjoint segments) plus a 4-vertex
    // two-segment batch, arranged so no two segments overlap.
    let batches = vec![
        lines_batch(rectangle_lines()),
        lines_batch(vec![
            [-0.9, -0.9, 0.0],
            [-0.6, -0.9, 0.0],
            [0.6, -0.9, 0.0],
            [0.9, -0.9, 0.0],
        ]),
    ];
    let (instanced, stats) = render_pixels(&mut renderer, &target, &batches, true, camera_2d());
    assert_eq!(
        renderer.instance_segment_count(),
        6,
        "4 + 2 segments must be instanced"
    );
    assert_eq!(stats.draw_calls, 1);

    let (legacy, _) = render_pixels(&mut renderer, &target, &batches, false, camera_2d());
    assert_eq!(
        instanced, legacy,
        "multi-vertex LineList batches must match the legacy path byte-for-byte"
    );
}

/// A mixed scene (converted line batches + a legacy mesh) must render exactly as
/// the all-legacy path: the instanced run is interleaved at its original draw
/// position, not reordered.
#[test]
fn mixed_scene_instanced_and_legacy_matches_legacy_pixels() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    let mesh = quad_mesh(
        0.2,
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        1.0,
        FULL_COVER_NORMAL,
        0,
    );
    let batches = vec![
        lines_batch(vec![[-0.9, 0.9, 0.0], [0.9, 0.9, 0.0]]),
        mesh,
        lines_batch(vec![[-0.9, -0.9, 0.0], [0.9, -0.9, 0.0]]),
    ];
    let (instanced, stats) = render_pixels(&mut renderer, &target, &batches, true, camera_2d());
    assert_eq!(stats.instanced_batches, 2, "both line batches converted");
    assert_eq!(
        stats.draw_calls, 3,
        "two coalesced runs cannot merge across the interleaved mesh"
    );

    let (legacy, _) = render_pixels(&mut renderer, &target, &batches, false, camera_2d());
    assert_eq!(
        instanced, legacy,
        "a mixed instanced/legacy frame must equal the all-legacy frame"
    );
}

/// Many 2-vertex batches collapse to one draw and still match the legacy frame;
/// the per-batch uniform arena is irrelevant to the instanced path.
#[test]
fn many_line_batches_collapse_and_match_legacy() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    let batches = grid_line_batches(6, 6); // 36 non-overlapping segments
    let (instanced, stats) = render_pixels(&mut renderer, &target, &batches, true, camera_2d());
    assert_eq!(renderer.instance_segment_count(), 36);
    assert_eq!(stats.draw_calls, 1, "36 batches -> one instanced draw");
    assert_eq!(stats.instanced_draws, 1);

    let (legacy, legacy_stats) =
        render_pixels(&mut renderer, &target, &batches, false, camera_2d());
    assert_eq!(legacy_stats.draw_calls, 36, "legacy path draws each batch");
    assert_eq!(
        instanced, legacy,
        "collapsed frame must equal the legacy frame"
    );
}

/// The upload byte charge must account the exact instance-stream bytes.
#[test]
fn instanced_upload_charges_exact_stream_bytes() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    let batches = grid_line_batches(4, 4); // 16 segments, 32 vertices
    let (_, stats) = render_pixels(&mut renderer, &target, &batches, true, camera_2d());
    // A fresh renderer has uploaded only this scene: 16 segments * 56 B.
    assert_eq!(
        stats.uploaded_bytes,
        16 * cad_render_wgpu::LINE_INSTANCE_BYTES as u64,
        "the frame byte budget must charge the exact instance stride"
    );
    assert_eq!(renderer.instance_segment_count(), 16);

    // The legacy path uploads 32 vertices * 12 B, a different exact number.
    let (_, legacy_stats) = render_pixels(&mut renderer, &target, &batches, false, camera_2d());
    assert_eq!(
        legacy_stats.uploaded_bytes - stats.uploaded_bytes,
        32 * 12,
        "the legacy path uploads its packed vertex bytes"
    );
}

/// A camera move (pan/zoom) must be a small uniform write only: the static
/// instance streams are never re-uploaded.
#[test]
fn camera_move_does_not_reupload_instance_streams() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    let batches = grid_line_batches(4, 4);
    renderer.set_line_instancing(true);
    renderer.clear_batches();
    renderer.upload(&scene(batches)).expect("upload grid");
    let uploads_after_upload = renderer.instance_stream_uploads();
    assert!(
        uploads_after_upload > 0,
        "the scene upload must create a stream"
    );

    let mut camera = camera_2d();
    for step in 0..8 {
        camera.center.x = step as f64 * 0.05;
        camera.world_per_px = 2.0 / 64.0 * (1.0 + step as f64 * 0.1);
        renderer
            .render(camera, &target)
            .expect("render pan/zoom frame");
    }
    assert_eq!(
        renderer.instance_stream_uploads(),
        uploads_after_upload,
        "pan/zoom must not re-upload the static instance streams"
    );
    assert_eq!(renderer.instance_segment_count(), 16);
}

/// Toggling the instancing flag without re-uploading must not leave the resident
/// scene with dangling stream references: the existing streams keep rendering.
#[test]
fn toggling_instancing_without_reupload_still_renders() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    renderer
        .upload(&scene(vec![lines_batch(rectangle_lines())]))
        .expect("upload");
    assert_eq!(renderer.instance_segment_count(), 4);
    renderer.set_line_instancing(false);
    // No re-upload: the resident scene must still be renderable.
    renderer
        .render(camera_2d(), &target)
        .expect("render after a toggle without re-upload");
    renderer.set_line_instancing(true);
    renderer
        .render(camera_2d(), &target)
        .expect("render after re-enabling");
}

/// The precision contract holds on the instanced path: a small segment at a very
/// large shared origin still rasterizes under the camera, and matches the legacy
/// frame byte-for-byte (the `f64` camera subtraction happens on the CPU).
#[test]
fn instanced_lines_keep_precision_at_large_coordinates() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    let origin = Point3 {
        x: 1.0e7,
        y: 1.0e7,
        z: 0.0,
    };
    let mut first = lines_batch(vec![[-0.5, -0.5, 0.0], [0.5, -0.5, 0.0]]);
    first.local_origin = origin;
    let mut second = lines_batch(vec![[-0.5, 0.5, 0.0], [0.5, 0.5, 0.0]]);
    second.local_origin = origin;
    let batches = vec![first, second];
    let camera = camera_2d_at(1.0e7, 1.0e7);

    let (instanced, _) = render_pixels(&mut renderer, &target, &batches, true, camera);
    let background = opaque_black(&instanced);
    assert!(
        instanced
            .chunks_exact(4)
            .any(|px| px != background.as_slice()),
        "a segment at 1e7 centred under the camera must still rasterize"
    );

    let (legacy, _) = render_pixels(&mut renderer, &target, &batches, false, camera);
    assert_eq!(
        instanced, legacy,
        "instanced large-coordinate segments must match the legacy path"
    );
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

/// Deep-zoom quality/parity fence. The camera is zoomed ~4.4e7:1 (5 µm/px) onto
/// one object at the far end of a 14 km drawing. A whole-scene shared origin (the
/// removed behaviour) would store that object's positions ~7e6 mm from the
/// origin, where the f32 ulp is 0.5 mm, and — combined with the
/// `sx·(origin − centre)` uniform term — move the vertical object to a different
/// pixel column. Per-chunk origins keep every stored coordinate within
/// `MAX_STREAM_RADIUS` (≈1 µm ulp), so the instanced frame stays within a small
/// pixel bound of the legacy per-batch path. This is deliberately **not** a
/// byte-identity claim: only the f32-representable shared-origin case is
/// byte-identical.
#[test]
fn instanced_deep_zoom_on_a_large_drawing_stays_within_the_precision_bound() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    let batches = large_extent_clusters();
    // Frame the vertical object at world x = 6e6 + 5000, offset by 0.1 mm (20
    // px) from the centre so the object is off-centre: an object exactly at the
    // camera centre would map to clip x = 0 under any origin (the `sx·p` and
    // `sx·(origin−centre)` terms cancel exactly), hiding the error.
    let camera = Camera2d {
        center: Point3 {
            x: 6.0e6 + 5000.1,
            y: 0.0,
            z: 0.0,
        },
        world_per_px: 0.005,
        z_plane: 0.0,
    };

    let (instanced, stats) = render_pixels(&mut renderer, &target, &batches, true, camera);
    assert!(
        stats.instanced_batches > 0,
        "the large-extent scene must convert to instance streams"
    );
    assert!(
        renderer.instance_stream_count() > 1,
        "a 14 km drawing must split into several per-chunk streams"
    );
    let (legacy, _) = render_pixels(&mut renderer, &target, &batches, false, camera);

    let background = opaque_black(&legacy);
    assert!(
        legacy.chunks_exact(4).any(|px| px != background.as_slice()),
        "the framed object must rasterize; the comparison would be vacuous"
    );
    let diff = differing_pixels(&instanced, &legacy, 4);
    assert!(
        diff <= 4,
        "instanced deep-zoom must stay within a small pixel bound of the legacy \
         path (positions bounded by MAX_STREAM_RADIUS); {diff} pixels differed"
    );
}

/// Overlapping segments with DISTINCT colours must coalesce into one run that
/// preserves the tie order. The coalesced instanced frame must equal the legacy
/// frame where the same segments are drawn separately; a reversed draw order is a
/// non-vacuous control that must differ, so the fence can detect a reorder.
#[test]
fn overlapping_distinct_color_runs_preserve_tie_order() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    let geometry = vec![[-0.9, 0.0, 0.0], [0.9, 0.0, 0.0]];
    let two_overlapping = |first: [f32; 3], second: [f32; 3]| {
        let mut a = lines_batch(geometry.clone());
        a.color = first;
        a.color_unresolved = false;
        a.draw_order = 0;
        let mut b = lines_batch(geometry.clone());
        b.color = second;
        b.color_unresolved = false;
        b.draw_order = 1;
        vec![a, b]
    };
    let forward = two_overlapping([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
    let reverse = two_overlapping([0.0, 0.0, 1.0], [1.0, 0.0, 0.0]);

    let (instanced, stats) = render_pixels(&mut renderer, &target, &forward, true, camera_2d());
    assert_eq!(
        stats.instanced_batches, 2,
        "both overlapping segments convert"
    );
    assert_eq!(
        stats.instanced_draws, 1,
        "the two overlapping segments coalesce into one run"
    );
    let (legacy, _) = render_pixels(&mut renderer, &target, &forward, false, camera_2d());
    assert_eq!(
        instanced, legacy,
        "coalescing distinct-colour overlapping segments must preserve the tie order"
    );

    let (reverse_legacy, _) = render_pixels(&mut renderer, &target, &reverse, false, camera_2d());
    assert_ne!(
        legacy, reverse_legacy,
        "draw order must decide the winner; otherwise the equality above is vacuous"
    );
}

/// Runs must not merge across a draw-order gap. Three segments are packed in
/// upload order but drawn in a different order (non-monotonic `draw_order`), so
/// their instance slices are only partly adjacent in draw order; the gap must
/// force a second coalesced draw, and the frame must still equal the legacy path.
#[test]
fn non_monotonic_draw_order_does_not_merge_runs_across_the_gap() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    let mut a = lines_batch(vec![[-0.9, -0.6, 0.0], [-0.2, -0.6, 0.0]]);
    a.draw_order = 2;
    let mut b = lines_batch(vec![[-0.9, 0.0, 0.0], [-0.2, 0.0, 0.0]]);
    b.draw_order = 0;
    let mut c = lines_batch(vec![[-0.9, 0.6, 0.0], [-0.2, 0.6, 0.0]]);
    c.draw_order = 1;
    // Upload order a, b, c; draw order b, c, a -> slices (1,2), (2,3), (0,1).
    let batches = vec![a, b, c];

    let (instanced, stats) = render_pixels(&mut renderer, &target, &batches, true, camera_2d());
    assert_eq!(stats.instanced_batches, 3);
    assert_eq!(
        stats.instanced_draws, 2,
        "the draw-order gap must split the run into (b,c) + (a), not merge all three"
    );
    let (legacy, _) = render_pixels(&mut renderer, &target, &batches, false, camera_2d());
    assert_eq!(instanced, legacy, "draw order must be preserved end to end");
}

/// A single batch larger than the injected per-stream cap is split across several
/// streams: no segment is lost, no empty stream is created and the frame matches
/// the legacy path.
#[test]
fn a_batch_beyond_the_stream_cap_splits_without_loss() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    renderer.set_max_instances_per_stream(3);
    // One batch of 8 disjoint segments (> cap 3): 3 + 3 + 2.
    let mut vertices = Vec::new();
    for i in 0..8 {
        let x = -0.9 + i as f32 * 0.22;
        vertices.push([x, 0.3, 0.0]);
        vertices.push([x + 0.12, 0.45, 0.0]);
    }
    let batches = vec![lines_batch(vertices)];
    let (instanced, stats) = render_pixels(&mut renderer, &target, &batches, true, camera_2d());
    assert_eq!(renderer.instance_segment_count(), 8, "no segment lost");
    assert_eq!(
        renderer.instance_stream_count(),
        3,
        "ceil(8 / 3) streams, and none of them empty"
    );
    assert_eq!(stats.instanced_batches, 1);
    assert_eq!(
        stats.instanced_draws, 3,
        "one coalesced draw per split piece"
    );
    let (legacy, _) = render_pixels(&mut renderer, &target, &batches, false, camera_2d());
    assert_eq!(instanced, legacy, "splitting must not change the frame");
}

/// Incremental `keep_prefix` commit plus stream GC must keep converted batches'
/// stream references valid through renumbering, and render exactly like the
/// equivalent batches uploaded in one go.
#[test]
fn incremental_commit_and_gc_renumber_converted_batches() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    renderer.set_line_instancing(true);
    renderer.clear_batches();

    // Three base batches (one stream), a two-batch overlay A (second stream),
    // then overlay B replaces A (keep_prefix = 3), dropping A's stream. The
    // overlay sits at a different y so it is visibly distinct from the base.
    let base = || grid_line_batches(1, 3);
    let overlay = |count: usize| -> Vec<RenderBatch> {
        (0..count)
            .map(|k| {
                let x = -0.9 + k as f32 * 0.22;
                lines_batch(vec![[x, -0.3, 0.0], [x + 0.12, -0.15, 0.0]])
            })
            .collect()
    };
    renderer.upload(&scene(base())).expect("upload base");
    assert_eq!(renderer.instance_stream_count(), 1);
    let staged = renderer
        .prepare_upload(&scene(overlay(2)))
        .expect("stage overlay A");
    renderer.commit_upload(staged, 3).expect("commit overlay A");
    assert_eq!(renderer.batch_count(), 5);
    assert_eq!(renderer.instance_stream_count(), 2);
    let staged = renderer
        .prepare_upload(&scene(overlay(1)))
        .expect("stage overlay B");
    renderer.commit_upload(staged, 3).expect("commit overlay B");
    assert_eq!(renderer.batch_count(), 4, "overlay A's batch is replaced");
    assert_eq!(
        renderer.instance_stream_count(),
        2,
        "overlay A's stream is collected and the survivors renumbered"
    );
    assert_eq!(renderer.instance_segment_count(), 4, "3 base + 1 overlay");

    // The same batches uploaded in one go render identically.
    let expected = render_pixels(
        &mut renderer,
        &target,
        &{
            let mut all = base();
            all.extend(overlay(1));
            all
        },
        true,
        camera_2d(),
    )
    .0;

    // Rebuild the incremental sequence and compare.
    renderer.clear_batches();
    renderer.upload(&scene(base())).expect("upload base");
    let staged = renderer
        .prepare_upload(&scene(overlay(2)))
        .expect("stage overlay A");
    renderer.commit_upload(staged, 3).expect("commit overlay A");
    let staged = renderer
        .prepare_upload(&scene(overlay(1)))
        .expect("stage overlay B");
    renderer.commit_upload(staged, 3).expect("commit overlay B");
    renderer
        .render(camera_2d(), &target)
        .expect("render incremental scene");
    let incremental = renderer
        .read_target_rgba()
        .expect("read incremental")
        .pixels;
    assert_eq!(
        incremental, expected,
        "GC renumbering must not change the rendered frame"
    );
}

/// Perspective (3D) line batches go through the instanced gate too; the instanced
/// frame must match the legacy perspective frame byte-for-byte (the f32
/// `translate_left` origin path).
#[test]
fn instanced_lines_match_legacy_in_perspective() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    let batches = vec![
        lines_batch(vec![[-0.8, -0.8, 0.0], [0.8, -0.8, 0.0]]),
        lines_batch(vec![[-0.8, 0.8, 0.0], [0.8, 0.8, 0.0]]),
    ];
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

    renderer.set_line_instancing(true);
    renderer.clear_batches();
    renderer.upload(&scene(batches.clone())).expect("upload 3D");
    let stats = renderer
        .render_3d(camera, &target)
        .expect("render 3D instanced");
    assert_eq!(stats.instanced_batches, 2, "3D line batches convert");
    let instanced = renderer
        .read_target_rgba()
        .expect("read 3D instanced")
        .pixels;
    assert!(
        instanced
            .chunks_exact(4)
            .any(|px| px != opaque_black(&instanced).as_slice()),
        "3D instanced frame rasterized nothing"
    );

    renderer.set_line_instancing(false);
    renderer.clear_batches();
    renderer.upload(&scene(batches)).expect("upload 3D legacy");
    renderer
        .render_3d(camera, &target)
        .expect("render 3D legacy");
    let legacy = renderer.read_target_rgba().expect("read 3D legacy").pixels;
    assert_eq!(
        instanced, legacy,
        "instanced 3D line batches must match the legacy perspective path"
    );
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

/// Flat-merged mesh runs must be byte-identical to the legacy per-batch mesh
/// path: the concatenated chunk reproduces the individual draws exactly (baked
/// tint, per-vertex colour, same order). A non-vacuous control (a different
/// frame) proves the fence can detect a change.
#[test]
fn merged_meshes_match_legacy_pixels() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    let batches = quad_grid_batches(6, 6, false);

    let (merged, stats) =
        render_pixels_full(&mut renderer, &target, &batches, true, true, camera_2d());
    assert!(
        renderer.mesh_chunk_count() >= 1,
        "the mesh batches must convert to merged chunks"
    );
    assert_eq!(
        stats.merged_mesh_batches, 36,
        "every mesh batch must be drawn through the merged path"
    );
    assert!(
        stats.mesh_draws < 36,
        "36 batches must collapse into fewer merged draws, got {}",
        stats.mesh_draws
    );
    let background = opaque_black(&merged);
    assert!(
        merged.chunks_exact(4).any(|px| px != background.as_slice()),
        "merged mesh frame rasterized nothing"
    );

    let (legacy, legacy_stats) =
        render_pixels_full(&mut renderer, &target, &batches, false, false, camera_2d());
    assert_eq!(legacy_stats.draw_calls, 36, "legacy draws each mesh batch");
    assert_eq!(
        merged, legacy,
        "merged mesh runs must be byte-identical to the legacy per-batch path"
    );

    // Non-vacuous control: moving one quad changes the frame.
    let mut moved = batches.clone();
    moved[0].local_origin.x += 0.05;
    let (changed, _) = render_pixels_full(&mut renderer, &target, &moved, true, true, camera_2d());
    assert_ne!(merged, changed, "the fence must be able to detect a change");
}

/// A glyph-heavy mixed scene (meshes + lines + mesh edge overlays together) must
/// render byte-identically to the all-legacy path. The scene is a
/// non-overlapping grid, so this fences the *path* equality (merging meshes
/// across an intervening line batch, instancing the edge overlays) without
/// relying on coplanar fragment order; the overlapping-tie fence below covers
/// the equal-depth reorder case.
#[test]
fn merged_meshes_lines_and_overlays_match_legacy_pixels() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);

    // Interleave meshes (some with wireframe overlays) and plain line segments,
    // all on the same non-overlapping grid.
    let mut batches = Vec::new();
    for k in 0..18 {
        let x = -0.9 + (k % 6) as f64 * 0.3;
        let y = -0.9 + (k / 6) as f64 * 0.3;
        batches.push(merged_quad(
            0.08,
            [x, y, 0.0],
            [0.2 + 0.03 * k as f32, 0.7, 0.3],
            k as u128 + 1,
            k % 3 == 0,
            0,
        ));
        // A horizontal line segment in the gap to the right of the quad.
        let mut line = lines_batch(vec![
            [(x + 0.16) as f32, y as f32, 0.0],
            [(x + 0.26) as f32, y as f32, 0.0],
        ]);
        line.draw_order = 0;
        batches.push(line);
    }

    let (merged, stats) =
        render_pixels_full(&mut renderer, &target, &batches, true, true, camera_2d());
    assert!(
        stats.merged_mesh_batches >= 18,
        "all 18 meshes must merge, got {}",
        stats.merged_mesh_batches
    );
    assert!(
        stats.instanced_batches >= 18,
        "all 18 line batches and the 6 overlays must instance, got {}",
        stats.instanced_batches
    );
    assert!(
        stats.draw_calls < batches.len() as u64,
        "the mixed scene must collapse, got {} draws for {} batches",
        stats.draw_calls,
        batches.len()
    );

    let (legacy, legacy_stats) =
        render_pixels_full(&mut renderer, &target, &batches, false, false, camera_2d());
    assert_eq!(
        legacy_stats.draw_calls,
        batches.len() as u64,
        "the legacy path draws one draw per batch"
    );
    let background = opaque_black(&legacy);
    assert!(
        legacy.chunks_exact(4).any(|px| px != background.as_slice()),
        "the mixed scene must rasterize something; the comparison would be vacuous"
    );
    assert_eq!(
        merged, legacy,
        "a glyph-heavy mixed scene must match the all-legacy frame byte-for-byte"
    );
}

/// The hard overlapping-tie case the non-overlapping grid fence cannot reach:
/// mesh-mesh, line-mesh and line-line fragments that share pixels at *equal*
/// depth. Under the frame-discipline invariant (mesh `Less`, line `LessEqual`,
/// both depth-writing, replace-blend) the merged/instanced path must reproduce
/// the legacy winners exactly.
#[test]
fn merged_overlapping_ties_match_legacy_pixels() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);

    // Two fully overlapping quads (mesh-mesh tie: the first drawn wins under
    // `Less`) and two collinear segments (line-line tie: the later drawn wins
    // under `LessEqual`), all coplanar at z = 0 so the segments also tie the
    // quads (line-mesh tie: the line wins).
    let overlap_scene = |order: [i64; 4]| {
        let mut l1 = lines_batch(vec![[-0.6, 0.0, 0.0], [0.6, 0.0, 0.0]]);
        l1.color = [0.0, 0.0, 1.0];
        l1.color_unresolved = false;
        l1.draw_order = order[2];
        let mut l2 = lines_batch(vec![[-0.6, 0.0, 0.0], [0.6, 0.0, 0.0]]);
        l2.color = [1.0, 1.0, 0.0];
        l2.color_unresolved = false;
        l2.draw_order = order[3];
        vec![
            merged_quad(0.5, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], 1, false, order[0]),
            merged_quad(0.5, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0], 2, false, order[1]),
            l1,
            l2,
        ]
    };
    let forward = overlap_scene([0, 1, 2, 3]);
    let reverse = overlap_scene([3, 2, 1, 0]);

    let (merged, stats) =
        render_pixels_full(&mut renderer, &target, &forward, true, true, camera_2d());
    assert_eq!(stats.merged_mesh_batches, 2, "both meshes must merge");
    assert_eq!(stats.instanced_batches, 2, "both segments must instance");
    let background = opaque_black(&merged);
    assert!(
        merged.chunks_exact(4).any(|px| px != background.as_slice()),
        "the overlapping scene must rasterize something"
    );

    let (legacy, _) =
        render_pixels_full(&mut renderer, &target, &forward, false, false, camera_2d());
    assert_eq!(
        merged, legacy,
        "merged/instanced overlapping ties must match the legacy winners exactly"
    );

    // Non-vacuous control: reversing the draw order changes which quad and which
    // segment win their ties, so the fence above can detect a reorder.
    let (legacy_reverse, _) =
        render_pixels_full(&mut renderer, &target, &reverse, false, false, camera_2d());
    assert_ne!(
        legacy, legacy_reverse,
        "draw order must decide the tie winners; otherwise the equality above is vacuous"
    );
}

/// The 3D perspective merged-vs-legacy fence. Unlike the 2D ortho camera, a
/// `Camera3d` gives every vertex a different clip depth, so equal-depth ties are
/// not produced by construction; the merged chunk must still reproduce the
/// legacy per-batch perspective frame.
#[test]
fn merged_meshes_match_legacy_in_perspective() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    // A grid of quads carrying *absolute* vertices and a shared zero origin, with
    // a per-quad depth offset. Both paths then narrow the same `f32` origin and
    // use identical per-vertex data, so the fence isolates the merge itself while
    // the perspective projection gives each quad a distinct clip depth.
    let mut batches = quad_grid_batches(3, 3, false);
    for (k, batch) in batches.iter_mut().enumerate() {
        let o = batch.local_origin;
        let dz = k as f32 * 0.03;
        for v in &mut batch.vertices {
            v[0] += o.x as f32;
            v[1] += o.y as f32;
            v[2] += o.z as f32 + dz;
        }
        batch.local_origin = Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
    }
    let camera = camera_3d();

    renderer.set_line_instancing(true);
    renderer.set_mesh_merging(true);
    renderer.clear_batches();
    renderer
        .upload(&scene(batches.clone()))
        .expect("upload 3D merged");
    let stats = renderer
        .render_3d(camera, &target)
        .expect("render 3D merged");
    assert_eq!(stats.merged_mesh_batches, 9, "all 9 meshes must merge");
    assert!(
        stats.mesh_draws < 9,
        "9 meshes must collapse into fewer 3D draws, got {}",
        stats.mesh_draws
    );
    let merged = renderer.read_target_rgba().expect("read 3D merged").pixels;
    let background = opaque_black(&merged);
    assert!(
        merged.chunks_exact(4).any(|px| px != background.as_slice()),
        "3D merged frame rasterized nothing"
    );

    renderer.set_line_instancing(false);
    renderer.set_mesh_merging(false);
    renderer.clear_batches();
    renderer.upload(&scene(batches)).expect("upload 3D legacy");
    let legacy_stats = renderer
        .render_3d(camera, &target)
        .expect("render 3D legacy");
    assert_eq!(
        legacy_stats.draw_calls, 9,
        "legacy draws each 3D mesh batch"
    );
    let legacy = renderer.read_target_rgba().expect("read 3D legacy").pixels;
    assert_eq!(
        merged, legacy,
        "merged meshes must match the legacy perspective frame byte-for-byte"
    );
}

/// A varying per-vertex colour (a HATCH-style gradient) with a non-white tint is
/// the case where baking `tint * c_i` at upload re-associates the f32 product
/// versus the legacy per-pixel `tint * lerp(c_i)`. This is the honest fence for
/// that scoping: the two paths must agree within a small tolerance (they are not
/// guaranteed bit-identical, and are documented as such).
#[test]
fn merged_gradient_mesh_matches_legacy_within_tolerance() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);

    // One quad with four distinct corner colours and a non-white batch tint.
    let mut batch = merged_quad(0.7, [0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 1, false, 0);
    batch.colors = vec![
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 1.0, 0.0],
    ];
    batch.color = [0.7, 0.8, 0.9];
    batch.color_unresolved = false;
    let batches = vec![batch];

    let (merged, stats) =
        render_pixels_full(&mut renderer, &target, &batches, true, true, camera_2d());
    assert_eq!(stats.merged_mesh_batches, 1);
    let (legacy, _) =
        render_pixels_full(&mut renderer, &target, &batches, false, false, camera_2d());
    // On the reference adapter (Quadro P620 / Vulkan) this ramp is currently
    // byte-identical (0 pixels differ), but bit-identity is *not* guaranteed: the
    // merged path interpolates `tint * c_i` while the legacy path interpolates
    // `c_i` then multiplies by `tint`, so a vendor whose f32 rounding differs may
    // see a 1-ULP divergence. The fence therefore allows ≤1/255 per channel.
    assert_eq!(
        differing_pixels(&merged, &legacy, 1),
        0,
        "a baked gradient must stay within 1/255 of the legacy per-pixel product"
    );
    // A gradient frame must actually vary (otherwise the fence is vacuous).
    assert!(
        merged
            .chunks_exact(4)
            .any(|px| px != merged[0..4].to_vec().as_slice()),
        "the gradient must produce more than one colour"
    );
}

/// The overlay-order residual: `mesh A + overlay A`, an intervening line batch,
/// then a contiguous `mesh B` must keep the legacy
/// `A.tri, A.overlay, line, B.tri` order. Without flushing the pending mesh run
/// when the line run ends, the merged `A+B.tri` run could draw after the line.
///
/// This exercises the interleaving end-to-end. Note the *pixel* outcome is
/// identical on the reference adapter either way: under a conforming depth test
/// the reorder is unobservable (a line wins an equal-depth tie regardless of
/// order, and non-equal depths are decided by depth, not order). The guarantee
/// is structural — the flush in the frame loop — and this fence pins that the
/// interleaving at least keeps rendering the legacy frame (a regression guard
/// against a change that drops or duplicates the mesh/overlay geometry).
#[test]
fn merged_mesh_overlay_line_mesh_keeps_legacy_order() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);

    // A and B overlap fully (shared origin); the line crosses them at equal
    // depth, so any reorder of `A.tri` relative to the line is observable only
    // through the depth test (which must still resolve line-wins at the tie).
    let batches = vec![
        merged_quad(0.5, [0.0, 0.0, 0.0], [0.9, 0.2, 0.2], 1, true, 0),
        {
            let mut l = lines_batch(vec![[-0.9, 0.0, 0.0], [0.9, 0.0, 0.0]]);
            l.color = [0.1, 0.1, 0.9];
            l.color_unresolved = false;
            l.draw_order = 1;
            l
        },
        merged_quad(0.5, [0.0, 0.0, 0.0], [0.2, 0.9, 0.2], 2, true, 2),
    ];

    let (merged, stats) =
        render_pixels_full(&mut renderer, &target, &batches, true, true, camera_2d());
    assert_eq!(stats.merged_mesh_batches, 2, "both meshes must merge");
    assert_eq!(
        stats.instanced_batches, 3,
        "the two overlays and the line must instance"
    );
    let background = opaque_black(&merged);
    assert!(
        merged.chunks_exact(4).any(|px| px != background.as_slice()),
        "the interleaved scene must rasterize something"
    );

    let (legacy, _) =
        render_pixels_full(&mut renderer, &target, &batches, false, false, camera_2d());
    assert_eq!(
        merged, legacy,
        "the interleaving must keep the legacy mesh/overlay/line order"
    );
}

/// Merged runs must not merge across a draw-order gap: the same batches packed in
/// upload order but drawn in a different order must still render in draw order.
#[test]
fn merged_mesh_runs_preserve_draw_order_with_distinct_colors() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);

    // Three fully overlapping quads (same z, same geometry) with distinct uniform
    // per-vertex colours. For opaque meshes the strict `Less` depth test makes the
    // FIRST drawn quad win a depth tie, so draw order decides the visible colour.
    let overlapping = |order: [i64; 3]| {
        vec![
            merged_quad(0.5, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], 1, false, order[0]),
            merged_quad(0.5, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0], 2, false, order[1]),
            merged_quad(0.5, [0.0, 0.0, 0.0], [0.0, 0.0, 1.0], 3, false, order[2]),
        ]
    };
    let forward = overlapping([0, 1, 2]);
    let reverse = overlapping([2, 1, 0]);

    let (merged, stats) =
        render_pixels_full(&mut renderer, &target, &forward, true, true, camera_2d());
    assert_eq!(stats.merged_mesh_batches, 3);
    let (legacy, _) =
        render_pixels_full(&mut renderer, &target, &forward, false, false, camera_2d());
    assert_eq!(
        merged, legacy,
        "merged runs must preserve the per-vertex colour tie order"
    );

    let (reverse_legacy, _) =
        render_pixels_full(&mut renderer, &target, &reverse, false, false, camera_2d());
    assert_ne!(
        legacy, reverse_legacy,
        "draw order must decide the winner; otherwise the equality above is vacuous"
    );
}

/// A camera move (pan/zoom) must be a small uniform write only: neither the
/// merged mesh chunks nor the instanced streams are ever re-uploaded.
#[test]
fn camera_move_does_not_reupload_mesh_chunks() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    let mut batches = quad_grid_batches(3, 3, true);
    batches.extend(grid_line_batches(3, 3));
    renderer.set_line_instancing(true);
    renderer.set_mesh_merging(true);
    renderer.clear_batches();
    renderer
        .upload(&scene(batches))
        .expect("upload mixed scene");
    let mesh_uploads = renderer.mesh_chunk_uploads();
    let line_uploads = renderer.instance_stream_uploads();
    assert!(mesh_uploads > 0, "the scene upload must create a chunk");
    assert!(line_uploads > 0, "the scene upload must create a stream");

    let mut camera = camera_2d();
    for step in 0..8 {
        camera.center.x = step as f64 * 0.05;
        camera.world_per_px = 2.0 / 64.0 * (1.0 + step as f64 * 0.1);
        renderer
            .render(camera, &target)
            .expect("render pan/zoom frame");
    }
    assert_eq!(
        renderer.mesh_chunk_uploads(),
        mesh_uploads,
        "pan/zoom must not re-upload the static merged mesh chunks"
    );
    assert_eq!(
        renderer.instance_stream_uploads(),
        line_uploads,
        "pan/zoom must not re-upload the static instance streams"
    );
    assert!(renderer.mesh_chunk_count() > 0);
}

/// A single mesh batch larger than the injected vertex cap splits across several
/// chunks: no triangle is lost, no empty chunk is created, and the frame matches
/// the legacy path.
#[test]
fn mesh_batch_beyond_the_vertex_cap_splits_without_loss() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    renderer.set_max_vertices_per_mesh_stream(8);
    // One mesh batch of six quads (24 vertices, 12 triangles) far beyond cap 8.
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for k in 0..6u32 {
        let x = -0.9 + k as f32 * 0.3;
        let base = vertices.len() as u32;
        vertices.extend_from_slice(&[
            [x, -0.1, 0.0],
            [x, 0.1, 0.0],
            [x + 0.15, 0.1, 0.0],
            [x + 0.15, -0.1, 0.0],
        ]);
        indices.push([base, base + 2, base + 1]);
        indices.push([base, base + 3, base + 2]);
    }
    let mut batch = merged_quad(0.1, [0.0, 0.0, 0.0], [0.8, 0.4, 0.2], 1, false, 0);
    batch.vertices = vertices;
    batch.normals = vec![[0.0, 0.0, 1.0]; batch.vertices.len()];
    batch.colors = Vec::new();
    batch.indices = indices;
    let batches = vec![batch];

    let (merged, stats) =
        render_pixels_full(&mut renderer, &target, &batches, true, true, camera_2d());
    assert_eq!(stats.merged_mesh_batches, 1);
    assert!(
        stats.mesh_chunks >= 3,
        "24 vertices with cap 8 must split into several chunks, got {}",
        stats.mesh_chunks
    );
    assert_eq!(stats.triangles, 12, "splitting must not lose any triangle");
    let (legacy, _) =
        render_pixels_full(&mut renderer, &target, &batches, false, false, camera_2d());
    assert_eq!(merged, legacy, "splitting must not change the frame");
}

/// Instancing a mesh edge overlay must be byte-identical to the legacy wireframe
/// overlay draw (which uses the line pipeline over the shared mesh vertices).
#[test]
fn instanced_edge_overlay_matches_legacy_pixels() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    let batches = vec![
        merged_quad(0.4, [-0.4, 0.0, 0.0], [0.9, 0.9, 0.9], 1, true, 0),
        merged_quad(0.4, [0.4, 0.0, 0.0], [0.2, 0.6, 0.9], 2, true, 0),
    ];

    let (instanced, stats) =
        render_pixels_full(&mut renderer, &target, &batches, true, true, camera_2d());
    assert_eq!(
        stats.instanced_batches, 2,
        "both edge overlays must instance"
    );
    assert!(
        renderer.instance_segment_count() > 0,
        "the overlays must be resident in an instance stream"
    );
    let (legacy, _) =
        render_pixels_full(&mut renderer, &target, &batches, false, false, camera_2d());
    assert_eq!(
        instanced, legacy,
        "instanced edge overlays must be byte-identical to the legacy overlay draw"
    );
}

/// The upload byte charge for a merged mesh run must be exact: one interleaved
/// vertex per stored vertex (44 B) plus one `u32` per triangle index, replacing
/// the legacy per-batch position/normal/colour/index buffers.
#[test]
fn merged_mesh_upload_charges_exact_chunk_bytes() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    let batches = quad_grid_batches(3, 1, false); // 3 quads, 4 vertices / 6 indices

    let (_, stats) = render_pixels_full(&mut renderer, &target, &batches, true, true, camera_2d());
    let per_merged = 4 * 44 + 6 * 4; // 200
    assert_eq!(
        stats.uploaded_bytes,
        (3 * per_merged) as u64,
        "merged chunks must charge the exact 44 B/vertex + 4 B/index"
    );

    // The legacy per-batch mesh path charges 12 B position + 12 B normal +
    // 12 B colour per vertex and 4 B per triangle index = 168 B per quad.
    // `uploaded_bytes` is cumulative over the renderer's lifetime, so compare
    // the delta this upload added.
    let (_, legacy) =
        render_pixels_full(&mut renderer, &target, &batches, false, false, camera_2d());
    assert_eq!(
        legacy.uploaded_bytes - stats.uploaded_bytes,
        (3 * (4 * 12 + 4 * 12 + 4 * 12 + 6 * 4)) as u64,
        "the legacy mesh path charges its packed per-batch bytes"
    );

    // With an overlay the merged run also charges 56 B per instanced segment;
    // a quad's wireframe is 5 de-duplicated edges = 5 segments.
    let with_edges = quad_grid_batches(1, 1, true);
    let (_, edge_stats) =
        render_pixels_full(&mut renderer, &target, &with_edges, true, true, camera_2d());
    assert_eq!(
        edge_stats.uploaded_bytes - legacy.uploaded_bytes,
        (per_merged + 5 * cad_render_wgpu::LINE_INSTANCE_BYTES as usize) as u64,
        "the instanced overlay must charge the exact instance stride"
    );
}

/// Crossing the byte budget with merged mesh batches must be reported, never
/// silent: the second quad (200 B) crosses a 300-byte budget after the first.
#[test]
fn merged_mesh_byte_budget_is_reported_not_silent() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    renderer.set_mesh_merging(true);
    renderer.frame_budget = FrameBudget {
        max_vertices: usize::MAX,
        max_triangles: usize::MAX,
        max_bytes: 300,
    };
    let batches = quad_grid_batches(2, 1, false);
    renderer.clear_batches();
    renderer.upload(&scene(batches)).expect("upload two quads");
    let target = RenderTarget::new(64, 64);
    let stats = renderer
        .render(camera_2d(), &target)
        .expect("over-budget frame must still render the batch that fits");
    let over = stats
        .over_budget
        .expect("crossing the byte budget must be reported");
    assert_eq!(over.report.category, "bytes");
    assert_eq!(over.report.limit, 300);
    assert_eq!(over.report.requested, 400, "two merged quads at 200 B each");
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

/// Mirrored merged meshes must render exactly like the legacy mirrored mesh
/// path: the merged pipeline selects the same `front_face = Cw`, and the chunk
/// never spans both mirror states.
#[test]
fn mirrored_merged_meshes_match_legacy_pixels() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    let batches = vec![
        mirrored_quad([-0.35, 0.0, 0.0], [0.9, 0.3, 0.2], 1),
        mirrored_quad([0.35, 0.0, 0.0], [0.2, 0.6, 0.9], 2),
    ];

    let (merged, stats) =
        render_pixels_full(&mut renderer, &target, &batches, true, true, camera_2d());
    assert_eq!(stats.merged_mesh_batches, 2);
    let background = opaque_black(&merged);
    assert!(
        merged.chunks_exact(4).any(|px| px != background.as_slice()),
        "a mirrored quad must not be culled away"
    );
    let (legacy, _) =
        render_pixels_full(&mut renderer, &target, &batches, false, false, camera_2d());
    assert_eq!(
        merged, legacy,
        "mirrored merged meshes must match the legacy mirrored path byte-for-byte"
    );
}

/// Incremental `keep_prefix` commit plus chunk GC must keep converted mesh
/// batches' chunk references valid through renumbering, and render exactly like
/// the equivalent batches uploaded in one go.
#[test]
fn incremental_commit_and_gc_renumber_merged_meshes() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    renderer.set_line_instancing(true);
    renderer.set_mesh_merging(true);
    renderer.clear_batches();

    let base = || quad_grid_batches(1, 2, false);
    let overlay = |count: usize| -> Vec<RenderBatch> {
        (0..count)
            .map(|k| {
                merged_quad(
                    0.06,
                    [-0.4 + k as f64 * 0.4, 0.4, 0.0],
                    [0.9, 0.9, 0.1],
                    (10 + k) as u128,
                    false,
                    0,
                )
            })
            .collect()
    };
    renderer.upload(&scene(base())).expect("upload base");
    assert_eq!(renderer.mesh_chunk_count(), 1);
    let staged = renderer
        .prepare_upload(&scene(overlay(2)))
        .expect("stage overlay A");
    renderer.commit_upload(staged, 2).expect("commit overlay A");
    assert_eq!(renderer.batch_count(), 4);
    assert_eq!(renderer.mesh_chunk_count(), 2);
    let staged = renderer
        .prepare_upload(&scene(overlay(1)))
        .expect("stage overlay B");
    renderer.commit_upload(staged, 2).expect("commit overlay B");
    assert_eq!(renderer.batch_count(), 3, "overlay A's batch is replaced");
    assert_eq!(
        renderer.mesh_chunk_count(),
        2,
        "overlay A's chunk is collected and the survivors renumbered"
    );

    // The same batches uploaded in one go render identically.
    let mut all = base();
    all.extend(overlay(1));
    let expected = render_pixels_full(&mut renderer, &target, &all, true, true, camera_2d()).0;

    // Rebuild the incremental sequence and compare.
    renderer.clear_batches();
    renderer.upload(&scene(base())).expect("upload base");
    let staged = renderer
        .prepare_upload(&scene(overlay(2)))
        .expect("stage overlay A");
    renderer.commit_upload(staged, 2).expect("commit overlay A");
    let staged = renderer
        .prepare_upload(&scene(overlay(1)))
        .expect("stage overlay B");
    renderer.commit_upload(staged, 2).expect("commit overlay B");
    renderer
        .render(camera_2d(), &target)
        .expect("render incremental scene");
    let incremental = renderer
        .read_target_rgba()
        .expect("read incremental")
        .pixels;
    assert_eq!(
        incremental, expected,
        "chunk GC renumbering must not change the rendered frame"
    );
}

/// Sentinel: a `MeshEdges` batch currently renders **nothing**. The legacy
/// `draw_batch` reads `GpuBatch::indices`, which `prepare_upload_batches` leaves
/// empty for `MeshEdges` (the identity line list goes to `edge_indices`), so the
/// draw early-returns; the topology is also deliberately not converted to the
/// instanced path. This is a pre-existing gap (see `line_batch_is_instanced`'s
/// doc). The fence pins the current no-op so P3 cannot silently start dropping
/// *or* drawing this topology without a deliberate, tested change.
#[test]
fn mesh_edges_batch_currently_renders_nothing() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    let mut batch = lines_batch(vec![
        [-0.8, -0.8, 0.0],
        [0.8, -0.8, 0.0],
        [0.8, -0.8, 0.0],
        [0.8, 0.8, 0.0],
    ]);
    batch.topology = RenderTopology::MeshEdges;

    // Legacy path (no conversion): the batch is submitted but draws nothing.
    let (legacy, legacy_stats) = render_pixels_full(
        &mut renderer,
        &target,
        &[batch.clone()],
        false,
        false,
        camera_2d(),
    );
    assert_eq!(legacy_stats.draw_calls, 1, "the batch is submitted once");
    let background = opaque_black(&legacy);
    assert!(
        legacy.chunks_exact(4).all(|px| px == background.as_slice()),
        "a MeshEdges batch must currently render nothing (pre-existing no-op)"
    );

    // Same under the converted paths: the topology is not converted.
    let (converted, _) =
        render_pixels_full(&mut renderer, &target, &[batch], true, true, camera_2d());
    assert!(
        converted
            .chunks_exact(4)
            .all(|px| px == background.as_slice()),
        "MeshEdges must stay a no-op on the instanced/merged paths too"
    );
}

/// Progressive-page boundary inside a merged run: a page that accepts only part
/// of a merged chunk must neither lose nor duplicate geometry — the accumulated
/// final page must equal the single full-budget frame.
#[test]
fn progressive_page_boundary_splits_a_merged_run_without_loss() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    // 8 non-overlapping quads that all merge into one chunk (one run per batch).
    let batches = quad_grid_batches(2, 4, false);

    renderer.set_line_instancing(true);
    renderer.set_mesh_merging(true);
    renderer.clear_batches();
    renderer
        .upload(&scene(batches.clone()))
        .expect("upload merged scene");
    assert_eq!(
        renderer.mesh_chunk_count(),
        1,
        "all 8 quads must share one chunk"
    );

    // Reference: the full-budget frame coalesces the whole chunk into one draw.
    let reference_stats = renderer
        .render(camera_2d(), &target)
        .expect("render full-budget frame");
    assert_eq!(reference_stats.merged_mesh_batches, 8);
    assert_eq!(
        reference_stats.mesh_draws, 1,
        "the full-budget frame coalesces the whole run"
    );
    let reference = renderer.read_target_rgba().expect("read full frame").pixels;

    // Force one merged batch per page (a merged quad charges 200 B), so the page
    // boundary falls *inside* the merged run.
    renderer.frame_budget = FrameBudget {
        max_vertices: usize::MAX,
        max_triangles: usize::MAX,
        max_bytes: 200,
    };
    renderer.set_progressive_rendering(true);
    let mut pages = 0;
    loop {
        let stats = renderer.render(camera_2d(), &target).expect("render page");
        assert_eq!(
            stats.merged_mesh_batches, 1,
            "each page must accept exactly one merged batch"
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
        renderer.read_target_rgba().expect("read pages").pixels,
        reference,
        "a page boundary inside a merged run must not lose or duplicate geometry"
    );
}
