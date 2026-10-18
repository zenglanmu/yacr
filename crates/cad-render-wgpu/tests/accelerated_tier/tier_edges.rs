use super::*;

#[test]
fn accelerated_page_boundary_falls_back_to_universal_without_loss() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let batches = quad_grid(2, 4, false);
    let mut renderer = init_tier(&gpu, true);
    renderer.clear_batches();
    renderer.upload(&scene(batches.clone())).unwrap();

    // Reference: a full-budget frame uses the accelerated per-class draw.
    let reference_stats = renderer.render(camera_2d(), &target).unwrap();
    assert!(reference_stats.accelerated);
    assert_eq!(reference_stats.mesh_draws, 1);
    let reference = renderer.read_target_rgba().unwrap().pixels;

    // A page boundary inside the merged run forces the universal per-run
    // organization for correctness (the per-class draw cannot pull geometry the
    // budget held back); the accumulated frame must still equal the reference.
    // One accelerated merged quad charges its merged share (200 B) plus its
    // pulled share (64 B/vertex + 4 B/index = 280 B) = 480 B, so this budget
    // accepts exactly one batch per page.
    renderer.frame_budget = cad_scene::FrameBudget {
        max_vertices: usize::MAX,
        max_triangles: usize::MAX,
        max_bytes: 480,
    };
    renderer.set_progressive_rendering(true);
    let mut pages = 0;
    loop {
        let stats = renderer.render(camera_2d(), &target).unwrap();
        assert!(
            !stats.accelerated,
            "a paginated frame must fall back to the universal per-run path"
        );
        pages += 1;
        assert!(
            pages <= batches.len(),
            "progressive frame did not terminate"
        );
        if !renderer.frame_pending() {
            break;
        }
    }
    assert_eq!(pages, batches.len(), "one page per merged batch");
    assert_eq!(
        renderer.read_target_rgba().unwrap().pixels,
        reference,
        "the fallback pages must reconstruct the accelerated reference frame"
    );
}

/// B1 fence: on the accelerated tier every stream in a storage segment shares
/// one buffer, so a line run's global instance index is `base + offset`. Two
/// streams in one segment with distinct entities must each pick their own
/// entity; reading `offset` alone (stream-local) would resolve both to the first
/// stream's key.
#[test]
fn accelerated_multi_stream_line_pick_reads_the_right_instance() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(96, 96);
    // Two bands at distinct y, three segments each so a scanline is reliably
    // covered. `set_max_instances_per_stream(1)` makes every segment its own
    // stream (six streams), and the whole scene fits one storage segment, so the
    // second band's runs have a non-zero `base`.
    let batches = vec![line_band(0.5, 3, 0xA1), line_band(-0.5, 3, 0xB2)];
    let mut renderer = init_tier(&gpu, true);
    renderer.set_max_instances_per_stream(1);
    let (pixels, stats) = upload_and_render(&mut renderer, &target, &batches, camera_2d());
    assert!(
        stats.accelerated,
        "the accelerated per-class draw must be live for this fence"
    );
    assert_eq!(
        renderer.instance_stream_count(),
        6,
        "one stream per segment"
    );
    assert_eq!(
        renderer.pulled_line_segment_count(),
        1,
        "all six streams must share ONE storage segment for base re-basing to matter"
    );

    let top = drawn_pixel_in_rows(&pixels, 96, 96, 0, 48).expect("the top band must rasterize");
    let bottom =
        drawn_pixel_in_rows(&pixels, 96, 96, 48, 96).expect("the bottom band must rasterize");
    // A band's first inked pixel can be an anti-aliased fringe below the id pass's
    // 0.5-coverage pick threshold, so scan the band for a **pickable** pixel: the
    // colour pass covered it and the id pass resolves it (the ±0.5 px AA-fringe
    // pick tolerance, matching `pick_arc`).
    let pickable = |renderer: &mut Renderer, y0: u32, y1: u32| -> Option<(u32, u32, u32)> {
        for y in y0..y1 {
            for x in 0..96u32 {
                let i = (y as usize * 96 + x as usize) * 4;
                if pixels[i] <= 8 && pixels[i + 1] <= 8 && pixels[i + 2] <= 8 {
                    continue;
                }
                if let Some(hit) = renderer.pick_at(x, y).expect("pick") {
                    return Some((x, y, hit.entity_key));
                }
            }
        }
        None
    };
    let (top_x, top_y, _) = pickable(&mut renderer, 0, 48).expect("the top band must be picked");
    let (bottom_x, bottom_y, _) =
        pickable(&mut renderer, 48, 96).expect("the bottom band must be picked");
    let top_hit = renderer
        .pick_at(top_x, top_y)
        .expect("pick")
        .expect("the top band must be picked");
    let bottom_hit = renderer
        .pick_at(bottom_x, bottom_y)
        .expect("pick")
        .expect("the bottom band must be picked");
    let _ = (top, bottom);
    assert_eq!(
        top_hit.source.expect("exact top").entity,
        EntityId(0xA1),
        "the top band must resolve to its own entity"
    );
    assert_eq!(
        bottom_hit.source.expect("exact bottom").entity,
        EntityId(0xB2),
        "the bottom band must resolve to its own entity (base + offset, not offset)"
    );
}

