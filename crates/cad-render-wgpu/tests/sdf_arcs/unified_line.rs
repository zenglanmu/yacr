use super::*;

/// Fence (i): **cross-class ink parity**. An SDF arc, an instanced straight
/// line, a `MeshEdges` line batch, a transparent line and a chord-expanded conic
/// (the dashed/ellipse fallback, a `Lines` batch) all render in one frame; every
/// class must carry ink ∈ [0.85, 1.15] px of **linear** coverage.
#[test]
fn unified_line_ink_parity_across_classes() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let wpp = 0.5;
    let cam = aligned_camera(
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        wpp,
    );

    // Class A: SDF circle (Conics) at world x = -48, radius 8.
    let circle_conic = circle(
        Point3 {
            x: -48.0,
            y: 0.0,
            z: 0.0,
        },
        8.0,
    );
    // Class B: an opaque instanced straight line at x = -24.
    let line = vertical_line(-24.0, 6.0, 1.0, [1.0, 1.0, 1.0], 2);
    // Class C: a `MeshEdges` line batch at x = 0 (previously a silent no-op).
    let mut mesh_edges = vertical_line(0.0, 6.0, 1.0, [1.0, 1.0, 1.0], 3);
    mesh_edges.topology = RenderTopology::MeshEdges;
    // Class D: a transparent line at x = 24 (alpha 0.5, drawn per-batch).
    let transparent = vertical_line(24.0, 6.0, 0.5, [1.0, 1.0, 1.0], 4);
    // Class E: a chord-expanded ellipse (the fallback path) at x = 48.
    let ellipse = RenderConic {
        center: Point3 {
            x: 48.0,
            y: 0.0,
            z: 0.0,
        },
        major: Point3 {
            x: 8.0,
            y: 0.0,
            z: 0.0,
        },
        minor: Point3 {
            x: 0.0,
            y: 6.0,
            z: 0.0,
        },
        start: 0.0,
        sweep: TAU,
        full: true,
    };
    let chord = chord_lines_batch(&[ellipse], 5);

    renderer
        .upload(&scene(vec![
            conic_batch(vec![circle_conic], 1),
            line,
            mesh_edges,
            transparent,
            chord,
        ]))
        .expect("upload mixed-class scene");
    let target = RenderTarget::new(SIZE, SIZE);
    renderer.render(cam, &target).expect("render mixed classes");
    let image = renderer.read_target_rgba().expect("read back");

    let arc_ink = radial_ink(&image, &cam, (-48.0, 0.0), 8.0, 0.0);
    let line_ink = perpendicular_ink(&image, &cam, (-24.0, 0.0), (1.0, 0.0));
    let edge_ink = perpendicular_ink(&image, &cam, (0.0, 0.0), (1.0, 0.0));
    // The transparent class blends at alpha 0.5, so its linear coverage is half a
    // stroke; normalise by the alpha to compare stroke weight.
    let transparent_ink = perpendicular_ink(&image, &cam, (24.0, 0.0), (1.0, 0.0)) / 0.5;
    let chord_ink = perpendicular_ink(&image, &cam, (56.0, 0.0), (1.0, 0.0));
    eprintln!(
        "unified ink (px): arc={arc_ink:.3} line={line_ink:.3} mesh_edges={edge_ink:.3} \
         transparent={transparent_ink:.3} chord={chord_ink:.3}"
    );
    for (name, ink) in [
        ("sdf arc", arc_ink),
        ("instanced line", line_ink),
        ("mesh edges", edge_ink),
        ("transparent line", transparent_ink),
        ("chord lines", chord_ink),
    ] {
        assert!(
            (0.85..=1.15).contains(&ink),
            "{name}: linear ink {ink:.3} px is outside the [0.85, 1.15] band"
        );
    }
}

