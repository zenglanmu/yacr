//! **Regression fence: a progressive restart must never make already-drawn
//! geometry vanish.**
//!
//! Progressive rendering is always on for the native session. Before the fix,
//! [`Renderer::render`] restarting with `progressive_frame == None` cleared the
//! color target at cursor 0 and then drew only page 0. Two mutation paths dropped
//! the cached frame even though the resident geometry was unchanged or only the
//! suffix changed, so a hover (flag-row update) or an overlay-suffix commit made
//! the next presented frame show **page 0 alone** until the walk caught up — the
//! user-observed TCH flicker at 142,399 ordered batches.
//!
//! These tests pin the base/overlay architecture at the renderer boundary with a
//! 6-batch base whose page budget holds exactly one batch per page:
//!
//! * a **flag-row change** (hover/selection tint) restarts the base walk at
//!   cursor 0, loads the retained base colour and clears only depth, so every
//!   page's pixels survive the first frame and the tint is applied as the walk
//!   redraws (the overlay is recomposited on top every frame), and
//! * a **suffix-only overlay commit** changes only the transient overlay: the
//!   base target and the base walk are untouched, and the overlay is composited
//!   on top of the copied base on the very next frame.
//!
//! A GPU is required; without an adapter every test skips with a clear message
//! rather than pretending to pass. Run serially:
//!
//! ```text
//! cargo test -p cad-render-wgpu --test progressive_restart_flash --locked -- --test-threads=1
//! ```

use cad_domain::{DocumentId, EntityId, InstancePath, Point3, SelectionRef};
use cad_render_wgpu::headless::{create_headless_gpu, HeadlessGpu, RgbaImage};
use cad_render_wgpu::{BackendPreference, Camera2d, RenderTarget, Renderer, FLAG_SELECTED};
use cad_scene::{RenderBatch, RenderTopology};

/// One batch per page: six vertices / two triangles per one-segment line batch.
const BATCH_COUNT: usize = 6;
const TARGET_SIZE: u32 = 64;
const FIRST_Y: f32 = -0.8;
const STEP_Y: f32 = 0.1;
/// The last base entity (entities are `1..=BATCH_COUNT`).
const LAST_ENTITY: u128 = BATCH_COUNT as u128;
/// The transient overlay suffix appended by case B.
const OVERLAY_Y: f32 = 0.6;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A distinct source entity so `set_entity_flags` can target one batch.
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
        Ok(gpu) => {
            eprintln!(
                "progressive restart flash adapter: {} ({}, {})",
                gpu.adapter.name, gpu.adapter.backend, gpu.adapter.device_type
            );
            Some(gpu)
        }
        Err(error) => {
            eprintln!("skipping progressive restart flash test: {error}; NOT a pass");
            None
        }
    }
}

