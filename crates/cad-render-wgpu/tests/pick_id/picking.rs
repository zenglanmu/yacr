use super::*;

#[test]
fn pick_before_any_frame_has_no_view() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    // No colour frame has been rendered, so there is no projection to pick with.
    assert!(renderer.pick_at(1, 1).expect("pick").is_none());
}

#[test]
fn a_line_pixel_resolves_to_its_entity() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(96, 96);
    renderer
        .upload(&scene(vec![line_band(6.0, 3, 0xABC)]))
        .expect("upload");
    let pixels = render(&mut renderer, &target);
    let (x, y) = first_pickable_pixel(&mut renderer, &pixels, 96, 96, 0, 96)
        .expect("the band must rasterize a pickable pixel");

    let hit = renderer.pick_at(x, y).expect("pick").expect("a hit");
    assert_eq!(hit.entity_key, renderer.pick_key_for_batch(0).unwrap());
    assert_eq!(
        hit.source.expect("exact").entity,
        EntityId(0xABC),
        "the id pass must resolve the line pixel to its own entity"
    );
    assert!(!hit.key_collision);
    // A line has no sub-element concept; the id pass never claims one.
    assert!(hit.sub_element.is_none());
}

#[test]
fn a_merged_glyph_mesh_pixel_resolves_to_its_entity() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(96, 96);
    // No edges => the glyph class, which is flat-merged by default.
    renderer
        .upload(&scene(vec![quad(0.0, 0.0, 8.0, 0x111, false)]))
        .expect("upload");
    assert_eq!(renderer.mesh_chunk_count(), 1, "the glyph quad is merged");

    let pixels = render(&mut renderer, &target);
    let (x, y) = world_to_pixel(&camera_2d(), 96, 96, [0.0, 0.0]);
    assert!(
        pixels[(y as usize * 96 + x as usize) * 4 + 1] > 0,
        "quad drawn"
    );

    let hit = renderer.pick_at(x, y).expect("pick").expect("a hit");
    assert_eq!(
        hit.source.expect("exact").entity,
        EntityId(0x111),
        "a merged-mesh fragment must carry its own batch's pick key"
    );
}

#[test]
fn a_mesh_with_an_edge_overlay_resolves_to_the_same_entity_on_both_paths() {
    let Some(available) = gpu() else {
        return;
    };
    drop(available);
    let target = RenderTarget::new(96, 96);
    let batches = vec![quad(0.0, 0.0, 8.0, 0x222, true)];
    let (x, y) = world_to_pixel(&camera_2d(), 96, 96, [0.0, 0.0]);

    // Converted (merged mesh + instanced overlay) and legacy (per-batch) must
    // both resolve the same pixel to the same entity.
    for merging in [true, false] {
        let Some(gpu) = gpu() else {
            return;
        };
        let mut renderer = init(gpu, None);
        renderer.set_mesh_merging(merging);
        renderer.set_line_instancing(merging);
        renderer.upload(&scene(batches.clone())).expect("upload");
        let pixels = render(&mut renderer, &target);
        assert!(
            pixels[(y as usize * 96 + x as usize) * 4 + 1] > 0,
            "merging={merging}: the quad must rasterize at its centre"
        );
        let hit = renderer.pick_at(x, y).expect("pick");
        let hit = hit.unwrap_or_else(|| {
            panic!(
                "merging={merging}: no id-pass hit at ({x}, {y}); chunks={} streams={}",
                renderer.mesh_chunk_count(),
                renderer.instance_stream_count()
            )
        });
        assert_eq!(
            hit.source.expect("exact").entity,
            EntityId(0x222),
            "merging={merging} must resolve the overlay mesh to its entity"
        );
    }
}

#[test]
fn separated_entities_pick_to_their_own_pixels() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(128, 128);
    renderer
        .upload(&scene(vec![
            quad(-16.0, 0.0, 6.0, 0x10, false),
            quad(16.0, 0.0, 6.0, 0x20, false),
        ]))
        .expect("upload");
    let _ = render(&mut renderer, &target);

    for (world_x, expected) in [(-16.0, 0x10u128), (16.0, 0x20)] {
        let (x, y) = world_to_pixel(&camera_2d(), 128, 128, [world_x, 0.0]);
        let hit = renderer.pick_at(x, y).expect("pick").expect("a hit");
        assert_eq!(
            hit.source.expect("exact").entity,
            EntityId(expected),
            "pixel at world x={world_x} must resolve to its own quad"
        );
    }
}

