use super::*;

/// Required gap (the fence that would have caught B1): cross-class intensity
/// consistency. A same-colour **circle** (SDF), an **ellipse** (chord-expanded
/// `Lines`) and **straight line segments** (also `Lines`) share one frame; every
/// class must reach comparable core intensity (>= 0.9, no class > 20% lighter).
/// A half-intensity or dashed SDF class fails here.
#[test]
fn sdf_arc_cross_class_intensity_is_consistent() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let wpp = 0.5;
    // Camera centre chosen so every integer multiple of `wpp` lands on a pixel
    // centre (`pixel_world == (px - 127) * wpp`), so each class's stroke runs
    // through pixel centres and its peak is the design peak.
    let cam = camera(
        Point3 {
            x: 0.5 * wpp,
            y: 0.5 * wpp,
            z: 0.0,
        },
        wpp,
    );

    // Class A: SDF circle at world x = -24 (px 79), radius 7 (14 px).
    let circle_conic = circle(
        Point3 {
            x: -24.0,
            y: 0.0,
            z: 0.0,
        },
        7.0,
    );
    // Class B: ellipse chord-expanded to `Lines`, centre at the origin (px
    // 127,127), major 10 (20 px) along x, minor 6 along y. The rightmost vertex
    // is on a pixel centre and the tangent there is vertical, so the chord
    // column is full-intensity.
    let ellipse = RenderConic {
        center: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        major: Point3 {
            x: 10.0,
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
    // Class C: straight segments about world x = 24 (px 175): a horizontal run
    // along the pixel row y = 0 and a vertical run along the column x = 24.
    let lines = straight_lines(
        Point3 {
            x: 24.0,
            y: 0.0,
            z: 0.0,
        },
        vec![
            [18.0, 0.0, 0.0],
            [30.0, 0.0, 0.0],
            [24.0, -6.0, 0.0],
            [24.0, 6.0, 0.0],
        ],
        3,
    );

    renderer
        .upload(&scene(vec![
            conic_batch(vec![circle_conic], 1),
            chord_lines_batch(&[ellipse], 2),
            lines,
        ]))
        .expect("upload mixed-class scene");
    let target = RenderTarget::new(SIZE, SIZE);
    renderer.render(cam, &target).expect("render mixed classes");
    let image = renderer.read_target_rgba().expect("read back");

    // Disjoint pixel bands: A (circle) world x < -12 -> px < 103; B (ellipse)
    // |world x| <= 12 -> px 103..=151; C (lines) world x > 12 -> px > 151.
    let peak_sdf = band_peak(&image, 0, 102);
    let peak_ellipse = band_peak(&image, 103, 151);
    let peak_lines = band_peak(&image, 152, SIZE - 1);
    eprintln!(
        "cross-class core peaks: sdf={peak_sdf:.3} ellipse={peak_ellipse:.3} lines={peak_lines:.3}"
    );
    for (name, peak) in [
        ("sdf circle", peak_sdf),
        ("ellipse chord", peak_ellipse),
        ("straight lines", peak_lines),
    ] {
        assert!(
            peak >= 0.9,
            "{name}: core intensity {peak:.3} is below the design peak 0.9"
        );
    }
    let lightest = peak_sdf.min(peak_ellipse).min(peak_lines);
    let heaviest = peak_sdf.max(peak_ellipse).max(peak_lines);
    assert!(
        lightest >= 0.8 * heaviest,
        "a class is >20% lighter than another (sdf={peak_sdf:.3} ellipse={peak_ellipse:.3} \
         lines={peak_lines:.3})"
    );

    // No gaps: sample the SDF circle's whole circumference; each sample must find
    // core ink within a ±2 px neighbourhood (the B1 forensics found an 11 px hole
    // = 15-16% of the circle near-black).
    let r = 7.0;
    let c = circle_conic.center;
    for k in 0..96 {
        let theta = k as f64 / 96.0 * TAU;
        let (wx, wy) = (c.x + r * theta.cos(), c.y + r * theta.sin());
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
        assert!(
            best >= 0.5,
            "SDF circle gap at angle {theta:.3}: no core ink within ±2 px (best {best:.3})"
        );
    }
}

/// Forensics-grade fence (a′): **profile ink-width parity**. A same-frame SDF
/// arc and a hard-aliased 1 px straight line must carry the same ink.
///
/// The image forensics measured the arc's stroke as 1.40-1.46 px vs the line's
/// 1.00 px (a √2 ratio). That was an artifact of integrating the **sRGB-encoded**
/// channel: a soft triangle stroke integrates to ~1.4 in sRGB while a hard 1 px
/// line stays 1.0, even though both carry exactly one pixel of **linear** light.
/// This fence integrates the linear coverage (`coverage_at`) across the stroke at
/// 48 angles and asserts ink ∈ [0.85, 1.15] px, matching the line within 15%.
/// A genuine width-scale regression (e.g. `w = √2 · world_per_px`, or a
/// non-orthonormal frame) fails here; the sRGB artifact does not.
#[test]
fn sdf_arc_ink_width_matches_straight_lines() {
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
    // SDF circle: 8 world units = 16 px on-screen radius, centred on a pixel
    // centre so the axis points are phase-aligned.
    let circle_conic = circle(
        Point3 {
            x: -20.0,
            y: 0.0,
            z: 0.0,
        },
        8.0,
    );
    // A hard-aliased 1 px vertical straight line, far from the ring.
    let lines = straight_lines(
        Point3 {
            x: 20.0,
            y: 0.0,
            z: 0.0,
        },
        vec![[20.0, -6.0, 0.0], [20.0, 6.0, 0.0]],
        5,
    );
    renderer
        .upload(&scene(vec![conic_batch(vec![circle_conic], 1), lines]))
        .expect("upload arc + line");
    let target = RenderTarget::new(SIZE, SIZE);
    renderer.render(cam, &target).expect("render arc + line");
    let image = renderer.read_target_rgba().expect("read back");

    // Reference: the straight line's ink per unit length.
    let (line_px, _) = pixel_at(&cam, 20.0, 0.0);
    let mut line_ink = 0.0;
    let mut rows = 0usize;
    for y in 120..=136u32 {
        let lo = line_px.saturating_sub(4);
        let hi = (line_px + 4).min(SIZE - 1);
        let mut sum = 0.0;
        for x in lo..=hi {
            sum += rendered_coverage(&image, x, y);
        }
        line_ink += sum;
        rows += 1;
    }
    let line_ink = line_ink / rows as f64;
    assert!(
        (0.9..=1.1).contains(&line_ink),
        "the hard 1 px straight line's ink {line_ink:.3} px is not 1.0"
    );

    let mut arc_ink = Vec::new();
    for k in 0..48 {
        let theta = k as f64 / 48.0 * TAU;
        arc_ink.push(radial_ink(&image, &cam, (-20.0, 0.0), 8.0, theta));
    }
    let mean = arc_ink.iter().sum::<f64>() / arc_ink.len() as f64;
    let min = arc_ink.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = arc_ink.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    eprintln!("ink width (px): sdf arc mean={mean:.3} [{min:.3}, {max:.3}] line={line_ink:.3}");
    for (k, &ink) in arc_ink.iter().enumerate() {
        assert!(
            (0.85..=1.15).contains(&ink),
            "SDF arc ink at angle {k} is {ink:.3} px, outside the [0.85, 1.15] band (the triangle \
             filter must carry exactly one pixel of linear coverage)"
        );
    }
    assert!(
        (mean - line_ink).abs() <= 0.15,
        "SDF arc ink {mean:.3} px vs straight-line ink {line_ink:.3} px differ by more than 15%"
    );
}

/// Forensics-grade fence (b′): **per-phase peak bounds**. The triangle filter's
/// peak is phase-dependent (0.5 when the curve runs exactly between two pixel
/// rows/columns, 1.0 when it passes through a pixel centre); it must never drop
/// below the geometric bound nor exceed full intensity. This is sampled across
/// 360 phases rather than asserted at one favourable pixel — the previous
/// `peak >= 0.9`-at-one-angle style claim is exactly what let a half-intensity
/// hairline hide.
#[test]
fn sdf_arc_peak_is_phase_dependent_within_the_aa_envelope() {
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
    let r = 8.0;
    let conic = circle(
        Point3 {
            x: -20.0,
            y: 0.0,
            z: 0.0,
        },
        r,
    );
    renderer
        .upload(&scene(vec![conic_batch(vec![conic], 1)]))
        .expect("upload circle");
    let target = RenderTarget::new(SIZE, SIZE);
    renderer.render(cam, &target).expect("render circle");
    let image = renderer.read_target_rgba().expect("read back");

    let mut peaks = Vec::new();
    for k in 0..360 {
        let theta = k as f64 / 360.0 * TAU;
        let (wx, wy) = (-20.0 + r * theta.cos(), r * theta.sin());
        let (sx, sy) = pixel_at(&cam, wx, wy);
        let mut best = 0.0f64;
        for dy in -1i64..=1 {
            for dx in -1i64..=1 {
                let (x, y) = (sx as i64 + dx, sy as i64 + dy);
                if x < 0 || y < 0 || x >= SIZE as i64 || y >= SIZE as i64 {
                    continue;
                }
                best = best.max(rendered_coverage(&image, x as u32, y as u32));
            }
        }
        peaks.push(best);
    }
    let min = peaks.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = peaks.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let mean = peaks.iter().sum::<f64>() / peaks.len() as f64;
    eprintln!("per-phase peak (linear coverage): mean={mean:.3} [{min:.3}, {max:.3}]");
    // The perpendicular distance from the curve to the nearest pixel centre is
    // at most 0.5 px (choose the pixel with the nearest axis offset), so the
    // triangle filter never peaks below 0.5; the 0.45 bar allows the 8-bit
    // readback + decode slack.
    assert!(
        min >= 0.45,
        "the triangle filter's phase-minimum peak {min:.3} is below 0.45 (a half-intensity or \
         dashed hairline)"
    );
    assert!(
        max <= 1.0 + 1e-3,
        "the triangle filter's peak {max:.3} exceeds full intensity"
    );
    assert!(
        max >= 0.9,
        "no sampled phase reaches core intensity (max {max:.3}); a phase-aligned angle must"
    );
    assert!(
        min < 0.9,
        "the peak is not phase-dependent (min {min:.3}); the AA profile collapsed to a hard edge"
    );
}

/// Forensics-grade fence (c′): **continuity**. The integrated ink around the ring
/// must never dip below 50% of its median — a hole (the B1 axis-point dashes) or
/// a half-intensity stretch fails here even if a single favourable angle still
/// reaches full coverage.
#[test]
fn sdf_arc_ink_is_continuous_around_the_ring() {
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
    let r = 8.0;
    let conic = circle(
        Point3 {
            x: -20.0,
            y: 0.0,
            z: 0.0,
        },
        r,
    );
    renderer
        .upload(&scene(vec![conic_batch(vec![conic], 1)]))
        .expect("upload circle");
    let target = RenderTarget::new(SIZE, SIZE);
    renderer.render(cam, &target).expect("render circle");
    let image = renderer.read_target_rgba().expect("read back");

    let mut ink = Vec::new();
    for k in 0..360 {
        let theta = k as f64 / 360.0 * TAU;
        ink.push(radial_ink(&image, &cam, (-20.0, 0.0), r, theta));
    }
    let mut sorted = ink.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = sorted[sorted.len() / 2];
    let min = ink.iter().cloned().fold(f64::INFINITY, f64::min);
    eprintln!("ring ink continuity: median={median:.3} min={min:.3}");
    assert!(
        median > 0.5,
        "the ring's median ink {median:.3} px is too low"
    );
    for (k, &value) in ink.iter().enumerate() {
        assert!(
            value >= 0.5 * median,
            "ring ink at angle {k} is {value:.3} px, below 50% of the median {median:.3} px \
             (a gap or a half-intensity stretch)"
        );
    }
}

/// Required gap (S3): the quad margin must be floored at ~2 world pixels so a
/// small on-screen circle's ~1 px anti-aliased fringe is not clipped at the four
/// axis points, where the quad edge is nearest the ring. The ring is placed
/// **half a pixel** off the pixel grid so the outer fringe is observable; a pure
/// 2% relative margin clips it below ~25 px on-screen radius.
#[test]
fn sdf_arc_small_circle_fringe_survives_zoom_out() {
    let Some(gpu0) = gpu() else {
        return;
    };
    drop(gpu0);
    let wpp = 1.0;
    let base = Point3 {
        x: 1_000_000.0,
        y: -500_000.0,
        z: 0.0,
    };
    // On-screen radius (px) sweeps from comfortable to deep zoom-out. The ring
    // radius is a half-integer number of pixels so it falls between pixel centres.
    for &screen_radius in &[40.0f64, 20.0, 10.0, 5.0, 3.0] {
        let Some(gpu) = gpu() else {
            return;
        };
        let mut renderer = init(gpu);
        let radius_px = screen_radius + 0.5;
        let world_radius = radius_px * wpp;
        let conic = circle(base, world_radius);
        let cam = aligned_camera(base, wpp);
        renderer
            .upload(&scene(vec![conic_batch(vec![conic], 1)]))
            .expect("upload small circle");
        let target = RenderTarget::new(SIZE, SIZE);
        renderer.render(cam, &target).expect("render small circle");
        let image = renderer.read_target_rgba().expect("read back");

        // Outermost ink extent (max distance-to-curve in px that still carries
        // ink) along the 4 axis directions and a 45-degree reference.
        let extent = |dir: (f64, f64)| -> f64 {
            let (dx, dy) = dir;
            let (rx, ry) = (base.x + world_radius * dx, base.y + world_radius * dy);
            let (sx, sy) = pixel_at(&cam, rx, ry);
            let mut outer = 0.0f64;
            for oy in -3i64..=3 {
                for ox in -3i64..=3 {
                    let (x, y) = (sx as i64 + ox, sy as i64 + oy);
                    if x < 0 || y < 0 || x >= SIZE as i64 || y >= SIZE as i64 {
                        continue;
                    }
                    let (wx, wy) = pixel_world(&cam, x as u32, y as u32);
                    let d_px = analytic_distance(&conic, (wx, wy)) / wpp;
                    if rendered_coverage(&image, x as u32, y as u32) >= 0.25 {
                        outer = outer.max(d_px);
                    }
                }
            }
            outer
        };
        let axis = [
            extent((1.0, 0.0)),
            extent((-1.0, 0.0)),
            extent((0.0, 1.0)),
            extent((0.0, -1.0)),
        ];
        let reference = extent((
            std::f64::consts::FRAC_1_SQRT_2,
            std::f64::consts::FRAC_1_SQRT_2,
        ));
        eprintln!(
            "screen radius {screen_radius}px: axis fringe extents {axis:?}, 45deg {reference:.3}"
        );
        for (i, &a) in axis.iter().enumerate() {
            assert!(
                a >= 0.5 * reference,
                "screen radius {screen_radius}px: axis {i} fringe extent {a:.3} is clipped vs the \
                 45-degree reference {reference:.3}"
            );
        }
    }
}

/// Required gap (S4): a transparent conic batch has no supported draw path, so it
/// is **rejected at upload** rather than silently dropped. An invisible
/// (`alpha <= 0`) conic is not rejected — it is reported as invisible.
#[test]
fn transparent_conic_batch_is_rejected_at_upload() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let mut batch = conic_batch(
        vec![circle(
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            10.0,
        )],
        1,
    );
    batch.alpha = 0.5;
    let err = renderer
        .upload(&scene(vec![batch]))
        .expect_err("a transparent conic batch must be rejected");
    assert!(
        matches!(err, cad_domain::CadError::InvalidInput(_)),
        "expected InvalidInput, got {err:?}"
    );

    // An invisible conic is accepted (it is reported, never silently drawn).
    let mut invisible = conic_batch(
        vec![circle(
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            10.0,
        )],
        2,
    );
    invisible.alpha = 0.0;
    renderer
        .upload(&scene(vec![invisible]))
        .expect("an invisible conic batch must be accepted (reported, not drawn)");
    assert_eq!(renderer.arc_instance_count(), 0);
}

