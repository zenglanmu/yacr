use super::*;

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

/// An odd-length `Lines` batch pairs its leading vertices and **counts** the
/// trailing unpaired one instead of dropping it silently (both the legacy quad
/// path and the instance path pair the same way). The batch still renders.
#[test]
fn odd_line_batch_counts_its_trailing_vertex() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    // Three vertices: one segment plus a trailing unpaired vertex.
    let batches = vec![lines_batch(vec![
        [-0.8, -0.5, 0.0],
        [0.8, -0.5, 0.0],
        [0.8, 0.5, 0.0],
    ])];
    let (pixels, _) = render_pixels(&mut renderer, &target, &batches, false, camera_2d());
    assert_eq!(
        renderer.line_odd_vertex_skips(),
        1,
        "the trailing unpaired vertex must be counted, never silently dropped"
    );
    let background = opaque_black(&pixels);
    assert!(
        pixels.chunks_exact(4).any(|px| px != background.as_slice()),
        "the paired segment must still render"
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

    // The legacy path uploads pre-expanded anti-aliased line quads: 6 vertices
    // of 32 B per 2-vertex segment, a different exact number.
    let (_, legacy_stats) = render_pixels(&mut renderer, &target, &batches, false, camera_2d());
    assert_eq!(
        legacy_stats.uploaded_bytes - stats.uploaded_bytes,
        (16 * 6 * cad_render_wgpu::LINE_QUAD_BYTES) as u64,
        "the legacy path uploads its pre-expanded line-quad bytes"
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
    // Anti-aliased strokes make every sub-unit-coverage fringe pixel sensitive to
    // the f32 origin reduction, so the instanced and legacy frames are no longer
    // byte-identical (only the f32-representable shared-origin case is). The
    // precision-meaningful fence is that the framed object stays in the **same
    // pixel columns**: the whole-scene shared-origin regression this fence exists
    // for moved the object to a different column, which this still catches.
    let inked_columns = |px: &[u8]| -> Vec<u32> {
        let bg = [px[0], px[1], px[2], px[3]];
        (0..64u32)
            .filter(|&x| {
                (0..64u32).any(|y| {
                    let i = (y as usize * 64 + x as usize) * 4;
                    px[i..i + 4] != bg
                })
            })
            .collect()
    };
    assert_eq!(
        inked_columns(&instanced),
        inked_columns(&legacy),
        "a per-chunk origin must keep the framed object in the same pixel columns"
    );
    // The fringe difference is bounded: the two AA paths agree on every pixel
    // outside the object's ~2 px stroke band.
    let diff = differing_pixels(&instanced, &legacy, 4);
    assert!(
        diff <= 4 * 64,
        "instanced deep-zoom AA fringes must stay bounded; {diff} pixels differed"
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