#[test]
fn a_background_pixel_picks_nothing() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(64, 64);
    renderer
        .upload(&scene(vec![quad(0.0, 0.0, 3.0, 7, false)]))
        .expect("upload");
    let _ = render(&mut renderer, &target);
    // A corner far from the small quad is the cleared id target.
    assert!(renderer.pick_at(1, 1).expect("pick").is_none());
    // Out-of-range pixels are not hits either.
    assert!(renderer.pick_at(9999, 9999).expect("pick").is_none());
}

#[test]
fn a_multi_source_batch_reports_every_candidate_and_never_the_first() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(96, 96);
    // Two entities concatenated into one batch, exactly what the scene builder
    // does for adjacent line fragments.
    let mut batch = line_band(6.0, 3, 0);
    batch.sources = vec![source(0x1000), source(0x2000)];
    renderer.upload(&scene(vec![batch])).expect("upload");
    let pixels = render(&mut renderer, &target);
    let (x, y) = first_pickable_pixel(&mut renderer, &pixels, 96, 96, 0, 96).expect("drawn");

    let hit = renderer.pick_at(x, y).expect("pick").expect("a hit");
    assert!(
        hit.source.is_none(),
        "a multi-source fragment must not be attributed to one entity"
    );
    assert_eq!(hit.candidates.len(), 2);
    assert_eq!(hit.candidates[0].entity, EntityId(0x1000));
    assert_eq!(hit.candidates[1].entity, EntityId(0x2000));
    assert!(hit.is_ambiguous());
    assert_eq!(hit.reason, Some("pick-multi-source-batch"));
}

#[test]
fn a_u128_truncation_collision_is_reported_and_still_resolves_exactly() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(128, 128);
    // Two distinct entities whose low 32 bits agree.
    let low = 0x0000_4242u128;
    let high = (1u128 << 96) | low;
    renderer
        .upload(&scene(vec![
            quad(-16.0, 0.0, 6.0, low, false),
            quad(16.0, 0.0, 6.0, high, false),
        ]))
        .expect("upload");
    let _ = render(&mut renderer, &target);

    let collisions = renderer.pick_collisions();
    assert_eq!(collisions.len(), 1, "the truncation must be detected");
    assert_eq!(collisions[0].truncated_key, 0x4242);
    assert_eq!(collisions[0].identities.len(), 2);

    // Resolution is still exact (the key is a renderer key), and the hazard is
    // surfaced on the hit instead of causing a silent mis-pick.
    for (world_x, expected) in [(-16.0, low), (16.0, high)] {
        let (x, y) = world_to_pixel(&camera_2d(), 128, 128, [world_x, 0.0]);
        let hit = renderer.pick_at(x, y).expect("pick").expect("a hit");
        assert_eq!(hit.source.expect("exact").entity, EntityId(expected));
        assert!(
            hit.key_collision,
            "the collision must be visible on the hit"
        );
    }
}

#[test]
fn the_id_pass_does_not_disturb_the_color_frame() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(96, 96);
    renderer
        .upload(&scene(vec![
            quad(-12.0, 0.0, 5.0, 0xAA, false),
            line_band(6.0, 3, 0xBB),
        ]))
        .expect("upload");

    let frame_only = render(&mut renderer, &target);
    // Pick several pixels: each one runs a whole id pass.
    for (x, y) in [(48u32, 48u32), (20, 48), (70, 20), (1, 1)] {
        let _ = renderer.pick_at(x, y).expect("pick");
    }
    // The color target must be untouched by the picks...
    let after_picks = renderer.read_target_rgba().expect("read").pixels;
    assert_eq!(
        after_picks, frame_only,
        "an id pass must not modify the colour target"
    );
    // ...and a frame rendered after a pick must be byte-identical to one rendered
    // without any pick at all.
    let frame_again = render(&mut renderer, &target);
    assert_eq!(
        frame_again, frame_only,
        "a pick must not perturb the following colour frame"
    );
}

