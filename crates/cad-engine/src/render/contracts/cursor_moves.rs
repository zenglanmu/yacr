use super::*;

// ---------------------------------------------------------------------------
// A plain cursor move is a renderer transient, not a scene mutation (real
// adapter). ADR 0028 phase 2b: the cursor left the scene/preparation/commit
// chain entirely; only `set_cursor_transient` + a frame invalidation remain.
// ---------------------------------------------------------------------------

/// Four lines forming a frame: entity 1 is the bottom horizontal (the one the
/// cursor hovers), 2 the top horizontal, 3/4 the verticals. At 400x240 with
/// `world_per_px = 1.0` they sit on unambiguous pixels.
#[cfg(not(target_arch = "wasm32"))]
fn db_with_four_lines() -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    for (id, (a, b_)) in [
        (1u128, (p(-80.0, -40.0), p(80.0, -40.0))),
        (2, (p(-80.0, 40.0), p(80.0, 40.0))),
        (3, (p(-80.0, -40.0), p(-80.0, 40.0))),
        (4, (p(80.0, -40.0), p(80.0, 40.0))),
    ] {
        b.insert_entity(line_entity(id, SpaceId::Model, a, b_))
            .unwrap();
    }
    b.finish().unwrap()
}

/// Probe points: the hovered bottom line, the three untouched base lines and
/// empty background.
#[cfg(not(target_arch = "wasm32"))]
const BOTTOM_LINE: [f64; 2] = [60.0, -40.0];
#[cfg(not(target_arch = "wasm32"))]
const BASE_PROBES: [[f64; 2]; 3] = [[60.0, 40.0], [-80.0, 0.0], [80.0, 0.0]];
#[cfg(not(target_arch = "wasm32"))]
const BACKGROUND: [f64; 2] = [0.0, 20.0];

/// The 400x240 view's exact world rectangle at `world_per_px = 1.0`.
#[cfg(not(target_arch = "wasm32"))]
fn view_bounds() -> (Point3, Point3) {
    (p(-200.0, -120.0), p(200.0, 120.0))
}

/// The renderer transient geometry a host would feed for a 2D view: the same
/// `cursor_geometry` call the host makes, at 20 % of the viewport dimension total
/// (10 % per arm) so the drawn pixels are local to the cursor (a full-view
/// crosshair would cover every probe).
#[cfg(not(target_arch = "wasm32"))]
fn cursor_at(world: [f64; 2]) -> cad_scene::TransientCursor {
    let (min, max) = view_bounds();
    cad_app::render_scene::cursor_geometry(
        Some(p(world[0], world[1])),
        Some((min, max)),
        Some(1.0),
        3.0,
        20,
    )
    .expect("a valid 2D cursor has transient geometry")
}

/// The last rendered colour target's RGBA pixels.
#[cfg(not(target_arch = "wasm32"))]
fn target_pixels(session: &RenderSessionHandle) -> (u32, u32, Vec<u8>) {
    let mut state = session.0.borrow_mut();
    let readback = state
        .runtime
        .renderer
        .as_mut()
        .expect("attached renderer")
        .read_target_rgba()
        .expect("read back the frame");
    (readback.width, readback.height, readback.pixels)
}

#[cfg(not(target_arch = "wasm32"))]
fn pixel_at(frame: &(u32, u32, Vec<u8>), world: [f64; 2]) -> [u8; 4] {
    let (x, y) = world_to_pixel_2d(frame.0, frame.1, world);
    let index = ((y * frame.0 + x) * 4) as usize;
    [
        frame.2[index],
        frame.2[index + 1],
        frame.2[index + 2],
        frame.2[index + 3],
    ]
}

/// Whether a pixel window around a world point contains ink (anti-aliased 1 px
/// strokes can land on either side of the analytic coordinate).
#[cfg(not(target_arch = "wasm32"))]
fn has_ink(frame: &(u32, u32, Vec<u8>), world: [f64; 2]) -> bool {
    let (x, y) = world_to_pixel_2d(frame.0, frame.1, world);
    for dy in -1i32..=1 {
        for dx in -1i32..=1 {
            let px = (i64::from(x) + i64::from(dx)).clamp(0, i64::from(frame.0) - 1) as u32;
            let py = (i64::from(y) + i64::from(dy)).clamp(0, i64::from(frame.1) - 1) as u32;
            let index = ((py * frame.0 + px) * 4) as usize;
            if u32::from(frame.2[index])
                + u32::from(frame.2[index + 1])
                + u32::from(frame.2[index + 2])
                > 0
            {
                return true;
            }
        }
    }
    false
}

