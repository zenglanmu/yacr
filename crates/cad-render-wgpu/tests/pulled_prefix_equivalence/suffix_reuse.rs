use super::*;

#[test]
fn accelerated_prefix_full_establish_then_second_suffix_reuses() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        eprintln!("skipping: device cannot host the accelerated tier");
        return;
    }
    let mut renderer = init_accelerated(&gpu);
    assert_eq!(renderer.render_tier(), RenderTier::Accelerated);

    let base = base_batches();
    let base_len = base.len();
    const OVERLAY_1: u128 = 900;
    const OVERLAY_2: u128 = 901;
    const OVERLAY_3: u128 = 902;

    // 1) Full base commit: no prefix is retained yet (keep_prefix == 0).
    commit(&mut renderer, &base, 0);
    assert_eq!(renderer.pulled_prefix_reuses(), 0);
    assert_eq!(
        renderer.pulled_prefix_rebuilds(),
        0,
        "a keep_prefix==0 commit must not establish a retained prefix"
    );

    // 2) Warmup: the first suffix commit at the base boundary is a full rebuild
    //    that RETAINS the base prefix.
    commit(&mut renderer, &overlay_batch(OVERLAY_1, 0.7), base_len);
    assert_eq!(renderer.pulled_prefix_reuses(), 0);
    assert_eq!(
        renderer.pulled_prefix_rebuilds(),
        1,
        "the warmup suffix must establish the retained base prefix"
    );
    let line_segments_after_warmup = renderer.pulled_line_segment_count();
    assert!(
        line_segments_after_warmup > 0,
        "the accelerated line class must own real pulled segments for this fence"
    );

    // 3) Second suffix commit at the SAME base boundary MUST reuse, not rebuild.
    //    This is the regression the file exists for: before the fix the fast path
    //    is unreachable, so the reuse counter never advances.
    let reuses = renderer.pulled_prefix_reuses();
    let rebuilds = renderer.pulled_prefix_rebuilds();
    commit(&mut renderer, &overlay_batch(OVERLAY_2, 0.75), base_len);
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        reuses + 1,
        "the second suffix commit must reuse the retained base prefix"
    );
    assert_eq!(
        renderer.pulled_prefix_rebuilds(),
        rebuilds,
        "a reused suffix commit must not rebuild the pulled base"
    );

    // A third suffix keeps reusing.
    commit(&mut renderer, &overlay_batch(OVERLAY_3, 0.8), base_len);
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        reuses + 2,
        "every further suffix at the same boundary keeps reusing"
    );
    assert_eq!(renderer.pulled_prefix_rebuilds(), rebuilds);
}

#[test]
fn suffix_reuse_matches_a_single_full_commit_pixels_and_picks() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        eprintln!("skipping: device cannot host the accelerated tier");
        return;
    }
    let base = base_batches();
    const OVERLAY: u128 = 910;
    let overlay = overlay_batch(OVERLAY, COMPARED_OVERLAY_Y);

    let (mut reused, reuse_count) = build_reused(&gpu, &base, &overlay);
    assert!(reuse_count >= 1, "the reused path must be exercised");

    let mut composed = base.clone();
    composed.extend(overlay.clone());
    let mut full = build_full(&gpu, &composed);

    let reused_px = render_pixels(&mut reused);
    let full_px = render_pixels(&mut full);
    assert!(!is_blank(&reused_px), "the reused frame must not be blank");
    assert_eq!(
        reused_px, full_px,
        "a reused-suffix scene must be byte-identical to the same scene committed in one full pass \
         (same device, so exact pixels are justified)"
    );

    // Choose ONE interior (fully covered) pixel per entity from the camera-derived
    // band geometry — no per-pixel GPU pick calls — then pick exactly those.
    let overlay_xy = band_interior_pixel(COMPARED_OVERLAY_Y);
    let base_xy = band_interior_pixel(BASE_PICK_Y);
    assert_ne!(
        overlay_xy, base_xy,
        "the two chosen interior pixels must be distinct coordinates"
    );
    assert_drawn(&reused_px, overlay_xy, "overlay band");
    assert_drawn(&reused_px, base_xy, "base band");

    let coords = [overlay_xy, base_xy];
    let reused_picks = pick_map(&mut reused, &coords);
    let full_picks = pick_map(&mut full, &coords);
    assert_eq!(
        reused_picks, full_picks,
        "reused and full commits must resolve the SAME chosen pixels to the same identities \
         (semantic SelectionRef, never the raw key)"
    );
    assert_eq!(
        source_at(&reused_picks, overlay_xy).map(|s| s.entity),
        Some(EntityId(OVERLAY)),
        "the overlay suffix entity must be picked at its core pixel"
    );
    assert_eq!(
        source_at(&reused_picks, base_xy).map(|s| s.entity),
        Some(EntityId(BASE_PICK_ENTITY)),
        "the retained base entity must be picked at its core pixel"
    );
}