/// Fence (ii): **per-class peak phase envelope**. Sixteen vertical lines placed
/// at sixteen sub-pixel phases in one frame: the triangle filter's peak must be
/// phase-dependent (never below ~0.5), never exceed full intensity, and reach
/// core intensity at some phase.
#[test]
fn unified_line_peak_is_phase_dependent_within_the_aa_envelope() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let wpp = 0.5;
    let cam = aligned_camera(
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        wpp,
    );
    // Sixteen lines, 8 px apart, each offset by a sixteenth of a pixel from the
    // previous phase.
    let phases = 16;
    let mut segments = Vec::new();
    for k in 0..phases {
        let x = -60.0 + k as f64 * 8.0 * wpp + (k as f64 / phases as f64) * wpp;
        segments.push([x, -3.0, 0.0]);
        segments.push([x, 3.0, 0.0]);
    }
    let batch = straight_lines(
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        segments,
        1,
    );
    renderer.upload(&scene(vec![batch])).expect("upload phases");
    let target = RenderTarget::new(SIZE, SIZE);
    renderer.render(cam, &target).expect("render phases");
    let image = renderer.read_target_rgba().expect("read back");

    let mut peaks = Vec::new();
    for k in 0..phases {
        let x = -60.0 + k as f64 * 8.0 * wpp + (k as f64 / phases as f64) * wpp;
        let (sx, sy) = pixel_at(&cam, x, 0.0);
        let mut best = 0.0f64;
        for dx in -1i64..=1 {
            let px = sx as i64 + dx;
            if px < 0 || px >= SIZE as i64 {
                continue;
            }
            best = best.max(rendered_coverage(&image, px as u32, sy));
        }
        peaks.push(best);
    }
    let min = peaks.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = peaks.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let mean = peaks.iter().sum::<f64>() / peaks.len() as f64;
    eprintln!("unified line per-phase peak: mean={mean:.3} [{min:.3}, {max:.3}]");
    assert!(
        min >= 0.45,
        "the line triangle filter's phase-minimum peak {min:.3} is below 0.45"
    );
    assert!(
        max <= 1.0 + 1e-3,
        "the line triangle filter's peak {max:.3} exceeds full intensity"
    );
    assert!(
        max >= 0.9,
        "no sampled phase reaches core intensity (max {max:.3})"
    );
    assert!(
        min < 0.9,
        "the line peak is not phase-dependent (min {min:.3})"
    );
}

/// Fence (iii)+(iv): **continuity and joint alpha union**. A polyline with a
/// collinear joint and a 90° angled joint: every point along it must carry core
/// ink (no gap at a joint), and a shared endpoint must not be darker than a
/// single stroke nor brighter than full intensity (the two overlapping AA quads
/// must union, not double).
#[test]
fn unified_line_polyline_joints_are_continuous_and_single_weight() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let wpp = 0.5;
    let cam = aligned_camera(
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        wpp,
    );
    // Four segments: an angled joint at (-4, 0) and a collinear joint at (4, 8).
    let batch = straight_lines(
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        vec![
            [-12.0, 0.0, 0.0],
            [-4.0, 0.0, 0.0],
            [-4.0, 0.0, 0.0],
            [-4.0, 8.0, 0.0],
            [-4.0, 8.0, 0.0],
            [4.0, 8.0, 0.0],
            [4.0, 8.0, 0.0],
            [12.0, 8.0, 0.0],
        ],
        1,
    );
    renderer
        .upload(&scene(vec![batch]))
        .expect("upload polyline");
    let target = RenderTarget::new(SIZE, SIZE);
    renderer.render(cam, &target).expect("render polyline");
    let image = renderer.read_target_rgba().expect("read back");

    // Core ink at a point: the maximum linear coverage in a ±2 px neighbourhood.
    let core_at = |wx: f64, wy: f64| -> f64 {
        let (sx, sy) = pixel_at(&cam, wx, wy);
        let mut best = 0.0f64;
        for dy in -2i64..=2 {
            for dx in -2i64..=2 {
                let (x, y) = (sx as i64 + dx, sy as i64 + dy);
                if x < 0 || y < 0 || x >= SIZE as i64 || y >= SIZE as i64 {
                    continue;
                }
                best = best.max(rendered_coverage(&image, x as u32, y as u32));
            }
        }
        best
    };

    // Continuity: sample every segment's interior AND both joint endpoints.
    let samples = [
        (-8.0, 0.0), // segment A mid
        (-4.0, 0.0), // angled joint
        (-4.0, 4.0), // segment B mid
        (0.0, 8.0),  // segment C mid
        (4.0, 8.0),  // collinear joint
        (8.0, 8.0),  // segment D mid
    ];
    let mut joint_peaks = Vec::new();
    for (wx, wy) in samples {
        let peak = core_at(wx, wy);
        joint_peaks.push(peak);
        assert!(
            peak >= 0.45,
            "the polyline has a gap at ({wx},{wy}): core ink {peak:.3} < 0.45"
        );
        assert!(
            peak <= 1.0 + 1e-3,
            "the polyline over-covers at ({wx},{wy}): core ink {peak:.3} > 1"
        );
    }
    // The joints (samples 1 and 4) must not be darker than a segment interior.
    let mid_min = joint_peaks[0]
        .min(joint_peaks[2])
        .min(joint_peaks[3])
        .min(joint_peaks[5]);
    for (i, &peak) in joint_peaks.iter().enumerate() {
        assert!(
            peak + 0.05 >= mid_min,
            "polyline sample {i} peak {peak:.3} is darker than a single stroke \
             (mid minimum {mid_min:.3}); the AA quads must union at a joint"
        );
    }
    eprintln!("polyline joint peaks: {joint_peaks:?} (mid min {mid_min:.3})");
}