/// Renderer commit/draw-plan counters that a pure transient move must not move:
/// `(draw_plan_rebuilds, gc_skipped_commits, pulled_suffix_skips)`.
#[cfg(not(target_arch = "wasm32"))]
fn freeze_counters(session: &RenderSessionHandle) -> (u64, u64, u64) {
    let state = session.0.borrow();
    let renderer = state.runtime.renderer.as_ref().expect("attached renderer");
    (
        renderer.draw_plan_rebuilds(),
        renderer.gc_skipped_commits(),
        renderer.pulled_suffix_skips(),
    )
}

/// A session with the four-line fixture attached and committed; axes/grid off so
/// the only overlay pixels are the ones each test adds.
#[cfg(not(target_arch = "wasm32"))]
fn committed_session() -> RenderSessionHandle {
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    let gpu = headless_gpu();
    session.attach_device(gpu.device, gpu.queue).unwrap();
    session.sync_drawing(Some(Arc::new(db_with_four_lines())));
    session.set_overlay_visibility(OverlayVisibility {
        axes: false,
        grid: false,
        ..OverlayVisibility::default()
    });
    session.prepare().unwrap();
    drive_uploads(&session);
    session.apply_ready_updates().unwrap();
    session
}

/// One frame with the transient set (a move tick presents without preparing,
/// applying or committing anything).
#[cfg(not(target_arch = "wasm32"))]
fn present_transient(
    session: &RenderSessionHandle,
    cursor: cad_scene::TransientCursor,
) -> (FrameCompleteness, (u32, u32, Vec<u8>)) {
    assert!(
        session.set_cursor_transient(Some(cursor)),
        "a moved cursor changes the transient state"
    );
    let completeness = session
        .render([400.0, 240.0], 1.0)
        .unwrap()
        .expect("a dirty transient presents a frame")
        .completeness;
    (completeness, target_pixels(session))
}

/// **ADR 0028 contract (real adapter).** Eight plain cursor moves:
/// * each presents `CurrentScene` without preparing/applying/committing,
/// * `overlay_suffix_commits` stays flat and no upload observation is produced,
/// * GC/pulled/draw-plan counters stay flat,
/// * the committed base pixels never change and the cursor pixels move,
/// * a click through the crosshair still picks the entity behind it.
#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "hardware GPU: run explicitly on a real adapter (P620/Vulkan)"]
fn plain_cursor_move_is_a_renderer_transient_not_a_suffix_commit() {
    let session = committed_session();

    // Baseline cursor position and frame.
    let (baseline_completeness, baseline) = present_transient(&session, cursor_at([-10.0, -20.0]));
    assert_eq!(baseline_completeness, FrameCompleteness::CurrentScene);
    let plain_line = pixel_at(&baseline, BOTTOM_LINE);
    let base_pixels: Vec<[u8; 4]> = BASE_PROBES
        .iter()
        .map(|probe| pixel_at(&baseline, *probe))
        .collect();
    assert_eq!(
        pixel_at(&baseline, BACKGROUND),
        [0, 0, 0, 255],
        "the probe outside every line must be background"
    );

    let full_batches = renderer_batch_count(&session);
    let suffix_commits = session.overlay_suffix_commits();
    let counters = freeze_counters(&session);
    // Drain any observation the fixture's base commit left behind.
    let _ = session.take_upload_timing_observation();

    let mut cursor_x = -70.0;
    let mut cursor_y = -20.0;
    for step in 0..8 {
        // Move farther than the pickbox half-side (3 px) + AA so the previous
        // position's pixels are provably cleared.
        cursor_x += 12.0;
        cursor_y += 6.0;
        let (completeness, pixels) = present_transient(&session, cursor_at([cursor_x, cursor_y]));
        assert_eq!(
            completeness,
            FrameCompleteness::CurrentScene,
            "step {step}: a transient move presents the current scene"
        );
        assert_eq!(
            session.overlay_suffix_commits(),
            suffix_commits,
            "step {step}: a pure move must not commit a suffix"
        );
        assert!(
            session.take_upload_timing_observation().is_none(),
            "step {step}: a pure move must not stage or commit an upload"
        );
        assert_eq!(
            freeze_counters(&session),
            counters,
            "step {step}: no GC/pulled/draw-plan work may be counted"
        );
        assert_eq!(
            renderer_batch_count(&session),
            full_batches,
            "step {step}: the resident batch set is untouched"
        );
        for (probe, expected) in BASE_PROBES.iter().zip(base_pixels.iter()) {
            assert_eq!(
                pixel_at(&pixels, *probe),
                *expected,
                "step {step}: a cursor move must not change base geometry"
            );
        }
        assert_eq!(
            pixel_at(&pixels, BOTTOM_LINE),
            plain_line,
            "step {step}: the committed line colour is unchanged"
        );
        // The cursor is drawn at its new position.
        assert!(
            has_ink(&pixels, [cursor_x, cursor_y]),
            "step {step}: the transient crosshair/pickbox is drawn at the new cursor"
        );
        // ...and the previous cursor position is cleared again (overlay recopy +
        // depth clear). Asserted on the exact old pixel, which is outside the
        // moved pickbox/arms.
        assert_eq!(
            pixel_at(&pixels, [cursor_x - 12.0, cursor_y - 6.0]),
            [0, 0, 0, 255],
            "step {step}: the previous cursor pixels must not linger"
        );
    }

    // Pick-through: park the crosshair on the bottom line and pick there. The
    // cursor has no pick key, so the id pass resolves the line behind it.
    let _ = present_transient(&session, cursor_at(BOTTOM_LINE));
    let (px, py) = world_to_pixel_2d(400, 240, BOTTOM_LINE);
    let hit = {
        let mut state = session.0.borrow_mut();
        state
            .runtime
            .renderer
            .as_mut()
            .expect("attached renderer")
            .pick_at(px, py)
            .expect("pick")
    };
    let hit = hit.expect("the committed line under the crosshair is pickable");
    assert_eq!(
        hit.source.map(|source| source.entity),
        Some(EntityId(1)),
        "a click through the crosshair must pick the entity behind it"
    );
}