#[test]
fn repeated_overlay_replace_remove_readd_keeps_base_and_reuses() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        eprintln!("skipping: device cannot host the accelerated tier");
        return;
    }
    let mut renderer = init_accelerated(&gpu);
    let base = base_batches();
    let base_len = base.len();
    const OV_A: u128 = 920;
    const OV_B: u128 = 921;

    commit(&mut renderer, &base, 0);
    commit(
        &mut renderer,
        &overlay_batch(OV_A, COMPARED_OVERLAY_Y),
        base_len,
    ); // establish
    assert_eq!(renderer.pulled_prefix_rebuilds(), 1);

    // replace A -> B
    commit(
        &mut renderer,
        &overlay_batch(OV_B, COMPARED_OVERLAY_Y),
        base_len,
    );
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        1,
        "the replace must reuse the retained base prefix"
    );

    // Chosen interior pixels before the removal (base geometry is unchanged by it).
    let before_px = render_pixels(&mut renderer);
    let overlay_xy = band_interior_pixel(COMPARED_OVERLAY_Y);
    let base_xy = band_interior_pixel(BASE_PICK_Y);
    assert_drawn(&before_px, overlay_xy, "overlay band before removal");
    assert_drawn(&before_px, base_xy, "base band before removal");
    let coords = [overlay_xy, base_xy];
    let before_picks = pick_map(&mut renderer, &coords);
    assert_eq!(
        source_at(&before_picks, overlay_xy).map(|s| s.entity),
        Some(EntityId(OV_B)),
        "the overlay must pick before the removal"
    );

    // remove (empty suffix) — assert IMMEDIATELY after the removal, before re-adding.
    commit(&mut renderer, &[], base_len);
    let _removed_px = render_pixels(&mut renderer);
    let after_remove = pick_map(&mut renderer, &coords);
    assert_ne!(
        source_at(&after_remove, overlay_xy).map(|s| s.entity),
        Some(EntityId(OV_B)),
        "the removed overlay must no longer pick at its former core pixel"
    );
    assert_eq!(
        source_at(&after_remove, base_xy).map(|s| s.entity),
        Some(EntityId(BASE_PICK_ENTITY)),
        "a retained base entity must still pick after the overlay removal"
    );

    // re-add B — it picks again, and the counters are unchanged (still reusing).
    commit(
        &mut renderer,
        &overlay_batch(OV_B, COMPARED_OVERLAY_Y),
        base_len,
    );
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        3,
        "replace, remove and re-add must each reuse the retained base prefix"
    );
    assert_eq!(
        renderer.pulled_prefix_rebuilds(),
        1,
        "no suffix step may rebuild the retained base"
    );
    let _readded_px = render_pixels(&mut renderer);
    let after_readd = pick_map(&mut renderer, &coords);
    assert_eq!(
        source_at(&after_readd, overlay_xy).map(|s| s.entity),
        Some(EntityId(OV_B)),
        "the re-added overlay must pick again"
    );
    assert_eq!(
        source_at(&after_readd, base_xy).map(|s| s.entity),
        Some(EntityId(BASE_PICK_ENTITY)),
        "the retained base entity must keep picking through every step"
    );
}

#[test]
fn full_base_replacement_invalidates_the_retained_prefix() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        eprintln!("skipping: device cannot host the accelerated tier");
        return;
    }
    let mut renderer = init_accelerated(&gpu);
    let base = base_batches();
    let base_len = base.len();

    commit(&mut renderer, &base, 0);
    commit(&mut renderer, &overlay_batch(930, 0.7), base_len); // establish
    let reuses = renderer.pulled_prefix_reuses();
    let rebuilds = renderer.pulled_prefix_rebuilds();
    assert_eq!(rebuilds, 1);

    // A new full base (keep_prefix == 0) replaces the scene and must invalidate.
    let new_base = vec![
        line_band(-0.5, 40, 1.0),
        line_band(0.0, 41, 1.0),
        line_band(0.5, 42, 1.0),
    ];
    commit(&mut renderer, &new_base, 0);
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        reuses,
        "a full base replacement must never be reported as a reuse"
    );
    assert_eq!(
        renderer.pulled_prefix_rebuilds(),
        rebuilds,
        "a keep_prefix==0 commit must not establish a prefix"
    );

    // A suffix at the NEW boundary re-establishes (not reuses the old base).
    commit(&mut renderer, &overlay_batch(931, 0.7), new_base.len());
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        reuses,
        "the old retained prefix must be gone"
    );
    assert_eq!(
        renderer.pulled_prefix_rebuilds(),
        rebuilds + 1,
        "the suffix at the new boundary must establish a fresh prefix"
    );
}

#[test]
fn tiny_binding_limit_falls_back_without_reuse_and_still_draws() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        eprintln!("skipping: device cannot host the accelerated tier");
        return;
    }
    let mut renderer = init_accelerated(&gpu);
    // A limit too small for even the smallest pulled line segment: the line class
    // must fall back to the universal per-run path (existing override API).
    renderer.set_pulled_binding_limit_override(Some(1));

    let base = base_batches();
    let base_len = base.len();
    commit(&mut renderer, &base, 0);
    commit(&mut renderer, &overlay_batch(940, 0.7), base_len);
    commit(&mut renderer, &overlay_batch(941, 0.75), base_len);

    assert_eq!(
        renderer.pulled_line_segment_count(),
        0,
        "the pulled line class cannot be built under a 1-byte binding limit"
    );
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        0,
        "a class with no pulled segments must never claim a retained-prefix reuse"
    );

    // The geometry must still draw on the universal per-run fallback.
    let pixels = render_pixels(&mut renderer);
    assert!(
        !is_blank(&pixels),
        "the universal fallback must still draw the real line geometry"
    );
}

#[test]
fn half_alpha_overlay_reuse_matches_full_commit() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        eprintln!("skipping: device cannot host the accelerated tier");
        return;
    }
    let base = base_batches();
    let mut overlay = line_band(0.75, 950, 0.5);
    overlay.alpha = 0.5;

    let (mut reused, _) = build_reused(&gpu, &base, std::slice::from_ref(&overlay));
    let mut composed = base.clone();
    composed.push(overlay.clone());
    let mut full = build_full(&gpu, &composed);

    let reused_px = render_pixels(&mut reused);
    let full_px = render_pixels(&mut full);
    assert!(!is_blank(&reused_px));
    assert_eq!(
        reused_px, full_px,
        "a half-alpha overlay must composite identically through the reuse and full paths"
    );
}