/// Fence (v): determinism and zero re-upload on pan/zoom for the converted line
/// streams (the AA change must not turn the static stream into a per-frame
/// upload).
#[test]
fn unified_line_streams_are_deterministic_and_static_across_a_camera_move() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let wpp = 0.5;
    let cam = aligned_camera(
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        wpp,
    );
    let batch = straight_lines(
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        vec![
            [-12.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            [0.0, 8.0, 0.0],
            [8.0, 8.0, 0.0],
        ],
        1,
    );
    renderer
        .upload(&scene(vec![batch]))
        .expect("upload polyline");
    assert_eq!(renderer.instance_stream_count(), 1);
    assert_eq!(renderer.instance_segment_count(), 2);

    let target = RenderTarget::new(SIZE, SIZE);
    renderer.render(cam, &target).expect("render");
    let first = renderer.read_target_rgba().unwrap();
    let uploads_after_first = renderer.instance_stream_uploads();

    renderer.render(cam, &target).expect("render again");
    let second = renderer.read_target_rgba().unwrap();
    assert_eq!(
        first.pixels, second.pixels,
        "the same scene must be deterministic"
    );

    for i in 0..5 {
        let moved = camera(
            Point3 {
                x: i as f64 * 3.0,
                y: -(i as f64) * 2.0,
                z: 0.0,
            },
            wpp + i as f64 * 0.1,
        );
        renderer.render(moved, &target).expect("render panned");
        assert_eq!(
            renderer.instance_stream_uploads(),
            uploads_after_first,
            "a camera move must not re-upload a converted line stream"
        );
    }
}

