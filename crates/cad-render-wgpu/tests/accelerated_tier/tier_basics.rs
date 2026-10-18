use super::*;

#[test]
fn default_tier_is_accelerated_on_a_storage_capable_device() {
    let Some(gpu) = gpu() else {
        return;
    };
    eprintln!(
        "accelerated-tier adapter: {} ({}, {})",
        gpu.adapter.name, gpu.adapter.backend, gpu.adapter.device_type
    );
    let mut renderer = Renderer::new(BackendPreference::Auto);
    renderer
        .initialize_with_device(gpu.device.clone(), gpu.queue.clone())
        .expect("initialize renderer");
    if supports_storage(&gpu) {
        assert_eq!(
            renderer.render_tier(),
            RenderTier::Accelerated,
            "a storage-buffer-capable device must select the accelerated tier automatically"
        );
    } else {
        assert_eq!(
            renderer.render_tier(),
            RenderTier::Universal,
            "a device without storage buffers must downgrade automatically"
        );
    }
}

#[test]
fn capability_fallback_downgrades_and_renders_identically() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        eprintln!("skipping: device has no vertex-stage storage buffers");
        return;
    }
    let target = RenderTarget::new(64, 64);
    let batches = vec![
        lines_batch(vec![[-0.9, -0.5, 0.0], [0.9, -0.5, 0.0]]),
        lines_batch(vec![[-0.9, 0.5, 0.0], [0.9, 0.5, 0.0]]),
        mesh_at(0.2, 0.0, 0.0, false, 7, 0),
    ];
    // The downgrade is silent (no error) and labelled by the tier accessor.
    let mut downgraded = init_tier(&gpu, false);
    let (downgraded_px, downgraded_stats) =
        upload_and_render(&mut downgraded, &target, &batches, camera_2d());
    assert_eq!(downgraded.render_tier(), RenderTier::Universal);
    assert!(!downgraded_stats.accelerated);
    assert_eq!(downgraded_stats.accelerated_draws, 0);

    let mut accelerated = init_tier(&gpu, true);
    let (accelerated_px, accelerated_stats) =
        upload_and_render(&mut accelerated, &target, &batches, camera_2d());
    assert_eq!(accelerated.render_tier(), RenderTier::Accelerated);
    assert!(accelerated_stats.accelerated);
    assert_eq!(
        downgraded_px, accelerated_px,
        "the automatic downgrade must not change a single pixel"
    );
}

#[test]
fn accelerated_collapses_many_line_streams_to_one_draw() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let batches = grid_line_batches(6, 6); // 36 non-overlapping segments

    // Force one segment per stream so the universal path needs many runs.
    let mut accelerated = init_tier(&gpu, true);
    accelerated.set_max_instances_per_stream(1);
    let (accel_px, accel_stats) =
        upload_and_render(&mut accelerated, &target, &batches, camera_2d());
    assert!(
        accelerated.instance_stream_count() > 1,
        "the scene must be split into several chunks to make the collapse meaningful"
    );
    assert_eq!(accel_stats.tier, RenderTier::Accelerated);
    assert!(accel_stats.accelerated);
    assert_eq!(
        accel_stats.instanced_draws, 1,
        "every line chunk must collapse into ONE accelerated draw"
    );
    assert_eq!(accel_stats.accelerated_draws, 1);
    assert_eq!(accel_stats.draw_calls, 1, "36 batches -> one class draw");
    assert_eq!(
        accelerated.pulled_line_segment_count(),
        1,
        "a small scene fits one storage segment"
    );
    assert_eq!(
        accelerated.pulled_line_instance_count() as usize,
        36,
        "the shared segment holds every instance"
    );

    let mut universal = init_tier(&gpu, false);
    universal.set_max_instances_per_stream(1);
    let (univ_px, univ_stats) = upload_and_render(&mut universal, &target, &batches, camera_2d());
    assert!(
        univ_stats.instanced_draws > 1,
        "the universal path draws one run per chunk ({} runs)",
        univ_stats.instanced_draws
    );
    assert_eq!(
        accel_px, univ_px,
        "the collapsed line draw must be byte-identical to the per-run draws"
    );
}

#[test]
fn accelerated_storage_is_one_segment_for_a_small_scene() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let mut renderer = init_tier(&gpu, true);
    let mut batches = grid_line_batches(4, 4);
    batches.extend(quad_grid(3, 3, false));
    renderer.clear_batches();
    renderer.upload(&scene(batches)).unwrap();
    // The line class shares one `VERTEX | STORAGE` segment with the per-run
    // instance buffer (no duplicate geometry); the mesh class has its own pulled
    // storage segment.
    assert_eq!(renderer.pulled_line_segment_count(), 1);
    assert_eq!(renderer.pulled_mesh_segment_count(), 1);
    // Every mesh vertex is resident in the pulled storage (3x3 quads -> 36).
    assert_eq!(
        renderer.pulled_mesh_vertex_count(),
        36,
        "the pulled mesh storage must cover every resident merged vertex"
    );
    let stats = renderer.render(camera_2d(), &target).unwrap();
    assert!(stats.accelerated);
    assert_eq!(
        stats.accelerated_draws,
        stats.instanced_draws + stats.mesh_draws,
        "every accelerated draw comes from a storage segment"
    );
}

