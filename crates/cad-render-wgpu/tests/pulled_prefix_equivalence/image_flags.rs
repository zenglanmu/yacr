use super::*;

#[test]
fn oversized_suffix_fallback_is_not_a_reuse_and_invalidates_the_checkpoint() {
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

    // 1) Full base commit with NO cap override: the base builds normally, and the
    //    measured pulled instance total tells us the cap the base actually needs.
    commit(&mut renderer, &base, 0);
    let base_instances = renderer.pulled_line_instance_count();
    assert!(
        base_instances > 0,
        "shape precondition: the base commit must build pulled line instances to derive the cap"
    );
    let cap = base_instances as usize;

    // The cap override is configured BEFORE the warmup (setting it later would
    // invalidate the checkpoint and miss the path this fence exists for). Every
    // base stream is <= the base total, so the base still fits.
    renderer.set_pulled_line_segment_elems_override(Some(cap));

    let warmup = overlay_batch(960, WARMUP_OVERLAY_Y);
    // One batch becomes one instance stream; `oversized_rows` is derived from the
    // fixture's own row count (the base is 4 batches of `BAND_ROWS`), not
    // hard-coded, so its stream is strictly larger than the base total = `cap`.
    let base_rows: u32 = 4 * BAND_ROWS;
    let oversized_rows = base_rows * 3 + 4;
    let oversized = vec![line_band_rows(
        WARMUP_OVERLAY_Y,
        961,
        1.0,
        oversized_rows,
        0.01,
    )];

    // 2) Warmup suffix: fits the cap, so it is a full rebuild that RETAINS the base.
    commit(&mut renderer, &warmup, base_len);
    assert!(
        renderer.pulled_line_segment_count() > 0,
        "shape precondition: the warmup must actually build the pulled line class under the cap"
    );
    assert_eq!(renderer.pulled_prefix_rebuilds(), 1);
    assert_eq!(renderer.pulled_prefix_reuses(), 0);

    // 3) Oversized suffix: one stream larger than the cap. The line class must
    //    degrade to the universal path, and that must NOT be reported as a reuse.
    let reuses = renderer.pulled_prefix_reuses();
    let rebuilds = renderer.pulled_prefix_rebuilds();
    commit(&mut renderer, &oversized, base_len);
    assert_eq!(
        renderer.pulled_line_segment_count(),
        0,
        "shape precondition: the oversized suffix stream must degrade the line class to the \
         universal path (a non-zero count would mean the fence never triggered)"
    );
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        reuses,
        "a fallback that re-uploads the base geometry must NOT be counted as a reuse"
    );
    assert_eq!(
        renderer.pulled_prefix_rebuilds(),
        rebuilds,
        "the fallback must invalidate the checkpoint, not establish a fresh one"
    );

    // The fallback frame must still match a single full commit of the same scene
    // under the SAME cap (colour and pick), so the degradation is not silently wrong.
    let mut composed = base.clone();
    composed.extend(oversized.clone());
    let mut full = build_full_with_cap(&gpu, &composed, cap);
    assert_eq!(
        full.pulled_line_segment_count(),
        0,
        "the forced-full comparison must degrade identically under the same cap"
    );
    let fallback_px = render_pixels(&mut renderer);
    let full_px = render_pixels(&mut full);
    assert!(
        !is_blank(&fallback_px),
        "the universal fallback must still draw the real geometry"
    );
    assert_eq!(
        fallback_px, full_px,
        "the fallback frame must be byte-identical to a single full commit of the same scene"
    );
    // Chosen interior pixels (camera-derived, then one pick each) for the
    // same-pixel semantic comparison.
    let base_xy = band_interior_pixel(BASE_PICK_Y);
    let overlay_xy = band_interior_pixel_rows(WARMUP_OVERLAY_Y, oversized_rows, 0.01);
    assert_drawn(&fallback_px, base_xy, "base band");
    assert_drawn(&fallback_px, overlay_xy, "oversized band");
    let coords = [base_xy, overlay_xy];
    assert_eq!(
        pick_map(&mut renderer, &coords),
        pick_map(&mut full, &coords),
        "the fallback and full commits must resolve the SAME chosen pixels to the same identities"
    );

    // 4) The checkpoint is gone: a follow-up normal suffix must REBUILD first
    //    (re-establish) and only the next one may reuse.
    commit(&mut renderer, &warmup, base_len);
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        reuses,
        "after invalidation the follow-up suffix must not reuse"
    );
    assert_eq!(
        renderer.pulled_prefix_rebuilds(),
        rebuilds + 1,
        "the follow-up suffix must re-establish the checkpoint"
    );
    commit(&mut renderer, &warmup, base_len);
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        reuses + 1,
        "only after re-establishing does the next suffix reuse again"
    );
}

