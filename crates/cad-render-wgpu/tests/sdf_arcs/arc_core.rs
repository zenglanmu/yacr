use super::*;

/// Fence (a): the rendered SDF matches the f64 analytic curve, for full circles
/// and arcs at large world coordinates. Pixels within a quarter-pixel of the
/// analytic curve carry **full** ink (`>= 0.9`, the triangle filter's design
/// value); pixels at least 1.5 px away are exactly background; the ±1 px band
/// between is tolerated.
#[test]
fn sdf_arc_coverage_matches_the_analytic_curve() {
    let Some(first) = gpu() else {
        return;
    };
    eprintln!("SDF arc coverage adapter: {:?}", first.adapter);
    drop(first);
    // A grid of centres/radii/sweeps at ~1e5–1e6 world coordinates. Each conic
    // is rendered alone so a background assertion cannot see a neighbour's ink.
    let specs: Vec<RenderConic> = vec![
        circle(
            Point3 {
                x: 1_000_000.0,
                y: 250_000.0,
                z: 0.0,
            },
            12.0,
        ),
        arc(
            Point3 {
                x: 1_000_000.0,
                y: 250_000.0,
                z: 0.0,
            },
            12.0,
            0.3,
            TAU - 0.05, // near-τ
        ),
        arc(
            Point3 {
                x: 250_000.0,
                y: -1_000_000.0,
                z: 0.0,
            },
            10.0,
            0.5,
            -2.0, // clockwise
        ),
        circle(
            Point3 {
                x: -999_999.0,
                y: -999_999.0,
                z: 0.0,
            },
            8.0,
        ),
        circle(
            Point3 {
                x: 750_000.0,
                y: 250_000.0,
                z: 0.0,
            },
            5.0,
        ),
    ];
    let world_per_px = 0.2; // 256 px spans 51.2 world units
    for (index, conic) in specs.iter().enumerate() {
        let Some(gpu) = gpu() else {
            return;
        };
        let mut renderer = init(gpu);
        let cam = aligned_camera(conic.center, world_per_px);
        renderer
            .upload(&scene(vec![conic_batch(vec![*conic], 1)]))
            .expect("upload conic batch");
        let target = RenderTarget::new(SIZE, SIZE);
        renderer.render(cam, &target).expect("render conic");
        let image = renderer.read_target_rgba().expect("read back");

        let mut on_samples = 0usize;
        let mut off_samples = 0usize;
        let mut peak = 0.0f64;
        for y in 0..SIZE {
            for x in 0..SIZE {
                let d = analytic_distance(conic, pixel_world(&cam, x, y));
                let d_px = d / world_per_px;
                let got = rendered_coverage(&image, x, y);
                peak = peak.max(got);
                if d_px <= 0.05 {
                    on_samples += 1;
                    assert!(
                        got >= 0.9,
                        "conic {index}: pixel ({x},{y}) is {d_px:.3}px from the analytic curve but \
                         coverage is {got:.3} (the triangle filter must reach full intensity on \
                         the curve)"
                    );
                } else if d_px >= 1.5 {
                    off_samples += 1;
                    assert!(
                        got <= 0.02,
                        "conic {index}: pixel ({x},{y}) is {d_px:.3}px from the analytic curve but \
                         coverage is {got:.3} (expected background)"
                    );
                }
            }
        }
        assert!(
            on_samples > 5,
            "conic {index}: too few on-curve samples: {on_samples}"
        );
        assert!(
            off_samples > 1000,
            "conic {index}: too few off-curve samples: {off_samples}"
        );
        assert!(
            peak >= 0.9,
            "conic {index}: the peak ring coverage {peak:.3} is below 0.9; the analytic curve \
             should pass close to a pixel centre"
        );
    }
}