#[test]
fn accelerated_lines_match_universal_pixels() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let batches = vec![
        lines_batch(vec![[-0.9, -0.5, 0.0], [0.9, -0.5, 0.0]]),
        lines_batch(vec![[-0.9, 0.5, 0.0], [0.9, 0.5, 0.0]]),
        lines_batch(vec![[0.0, -0.9, 0.0], [0.0, 0.9, 0.0]]),
    ];
    let stats = assert_tiers_identical(&gpu, &target, &batches, camera_2d(), "instanced lines");
    assert_eq!(stats.instanced_draws, 1);
}

#[test]
fn accelerated_merged_meshes_match_universal_pixels() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let batches = quad_grid(6, 6, false);
    let stats = assert_tiers_identical(&gpu, &target, &batches, camera_2d(), "merged meshes");
    assert_eq!(
        stats.mesh_draws, 1,
        "every merged chunk collapses to one draw"
    );
    assert_eq!(stats.accelerated_draws, 1);
}

#[test]
fn accelerated_mirrored_meshes_match_universal_pixels() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    // Non-overlapping, so the mirrored/non-mirrored group reorder cannot change a
    // tie (the reorder caveat is documented in the ADR).
    let mut batches = quad_grid(3, 3, false);
    batches.extend(quad_grid(3, 3, true));
    let stats = assert_tiers_identical(
        &gpu,
        &target,
        &batches,
        camera_2d(),
        "mirrored merged meshes",
    );
    assert!(
        stats.mesh_draws <= 2,
        "the mirrored and non-mirrored classes each collapse to one draw, got {}",
        stats.mesh_draws
    );
}

#[test]
fn accelerated_mixed_lines_and_meshes_match_universal_pixels() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let batches = vec![
        lines_batch(vec![[-0.9, 0.9, 0.0], [0.9, 0.9, 0.0]]),
        mesh_at(0.2, 0.0, 0.0, false, 3, 0),
        lines_batch(vec![[-0.9, -0.9, 0.0], [0.9, -0.9, 0.0]]),
    ];
    let stats = assert_tiers_identical(&gpu, &target, &batches, camera_2d(), "mixed classes");
    // One line class draw + one mesh class draw.
    assert_eq!(stats.draw_calls, 2);
}

#[test]
fn accelerated_lines_keep_precision_at_large_coordinates() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    // A drawing far from the world origin, drawn through a camera centred there.
    let origin = [1.0e7, 1.0e7, 0.0];
    let batches = vec![
        lines_batch_at(origin, vec![[-0.9, -0.5, 0.0], [0.9, -0.5, 0.0]]),
        lines_batch_at(origin, vec![[-0.9, 0.5, 0.0], [0.9, 0.5, 0.0]]),
    ];
    let camera = camera_2d_at(1.0e7, 1.0e7, 2.0 / 64.0);
    let stats = assert_tiers_identical(&gpu, &target, &batches, camera, "large-coordinate lines");
    assert_eq!(stats.instanced_draws, 1);
}

#[test]
fn accelerated_deep_zoom_mixed_origin_stays_within_the_bound() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(128, 128);
    // Two line batches with distinct origins inside one chunk (mixed origin),
    // zoomed in hard.
    let batches = vec![
        lines_batch_at(
            [1000.0, 0.0, 0.0],
            vec![[-0.5, -0.2, 0.0], [0.5, -0.2, 0.0]],
        ),
        lines_batch_at([1000.5, 0.0, 0.0], vec![[-0.5, 0.2, 0.0], [0.5, 0.2, 0.0]]),
    ];
    let camera = camera_2d_at(1000.0, 0.0, 4.0 / 128.0);
    assert_tiers_identical(&gpu, &target, &batches, camera, "deep-zoom mixed origin");
}

#[test]
fn accelerated_matches_universal_in_perspective() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let batches = vec![
        mesh_at(0.3, 0.0, 0.0, false, 1, 0),
        lines_batch(vec![[-0.9, -0.5, 0.0], [0.9, -0.5, 0.0]]),
    ];
    let mut accelerated = init_tier(&gpu, true);
    let mut universal = init_tier(&gpu, false);
    accelerated.clear_batches();
    accelerated.upload(&scene(batches.clone())).unwrap();
    universal.clear_batches();
    universal.upload(&scene(batches.clone())).unwrap();
    let accel = accelerated.render_3d(camera_3d(), &target).unwrap();
    let accel_px = accelerated.read_target_rgba().unwrap().pixels;
    let univ = universal.render_3d(camera_3d(), &target).unwrap();
    let univ_px = universal.read_target_rgba().unwrap().pixels;
    assert!(accel.accelerated);
    assert!(!univ.accelerated);
    assert_eq!(
        accel_px, univ_px,
        "the 3D perspective path must render byte-identically on both tiers"
    );
}

