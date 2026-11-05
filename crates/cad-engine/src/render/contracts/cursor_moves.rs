use super::*;

// ---------------------------------------------------------------------------
// A plain cursor move is not a scene mutation (real adapter).
// ---------------------------------------------------------------------------

/// Four lines forming a frame: entity 1 is the bottom horizontal (the one the
/// cursor hovers), 2 the top horizontal, 3/4 the verticals. They are far enough
/// apart that one pixel per entity is unambiguous at 400x240 with
/// `world_per_px = 1.0`.
#[cfg(not(target_arch = "wasm32"))]
fn db_with_four_lines() -> DrawingDatabase {
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
    b.insert_entity(line_entity(
        3,
        SpaceId::Model,
        p(-80.0, -40.0),
        p(-80.0, 40.0),
    ))
    .unwrap();
    b.insert_entity(line_entity(
        4,
        SpaceId::Model,
        p(80.0, -40.0),
        p(80.0, 40.0),
    ))
    .unwrap();
    b.finish().unwrap()
}

/// One pixel per probe: the hovered bottom line, the three untouched base lines
/// (top horizontal, left/right verticals) and empty background.
#[cfg(not(target_arch = "wasm32"))]
const BOTTOM_LINE: [f64; 2] = [60.0, -40.0];
#[cfg(not(target_arch = "wasm32"))]
const BASE_PROBES: [[f64; 2]; 3] = [[60.0, 40.0], [-80.0, 0.0], [80.0, 0.0]];
#[cfg(not(target_arch = "wasm32"))]
const BACKGROUND: [f64; 2] = [0.0, 20.0];

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

/// Renderer commit/draw-plan diagnostics for the run-free suffix contract:
/// `(draw_plan_rebuilds, gc_skipped_commits, pulled_suffix_skips, accelerated)`.
#[cfg(not(target_arch = "wasm32"))]
fn suffix_skip_diagnostics(session: &RenderSessionHandle) -> (u64, u64, u64, bool) {
    let state = session.0.borrow();
    let renderer = state.runtime.renderer.as_ref().expect("attached renderer");
    (
        renderer.draw_plan_rebuilds(),
        renderer.gc_skipped_commits(),
        renderer.pulled_suffix_skips(),
        renderer.render_tier() == cad_render_wgpu::RenderTier::Accelerated,
    )
}

/// One host tick that processed a plain cursor move, in the host's real order:
/// the input pump publishes the move (`set_overlays`), then the frame is applied
/// and presented (`apply_ready_updates` + `render`, see
/// `apps/app-linux/src/gtk_bridge/present.rs`).
#[cfg(not(target_arch = "wasm32"))]
fn move_tick(
    session: &RenderSessionHandle,
    cursor: Option<[f64; 2]>,
    hovered: &[SelectionRef],
) -> (FrameCompleteness, (u32, u32, Vec<u8>)) {
    let mut overlays = session.overlays();
    overlays.cursor = cursor.map(|[x, y]| p(x, y));
    overlays.hovered = SelectionSet::from_refs(hovered.iter().cloned());
    assert!(
        session.set_overlays(overlays),
        "a move with a new cursor/hover changes the overlay inputs"
    );
    session.apply_ready_updates().unwrap();
    let frame = session
        .render([400.0, 240.0], 1.0)
        .unwrap()
        .expect("a frame is presented for the move tick");
    (frame.completeness, target_pixels(session))
}

/// One **settled** host tick: publish the inputs, let the preparation land, commit
/// it, then present. This is the reference frame the move ticks are compared to.
#[cfg(not(target_arch = "wasm32"))]
fn settled_tick(
    session: &RenderSessionHandle,
    cursor: Option<[f64; 2]>,
    hovered: &[SelectionRef],
) -> (FrameCompleteness, (u32, u32, Vec<u8>)) {
    let mut overlays = session.overlays();
    overlays.cursor = cursor.map(|[x, y]| p(x, y));
    overlays.hovered = SelectionSet::from_refs(hovered.iter().cloned());
    session.set_overlays(overlays);
    session.prepare().unwrap();
    drive_uploads(session);
    session.apply_ready_updates().unwrap();
    let frame = session
        .render([400.0, 240.0], 1.0)
        .unwrap()
        .expect("a settled frame");
    (frame.completeness, target_pixels(session))
}