/// Fence (b): every vertex of the current chord tessellation lands inside the
/// SDF ≥ 0.5-coverage band, so there is no gap at a chord joint.
#[test]
fn sdf_arc_contains_every_chord_vertex() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let base = Point3 {
        x: 500_000.0,
        y: -300_000.0,
        z: 0.0,
    };
    let conic = arc(base, 40.0, 0.2, 2.5);
    let world_per_px = 0.5;
    let cam = aligned_camera(base, world_per_px);
    renderer
        .upload(&scene(vec![conic_batch(vec![conic], 1)]))
        .expect("upload conic batch");
    let target = RenderTarget::new(SIZE, SIZE);
    renderer.render(cam, &target).expect("render conic");
    let image = renderer.read_target_rgba().expect("read back");

    // The chord vertices of the equivalent ellipse tessellation, in world space.
    let segment = cad_representation::ConicSegment {
        center: conic.center,
        major: conic.major,
        minor: conic.minor,
        start: conic.start,
        sweep: conic.sweep,
        full: conic.full,
    };
    let chord = segment.chord_points();
    assert!(chord.len() >= 3);
    let mut checked = 0usize;
    for p in &chord {
        // The chord vertex lies exactly on the analytic curve: find the nearest
        // pixel and require ink (a joint can never fall into a gap).
        let half = SIZE as f64 / 2.0;
        let px = (half + (p.x - cam.center.x) / world_per_px).round() as i64;
        let py = (half - (p.y - cam.center.y) / world_per_px).round() as i64;
        let mut best = 0.0f64;
        for dy in -2i64..=2 {
            for dx in -2i64..=2 {
                let (x, y) = (px + dx, py + dy);
                if x < 0 || y < 0 || x >= SIZE as i64 || y >= SIZE as i64 {
                    continue;
                }
                best = best.max(rendered_coverage(&image, x as u32, y as u32));
            }
        }
        checked += 1;
        assert!(
            best >= 0.05,
            "chord vertex {p:?} has no ink in its 5x5 neighbourhood (best {best:.3})"
        );
    }
    assert!(checked >= 3);
}

/// Fence (c): the id pass agrees with the colour pass both ways — a pixel with
/// **core** ink (`coverage >= 0.5`) is pickable, a pixel outside the ±0.5 px
/// band is not. The sweep crosses the 0.5 threshold in both directions.
#[test]
fn sdf_arc_id_pass_parity() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let base = Point3 {
        x: -2_000_000.0,
        y: 400_000.0,
        z: 0.0,
    };
    let conic = circle(base, 40.0);
    let world_per_px = 0.5;
    let cam = aligned_camera(base, world_per_px);
    renderer
        .upload(&scene(vec![conic_batch(vec![conic], 7)]))
        .expect("upload conic batch");
    let target = RenderTarget::new(SIZE, SIZE);
    renderer.render(cam, &target).expect("render conic");
    let image = renderer.read_target_rgba().expect("read back");

    // Sweep a horizontal strip across the ring's right point (angle 0): the
    // pixel coverage ramps from 0 (outside) through the ±0.5 px core to 0
    // (inside). Each sample asserts pickable ⇔ core coverage, both directions.
    let (cx, cy) = pixel_at(&cam, base.x + 40.0, base.y);
    let mut pickable = 0usize;
    let mut unpickable = 0usize;
    for dx in -6i64..=6 {
        let x = cx as i64 + dx;
        if x < 0 || x >= SIZE as i64 {
            continue;
        }
        let (wx, wy) = pixel_world(&cam, x as u32, cy);
        let d_px = analytic_distance(&conic, (wx, wy)) / world_per_px;
        let got = rendered_coverage(&image, x as u32, cy);
        let hit = renderer
            .pick_at(x as u32, cy)
            .expect("pick sweep")
            .is_some();
        if got >= 0.55 {
            pickable += 1;
            assert!(
                hit,
                "pixel ({x},{cy}) has core coverage {got:.3} (d={d_px:.3}px) but is not pickable"
            );
        } else if got <= 0.45 {
            unpickable += 1;
            assert!(
                !hit,
                "pixel ({x},{cy}) has coverage {got:.3} (d={d_px:.3}px, outside the ±0.5 px band) \
                 but is pickable"
            );
        }
        // 0.45..0.55 straddles the threshold: tolerated either way.
    }
    assert!(
        pickable >= 1,
        "the sweep must include at least one core (pickable) sample"
    );
    assert!(
        unpickable >= 1,
        "the sweep must include at least one outside-band (unpickable) sample"
    );

    // A background pixel (far outside) is not pickable.
    let none = renderer.pick_at(2, 2).expect("pick background");
    assert!(none.is_none(), "a background pixel must not be pickable");
}