/// N2: a degenerate conic skipped at pack is **counted**, never silent.
#[test]
fn degenerate_conic_skip_is_counted() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let degenerate = RenderConic {
        center: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        major: Point3 {
            x: 10.0,
            y: 0.0,
            z: 0.0,
        },
        minor: Point3 {
            x: 0.0,
            y: 10.0,
            z: 0.0,
        },
        start: 0.0,
        sweep: 0.0,
        full: false,
    };
    renderer
        .upload(&scene(vec![conic_batch(vec![degenerate], 1)]))
        .expect("upload degenerate conic batch");
    assert_eq!(
        renderer.arc_instance_count(),
        0,
        "a degenerate conic must not be instanced"
    );
    assert_eq!(
        renderer.arc_degenerate_skips(),
        1,
        "the degenerate skip must be counted"
    );
}

/// Fence (h): visual acceptance. Render a **mixed arc + line real drawing** twice
/// — once with the conics chord-expanded (`before`, the pre-SDF draw) and once as
/// SDF instances (`after`) — into a NEW output directory. The straight lines are
/// identical in both frames, so the SDF ring can be compared against them. The
/// SDF frame must be continuous (no gaps), reach full core opacity, and carry a
/// **profile ink** (integrated linear coverage) consistent with the lines; there
/// is **no** byte-identity claim with chords. This is the sanctioned
/// semantic-change evidence (`docs/adr/0008-sdf-conic-rendering.md`).
#[test]
#[ignore = "real-GPU visual acceptance; writes PNGs to YACR_SDF_ARC_OUT"]
fn sdf_arc_visual_acceptance_before_after() {
    let Some(gpu) = gpu() else {
        return;
    };
    eprintln!("SDF arc visual adapter: {:?}", gpu.adapter);
    let mut renderer = init(gpu);
    let base = Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    // A small "part" drawing: a bolt circle, two arcs and a hub, plus straight
    // line segments (identical in both frames) so weight can be compared.
    let conics = vec![
        circle(base, 0.8),
        arc(base, 0.6, 0.0, 1.6),
        arc(base, 0.6, 2.0, -1.2),
        circle(base, 0.35),
    ];
    let lines = straight_lines(
        base,
        vec![
            [-0.95, 0.0, 0.0],
            [-0.55, 0.0, 0.0],
            [0.55, 0.0, 0.0],
            [0.95, 0.0, 0.0],
            [0.0, -0.95, 0.0],
            [0.0, -0.55, 0.0],
        ],
        9,
    );
    let world_per_px = 2.0 / 256.0; // 256 px spans 2 world units
    let cam = aligned_camera(base, world_per_px);
    let target = RenderTarget::new(256, 256);

    // after: the SDF instance draw plus the straight lines.
    renderer
        .upload(&scene(vec![conic_batch(conics.clone(), 1), lines.clone()]))
        .expect("upload SDF conics");
    renderer.render(cam, &target).expect("render SDF");
    let after = renderer.read_target_rgba().expect("read SDF frame");

    // before: the same analytic geometry chord-expanded plus the straight lines.
    renderer
        .upload(&scene(vec![chord_lines_batch(&conics, 1), lines.clone()]))
        .expect("upload chord lines");
    renderer.render(cam, &target).expect("render chords");
    let before = renderer.read_target_rgba().expect("read chord frame");

    let dir = std::env::var("YACR_SDF_ARC_OUT")
        .unwrap_or_else(|_| "/tmp/opencode/yacr-sdf-arcs-v3".to_string());
    std::fs::create_dir_all(&dir).expect("create output dir");
    std::fs::write(
        format!("{dir}/before-chord.png"),
        cad_render_wgpu::headless::encode_png(&before).expect("encode before"),
    )
    .expect("write before");
    std::fs::write(
        format!("{dir}/after-sdf.png"),
        cad_render_wgpu::headless::encode_png(&after).expect("encode after"),
    )
    .expect("write after");
    eprintln!("wrote SDF arc before/after to {dir}");

    assert!(
        before.count_differing_from(before.pixel(0, 0), 8) > 100,
        "the chord (before) frame is blank"
    );
    assert!(
        after.count_differing_from(after.pixel(0, 0), 8) > 100,
        "the SDF (after) frame is blank"
    );
    assert_ne!(
        before.pixels, after.pixels,
        "the SDF draw must differ from the chord draw (no byte-identity claim)"
    );

    // Acceptance bar (image forensics): the SDF ring is continuous (no gaps),
    // reaches full core opacity, and matches the straight-line weight.
    let r = 0.8;
    let mut ring_peak = 0.0f64;
    for k in 0..144 {
        let theta = k as f64 / 144.0 * TAU;
        let (wx, wy) = (base.x + r * theta.cos(), base.y + r * theta.sin());
        let (sx, sy) = pixel_at(&cam, wx, wy);
        let mut best = 0.0f64;
        for dy in -2i64..=2 {
            for dx in -2i64..=2 {
                let (x, y) = (sx as i64 + dx, sy as i64 + dy);
                if x < 0 || y < 0 || x >= 256 || y >= 256 {
                    continue;
                }
                best = best.max(rendered_coverage(&after, x as u32, y as u32));
            }
        }
        assert!(
            best >= 0.5,
            "SDF ring gap at angle {theta:.3}: no core ink within ±2 px (best {best:.3})"
        );
        ring_peak = ring_peak.max(best);
    }
    // A straight-line pixel (the horizontal run at y = 0, world x in [-0.95,-0.55])
    // is fully opaque; the SDF ring core must be comparable.
    let (lx, ly) = pixel_at(&cam, -0.75, 0.0);
    let line_peak = rendered_coverage(&after, lx, ly);
    eprintln!("(h) ring core peak={ring_peak:.3} line core={line_peak:.3}");
    assert!(
        ring_peak >= 0.9,
        "the SDF ring core {ring_peak:.3} does not reach full opacity"
    );
    assert!(
        ring_peak >= 0.8 * line_peak,
        "the SDF ring ({ring_peak:.3}) is >20% lighter than the straight lines ({line_peak:.3})"
    );

    // Profile **ink** parity (the forensics-grade claim): the ring's integrated
    // *linear* coverage per unit length must match the hard 1 px straight line's.
    // The lines cross the ring near 0/180/270 degrees, so those rays are skipped.
    let mut line_ink = 0.0f64;
    for y in ly.saturating_sub(2)..=(ly + 2).min(255) {
        line_ink += rendered_coverage(&after, lx, y);
    }
    let mut ring_ink = Vec::new();
    for k in (5..=175).chain(185..=265).chain(275..=355) {
        let theta = k as f64 * std::f64::consts::PI / 180.0;
        ring_ink.push(radial_ink(&after, &cam, (base.x, base.y), r, theta));
    }
    let ring_ink_mean = ring_ink.iter().sum::<f64>() / ring_ink.len() as f64;
    let ring_ink_min = ring_ink.iter().cloned().fold(f64::INFINITY, f64::min);
    let ring_ink_max = ring_ink.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    eprintln!(
        "(h) ring ink mean={ring_ink_mean:.3} [{ring_ink_min:.3}, {ring_ink_max:.3}] px, \
         line ink={line_ink:.3} px"
    );
    for (i, &ink) in ring_ink.iter().enumerate() {
        assert!(
            (0.85..=1.15).contains(&ink),
            "(h) SDF ring ink sample {i} is {ink:.3} px, outside the [0.85, 1.15] band"
        );
    }
    assert!(
        (ring_ink_mean - line_ink).abs() <= 0.15,
        "(h) the SDF ring ink {ring_ink_mean:.3} px and the straight-line ink {line_ink:.3} px \
         differ by more than 15%"
    );

    // Smoother: both paths now share the unified AA stroke (ADR-0009), so the
    // chord draw is no longer a hard-aliased baseline and the "SDF at least as
    // anti-aliased as the chord draw" *ordering* (distinct sRGB grey-level
    // counts) is no longer a sound smoothness metric — the two AA rasterizations
    // merely produce different grey populations (measured ~137 vs ~152; a
    // grey-level count is an sRGB-encoded heuristic, see ADR-0008 §2). Assert
    // instead that both frames carry genuine anti-aliasing (a hard/binary draw
    // has only a handful of levels) and report the counts for the record.
    let levels = |image: &RgbaImage| -> usize {
        let mut set = std::collections::BTreeSet::new();
        for p in image.pixels.chunks_exact(4) {
            if p[0] > 8 && p[0] < 247 {
                set.insert(p[0]);
            }
        }
        set.len()
    };
    eprintln!(
        "AA grey levels: chord={} sdf={}",
        levels(&before),
        levels(&after)
    );
    assert!(
        levels(&before) >= 32 && levels(&after) >= 32,
        "both the chord and the SDF frame must carry anti-aliased edges \
         (chord={} sdf={})",
        levels(&before),
        levels(&after)
    );
}