#[test]
fn suffix_reuse_with_a_resident_image_matches_full_commit() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        eprintln!("skipping: device cannot host the accelerated tier");
        return;
    }
    const OVERLAY: u128 = 0xf0;
    let base = base_batches();
    let base_len = base.len();
    let overlay = overlay_batch(OVERLAY, COMPARED_OVERLAY_Y);
    let cache = red_cache();

    // Reused path: base geometry + resident image uploaded once, then a warmup
    // suffix and a reusing suffix.
    let mut reused = init_accelerated(&gpu);
    commit(&mut reused, &base, 0);
    let report = reused
        .upload_images(&[image_quad(0)], &cache)
        .expect("upload image");
    assert_eq!(report.uploaded, 1, "the resident image must upload");
    assert!(
        report.unresolved.is_empty(),
        "the image resource must resolve"
    );
    let textures = reused.image_texture_count();
    assert_eq!(textures, 1, "one resident image texture");

    commit(&mut reused, &overlay_batch(7, WARMUP_OVERLAY_Y), base_len);
    let before = reused.pulled_prefix_reuses();
    commit(&mut reused, &overlay, base_len);
    assert_eq!(
        reused.pulled_prefix_reuses(),
        before + 1,
        "the suffix must reuse the retained base with an image resident"
    );
    assert_eq!(
        reused.image_texture_count(),
        textures,
        "a reused suffix commit must not disturb the resident image"
    );

    // Full counterpart: the same batches and the same image in one commit.
    let mut composed = base.clone();
    composed.extend(overlay.clone());
    let mut full = init_accelerated(&gpu);
    commit(&mut full, &composed, 0);
    let full_report = full
        .upload_images(&[image_quad(0)], &cache)
        .expect("upload image");
    assert_eq!(full_report.uploaded, 1);

    let (reused_images, reused_px) = render_stats_pixels(&mut reused);
    let (full_images, full_px) = render_stats_pixels(&mut full);
    assert_eq!(
        reused_images, 1,
        "the resident image must be drawn (reused)"
    );
    assert_eq!(full_images, 1, "the resident image must be drawn (full)");
    assert!(!is_blank(&reused_px));
    assert_eq!(
        reused_px, full_px,
        "a resident image plus a reused suffix must be byte-identical to the full commit"
    );

    // The image's own opaque red must actually be on screen (non-vacuous).
    let image_xy = world_pixel(IMAGE_CENTRE.0, IMAGE_CENTRE.1);
    let [r, g, b, a] = rgb(&reused_px, image_xy.0, image_xy.1);
    assert!(
        r > 240 && g < 15 && b < 15 && a > 240,
        "the resident image centre {image_xy:?} must be opaque red, got {:?}",
        [r, g, b, a]
    );

    // Same chosen interior pixels on both: semantic identity equality.
    let base_xy = band_interior_pixel(BASE_PICK_Y);
    let overlay_xy = band_interior_pixel(COMPARED_OVERLAY_Y);
    assert_drawn(&reused_px, base_xy, "base band");
    assert_drawn(&reused_px, overlay_xy, "overlay band");
    let coords = [base_xy, overlay_xy, image_xy];
    let reused_picks = pick_map(&mut reused, &coords);
    let full_picks = pick_map(&mut full, &coords);
    assert_eq!(
        reused_picks, full_picks,
        "reused and full must resolve the same pixels to the same identities with an image resident"
    );
    assert_eq!(
        source_at(&reused_picks, base_xy).map(|s| s.entity),
        Some(EntityId(BASE_PICK_ENTITY)),
        "the retained base entity must pick with an image resident"
    );
    assert_eq!(
        source_at(&reused_picks, overlay_xy).map(|s| s.entity),
        Some(EntityId(OVERLAY)),
        "the suffix entity must pick with an image resident"
    );
    assert_eq!(
        source_at(&reused_picks, image_xy).map(|s| s.entity),
        Some(EntityId(IMAGE_ENTITY)),
        "the resident image must resolve to its own source"
    );
}