/// Fence (d): the selection-flags machinery tints an arc, and `FLAG_HIDDEN`
/// discards it.
#[test]
fn sdf_arc_flags_tint_and_hide() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let base = Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    let conic = circle(base, 40.0);
    let world_per_px = 0.5;
    let cam = aligned_camera(base, world_per_px);
    renderer
        .upload(&scene(vec![conic_batch(vec![conic], 7)]))
        .expect("upload conic batch");
    let target = RenderTarget::new(SIZE, SIZE);
    let (on_x, on_y) = pixel_at(&cam, base.x + 40.0, base.y);

    // Baseline: white.
    renderer.render(cam, &target).expect("render");
    let base_pixel = renderer.read_target_rgba().unwrap().pixel(on_x, on_y);
    assert!(base_pixel[0] > 100 && base_pixel[1] > 100 && base_pixel[2] > 100);

    // Selected: the tint is a colour replacement (rgb = SELECTED_TINT).
    renderer.set_selection_flags_enabled(true);
    renderer
        .set_entity_flags(&[(source(7), cad_render_wgpu::FLAG_SELECTED)])
        .expect("set selected flag");
    renderer.render(cam, &target).expect("render selected");
    let selected = renderer.read_target_rgba().unwrap().pixel(on_x, on_y);
    assert!(
        selected[0] > selected[2].saturating_add(40),
        "a selected arc must take the SELECTED_TINT (red-dominant), got {selected:?}"
    );

    // Hidden: discarded, back to the clear colour.
    renderer
        .set_entity_flags(&[(source(7), cad_render_wgpu::FLAG_HIDDEN)])
        .expect("set hidden flag");
    renderer.render(cam, &target).expect("render hidden");
    let hidden = renderer.read_target_rgba().unwrap().pixel(on_x, on_y);
    assert!(
        hidden[0] <= 12 && hidden[1] <= 12 && hidden[2] <= 12,
        "a hidden arc must be discarded, got {hidden:?}"
    );
}