/// Fence (h2): visual acceptance for the line unification. A mixed scene (an SDF
/// circle, straight lines and polyline joints) is rendered **twice with the
/// identical geometry** into a NEW output directory: `before` with the line ramp
/// overridden to a sub-pixel width (the pre-unification hard-aliased 1 px look)
/// and `after` with the unified AA strokes. The two captures differ **only** in
/// the line stroke rendering mode, so the fence asserts their core-ink masks
/// cover the same geometry (equal connected components, a bounded symmetric
/// difference, and per-piece analytic coverage in both frames) and reports the
/// per-class linear ink. There is **no** byte-identity claim.
///
/// The scene is defined in screen-pixel coordinates so every stroke runs through
/// pixel centres. That is required by the harness: the hard look is emulated with
/// a sub-pixel line ramp whose `coverage <= 0` discard removes a stroke whose
/// perpendicular phase falls between pixel rows — an off-phase scene drops whole
/// segments from the `before` capture (the v4 evidence defect: two horizontal
/// segments absent from `before`).
#[test]
#[ignore = "real-GPU visual acceptance; writes PNGs to YACR_SDF_ARC_OUT"]
fn unified_line_visual_acceptance_before_after() {
    let Some(gpu) = gpu() else {
        return;
    };
    eprintln!("line unification visual adapter: {:?}", gpu.adapter);
    let mut renderer = init(gpu);
    let wpp = 2.0 / 256.0; // 256 px spans 2 world units
    let cam = aligned_camera(
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        wpp,
    );
    let target = RenderTarget::new(256, 256);

    // Screen-pixel -> world. Every argument is an integer pixel coordinate, so
    // the result is an exact multiple of `wpp` on both axes and therefore lands
    // on a pixel centre under `aligned_camera` (whose pixel centre `(px, py)`
    // maps to `((px - 127) * wpp, (128 - py) * wpp)`). Defining the scene in
    // screen pixels keeps every stroke phase-aligned to a pixel centre.
    let at = |sx: f64, sy: f64| [(sx - 127.0) * wpp, (127.0 - sy) * wpp, 0.0];

    let conics = vec![circle(
        Point3 {
            x: -0.5,
            y: 0.0,
            z: 0.0,
        },
        0.25,
    )];
    // Straight lines and a polyline with an angled joint on the right half; a
    // 45-degree diagonal (phase-aligned through pixel centres) gives the AA draw
    // intermediate grey levels the hard draw cannot.
    let lines = straight_lines(
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        vec![
            at(139.0, 207.0),
            at(239.0, 207.0),
            at(139.0, 127.0),
            at(191.0, 127.0),
            at(191.0, 127.0),
            at(191.0, 47.0),
            at(191.0, 47.0),
            at(239.0, 47.0),
            at(139.0, 110.0),
            at(171.0, 78.0),
        ],
        9,
    );

    // Upload the scene ONCE: the two captures must differ only in the line
    // stroke rendering mode, which is a per-frame uniform value (the ramp
    // override), not a scene change. Re-uploading the same delta would append
    // duplicate batches — `Renderer::upload` stages an incremental delta and
    // never replaces the resident scene — which double-blends the circle and is
    // the other half of the v4 "different scene" defect.
    renderer.set_line_ramp_override(None);
    renderer
        .upload(&scene(vec![conic_batch(conics.clone(), 1), lines.clone()]))
        .expect("upload scene");
    renderer.render(cam, &target).expect("render unified");
    let after = renderer.read_target_rgba().expect("read unified frame");
    let bottom_mid = at(189.0, 207.0);
    let after_ink = perpendicular_ink(&after, &cam, (bottom_mid[0], bottom_mid[1]), (0.0, 1.0));

    // before: the same resident geometry with a sub-pixel line ramp (hard look).
    renderer.set_line_ramp_override(Some(1e-3));
    renderer.render(cam, &target).expect("render hard");
    let before = renderer.read_target_rgba().expect("read hard frame");
    renderer.set_line_ramp_override(None);

    let dir = std::env::var("YACR_SDF_ARC_OUT")
        .unwrap_or_else(|_| "/tmp/opencode/yacr-sdf-arcs-v5".to_string());
    std::fs::create_dir_all(&dir).expect("create output dir");
    std::fs::write(
        format!("{dir}/before-hard-lines.png"),
        cad_render_wgpu::headless::encode_png(&before).expect("encode before"),
    )
    .expect("write before");
    std::fs::write(
        format!("{dir}/after-unified-aa.png"),
        cad_render_wgpu::headless::encode_png(&after).expect("encode after"),
    )
    .expect("write after");
    eprintln!("wrote line-unification before/after to {dir} (after line ink {after_ink:.3} px)");

    assert!(
        before.count_differing_from(before.pixel(0, 0), 8) > 100,
        "the hard-line (before) frame is blank"
    );
    assert!(
        after.count_differing_from(after.pixel(0, 0), 8) > 100,
        "the unified (after) frame is blank"
    );
    assert!(
        (0.85..=1.15).contains(&after_ink),
        "the unified line ink {after_ink:.3} px is outside [0.85, 1.15]"
    );

    // Both captures must draw the SAME geometry, differing only in stroke
    // rendering mode, so their core-ink masks must cover the same strokes: equal
    // connected components and a bounded symmetric difference. (v4 dropped two
    // whole horizontal segments from `before`; this fails here.)
    let mask_before = core_mask(&before);
    let mask_after = core_mask(&after);
    let comps_before = mask_components(&mask_before);
    let comps_after = mask_components(&mask_after);
    let xor = mask_before
        .iter()
        .zip(&mask_after)
        .filter(|(a, b)| a != b)
        .count();
    let union = mask_before
        .iter()
        .zip(&mask_after)
        .filter(|(a, b)| **a || **b)
        .count();
    eprintln!(
        "core ink masks: before components={comps_before} after={comps_after} xor={xor} \
         union={union}"
    );
    assert!(
        union > 300,
        "the core ink masks are too small to be meaningful ({union} px)"
    );
    assert_eq!(
        comps_before, comps_after,
        "the before/after core masks have different connected-component counts ({comps_before} vs \
         {comps_after}); the captures are not the same scene"
    );
    assert!(
        xor * 5 <= union,
        "the before/after core masks differ on {xor} of {union} px; the captures must cover the \
         same geometry"
    );

    // Direct per-piece coverage: every analytic piece of the scene must carry
    // core ink in BOTH frames. This is the assertion that would have caught the
    // v4 defect (the two off-phase horizontal segments vanished from `before`).
    let mut probes: Vec<(f64, f64)> = Vec::new();
    for k in 0..8 {
        let theta = k as f64 / 8.0 * TAU;
        probes.push((-0.5 + 0.25 * theta.cos(), 0.25 * theta.sin()));
    }
    for (sx, sy) in [
        (139.0, 207.0),
        (189.0, 207.0),
        (239.0, 207.0),
        (139.0, 127.0),
        (165.0, 127.0),
        (191.0, 87.0),
        (191.0, 47.0),
        (215.0, 47.0),
        (239.0, 47.0),
        (139.0, 110.0),
        (155.0, 94.0),
        (171.0, 78.0),
    ] {
        let p = at(sx, sy);
        probes.push((p[0], p[1]));
    }
    let core_at = |image: &RgbaImage, wx: f64, wy: f64| -> f64 {
        let (sx, sy) = pixel_at(&cam, wx, wy);
        let mut best = 0.0f64;
        for dy in -2i64..=2 {
            for dx in -2i64..=2 {
                let (x, y) = (sx as i64 + dx, sy as i64 + dy);
                if x < 0 || y < 0 || x >= 256 || y >= 256 {
                    continue;
                }
                best = best.max(rendered_coverage(image, x as u32, y as u32));
            }
        }
        best
    };
    for (wx, wy) in probes {
        let b = core_at(&before, wx, wy);
        let a = core_at(&after, wx, wy);
        assert!(
            b >= 0.5,
            "the hard (before) frame is missing geometry near world ({wx:.4},{wy:.4}) \
             (core ink {b:.3})"
        );
        assert!(
            a >= 0.5,
            "the unified (after) frame is missing geometry near world ({wx:.4},{wy:.4}) \
             (core ink {a:.3})"
        );
    }

    // The unified AA line is smoother than the hard one: in the line region
    // (x >= 100, clear of the circle) the sub-pixel hard ramp is binary (no
    // intermediate pixels) while the AA strokes carry partial-coverage edge
    // pixels (the 45-degree diagonal, in particular). Measured there so the
    // circle's own AA greys cannot mask the difference; a whole-frame distinct
    // sRGB grey-level count is not a sound smoothness metric (ADR-0008 §2).
    let partial_in_lines = |image: &RgbaImage| -> usize {
        let mut n = 0usize;
        for y in 0..SIZE {
            for x in 100..SIZE {
                let c = rendered_coverage(image, x, y);
                if c > 0.02 && c < 0.98 {
                    n += 1;
                }
            }
        }
        n
    };
    let partial_hard = partial_in_lines(&before);
    let partial_aa = partial_in_lines(&after);
    eprintln!("line-region partial-coverage pixels: hard={partial_hard} unified={partial_aa}");
    assert!(
        partial_aa >= 16 && partial_aa > partial_hard,
        "the unified AA line must carry partial-coverage edge pixels the hard ramp cannot \
         (hard={partial_hard} unified={partial_aa})"
    );
}
