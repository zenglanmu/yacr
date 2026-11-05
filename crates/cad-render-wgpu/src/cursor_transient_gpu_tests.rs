//! Real-GPU contract for the renderer-owned transient cursor (ADR 0028 phase 2a):
//! the base is untouched, cursor pixels move without any commit, the instance
//! buffer handle is stable across moves, a click through the crosshair picks the
//! entity behind it, and `FrameStats` counts the transient draw exactly once.
//!
//! Requires a real adapter (P620/Vulkan): run serially with
//! `cargo test -p cad-render-wgpu --lib cursor_transient_moves -- --ignored --test-threads=1`.
//! Without an adapter the test skips with a printed reason (never a pass).

use crate::headless::{create_headless_gpu, HeadlessGpu};
use crate::{BackendPreference, Camera2d, FrameStats, RenderTarget, Renderer};
use cad_domain::{DocumentId, EntityId, InstancePath, Point3, SelectionRef};
use cad_scene::{RenderBatch, RenderTopology, TransientCursor};

use crate::plan::LINE_INSTANCE_STRIDE;

const SIZE: u32 = 64;
const BASE_ENTITY: u128 = 7;
/// Cursor draw order between the tool preview (2_000_000) and snap hints
/// (3_000_000); the renderer only sorts by the value.
const CURSOR_DRAW_ORDER: i64 = 2_500_000;

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

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
                "[cursor-transient-gpu] adapter backend={} name={} device_type={} driver={}",
                gpu.adapter.backend, gpu.adapter.name, gpu.adapter.device_type, gpu.adapter.driver
            );
            Some(gpu)
        }
        Err(error) => {
            eprintln!("[cursor-transient-gpu] SKIP — no headless adapter ({error}); NOT a pass");
            None
        }
    }
}

fn camera() -> Camera2d {
    Camera2d {
        center: p(0.0, 0.0),
        world_per_px: 2.0 / f64::from(SIZE),
        z_plane: 0.0,
    }
}

/// Three stacked horizontal segments (about three pixels) forming one solid band
/// at world y = -0.6, so a centre pixel is fully covered in both colour and id.
fn base_band() -> RenderBatch {
    let mut vertices = Vec::new();
    for offset in [-2.0_f64, 0.0, 2.0] {
        let y = (-0.6 + offset * 2.0 / f64::from(SIZE)) as f32;
        vertices.push([-0.9, y, 0.0]);
        vertices.push([0.9, y, 0.0]);
    }
    RenderBatch {
        conics: Vec::new(),
        local_origin: p(0.0, 0.0),
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
        sources: vec![source(BASE_ENTITY)],
        draw_order: 0,
    }
}

/// A vertical transient segment crossing the base band; `x` moves between frames.
fn cursor_at(x: f64) -> TransientCursor {
    TransientCursor {
        segments: vec![[p(x, -0.95), p(x, 0.1)]],
        color: [0.72, 0.75, 0.80, 1.0],
        draw_order: CURSOR_DRAW_ORDER,
    }
}

/// Sample point on the transient arm that is above the base band.
const CURSOR_SAMPLE_Y: f64 = -0.2;
/// A base-band point away from both cursor positions.
const BASE_SAMPLE: (f64, f64) = (0.7, -0.6);

fn to_pixel(x: f64, y: f64) -> (u32, u32) {
    let px = ((x + 1.0) * f64::from(SIZE) / 2.0).floor();
    let py = ((1.0 - y) * f64::from(SIZE) / 2.0).floor();
    (
        px.clamp(0.0, f64::from(SIZE - 1)) as u32,
        py.clamp(0.0, f64::from(SIZE - 1)) as u32,
    )
}

fn pixel(pixels: &[u8], (x, y): (u32, u32)) -> [u8; 4] {
    let index = ((y * SIZE + x) * 4) as usize;
    [
        pixels[index],
        pixels[index + 1],
        pixels[index + 2],
        pixels[index + 3],
    ]
}

/// Any ink in the pixel or its immediate neighbours (anti-aliased 1 px strokes
/// can land on either side of the analytic coordinate).
fn has_ink(pixels: &[u8], center: (u32, u32)) -> bool {
    let (cx, cy) = center;
    for dy in -1i32..=1 {
        for dx in -1i32..=1 {
            let x = (cx as i32 + dx).clamp(0, SIZE as i32 - 1) as u32;
            let y = (cy as i32 + dy).clamp(0, SIZE as i32 - 1) as u32;
            let [r, g, b, _] = pixel(pixels, (x, y));
            if u32::from(r) + u32::from(g) + u32::from(b) > 0 {
                return true;
            }
        }
    }
    false
}

fn frame(renderer: &mut Renderer) -> (FrameStats, Vec<u8>) {
    let target = RenderTarget::new(SIZE, SIZE);
    let stats = renderer.render(camera(), &target).expect("render frame");
    let pixels = renderer
        .read_target_rgba()
        .expect("read back the frame")
        .pixels;
    (stats, pixels)
}

fn assert_draw_identity(stats: &FrameStats, what: &str) {
    assert_eq!(
        stats.legacy_opaque_draws
            + stats.arc_draws
            + stats.instanced_draws
            + stats.mesh_draws
            + stats.transparent_batches as u64
            + stats.image_batches as u64
            + stats.transient_draws,
        stats.draw_calls,
        "{what}: the documented draw_calls identity (including transient_draws)"
    );
}