/// **Regression fence (entity flicker on plain mouse movement).**
///
/// A plain cursor move (no press, no pan/zoom) changes exactly one thing: the
/// transient overlay suffix (cursor crosshair/pickbox), plus — when the hover
/// resolves — the entity-flag tint. It is *not* a scene mutation. Presenting the
/// move must therefore never truncate the overlay suffix and never revert the
/// hover tint to an older flag set: the old base-only fallback did both for one
/// frame per move, which is why the hovered entity blinked.
#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "hardware GPU: run explicitly on a real adapter (P620/Vulkan)"]
fn cursor_moves_never_drop_the_tint_or_the_overlay_suffix() {
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

    // Baseline: a still cursor, nothing hovered.
    let (baseline_completeness, baseline) = settled_tick(&session, Some([-10.0, -20.0]), &[]);
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

    // The tint the renderer produces for a hovered entity (a tint, not the plain
    // line colour).
    let (tinted_completeness, tinted_pixels) =
        settled_tick(&session, Some([-10.0, -20.0]), &[model_entity(1)]);
    assert_eq!(tinted_completeness, FrameCompleteness::CurrentScene);
    let tinted_line = pixel_at(&tinted_pixels, BOTTOM_LINE);
    assert_ne!(
        tinted_line, plain_line,
        "the hover must tint the bottom line (fixture sanity)"
    );
    let suffix_commits_tinted = session.overlay_suffix_commits();
    // Read the renderer's own upload counters, never through a throwaway render
    // (that would replace the session's target texture).
    let counters_tinted = renderer_counters(&session);
    let full_batches_tinted = renderer_batch_count(&session);
    assert_eq!(
        full_batches_tinted, full_batches,
        "no batch churn on a hover"
    );
    assert_eq!(
        suffix_commits_tinted, suffix_commits,
        "a hover is a flags-only change: it commits no suffix"
    );

    // The mouse now moves: eight ticks, each one carrying a moved cursor while
    // the hover stays on the bottom line.
    let mut cursor_x = -10.0;
    for step in 0..8 {
        cursor_x += 4.0;
        let (completeness, pixels) =
            move_tick(&session, Some([cursor_x, -20.0]), &[model_entity(1)]);
        assert_eq!(
            pixel_at(&pixels, BOTTOM_LINE),
            tinted_line,
            "step {step}: the hover tint must never revert while the cursor moves \
             (frame {completeness:?})"
        );
        for (probe, expected) in BASE_PROBES.iter().zip(base_pixels.iter()) {
            assert_eq!(
                pixel_at(&pixels, *probe),
                *expected,
                "step {step}: a cursor move must not change base geometry"
            );
        }
        assert_eq!(
            renderer_batch_count(&session),
            full_batches,
            "step {step}: the overlay suffix must stay resident (never truncated)"
        );
        assert_eq!(
            session.overlay_suffix_commits(),
            suffix_commits_tinted,
            "step {step}: a pending overlay input must not commit the suffix"
        );
        assert_eq!(
            renderer_counters(&session),
            counters_tinted,
            "step {step}: presenting a pending move must upload nothing"
        );
    }

    // The mouse stops: the pending overlay publishes and commits. The whole
    // scene is still there, the tint is still the same, and nothing was restored
    // because nothing was ever dropped.
    //
    // The commit-side contract (base_changed=false and the GC/pulled skip
    // counters) lives in `cursor_move_suffix_commit_reports_the_run_free_skips`
    // below: this pixel fence has an independent pre-existing step-0 failure on
    // the current host, so any counter assertion placed after it would be dead
    // and could never be cited as evidence.
    session.prepare().unwrap();
    drive_uploads(&session);
    session.apply_ready_updates().unwrap();
    let settled = session
        .render([400.0, 240.0], 1.0)
        .unwrap()
        .expect("settled frame");
    assert_eq!(settled.completeness, FrameCompleteness::CurrentScene);
    let settled_pixels = target_pixels(&session);
    assert_eq!(
        pixel_at(&settled_pixels, BOTTOM_LINE),
        tinted_line,
        "the settled frame keeps the same tint"
    );
    for (probe, expected) in BASE_PROBES.iter().zip(base_pixels.iter()) {
        assert_eq!(pixel_at(&settled_pixels, *probe), *expected);
    }
    assert_eq!(
        renderer_batch_count(&session),
        full_batches,
        "the settled commit replaces the suffix, it never changes the batch count"
    );
    assert_eq!(
        session.overlay_suffix_commits(),
        suffix_commits_tinted + 1,
        "the settled cursor position is one real suffix commit"
    );
    assert_eq!(
        renderer_counters(&session),
        counters_tinted,
        "the settled cursor commit re-uploads the transient suffix only: no \
         instanced/mesh stream and no flags row changes"
    );
}

/// The engine's cursor move path is the renderer's run-free suffix fast path: the
/// settled suffix commit reports `base_changed == false` and bumps the GC/pulled
/// skip counters while leaving the base-only draw plan intact.
///
/// Split out from the pixel fence above because that fence has an independent
/// pre-existing pixel failure on this host; this contract exercises only the
/// commit counters.
#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "hardware GPU: run explicitly on a real adapter (P620/Vulkan)"]
fn cursor_move_suffix_commit_reports_the_run_free_skips() {
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

    // Settle a first position: the suffix is live and the base plan is built.
    let _ = settled_tick(&session, Some([-10.0, -20.0]), &[]);
    let (plan_before, gc_before, pulled_before, accelerated) = suffix_skip_diagnostics(&session);
    eprintln!("cursor suffix skip contract: accelerated={accelerated}");
    // Non-vacuous pin: the base-only draw plan must have been built at least
    // once, so the "kept across the suffix commit" comparison below cannot pass
    // by comparing a never-incremented zero.
    assert!(
        plan_before >= 1,
        "the base-only draw plan must be built before the suffix-kept assertion \
         (counter was {plan_before})"
    );

    // A plain move: publish, prepare, commit the transient suffix, present.
    let _ = settled_tick(&session, Some([-6.0, -20.0]), &[]);
    let commit = session
        .take_upload_timing_observation()
        .expect("the settled cursor move is observed");
    assert!(
        commit.committed,
        "the cursor move must commit the transient suffix"
    );
    assert!(
        !commit.base_changed,
        "a cursor move is still a suffix commit, never a base change"
    );

    let (plan_after, gc_after, pulled_after, _) = suffix_skip_diagnostics(&session);
    assert_eq!(
        plan_after, plan_before,
        "a suffix commit must keep the base-only draw plan (no rebuild)"
    );
    assert_eq!(
        gc_after,
        gc_before + 1,
        "the run-free suffix commit must skip exactly one GC scan"
    );
    if accelerated {
        assert_eq!(
            pulled_after,
            pulled_before + 1,
            "the accelerated tier must skip exactly one pulled rebuild"
        );
    } else {
        assert_eq!(
            pulled_after, pulled_before,
            "the universal tier has no pulled organisation to skip"
        );
    }
}