#[test]
fn flags_are_off_by_default_and_a_flag_update_leaves_pixels_unchanged() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(96, 96);
    renderer
        .upload(&scene(vec![quad(0.0, 0.0, 8.0, 0x55, false)]))
        .expect("upload");
    assert!(
        !renderer.selection_flags_enabled(),
        "the flags tint is off by default so highlight overlays keep working"
    );
    let before = render(&mut renderer, &target);

    let key = renderer.pick_key_for_batch(0).expect("key");
    renderer
        .set_pick_key_flags(key, FLAG_SELECTED)
        .expect("set flags");
    assert_eq!(renderer.pick_key_flags(key), FLAG_SELECTED);

    let after = render(&mut renderer, &target);
    assert_eq!(
        after, before,
        "with the toggle off, a flag update must not change a single pixel"
    );
}

#[test]
fn enabling_the_toggle_applies_the_selected_tint() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(96, 96);
    renderer
        .upload(&scene(vec![quad(0.0, 0.0, 8.0, 0x66, false)]))
        .expect("upload");
    let plain = render(&mut renderer, &target);
    let key = renderer.pick_key_for_batch(0).expect("key");
    renderer
        .set_pick_key_flags(key, FLAG_SELECTED)
        .expect("set flags");
    renderer.set_selection_flags_enabled(true);
    let tinted = render(&mut renderer, &target);
    assert_ne!(
        tinted, plain,
        "with the toggle on, a selected batch must be tinted"
    );

    // HIDDEN must actually hide the batch again.
    renderer
        .set_pick_key_flags(key, FLAG_HIDDEN)
        .expect("set flags");
    let hidden = render(&mut renderer, &target);
    let (x, y) = world_to_pixel(&camera_2d(), 96, 96, [0.0, 0.0]);
    let i = (y as usize * 96 + x as usize) * 4;
    assert!(
        hidden[i] <= 8 && hidden[i + 1] <= 8 && hidden[i + 2] <= 8,
        "a HIDDEN batch must be discarded, got {:?}",
        &hidden[i..i + 4]
    );
}

#[test]
fn a_flag_update_uploads_only_the_flags_table() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(96, 96);
    renderer
        .upload(&scene(vec![
            quad(-12.0, 0.0, 5.0, 1, false),
            line_band(6.0, 3, 2),
        ]))
        .expect("upload");
    let _ = render(&mut renderer, &target);

    let streams_before = renderer.instance_stream_uploads();
    let chunks_before = renderer.mesh_chunk_uploads();
    let bytes_before = renderer.flags_upload_bytes();

    for (index, key) in [(0usize, 1u32), (1, 2)] {
        assert_eq!(renderer.pick_key_for_batch(index), Some(key));
        renderer
            .set_pick_key_flags(key, FLAG_SELECTED)
            .expect("set flags");
    }
    assert!(
        renderer.flags_upload_bytes() > bytes_before,
        "a flags update must upload the flags table"
    );
    assert_eq!(
        renderer.instance_stream_uploads(),
        streams_before,
        "a flags update must not re-upload the static instance streams"
    );
    assert_eq!(
        renderer.mesh_chunk_uploads(),
        chunks_before,
        "a flags update must not re-upload the static merged chunks"
    );
}

#[test]
fn a_flag_update_outside_the_key_space_is_rejected() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    assert!(renderer.set_pick_key_flags(0, FLAG_SELECTED).is_err());
    assert!(renderer
        .set_pick_key_flags(u32::MAX, FLAG_SELECTED)
        .is_err());
}

#[test]
fn r32uint_is_the_default_id_target_when_the_adapter_reports_it_renderable() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let format = renderer.id_target_format().expect("device initialized");
    let expected = Renderer::id_format_for_features(renderer.device_features());
    assert_eq!(
        format, expected,
        "the reported format must match the adapter probe"
    );
    // Whichever tier the adapter lands on, a pick must work through it.
    let target = RenderTarget::new(64, 64);
    renderer
        .upload(&scene(vec![quad(0.0, 0.0, 6.0, 9, false)]))
        .expect("upload");
    let _ = render(&mut renderer, &target);
    let (x, y) = world_to_pixel(&camera_2d(), 64, 64, [0.0, 0.0]);
    let hit = renderer.pick_at(x, y).expect("pick").expect("a hit");
    assert_eq!(hit.source.expect("exact").entity, EntityId(9));
}