/// A live LINE preview with one captured point (a committed overlay batch).
#[cfg(not(target_arch = "wasm32"))]
fn line_preview(cursor: [f64; 2]) -> cad_app::DrawPreview {
    cad_app::DrawPreview {
        kind: cad_app::DrawToolKind::Line,
        points: vec![p(-100.0, 0.0)],
        picks: Vec::new(),
        cursor: Some(p(cursor[0], cursor[1])),
        candidate: None,
        remaining: 1,
        selection_count: 0,
        ready: false,
        polygon_sides: None,
        awaiting_text: false,
        text_prefill: None,
    }
}

/// **ADR 0028 non-material case (real adapter).** A committed overlay batch (a
/// draw-preview rubber band) and the renderer transient cursor co-exist in one
/// frame: the preview stays a committed suffix batch, the cursor is drawn on top
/// without adding a commit or changing the suffix.
#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "hardware GPU: run explicitly on a real adapter (P620/Vulkan)"]
fn committed_overlay_and_transient_cursor_share_a_frame() {
    let session = committed_session();
    let batches_before = renderer_batch_count(&session);
    // Publish a committed draw-preview overlay (a rubber band near x <= -20).
    assert!(session.set_draw_preview(Some(line_preview([-20.0, 60.0]))));
    session.prepare().unwrap();
    drive_uploads(&session);
    session.apply_ready_updates().unwrap();
    let suffix_after_preview = session.overlay_suffix_commits();
    let batches_after_preview = renderer_batch_count(&session);
    assert!(
        batches_after_preview > batches_before,
        "the committed draw-preview overlay suffix must be resident \
         (before {batches_before}, after {batches_after_preview})"
    );

    // One frame: the committed preview AND the transient cursor are visible.
    let (completeness, pixels) = present_transient(&session, cursor_at([0.0, -20.0]));
    assert_eq!(completeness, FrameCompleteness::CurrentScene);
    assert!(
        has_ink(&pixels, [0.0, -20.0]),
        "the transient cursor is drawn in the same frame"
    );
    // The preview's first segment midpoint (captured (-100, 0) -> cursor (-20, 60)).
    const PREVIEW_PROBE: [f64; 2] = [-60.0, 30.0];
    let preview_pixel = pixel_at(&pixels, PREVIEW_PROBE);
    assert_ne!(
        preview_pixel,
        [0, 0, 0, 255],
        "the committed draw-preview batch is drawn in the same frame"
    );

    // A further pure move does not touch the committed overlay suffix.
    let (_, moved) = present_transient(&session, cursor_at([20.0, -10.0]));
    assert_eq!(
        session.overlay_suffix_commits(),
        suffix_after_preview,
        "moving the transient must not commit or replace the overlay suffix"
    );
    assert_eq!(
        renderer_batch_count(&session),
        batches_after_preview,
        "the batch set is unchanged by the move"
    );
    assert_eq!(
        pixel_at(&moved, PREVIEW_PROBE),
        preview_pixel,
        "the committed preview batch's pixels are unchanged"
    );
    assert!(
        has_ink(&moved, [20.0, -10.0]),
        "the moved transient is still drawn"
    );
}