/// S4 fence: force the line class across several storage segments and assert the
/// per-segment accelerated draws are byte-identical to the universal per-run
/// frame.
#[test]
fn accelerated_forced_multi_segment_lines_match_universal_pixels() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let batches = grid_line_batches(6, 6); // 36 non-overlapping segments

    let mut accelerated = init_tier(&gpu, true);
    // 36 streams, split into segments of two: eighteen storage segments, each
    // drawn once, still covering every instance.
    accelerated.set_max_instances_per_stream(1);
    accelerated.set_pulled_line_segment_elems_override(Some(2));
    let (accel_px, accel_stats) =
        upload_and_render(&mut accelerated, &target, &batches, camera_2d());
    assert!(accel_stats.accelerated);
    assert!(
        accelerated.pulled_line_segment_count() > 1,
        "the override must force several storage segments, got {}",
        accelerated.pulled_line_segment_count()
    );
    assert_eq!(
        accelerated.pulled_line_instance_count() as usize,
        36,
        "every instance must still be resident across the segments"
    );
    assert_eq!(
        accel_stats.instanced_draws as usize,
        accelerated.pulled_line_segment_count(),
        "the accelerated frame must issue one draw per storage segment"
    );

    let mut universal = init_tier(&gpu, false);
    universal.set_max_instances_per_stream(1);
    let (univ_px, univ_stats) = upload_and_render(&mut universal, &target, &batches, camera_2d());
    assert!(
        univ_stats.instanced_draws > 1,
        "the universal path uses per-run draws"
    );
    assert_eq!(
        accel_px, univ_px,
        "the multi-segment accelerated frame must match the universal frame"
    );
}

/// B2 fence: an oversized line stream at reduced limits must not emit an invalid
/// storage binding. The class downgrades to the universal per-run path, whose
/// per-stream buffers must hold the real geometry (not the prepare-time
/// placeholder), and the frame is pixel-identical to the universal renderer.
#[test]
fn accelerated_oversized_line_stream_downgrades_without_validation_error() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    // One stream of 2048 instances, far past a 64 KiB binding's 1024 records.
    let mut vertices = Vec::new();
    for i in 0..2048u32 {
        let x = -0.9 + (i / 64) as f32 * 0.028;
        let y = -0.9 + (i % 64) as f32 * 0.028;
        vertices.push([x, y, 0.0]);
        vertices.push([x + 0.02, y + 0.02, 0.0]);
    }
    let batches = vec![lines_batch(vertices)];

    let mut renderer = init_tier(&gpu, true);
    renderer.set_pulled_binding_limit_override(Some(64 * 1024));
    let (pixels, stats) = upload_and_render(&mut renderer, &target, &batches, camera_2d());
    assert_eq!(
        renderer.render_tier(),
        RenderTier::Accelerated,
        "the device is still accelerated; only this class falls back"
    );
    assert!(
        !stats.accelerated,
        "an oversized stream must fall back to the universal per-run path, not accelerate"
    );
    assert_eq!(
        renderer.pulled_line_segment_count(),
        0,
        "no storage segment may be built for an oversized stream"
    );
    assert_eq!(
        renderer.instance_stream_count(),
        1,
        "one stream for the batch"
    );
    // The fallback must draw real geometry, not the prepare-time placeholder.
    let background = [pixels[0], pixels[1], pixels[2], pixels[3]];
    assert!(
        pixels.chunks_exact(4).any(|px| px != background.as_slice()),
        "the fallback frame is blank; the per-stream buffers held no real geometry"
    );

    let mut universal = init_tier(&gpu, false);
    let (univ_px, _) = upload_and_render(&mut universal, &target, &batches, camera_2d());
    assert_eq!(
        pixels, univ_px,
        "the reduced-limit fallback must draw the same geometry as the universal path"
    );
}

/// S2 fence: the lifetime `uploaded_bytes` counter must charge the accelerated
/// pulled geometry exactly once. A 2-segment line batch plus one merged quad
/// charges, at prepare, line `2*64` + merged `4*44 + 6*4` + pulled
/// `4*64 + 6*4`; the commit then adds only the two 16-byte chunk tables — not
/// the segment buffers a second time.
#[test]
fn accelerated_uploaded_bytes_charge_pulled_geometry_once() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let batches = vec![
        lines_batch(vec![
            [-0.9, -0.5, 0.0],
            [0.9, -0.5, 0.0],
            [-0.9, 0.5, 0.0],
            [0.9, 0.5, 0.0],
        ]),
        mesh_at(0.3, 0.0, 0.0, false, 3, 0),
    ];
    let mut renderer = init_tier(&gpu, true);
    let (_, stats) = upload_and_render(&mut renderer, &target, &batches, camera_2d());
    assert!(stats.accelerated);
    let line = 2 * 64;
    let merged = 4 * 44 + 6 * 4;
    let pulled = 4 * 64 + 6 * 4;
    let tables = 2 * 16; // one line storage segment + one mesh storage segment
    assert_eq!(
        stats.uploaded_bytes,
        (line + merged + pulled + tables) as u64,
        "pulled geometry must be charged once, not again on every rebuild"
    );
}