#[test]
fn accelerated_overlapping_ties_match_universal_pixels() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    // Coplanar overlapping quads and a line crossing them at equal depth: the
    // cross-class tie must resolve identically on both tiers.
    let batches = vec![
        mesh_at(0.6, 0.0, 0.0, false, 1, 0),
        mesh_at(0.4, 0.0, 0.0, false, 2, 0),
        lines_batch(vec![[-0.8, 0.0, 0.0], [0.8, 0.0, 0.0]]),
    ];
    assert_tiers_identical(&gpu, &target, &batches, camera_2d(), "overlapping ties");
}

#[test]
fn accelerated_selection_flags_tint_identically() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let batches = vec![
        lines_batch(vec![[-0.9, -0.5, 0.0], [0.9, -0.5, 0.0]]),
        mesh_at(0.4, 0.0, 0.3, false, 5, 0),
    ];
    let mut accelerated = init_tier(&gpu, true);
    let mut universal = init_tier(&gpu, false);
    accelerated.clear_batches();
    accelerated.upload(&scene(batches.clone())).unwrap();
    universal.clear_batches();
    universal.upload(&scene(batches.clone())).unwrap();
    // Flag the first resident key (the line batch) as selected on both.
    accelerated
        .set_entity_flags(&[(source(1), FLAG_SELECTED)])
        .expect("flag entity");
    universal
        .set_entity_flags(&[(source(1), FLAG_SELECTED)])
        .expect("flag entity");
    accelerated.set_selection_flags_enabled(true);
    universal.set_selection_flags_enabled(true);

    let _ = accelerated.render(camera_2d(), &target).unwrap();
    let accel_px = accelerated.read_target_rgba().unwrap().pixels;
    let _ = universal.render(camera_2d(), &target).unwrap();
    let univ_px = universal.read_target_rgba().unwrap().pixels;
    assert_eq!(
        accel_px, univ_px,
        "FLAG_SELECTED must tint identically on the pulled and universal shaders"
    );
    // The tint must be observable (a non-vacuous fence).
    let unselected = {
        let mut plain = init_tier(&gpu, true);
        let (px, _) = upload_and_render(&mut plain, &target, &batches, camera_2d());
        px
    };
    assert_ne!(accel_px, unselected, "the selected tint must be visible");
}

#[test]
fn accelerated_hidden_flag_discards_identically() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let batches = vec![mesh_at(0.5, 0.0, 0.0, false, 9, 0)];
    let mut accelerated = init_tier(&gpu, true);
    accelerated.clear_batches();
    accelerated.upload(&scene(batches.clone())).unwrap();
    accelerated
        .set_entity_flags(&[(source(9), FLAG_HIDDEN)])
        .expect("flag entity");
    accelerated.set_selection_flags_enabled(true);
    let stats = accelerated.render(camera_2d(), &target).unwrap();
    assert!(stats.accelerated);
    let pixels = accelerated.read_target_rgba().unwrap().pixels;
    let background = [pixels[0], pixels[1], pixels[2], pixels[3]];
    assert!(
        pixels.chunks_exact(4).all(|px| px == background.as_slice()),
        "a FLAG_HIDDEN mesh must be discarded by the pulled shader"
    );
}

#[test]
fn accelerated_camera_move_does_not_reupload_static_storage() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let batches = quad_grid(4, 4, false);
    let mut renderer = init_tier(&gpu, true);
    renderer.clear_batches();
    renderer.upload(&scene(batches)).unwrap();
    let (stream_uploads, chunk_uploads) = (
        renderer.instance_stream_uploads(),
        renderer.mesh_chunk_uploads(),
    );
    let _ = renderer.render(camera_2d(), &target).unwrap();
    for step in 0..10 {
        let camera = camera_2d_at(step as f64 * 0.1, step as f64 * 0.05, 2.0 / 64.0);
        let stats = renderer.render(camera, &target).unwrap();
        assert!(
            stats.accelerated,
            "the accelerated path must stay live across a pan"
        );
    }
    assert_eq!(
        renderer.instance_stream_uploads(),
        stream_uploads,
        "a camera move must not re-upload the static instance streams"
    );
    assert_eq!(
        renderer.mesh_chunk_uploads(),
        chunk_uploads,
        "a camera move must not re-upload the static merged chunks"
    );
}

#[test]
fn accelerated_pick_uses_the_universal_id_pass() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let batches = vec![mesh_at(0.9, 0.0, 0.0, false, 42, 0)];
    let mut renderer = init_tier(&gpu, true);
    renderer.clear_batches();
    renderer.upload(&scene(batches)).unwrap();
    let stats = renderer.render(camera_2d(), &target).unwrap();
    assert!(
        stats.accelerated,
        "the color frame uses the accelerated tier"
    );
    let hit = renderer
        .pick_at(32, 32)
        .expect("pick must not error")
        .expect("the centred mesh must be picked");
    assert_eq!(
        hit.source.as_ref().map(|s| s.entity),
        Some(EntityId(42)),
        "the id pass must resolve the entity the accelerated color pass drew"
    );
}
