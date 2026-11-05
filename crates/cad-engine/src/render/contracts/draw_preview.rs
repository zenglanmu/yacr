use super::*;

// ---------------------------------------------------------------------------
// A live draw preview is a transient overlay suffix (real adapter).
// ---------------------------------------------------------------------------

/// Two horizontal base lines at y = ±40, x = -80..80. Both are outside the
/// preview region and far enough from the probes below to be unambiguous at
/// 400x240 with `world_per_px = 1.0`.
#[cfg(not(target_arch = "wasm32"))]
fn db_with_two_horizontal_lines() -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(line_entity(
        1,
        SpaceId::Model,
        p(-80.0, -40.0),
        p(80.0, -40.0),
    ))
    .unwrap();
    b.insert_entity(line_entity(
        2,
        SpaceId::Model,
        p(-80.0, 40.0),
        p(80.0, 40.0),
    ))
    .unwrap();
    b.finish().unwrap()
}

/// One pixel per base line. `x = 60` keeps both probes off the cursor crosshair
/// arms (the cursor's `x` stays in `[-10, 40]` and its `y` in `[12, 60]`) and off
/// the preview segment (which lives at `x <= -20`).
#[cfg(not(target_arch = "wasm32"))]
const BASE_PROBES: [[f64; 2]; 2] = [[60.0, -40.0], [60.0, 40.0]];
/// Empty background: not on a base line, not on the preview chain, not on any
/// crosshair arm used by this test.
#[cfg(not(target_arch = "wasm32"))]
const BACKGROUND: [f64; 2] = [0.0, 0.0];
/// The first preview segment's midpoint (captured point `(-100, 0)`, cursor
/// `(-20, 60)`), not on a crosshair arm. It exercises the rubber band.
#[cfg(not(target_arch = "wasm32"))]
const FIRST_PREVIEW_PROBE: [f64; 2] = [-60.0, 30.0];
#[cfg(not(target_arch = "wasm32"))]
const BACKGROUND_PIXEL: [u8; 4] = [0, 0, 0, 255];

/// The last rendered presented (overlay) target's RGBA pixels.
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

/// The runtime's resident base-batch prefix (the boundary the overlay suffix is
/// committed after). Test-only getter, reachable from this crate-internal module.
#[cfg(not(target_arch = "wasm32"))]
fn base_batch_prefix(session: &RenderSessionHandle) -> usize {
    session.0.borrow().runtime.base_batches_for_test()
}

/// A live LINE preview with one captured point; only `cursor` varies between
/// moves. Every field is set explicitly (the type has no `Default`).
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

/// Publish one preview input, let it commit, present one frame and read the
/// presented pixels back. Returns the overlay-suffix commit count **after** the
/// tick so each step can assert exactly one overlay-only commit.
#[cfg(not(target_arch = "wasm32"))]
fn preview_tick(
    session: &RenderSessionHandle,
    preview: Option<cad_app::DrawPreview>,
) -> (u64, (u32, u32, Vec<u8>)) {
    assert!(
        session.set_draw_preview(preview),
        "a changed preview must change the overlay inputs"
    );
    session.prepare().unwrap();
    drive_uploads(session);
    session.apply_ready_updates().unwrap();
    session
        .render([400.0, 240.0], 1.0)
        .unwrap()
        .expect("a frame is presented for the preview tick");
    (session.overlay_suffix_commits(), target_pixels(session))
}