#[test]
fn the_packed_rgba8_fallback_picks_identically() {
    let Some(available) = gpu() else {
        return;
    };
    drop(available);
    let target = RenderTarget::new(96, 96);
    let batches = vec![quad(-12.0, 0.0, 5.0, 0x31, false), line_band(6.0, 3, 0x32)];
    let (qx, qy) = world_to_pixel(&camera_2d(), 96, 96, [-12.0, 0.0]);

    let mut picked = Vec::new();
    for format in [IdTargetFormat::R32Uint, IdTargetFormat::PackedRgba8] {
        let Some(gpu) = gpu() else {
            return;
        };
        let mut renderer = init(gpu, Some(format));
        assert_eq!(
            renderer.id_target_format(),
            Some(format),
            "the override must select the requested tier"
        );
        renderer.upload(&scene(batches.clone())).expect("upload");
        let pixels = render(&mut renderer, &target);
        let (lx, ly) = first_pickable_pixel(&mut renderer, &pixels, 96, 96, 0, 96).expect("drawn");
        let quad_hit = renderer.pick_at(qx, qy).expect("pick").expect("a hit");
        let line_hit = renderer.pick_at(lx, ly).expect("pick").expect("a hit");
        picked.push((
            quad_hit.source.expect("exact").entity,
            line_hit.source.expect("exact").entity,
        ));
    }
    assert_eq!(
        picked[0], picked[1],
        "the packed fallback must resolve the same identities as R32Uint"
    );
    assert_eq!(picked[0].0, EntityId(0x31));
    assert_eq!(picked[0].1, EntityId(0x32));
}

#[test]
fn a_scene_reset_clears_the_flag_table() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(96, 96);
    renderer
        .upload(&scene(vec![quad(0.0, 0.0, 8.0, 0x77, false)]))
        .expect("upload");
    let key = renderer.pick_key_for_batch(0).expect("key");
    renderer
        .set_pick_key_flags(key, FLAG_SELECTED)
        .expect("set flags");
    renderer.set_selection_flags_enabled(true);
    let tinted = render(&mut renderer, &target);

    // Retire the scene and upload another one. The key space restarts, so the new
    // scene reuses the same key: its bits must be gone, not inherited.
    renderer.clear_batches();
    renderer
        .upload(&scene(vec![quad(0.0, 0.0, 8.0, 0x77, false)]))
        .expect("re-upload");
    assert_eq!(
        renderer.pick_key_for_batch(0),
        Some(key),
        "the key space restarts after a clear"
    );
    assert_eq!(
        renderer.pick_key_flags(key),
        0,
        "a retired key must not keep its flag bits"
    );
    let after = render(&mut renderer, &target);
    assert_ne!(
        after, tinted,
        "the re-uploaded scene must not inherit the retired selection"
    );

    // Re-selecting it in the new scene tints it again, proving the table is live.
    renderer
        .set_pick_key_flags(key, FLAG_SELECTED)
        .expect("set flags");
    let reselected = render(&mut renderer, &target);
    assert_eq!(
        reselected, tinted,
        "re-selecting the same key in the new scene must tint it identically"
    );
}