#[test]
fn resident_image_and_flags_stay_exact_under_suffix_reuse() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        eprintln!("skipping: device cannot host the accelerated tier");
        return;
    }
    const OVERLAY: u128 = 0xf2;
    let base = base_batches();
    let base_len = base.len();
    // The overlay band sits on the image's rows and spans only the image's right
    // half (`x 0.0..0.9`), so an image-only pixel, an overlay-only pixel and a
    // crossing pixel are all disjoint and individually assertable.
    let crossing = vec![line_span_rows(
        0.0,
        0.9,
        IMAGE_CENTRE.1,
        OVERLAY,
        1.0,
        BAND_ROWS,
        BAND_SPACING,
    )];
    let cache = red_cache();

    let mut renderer = init_accelerated(&gpu);
    commit(&mut renderer, &base, 0);
    let report = renderer
        .upload_images(&[image_quad(1)], &cache)
        .expect("upload image");
    assert_eq!(report.uploaded, 1);
    let textures = renderer.image_texture_count();

    commit(
        &mut renderer,
        &overlay_batch(0xf3, WARMUP_OVERLAY_Y),
        base_len,
    );
    let before = renderer.pulled_prefix_reuses();
    commit(&mut renderer, &crossing, base_len);
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        before + 1,
        "the crossing suffix must reuse the retained base"
    );
    assert_eq!(
        renderer.image_texture_count(),
        textures,
        "the resident image must survive the reused crossing suffix"
    );

    // Full counterpart: same batches, same image, one commit.
    let mut composed = base.clone();
    composed.extend(crossing.clone());
    let mut full = init_accelerated(&gpu);
    commit(&mut full, &composed, 0);
    assert_eq!(
        full.upload_images(&[image_quad(1)], &cache)
            .expect("upload image")
            .uploaded,
        1
    );

    let (images, reused_px) = render_stats_pixels(&mut renderer);
    let (full_images, full_px) = render_stats_pixels(&mut full);
    assert_eq!((images, full_images), (1, 1), "the image must be drawn");
    assert_eq!(
        reused_px, full_px,
        "a crossing overlay + resident image must match the full commit byte-for-byte"
    );

    // Disjoint interiors: image-only (left half), overlay-only (right, outside the
    // image), and the crossing pixel (inside both). The overlay band's own
    // interior row is used (its y span is narrower than the image's), so no chosen
    // pixel is band-fringe.
    let band_y = band_interior_pixel(IMAGE_CENTRE.1).1;
    let image_only = (
        world_pixel(IMAGE_CENTRE.0 - 0.125, IMAGE_CENTRE.1).0,
        band_y,
    );
    let overlay_only = (world_pixel(IMAGE_CENTRE.0 + 0.5, IMAGE_CENTRE.1).0, band_y);
    let crossing_xy = (
        world_pixel(IMAGE_CENTRE.0 + 0.125, IMAGE_CENTRE.1).0,
        band_y,
    );
    let coords = [image_only, overlay_only, crossing_xy];
    for xy in coords {
        assert_drawn(&reused_px, xy, "resident image / crossing overlay region");
    }

    let picks = pick_map(&mut renderer, &coords);
    let full_picks = pick_map(&mut full, &coords);
    assert_eq!(
        picks, full_picks,
        "the crossing overlay must resolve identically in the reused and full commits"
    );
    assert_eq!(
        source_at(&picks, image_only).map(|s| s.entity),
        Some(EntityId(IMAGE_ENTITY)),
        "an image-only pixel must resolve to the image"
    );
    assert_eq!(
        source_at(&picks, overlay_only).map(|s| s.entity),
        Some(EntityId(OVERLAY)),
        "an overlay-only pixel must resolve to the overlay"
    );
    assert!(
        source_at(&picks, crossing_xy).is_some(),
        "the crossing pixel must resolve to exactly one of the two declared entities"
    );

    // --- selection flags: tint is visible but never changes pick identity ------
    let overlay_key = renderer
        .pick_key_for_batch(renderer.batch_count() - 1)
        .expect("the crossing overlay must own a pick key");
    renderer.set_selection_flags_enabled(true);
    renderer
        .set_pick_key_flags(overlay_key, FLAG_SELECTED)
        .expect("set selection flag");
    assert_eq!(renderer.pick_key_flags(overlay_key), FLAG_SELECTED);
    let (_, tinted_px) = render_stats_pixels(&mut renderer);
    assert_ne!(
        tinted_px, reused_px,
        "an enabled selection flag on the overlay must tint the frame (non-vacuous)"
    );
    let tinted_picks = pick_map(&mut renderer, &coords);
    assert_eq!(
        tinted_picks, picks,
        "a selection flag must not change any pick identity"
    );

    // Clearing the flag and the toggle restores the exact frame.
    renderer
        .set_pick_key_flags(overlay_key, 0)
        .expect("clear flags");
    renderer.set_selection_flags_enabled(false);
    let (_, restored_px) = render_stats_pixels(&mut renderer);
    assert_eq!(
        restored_px, reused_px,
        "clearing the flag must restore the exact frame"
    );

    // --- visibility: HIDDEN removes only the overlay, nothing else -------------
    // `flags_enabled` is the flags control word (`write_flags_ctl`), so it gates
    // ALL flag behaviour including the FLAG_HIDDEN discard; the toggle must be
    // enabled while hiding (the repo's own visibility fence keeps it enabled
    // through FLAG_HIDDEN). The tint/clear restore gate above is a separate step.
    renderer.set_selection_flags_enabled(true);
    renderer
        .set_pick_key_flags(overlay_key, FLAG_HIDDEN)
        .expect("hide the overlay");
    let (_, hidden_px) = render_stats_pixels(&mut renderer);
    assert_ne!(
        hidden_px, reused_px,
        "hiding the overlay must change the frame"
    );
    let hidden_picks = pick_map(&mut renderer, &coords);
    assert_ne!(
        source_at(&hidden_picks, overlay_only).map(|s| s.entity),
        Some(EntityId(OVERLAY)),
        "the hidden overlay must no longer pick"
    );
    assert_eq!(
        source_at(&hidden_picks, image_only).map(|s| s.entity),
        Some(EntityId(IMAGE_ENTITY)),
        "the resident image must still pick while the overlay is hidden"
    );
    assert_eq!(
        source_at(&hidden_picks, crossing_xy).map(|s| s.entity),
        Some(EntityId(IMAGE_ENTITY)),
        "the crossing pixel must fall back to the still-visible image"
    );
    renderer
        .set_pick_key_flags(overlay_key, 0)
        .expect("clear flags");
}