/// One horizontal line batch (`y` fixed, `x` from -0.8 to 0.8) from `entity`.
fn line_batch(entity: u128, y: f32) -> RenderBatch {
    RenderBatch {
        conics: Vec::new(),
        local_origin: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        topology: RenderTopology::Lines,
        vertices: vec![[-0.8, y, 0.0], [0.8, y, 0.0]],
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

/// `BATCH_COUNT` independent one-segment line batches, each with its own source
/// entity and its own y line (`-0.8, -0.7, ..., -0.3`).
fn base_batches() -> Vec<RenderBatch> {
    (0..BATCH_COUNT)
        .map(|i| line_batch(i as u128 + 1, FIRST_Y + i as f32 * STEP_Y))
        .collect()
}

fn init(gpu: HeadlessGpu) -> Renderer {
    let HeadlessGpu {
        device,
        queue,
        adapter: _,
    } = gpu;
    let mut renderer = Renderer::new(BackendPreference::Auto);
    // The universal per-run path (progressive pages); the accelerated tier is
    // fenced in `tests/accelerated_tier.rs`.
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

/// A renderer with the 6-batch base scene resident, progressive rendering on and
/// a budget so small that exactly one batch fits a page.
///
/// The tiny `target_ms` keeps the pacing controller shrinking (never growing
/// past `1.0`), so the effective page budget can never hold two batches and the
/// one-batch-per-page premise stays true; when the scaled budget falls below one
/// batch the planner's progress floor still draws exactly the cursor batch.
fn paged_renderer(gpu: HeadlessGpu) -> (Renderer, RenderTarget) {
    let mut renderer = init(gpu);
    renderer.frame_budget.max_vertices = 6;
    renderer.frame_budget.max_triangles = 2;
    let staged = renderer
        .prepare_upload_batches(&base_batches())
        .expect("stage the base scene");
    renderer
        .commit_upload(staged, 0)
        .expect("commit the base scene");
    renderer.set_progressive_rendering(true);
    renderer.set_frame_budget_target_ms(0.001);
    let target = RenderTarget::new(TARGET_SIZE, TARGET_SIZE);
    (renderer, target)
}

/// Render pages until the progressive frame is complete and return the settled
/// frame. The page count and the reported progress are asserted so the fence can
/// never go vacuous: if the budget ever held the whole scene on page 0, a restart
/// would redraw everything and these tests would not reproduce anything.
fn settle(renderer: &mut Renderer, target: &RenderTarget) -> RgbaImage {
    let mut pages = 0usize;
    loop {
        renderer
            .render(camera_2d(), target)
            .expect("render a progressive page");
        pages += 1;
        if !renderer.frame_pending() {
            break;
        }
        assert!(
            pages < 4 * BATCH_COUNT,
            "the progressive frame did not settle after {pages} pages"
        );
    }
    assert_eq!(
        pages, BATCH_COUNT,
        "fixture: the page budget must hold exactly one batch per page, otherwise \
         a restart would still draw the whole scene on page 0 and this fence would \
         be vacuous"
    );
    let (cursor, total) = renderer
        .frame_progress()
        .expect("the progressive frame plan must exist after settling");
    assert_eq!(total, BATCH_COUNT, "fixture: every batch must be ordered");
    assert_eq!(
        cursor, total,
        "fixture: the progressive walk must have drawn every page before the \
         mutation under test"
    );
    renderer.read_target_rgba().expect("read the settled frame")
}

// ---------------------------------------------------------------------------
// Probe
// ---------------------------------------------------------------------------

/// The pixel row a world-space `y_world` line lands on. A `Camera2d` maps world
/// `+1` to the top row and `-1` to the bottom row.
fn line_row(image: &RgbaImage, y_world: f32) -> i64 {
    ((1.0 - y_world) * 0.5 * image.height as f32 - 0.5).round() as i64
}

/// Peak RGB luma in a small window around the world-space line at `y_world`.
///
/// The window is ±1 row (the lines are 3+ px apart) and ±2 columns around the
/// horizontal centre (every fixture line spans x -0.8..0.8 and therefore crosses
/// the centre column).
fn line_luma(image: &RgbaImage, y_world: f32) -> u8 {
    let row = line_row(image, y_world);
    let col = (image.width / 2) as i64;
    let mut peak = 0u8;
    for dy in -1..=1i64 {
        for dx in -2..=2i64 {
            let pixel = image.pixel((col + dx) as u32, (row + dy) as u32);
            peak = peak.max(pixel[0]).max(pixel[1]).max(pixel[2]);
        }
    }
    peak
}

/// The RGBA at the line's centre pixel (the probe `line_luma` peaks on for these
/// fixtures). Used to compare the plain vs tinted shade of the same line.
fn line_pixel(image: &RgbaImage, y_world: f32) -> [u8; 4] {
    image.pixel(image.width / 2, line_row(image, y_world) as u32)
}

/// The target's default clear is opaque black, so any real stroke is well above
/// this threshold (a 1 px anti-aliased white line peaks near 255).
const BACKGROUND_LUMA: u8 = 8;

fn line_y(index: usize) -> f32 {
    FIRST_Y + index as f32 * STEP_Y
}

fn assert_line_drawn(image: &RgbaImage, y_world: f32, context: &str) {
    let luma = line_luma(image, y_world);
    assert!(
        luma > BACKGROUND_LUMA,
        "{context}: the line at y={y_world} is background (peak luma {luma}); \
         already-drawn geometry vanished"
    );
}

fn assert_all_base_lines_drawn(image: &RgbaImage, context: &str) {
    for index in 0..BATCH_COUNT {
        assert_line_drawn(image, line_y(index), context);
    }
}

// ---------------------------------------------------------------------------
// Case A: a flag-row update preserves the retained colour across the restart
// ---------------------------------------------------------------------------

#[test]
fn flag_row_update_preserves_drawn_pages_and_redraws_the_tint() {
    let Some(gpu) = gpu() else {
        return;
    };
    let (mut renderer, target) = paged_renderer(gpu);
    // Enable the tint before settling so the toggle's own progressive reset cannot
    // be confused with the flag-row restart this case is about. With no bits set
    // the tint is a no-op on the settled frame.
    renderer.set_selection_flags_enabled(true);

    let settled = settle(&mut renderer, &target);
    assert_all_base_lines_drawn(&settled, "settled frame");
    let last_y = line_y(BATCH_COUNT - 1);
    let plain = line_pixel(&settled, last_y);

    // A transient overlay (a cursor-like line) is resident before the flag
    // update. It lives outside the base target, so the base target itself never
    // contains it; the presented frame must keep it visible across the flag
    // restart because the overlay is recomposited every frame.
    let overlay = renderer
        .prepare_upload_batches_legacy(&[line_batch(101, OVERLAY_Y)])
        .expect("stage a transient overlay");
    renderer
        .commit_upload_with_base(overlay, BATCH_COUNT, BATCH_COUNT)
        .expect("commit the transient overlay");
    renderer
        .render(camera_2d(), &target)
        .expect("composite the transient overlay");
    let with_overlay = renderer
        .read_target_rgba()
        .expect("read the frame with the transient overlay");
    assert_all_base_lines_drawn(&with_overlay, "frame with a transient overlay");
    assert_line_drawn(&with_overlay, OVERLAY_Y, "frame with a transient overlay");

    // A hover/selection-style flag update: only the flags-table row changes, no
    // geometry is re-uploaded. The base walk restarts at cursor 0, but the
    // retained base colour stays valid, so it loads colour and clears only depth.
    renderer
        .set_entity_flags(&[(source(LAST_ENTITY), FLAG_SELECTED)])
        .expect("select the last base entity");

    // Exactly one frame is presented after the mutation, as in the runtime.
    renderer
        .render(camera_2d(), &target)
        .expect("render the frame after the flag update");
    let after = renderer
        .read_target_rgba()
        .expect("read the frame after flags");

    // The fix: page 0 is repainted over loaded colour; every later page's pixels
    // are still there. A regression (clearing colour and drawing page 0 only)
    // leaves batches 1..=5 as background.
    assert_all_base_lines_drawn(
        &after,
        "one frame after a flag-row update: the restart must load the retained \
         colour, so no already-drawn page may vanish",
    );
    // The overlay is recomposited from the (partial) base every frame, so the
    // cursor-like overlay stays visible during the restart too.
    assert_line_drawn(
        &after,
        OVERLAY_Y,
        "one frame after a flag-row update: the transient overlay must be \
         recomposited on top of the base",
    );

    // Let the redraw walk finish, then prove the flag tint really was applied:
    // the selected line's shade must differ from the plain white stroke.
    while renderer.frame_pending() {
        renderer
            .render(camera_2d(), &target)
            .expect("render the remaining tint pages");
    }
    let tinted_frame = renderer
        .read_target_rgba()
        .expect("read the completed tinted frame");
    assert_all_base_lines_drawn(&tinted_frame, "completed tinted frame");
    assert_line_drawn(
        &tinted_frame,
        OVERLAY_Y,
        "completed tinted frame: the transient overlay must still be composited",
    );
    let tinted = line_pixel(&tinted_frame, last_y);
    assert_ne!(
        tinted, plain,
        "the selected line at y={last_y} must be tinted after the flag redraw \
         completes (plain {plain:?}, got {tinted:?})"
    );
}

// ---------------------------------------------------------------------------
// Case B: a suffix-only overlay commit never touches the base or its walk
// ---------------------------------------------------------------------------

#[test]
fn suffix_overlay_commit_recomposites_over_the_untouched_base_walk() {
    let Some(gpu) = gpu() else {
        return;
    };
    let (mut renderer, target) = paged_renderer(gpu);

    let settled = settle(&mut renderer, &target);
    assert_all_base_lines_drawn(&settled, "settled frame");
    assert_eq!(
        renderer.frame_progress(),
        Some((BATCH_COUNT, BATCH_COUNT)),
        "fixture: the base walk must be complete before the overlay commit"
    );

    // The runtime's deferred overlay refresh: stage one transient batch
    // **legacy** (the overlay pass draws it with per-batch uniforms only) and
    // commit it as a suffix on top of the whole resident base
    // (`keep_prefix == base_end == resident_batch_count`).
    let resident = renderer.batch_count();
    assert_eq!(resident, BATCH_COUNT, "the base scene must be resident");
    let overlay = renderer
        .prepare_upload_batches_legacy(&[line_batch(100, OVERLAY_Y)])
        .expect("stage the overlay suffix");
    renderer
        .commit_upload_with_base(overlay, resident, resident)
        .expect("commit the overlay suffix");
    assert_eq!(
        renderer.batch_count(),
        BATCH_COUNT + 1,
        "the overlay suffix must be resident"
    );
    assert_eq!(
        renderer.frame_progress(),
        Some((BATCH_COUNT, BATCH_COUNT)),
        "a suffix commit must not reset the base walk (the base prefix is unchanged)"
    );

    // Exactly one frame is presented after the commit, as in the runtime. The
    // base pass has no page work (cursor is complete, no clear); the base is
    // copied and the overlay is composited on top.
    renderer
        .render(camera_2d(), &target)
        .expect("render the frame after the suffix commit");
    let after = renderer
        .read_target_rgba()
        .expect("read the frame after the suffix commit");

    assert_all_base_lines_drawn(
        &after,
        "one frame after a suffix-only overlay commit: the untouched base must \
         still be presented under the overlay",
    );
    assert_line_drawn(
        &after,
        OVERLAY_Y,
        "one frame after a suffix-only overlay commit: the appended overlay must \
         be composited on top of the base",
    );
    assert_eq!(
        renderer.frame_progress(),
        Some((BATCH_COUNT, BATCH_COUNT)),
        "the base walk must still be complete after the overlay frame"
    );
}

// ---------------------------------------------------------------------------
// A suffix commit must not clear or redraw the base pixels
// ---------------------------------------------------------------------------

#[test]
fn suffix_commit_does_not_clear_the_base_pixels_or_reset_the_walk() {
    let Some(gpu) = gpu() else {
        return;
    };
    let (mut renderer, target) = paged_renderer(gpu);

    let before = settle(&mut renderer, &target);
    assert_eq!(
        renderer.frame_progress(),
        Some((BATCH_COUNT, BATCH_COUNT)),
        "fixture: the base walk must be complete before the suffix commit"
    );
    // Base line 0 is far below the overlay line below, so the overlay cannot
    // cover the probe.
    let probe_index = 0;
    let probe_y = line_y(probe_index);
    let before_pixel = line_pixel(&before, probe_y);

    let overlay = renderer
        .prepare_upload_batches_legacy(&[line_batch(100, 0.9)])
        .expect("stage the overlay suffix");
    renderer
        .commit_upload_with_base(overlay, BATCH_COUNT, BATCH_COUNT)
        .expect("commit the overlay suffix");

    renderer
        .render(camera_2d(), &target)
        .expect("render the frame after the suffix commit");
    let after = renderer
        .read_target_rgba()
        .expect("read the frame after the suffix commit");

    assert_eq!(
        line_pixel(&after, probe_y),
        before_pixel,
        "a suffix commit must not clear or redraw the base: the base probe ({probe_y}) \
         changed from {before_pixel:?} to {:?}",
        line_pixel(&after, probe_y)
    );
    assert_eq!(
        renderer.frame_progress(),
        Some((BATCH_COUNT, BATCH_COUNT)),
        "a suffix commit must not reset the base walk"
    );
}