#[test]
fn an_image_pixel_resolves_to_its_source_through_the_id_pass() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(96, 96);
    let mut cache = DecodedImageCache::new();
    cache
        .insert(
            ResourceKey("red".into()),
            solid_image(2, 2, [255, 0, 0, 255]),
            &ResourceLimits::default(),
        )
        .expect("insert red image");

    // A line batch *and* a resident image. Images are appended to the id pass
    // unconditionally, so before the pipeline-layout fix every `pick_at` failed
    // wgpu validation (`MissingBindGroup { index: 1 }`) as soon as any image was
    // resident, whether or not the picked pixel was the image.
    renderer
        .upload(&scene(vec![line_band(-20.0, 3, 0xB0)]))
        .expect("upload");
    let report = renderer
        .upload_images(&[image_batch("red", 0xE0)], &cache)
        .expect("upload image");
    assert_eq!(report.uploaded, 1);

    let pixels = render(&mut renderer, &target);
    // The image covers rows ~40..56 and the line sits below it (rows > 60), so
    // scan the lower band for a pixel the color pass actually drew.
    let (lx, ly) = first_pickable_pixel(&mut renderer, &pixels, 96, 96, 60, 96)
        .expect("the line must rasterize below the image");
    let line_hit = renderer.pick_at(lx, ly).expect("pick").expect("a hit");
    assert_eq!(
        line_hit.source.expect("exact").entity,
        EntityId(0xB0),
        "a geometry pixel must still resolve to its own batch with an image resident"
    );

    let (ix, iy) = world_to_pixel(&camera_2d(), 96, 96, [0.0, 0.0]);
    assert!(
        pixels[(iy as usize * 96 + ix as usize) * 4] > 8,
        "the image must rasterize at the centre"
    );
    let image_hit = renderer.pick_at(ix, iy).expect("pick").expect("a hit");
    assert_eq!(
        image_hit.source.expect("exact").entity,
        EntityId(0xE0),
        "the id pass must resolve an image fragment to the image's own source"
    );
}

#[test]
fn an_image_pick_works_through_the_packed_rgba8_fallback() {
    let Some(available) = gpu() else {
        return;
    };
    drop(available);
    let target = RenderTarget::new(96, 96);
    for format in [IdTargetFormat::R32Uint, IdTargetFormat::PackedRgba8] {
        let Some(gpu) = gpu() else {
            return;
        };
        let mut renderer = init(gpu, Some(format));
        let mut cache = DecodedImageCache::new();
        cache
            .insert(
                ResourceKey("red".into()),
                solid_image(2, 2, [255, 0, 0, 255]),
                &ResourceLimits::default(),
            )
            .expect("insert red image");
        renderer
            .upload_images(&[image_batch("red", 0xE0)], &cache)
            .expect("upload image");
        let _ = render(&mut renderer, &target);
        let (x, y) = world_to_pixel(&camera_2d(), 96, 96, [0.0, 0.0]);
        let hit = renderer.pick_at(x, y).expect("pick").expect("a hit");
        assert_eq!(
            hit.source.expect("exact").entity,
            EntityId(0xE0),
            "{}: an image pixel must resolve through this id-target tier",
            format.as_str()
        );
    }
}

#[test]
fn a_scene_reset_with_resident_images_does_not_alias_pick_keys() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(96, 96);
    let mut cache = DecodedImageCache::new();
    cache
        .insert(
            ResourceKey("red".into()),
            solid_image(2, 2, [255, 0, 0, 255]),
            &ResourceLimits::default(),
        )
        .expect("insert red image");

    // One batch (key 1) plus one image (key 2) placed away from the batches, so a
    // stale image key would later alias a *different* batch's pixel. Then a full
    // reset.
    renderer
        .upload(&scene(vec![quad(-30.0, 0.0, 4.0, 0xB0, false)]))
        .expect("upload");
    renderer
        .upload_images(&[image_batch_at("red", 0xE0, [30.0, 30.0])], &cache)
        .expect("image");
    renderer.clear_batches();

    // Re-upload three batches (keys 1..3) and no images. A stale resident image
    // would still hold key 2; the id pass draws it unconditionally, so a pick on
    // it would resolve to whatever batch also owns key 2 — an entity nowhere near
    // the picked pixel. The reset must have dropped the image.
    renderer
        .upload(&scene(vec![
            quad(-30.0, 0.0, 4.0, 0x11, false),
            quad(0.0, 0.0, 4.0, 0x22, false),
            quad(30.0, 0.0, 4.0, 0x33, false),
        ]))
        .expect("re-upload");
    let stats = renderer.render(camera_2d(), &target).expect("render");
    assert_eq!(
        stats.image_batches, 0,
        "a scene reset must drop the resident image, not keep it for the next frame"
    );
    for (world_x, expected) in [(-30.0, 0x11u128), (0.0, 0x22), (30.0, 0x33)] {
        let (x, y) = world_to_pixel(&camera_2d(), 96, 96, [world_x, 0.0]);
        let hit = renderer.pick_at(x, y).expect("pick").expect("a hit");
        assert_eq!(
            hit.source.expect("exact").entity,
            EntityId(expected),
            "world x={world_x} must resolve to its own batch after a reset with resident images"
        );
    }
    // The old image's pixel is now empty: no stale image and no aliased batch.
    let (ix, iy) = world_to_pixel(&camera_2d(), 96, 96, [30.0, 30.0]);
    assert!(
        renderer.pick_at(ix, iy).expect("pick").is_none(),
        "a reset must not leave a stale image key to alias a batch on an unrelated pixel"
    );
}