/// S2: the accelerated tier's **residual** SDF conic path is exercised, and an
/// arc-vs-legacy-batch equal-depth tie still resolves by plan order (the S1
/// flush), so the accelerated frame matches the universal frame.
///
/// The ADR's "arc-vs-pulled-class tie exception" is about *pulled* classes; this
/// fence pins the residual path and a legacy-batch tie, which the S1 fix
/// (`flush_pending_arc` before the legacy `draw_batch`) makes order-correct.
#[test]
fn accelerated_sdf_arc_residual_path_and_legacy_tie_match_universal() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    // The circle (red, SDF) and a horizontal legacy `Lines` batch (default
    // colour) cross at the ring's two axis points at equal depth.
    let arc = conic_batch(
        vec![circle(
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            0.5,
        )],
        1,
    );
    // Three vertices -> not an instanced pair, so it stays a legacy per-batch
    // draw on the accelerated tier (the residual path's neighbour).
    let mut legacy = lines_batch(vec![[-0.9, 0.0, 0.0], [0.9, 0.0, 0.0], [0.9, 0.01, 0.0]]);
    legacy.color = [0.0, 0.0, 1.0];
    legacy.color_unresolved = false;
    // A converted (pulled) line class elsewhere, so the accelerated tier actually
    // engages its per-class pulled path (which the arc residual path sits beside).
    let pulled_line = lines_batch(vec![[-0.9, 0.8, 0.0], [0.9, 0.8, 0.0]]);

    let stats = assert_tiers_identical(
        &gpu,
        &target,
        &[arc, legacy, pulled_line],
        camera_2d(),
        "residual SDF arc + legacy tie",
    );
    assert!(
        stats.arc_draws > 0,
        "the accelerated frame must draw the SDF arc through the residual path"
    );
    assert_eq!(
        stats.arc_batches, 1,
        "one conic batch must go through the residual path"
    );
}

/// S2: the accelerated tier's documented **arc-vs-pulled-class** tie exception.
///
/// Pulled classes draw before the residual SDF conic class, so on the accelerated
/// tier an arc-vs-pulled-line equal-depth tie is always won by the arc (drawn
/// after, `LessEqual`), independent of plan order — whereas the universal tier
/// resolves the tie by plan order. This fence pins that difference (and therefore
/// the honesty of the ADR claim).
#[test]
fn accelerated_sdf_arc_vs_pulled_class_tie_is_the_documented_exception() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let wpp = 2.0 / 64.0;
    // World coordinates that are integer multiples of `wpp` land on pixel centres
    // (the ring's right point and the line's row are both centred).
    let cam = Camera2d {
        center: Point3 {
            x: 0.5 * wpp,
            y: 0.5 * wpp,
            z: 0.0,
        },
        world_per_px: wpp,
        z_plane: 0.0,
    };
    let origin = Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    let mut arc = conic_batch(vec![circle(origin, 0.5)], 1);
    arc.color = [1.0, 0.0, 0.0];
    let mut line = lines_batch(vec![[-0.9, 0.0, 0.0], [0.9, 0.0, 0.0]]);
    line.color = [0.0, 0.0, 1.0];
    line.color_unresolved = false;

    let sample = |renderer: &mut Renderer, batches: &[RenderBatch]| -> [u8; 4] {
        renderer.clear_batches();
        renderer
            .upload(&scene(batches.to_vec()))
            .expect("upload tie scene");
        renderer.render(cam, &target).expect("render tie scene");
        renderer.read_target_rgba().unwrap().pixel(47, 32)
    };

    let mut accel = init_tier(&gpu, true);
    let mut univ = init_tier(&gpu, false);
    let accel_arc_first = sample(&mut accel, &[arc.clone(), line.clone()]);
    let accel_line_first = sample(&mut accel, &[line.clone(), arc.clone()]);
    let univ_arc_first = sample(&mut univ, &[arc.clone(), line.clone()]);
    let univ_line_first = sample(&mut univ, &[line.clone(), arc.clone()]);

    // Accelerated: the residual arc is drawn after the pulled line, so it always
    // wins the tie (red), whatever the plan order.
    for (name, px) in [
        ("arc-first", accel_arc_first),
        ("line-first", accel_line_first),
    ] {
        assert!(
            px[0] > px[2].saturating_add(40),
            "accelerated ({name}): the residual arc must win the pulled-line tie (red), got {px:?}"
        );
    }
    // Universal: plan order decides (the later draw wins).
    assert!(
        univ_arc_first[2] > univ_arc_first[0].saturating_add(40),
        "universal (arc-first): the later line must win the tie (blue), got {univ_arc_first:?}"
    );
    assert!(
        univ_line_first[0] > univ_line_first[2].saturating_add(40),
        "universal (line-first): the later arc must win the tie (red), got {univ_line_first:?}"
    );
    // The exception: with the arc first in plan order the two tiers disagree.
    assert_ne!(
        accel_arc_first, univ_arc_first,
        "the accelerated tier's pulled-before-residual order is the documented tie exception"
    );
}