#[test]
#[ignore = "hardware GPU: run explicitly on a real adapter (P620/Vulkan)"]
fn cursor_transient_moves_without_touching_base_pixels_and_picks_through() {
    const WHAT: &str = "cursor_transient_gpu";
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = Renderer::new(BackendPreference::Auto);
    renderer
        .initialize_with_device(gpu.device, gpu.queue)
        .expect("initialize headless renderer");

    // Base: one run-free scene batch committed as the whole resident scene.
    let staged = renderer
        .prepare_upload_batches(&[base_band()])
        .expect("stage the base band");
    renderer
        .commit_upload(staged, 0)
        .expect("commit the base band");

    let (base_stats, base_pixels) = frame(&mut renderer);
    assert_eq!(
        base_stats.transient_draws, 0,
        "{WHAT}: no cursor means no transient draw"
    );
    assert_draw_identity(&base_stats, WHAT);
    let base_sample = to_pixel(BASE_SAMPLE.0, BASE_SAMPLE.1);
    let base_rgba = pixel(&base_pixels, base_sample);
    assert!(
        base_rgba[0] as u32 + base_rgba[1] as u32 + base_rgba[2] as u32 > 0,
        "{WHAT}: the fixture band must be drawn at {base_sample:?}"
    );
    let sample_a = to_pixel(0.0, CURSOR_SAMPLE_Y);
    let sample_b = to_pixel(0.45, CURSOR_SAMPLE_Y);
    assert!(
        !has_ink(&base_pixels, sample_a),
        "{WHAT}: no cursor yet (A)"
    );
    assert!(
        !has_ink(&base_pixels, sample_b),
        "{WHAT}: no cursor yet (B)"
    );

    // Move A: a transient, never a scene commit.
    assert!(renderer.set_cursor_transient(Some(cursor_at(0.0))));
    let (a_stats, a_pixels) = frame(&mut renderer);
    assert_eq!(
        a_stats.transient_draws, 1,
        "{WHAT}: the cursor is exactly one instanced draw"
    );
    assert_draw_identity(&a_stats, WHAT);
    // The base pixels are unchanged (the overlay is copied on top; the band is
    // outside the cursor).
    assert_eq!(
        pixel(&a_pixels, base_sample),
        base_rgba,
        "{WHAT}: a cursor move must not touch base pixels"
    );
    assert!(has_ink(&a_pixels, sample_a), "{WHAT}: cursor A is drawn");
    assert!(!has_ink(&a_pixels, sample_b), "{WHAT}: cursor B is absent");
    let buffer_a = renderer
        .cursor_transient_buffer
        .as_ref()
        .cloned()
        .expect("the transient buffer is created lazily");
    assert_eq!(
        buffer_a.size(),
        u64::from(LINE_INSTANCE_STRIDE) * cad_scene::MAX_TRANSIENT_CURSOR_SEGMENTS as u64,
        "{WHAT}: the buffer is the fixed maximum size"
    );

    // Pick-through: the crosshair covers the band pixel but has no pick key, so
    // the id pass resolves the entity behind it.
    let pick_xy = to_pixel(0.0, -0.6);
    let hit = renderer
        .pick_at(pick_xy.0, pick_xy.1)
        .expect("pick")
        .expect("the base band is under the crosshair");
    assert_eq!(
        hit.source.map(|source| source.entity),
        Some(EntityId(BASE_ENTITY)),
        "{WHAT}: a click through the crosshair must hit the entity behind it"
    );

    // Move B: the same buffer handle is reused; only its contents change.
    assert!(renderer.set_cursor_transient(Some(cursor_at(0.45))));
    let (b_stats, b_pixels) = frame(&mut renderer);
    assert_eq!(b_stats.transient_draws, 1, "{WHAT}");
    assert_draw_identity(&b_stats, WHAT);
    assert_eq!(
        renderer.cursor_transient_buffer.as_ref(),
        Some(&buffer_a),
        "{WHAT}: two moves must not recreate the instance buffer"
    );
    assert!(
        !has_ink(&b_pixels, sample_a),
        "{WHAT}: cursor A pixels are gone after the move"
    );
    assert!(has_ink(&b_pixels, sample_b), "{WHAT}: cursor B is drawn");
    assert_eq!(
        pixel(&b_pixels, base_sample),
        base_rgba,
        "{WHAT}: the base band is still untouched"
    );

    // Clearing removes the transient draw and leaves the base intact (the overlay
    // is recomposited from the copied base + cleared depth every frame).
    assert!(renderer.set_cursor_transient(None));
    let (cleared_stats, cleared_pixels) = frame(&mut renderer);
    assert_eq!(cleared_stats.transient_draws, 0, "{WHAT}");
    assert_draw_identity(&cleared_stats, WHAT);
    assert!(!has_ink(&cleared_pixels, sample_b), "{WHAT}");
    assert_eq!(pixel(&cleared_pixels, base_sample), base_rgba, "{WHAT}");

    // `clear_batches` keeps the CPU transient (a document teardown, not a cursor
    // clear); a device rebuild drops only its GPU buffer.
    renderer.clear_batches();
    assert!(
        renderer.cursor_transient().is_none(),
        "{WHAT}: the fixture cleared the transient above; nothing to keep"
    );
    assert!(renderer.set_cursor_transient(Some(cursor_at(0.0))));
    renderer.clear_batches();
    assert!(
        renderer.cursor_transient().is_some(),
        "{WHAT}: clear_batches must keep the CPU transient cursor"
    );
    let target = RenderTarget::new(SIZE, SIZE);
    assert!(renderer.rebuild_device(&target).is_err());
    assert!(
        renderer.cursor_transient_buffer.is_none(),
        "{WHAT}: a device rebuild drops the transient GPU buffer"
    );
    assert!(
        renderer.cursor_transient().is_some(),
        "{WHAT}: the CPU transient survives a device rebuild"
    );
    assert!(
        renderer.cursor_transient_dirty,
        "{WHAT}: the recreated buffer must be re-uploaded after the rebuild"
    );
}