#[test]
fn selection_flags_survive_an_incremental_commit_that_grows_the_key_space() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(96, 96);
    renderer
        .upload(&scene(vec![quad(0.0, 0.0, 8.0, 0x55, false)]))
        .expect("upload");
    let key = renderer.pick_key_for_batch(0).expect("key");
    renderer
        .set_pick_key_flags(key, FLAG_SELECTED)
        .expect("set flags");
    renderer.set_selection_flags_enabled(true);
    let before = render(&mut renderer, &target);
    let (x, y) = world_to_pixel(&camera_2d(), 96, 96, [0.0, 0.0]);
    let i = (y as usize * 96 + x as usize) * 4;
    let selected = [before[i], before[i + 1], before[i + 2], before[i + 3]];

    // Incremental commit adding batches: the key space grows, which used to
    // recreate the flags table on the width change and zero every bit. The host
    // must NOT have to re-push the flags after every commit (P3a-2).
    let added: Vec<RenderBatch> = (0..8)
        .map(|n| quad(-40.0 + n as f32 * 4.0, -30.0, 1.0, 0x100 + n as u128, false))
        .collect();
    let prepared = renderer.prepare_upload(&scene(added)).expect("prepare");
    renderer.commit_upload(prepared, 1).expect("commit");
    assert_eq!(
        renderer.pick_key_flags(key),
        FLAG_SELECTED,
        "the flag bit must survive an incremental commit that grows the key space"
    );

    let after = render(&mut renderer, &target);
    assert_eq!(
        [after[i], after[i + 1], after[i + 2], after[i + 3]],
        selected,
        "the selected batch must stay tinted after the key space grows"
    );
}

#[test]
fn flags_disabled_is_pixel_identical_for_every_draw_class() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(96, 96);
    let mut cache = DecodedImageCache::new();
    cache
        .insert(
            ResourceKey("red".into()),
            solid_image(2, 2, [255, 0, 0, 255]),
            &ResourceLimits::default(),
        )
        .expect("insert red image");
    // One batch per visible class: an instanced line, a merged glyph mesh, a
    // mesh with an edge overlay (instanced overlay), and a resident image.
    renderer
        .upload(&scene(vec![
            line_band(-20.0, 3, 0x01),
            quad(0.0, 0.0, 8.0, 0x02, false),
            quad(-30.0, 0.0, 4.0, 0x03, true),
        ]))
        .expect("upload");
    renderer
        .upload_images(&[image_batch("red", 0x04)], &cache)
        .expect("image");
    assert!(
        !renderer.selection_flags_enabled(),
        "the tint is off by default"
    );
    let before = render(&mut renderer, &target);
    assert_eq!(renderer.pick_table_len(), 4, "3 batches + 1 image");

    // Set flags on every resident key. With the toggle off this must be a no-op
    // for every shader class (the legacy quad case is covered by
    // `flags_are_off_by_default_and_a_flag_update_leaves_pixels_unchanged`).
    for key in 1..=4 {
        renderer
            .set_pick_key_flags(key, FLAG_SELECTED | FLAG_HOVERED)
            .expect("set flags");
    }
    let after = render(&mut renderer, &target);
    assert_eq!(
        after, before,
        "with the toggle off, flag bits must not change any class's pixels"
    );
}