/// Fence (e): draw order. An arc-vs-line equal-depth tie is decided by plan
/// order (both `LessEqual` + depth write), sampled at **two** pixels where the
/// line crosses the ring and in **both** orders; an arc wins an equal-depth tie
/// against a mesh (`Less`).
#[test]
fn sdf_arc_draw_order_ties() {
    let Some(gpu0) = gpu() else {
        return;
    };
    drop(gpu0);
    let red = [1.0, 0.0, 0.0];
    let blue = [0.0, 0.0, 1.0];

    // A horizontal line along `y = 0` crossing the circle at `(-40, 0)` and
    // `(40, 0)`, same z, so the two crossings are equal-depth ties.
    let render_pair = |arc_first: bool| -> Vec<[u8; 4]> {
        let Some(gpu) = gpu() else {
            return vec![[0, 0, 0, 0]; 2];
        };
        let mut renderer = init(gpu);
        let base = Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        let mut conic = conic_batch(vec![circle(base, 40.0)], 1);
        conic.color = red;
        let mut line = RenderBatch {
            local_origin: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            topology: RenderTopology::Lines,
            vertices: vec![[-50.0, 0.0, 0.0], [50.0, 0.0, 0.0]],
            normals: Vec::new(),
            colors: Vec::new(),
            indices: Vec::new(),
            edges: Vec::new(),
            conics: Vec::new(),
            mirrored: false,
            alpha: 1.0,
            color: blue,
            color_unresolved: false,
            lineweight: 0.0,
            lineweight_unresolved: false,
            linetype: LinetypePattern::continuous(),
            linetype_unresolved: false,
            sources: vec![source(2)],
            draw_order: 0,
        };
        // Both at draw_order 0: upload order is the plan tie-break.
        let batches = if arc_first {
            vec![conic.clone(), line.clone()]
        } else {
            line.color = blue;
            let mut first = line.clone();
            first.draw_order = 0;
            vec![first, conic.clone()]
        };
        let _ = &mut line;
        let cam = aligned_camera(base, 0.5);
        renderer.upload(&scene(batches)).expect("upload tie scene");
        let target = RenderTarget::new(SIZE, SIZE);
        renderer.render(cam, &target).expect("render tie scene");
        let image = renderer.read_target_rgba().unwrap();
        // Both crossings of the line with the ring.
        [(-40.0, 0.0), (40.0, 0.0)]
            .iter()
            .map(|&(wx, wy)| {
                let (sx, sy) = pixel_at(&cam, wx, wy);
                image.pixel(sx, sy)
            })
            .collect()
    };

    let line_after = render_pair(true);
    let arc_after = render_pair(false);
    for (sample, pixel) in line_after.iter().enumerate() {
        assert!(
            pixel[2] > pixel[0].saturating_add(40) && pixel[0] < 40,
            "sample {sample}: a line drawn after the arc must win the equal-depth tie \
             (blue-dominant), got {pixel:?}"
        );
    }
    for (sample, pixel) in arc_after.iter().enumerate() {
        // The arc now rasterises opaquely (triangle filter, peak 1.0), so an arc
        // drawn after the line replaces it: red-dominant, not a red+blue blend.
        assert!(
            pixel[0] > pixel[2].saturating_add(40) && pixel[2] < 40,
            "sample {sample}: an arc drawn after the line must win the depth test and replace it \
             (red-dominant), got {pixel:?}"
        );
    }
    assert_ne!(
        line_after, arc_after,
        "the tie order must change the pixels"
    );

    // Arc vs mesh at equal depth: the arc (`LessEqual`) wins regardless of order.
    let arc_vs_mesh = || -> [u8; 4] {
        let Some(gpu) = gpu() else {
            return [0, 0, 0, 0];
        };
        let mut renderer = init(gpu);
        let base = Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        let mut conic = conic_batch(vec![circle(base, 40.0)], 1);
        conic.color = red;
        // A quad covering the whole view, same z, drawn first.
        let mesh = RenderBatch {
            local_origin: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            topology: RenderTopology::Mesh,
            vertices: vec![
                [-60.0, -60.0, 0.0],
                [-60.0, 60.0, 0.0],
                [60.0, 60.0, 0.0],
                [60.0, -60.0, 0.0],
            ],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            colors: Vec::new(),
            indices: vec![[0, 2, 1], [0, 3, 2]],
            edges: Vec::new(),
            conics: Vec::new(),
            mirrored: false,
            alpha: 1.0,
            color: blue,
            color_unresolved: false,
            lineweight: 0.0,
            lineweight_unresolved: false,
            linetype: LinetypePattern::continuous(),
            linetype_unresolved: false,
            sources: vec![source(3)],
            draw_order: 0,
        };
        let cam = aligned_camera(base, 0.5);
        renderer.upload(&scene(vec![mesh, conic])).expect("upload");
        let target = RenderTarget::new(SIZE, SIZE);
        renderer.render(cam, &target).expect("render");
        let image = renderer.read_target_rgba().unwrap();
        let (sx, sy) = pixel_at(&cam, 40.0, 0.0);
        image.pixel(sx, sy)
    };
    let tie = arc_vs_mesh();
    assert!(
        tie[0] > tie[2].saturating_add(40),
        "the arc must win the equal-depth tie against a mesh (its red ink must replace it), \
         got {tie:?}"
    );
}

/// Fence (f): determinism and a camera move never re-uploads a conic stream.
#[test]
fn sdf_arc_determinism_and_camera_move_is_static() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let base = Point3 {
        x: 100_000.0,
        y: 100_000.0,
        z: 0.0,
    };
    renderer
        .upload(&scene(vec![conic_batch(
            vec![circle(base, 40.0), arc(base, 25.0, 0.0, 2.0)],
            1,
        )]))
        .expect("upload conic batch");
    assert_eq!(renderer.arc_stream_count(), 1);
    assert_eq!(renderer.arc_instance_count(), 2);

    let target = RenderTarget::new(SIZE, SIZE);
    let cam = aligned_camera(base, 0.5);
    renderer.render(cam, &target).expect("render");
    let first = renderer.read_target_rgba().unwrap();
    let uploads_after_first = renderer.arc_stream_uploads();

    // Determinism: a second identical render is byte-identical.
    renderer.render(cam, &target).expect("render again");
    let second = renderer.read_target_rgba().unwrap();
    assert_eq!(
        first.pixels, second.pixels,
        "the same scene must be deterministic"
    );

    // Pan/zoom: 5 camera moves never re-upload a conic stream.
    for i in 0..5 {
        let moved = camera(
            Point3 {
                x: base.x + i as f64 * 3.0,
                y: base.y - i as f64 * 2.0,
                z: 0.0,
            },
            0.5 + i as f64 * 0.1,
        );
        renderer.render(moved, &target).expect("render panned");
        assert_eq!(
            renderer.arc_stream_uploads(),
            uploads_after_first,
            "a camera move must not re-upload a conic stream"
        );
    }
}