/// **Regression fence: an in-progress draw preview is an overlay suffix and never
/// touches the base target, the base batch prefix or the base progressive walk.**
///
/// A LINE rubber band is not committed to `cad-db`: `DrawPreview` changes hash
/// into `overlay_fingerprint`, so the controller republishes only the transient
/// overlay `Arc` with the same `base_revision`. The runtime stages that group with
/// `prepare_upload_batches_legacy` and commits with
/// `commit_upload_with_base(prepared, keep_prefix = base_batches, base_end =
/// base_batches)`. This test drives that real session path on a real adapter and
/// pins the observable consequence: the base pixels and the base page walk stay
/// identical while the preview appears, moves and disappears.
#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "hardware GPU: run explicitly on a real adapter (P620/Vulkan)"]
fn live_draw_preview_is_an_overlay_suffix_and_never_touches_the_base() {
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    let gpu = headless_gpu();
    session.attach_device(gpu.device, gpu.queue).unwrap();
    session.sync_drawing(Some(Arc::new(db_with_two_horizontal_lines())));
    session.set_overlay_visibility(OverlayVisibility {
        axes: false,
        grid: false,
        ..OverlayVisibility::default()
    });
    // A still cursor away from every probe (and from every preview midpoint
    // below), so the baseline contains only the two base lines plus the cursor.
    // ADR 0028: the pointer cursor is a renderer transient now, fed directly.
    assert!(
        session.set_cursor_transient(cad_app::render_scene::cursor_geometry(
            Some(p(-150.0, 100.0)),
            Some((p(-200.0, -120.0), p(200.0, 120.0))),
            Some(1.0),
            3.0,
            100,
        ))
    );
    session.prepare().unwrap();
    drive_uploads(&session);
    session.apply_ready_updates().unwrap();
    session
        .render([400.0, 240.0], 1.0)
        .unwrap()
        .expect("a baseline frame is presented");

    // ---------------------------------------------------------------------
    // Settled baseline.
    // ---------------------------------------------------------------------
    let baseline = target_pixels(&session);
    let baseline_base_pixels: Vec<[u8; 4]> = BASE_PROBES
        .iter()
        .map(|probe| pixel_at(&baseline, *probe))
        .collect();
    for (probe, pixel) in BASE_PROBES.iter().zip(baseline_base_pixels.iter()) {
        assert_ne!(
            *pixel, BACKGROUND_PIXEL,
            "fixture: the base line at {probe:?} must be drawn in the baseline"
        );
    }
    assert_eq!(
        pixel_at(&baseline, BACKGROUND),
        BACKGROUND_PIXEL,
        "fixture: the background probe must be empty in the baseline"
    );
    let baseline_batches = renderer_batch_count(&session);
    let baseline_suffix_commits = session.overlay_suffix_commits();
    let baseline_base_prefix = base_batch_prefix(&session);
    let baseline_applied_base = runtime_applied_base(&session);
    let baseline_progress = session
        .drawing_progress()
        .expect("the baseline render starts the progressive frame");
    let (base_total, ordered_total) = baseline_progress;
    assert_eq!(
        base_total, ordered_total,
        "fixture: the baseline base walk must be complete"
    );
    assert_eq!(
        base_total, baseline_base_prefix,
        "fixture: the progressive walk covers exactly the base prefix"
    );
    assert!(
        !session.drawing_pages(),
        "fixture: the baseline base walk has no pending pages"
    );

    // ---------------------------------------------------------------------
    // First live preview frame: one overlay-only suffix commit.
    // ---------------------------------------------------------------------
    let (commits, first_preview) = preview_tick(&session, Some(line_preview([-20.0, 60.0])));
    assert_eq!(
        commits,
        baseline_suffix_commits + 1,
        "a live draw preview must be exactly one overlay-suffix commit"
    );
    assert!(
        renderer_batch_count(&session) > baseline_batches,
        "the preview batch must be resident in the overlay suffix ({} vs baseline {})",
        renderer_batch_count(&session),
        baseline_batches
    );
    assert_eq!(
        base_batch_prefix(&session),
        baseline_base_prefix,
        "a preview must not change the base batch prefix"
    );
    assert_eq!(
        runtime_applied_base(&session),
        baseline_applied_base,
        "a preview must not change the applied base revision"
    );
    for (probe, expected) in BASE_PROBES.iter().zip(baseline_base_pixels.iter()) {
        assert_eq!(
            pixel_at(&first_preview, *probe),
            *expected,
            "a preview must not touch the base target (probe {probe:?})"
        );
    }
    let preview_pixel = pixel_at(&first_preview, FIRST_PREVIEW_PROBE);
    assert_ne!(
        preview_pixel, BACKGROUND_PIXEL,
        "the preview rubber band must be visible at {FIRST_PREVIEW_PROBE:?}"
    );
    for (probe, base_pixel) in BASE_PROBES.iter().zip(baseline_base_pixels.iter()) {
        assert_ne!(
            preview_pixel, *base_pixel,
            "the preview segment must not be the base line colour at {probe:?}"
        );
    }
    assert_eq!(
        session.drawing_progress(),
        Some((base_total, ordered_total)),
        "the suffix commit must not reset or shorten the base progressive walk"
    );
    assert!(
        !session.drawing_pages(),
        "the suffix commit must not leave base pages pending"
    );

    // ---------------------------------------------------------------------
    // Six simulated mouse moves while the LINE tool is live.
    // ---------------------------------------------------------------------
    let mut cursor = [-20.0, 60.0];
    let mut last_preview_probe = FIRST_PREVIEW_PROBE;
    for step in 0..6 {
        let before = session.overlay_suffix_commits();
        cursor[0] += 10.0;
        cursor[1] -= 8.0;
        let (commits, pixels) = preview_tick(&session, Some(line_preview(cursor)));
        assert_eq!(
            commits,
            before + 1,
            "step {step}: each live preview move is exactly one overlay-suffix commit"
        );
        assert!(
            renderer_batch_count(&session) > baseline_batches,
            "step {step}: the preview suffix must stay resident"
        );
        assert_eq!(
            base_batch_prefix(&session),
            baseline_base_prefix,
            "step {step}: a preview move must not change the base batch prefix"
        );
        assert_eq!(
            runtime_applied_base(&session),
            baseline_applied_base,
            "step {step}: a preview move must not change the applied base revision"
        );
        for (probe, expected) in BASE_PROBES.iter().zip(baseline_base_pixels.iter()) {
            assert_eq!(
                pixel_at(&pixels, *probe),
                *expected,
                "step {step}: a preview move must not touch the base target (probe {probe:?})"
            );
        }
        // The chain's midpoint for this cursor, deliberately off both crosshair
        // arms (`x = cursor.x` / `y = cursor.y`).
        let midpoint = [(-100.0 + cursor[0]) / 2.0, cursor[1] / 2.0];
        last_preview_probe = midpoint;
        let pixel = pixel_at(&pixels, midpoint);
        assert_ne!(
            pixel, BACKGROUND_PIXEL,
            "step {step}: the moved preview segment must be visible at {midpoint:?}"
        );
        for (probe, base_pixel) in BASE_PROBES.iter().zip(baseline_base_pixels.iter()) {
            assert_ne!(
                pixel, *base_pixel,
                "step {step}: the preview segment must not be the base line colour at {probe:?}"
            );
        }
        assert_eq!(
            session.drawing_progress(),
            Some((base_total, ordered_total)),
            "step {step}: the base walk must stay complete and unchanged"
        );
        assert!(
            !session.drawing_pages(),
            "step {step}: the base walk must not become pending"
        );
    }

    // ---------------------------------------------------------------------
    // Preview cleared: the suffix is truncated, never appended.
    // ---------------------------------------------------------------------
    let before_clear = session.overlay_suffix_commits();
    let (commits, cleared) = preview_tick(&session, None);
    assert_eq!(
        commits,
        before_clear + 1,
        "clearing the preview is one overlay-suffix commit"
    );
    assert_eq!(
        renderer_batch_count(&session),
        baseline_batches,
        "clearing the preview must truncate the suffix back to the baseline count"
    );
    for (probe, expected) in BASE_PROBES.iter().zip(baseline_base_pixels.iter()) {
        assert_eq!(
            pixel_at(&cleared, *probe),
            *expected,
            "clearing the preview must not touch the base target (probe {probe:?})"
        );
    }
    assert_eq!(
        pixel_at(&cleared, last_preview_probe),
        BACKGROUND_PIXEL,
        "the previously drawn preview pixel {last_preview_probe:?} must be background again"
    );
    assert_eq!(
        base_batch_prefix(&session),
        baseline_base_prefix,
        "clearing the preview must not change the base batch prefix"
    );
    assert_eq!(
        runtime_applied_base(&session),
        baseline_applied_base,
        "clearing the preview must not change the applied base revision"
    );
    assert_eq!(
        session.drawing_progress(),
        Some((base_total, ordered_total)),
        "clearing the preview must leave the base walk complete"
    );
    assert!(
        !session.drawing_pages(),
        "clearing the preview must not leave base pages pending"
    );
}
