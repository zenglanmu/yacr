//! SDF conic (analytic circle/arc) fences — real GPU, slice B.
//!
//! These are **real-GPU** integration tests (the same headless path as
//! `headless_render.rs`: a real wgpu device on whatever adapter is visible, the
//! Quadro P620 / Vulkan on the project machine). They assert the SDF raster
//! against the **f64 analytic curve** the conic represents, so a wrong
//! parameterization, a missing `discard`, or a bad precision reduction fails.
//! See `docs/adr/0008-sdf-conic-rendering.md`.

use cad_domain::{DocumentId, EntityId, InstancePath, Point3, SelectionRef, TaskStamp};
use cad_render_wgpu::headless::{create_headless_gpu, HeadlessGpu, RgbaImage};
use cad_render_wgpu::{BackendPreference, Camera2d, RenderTarget, Renderer};
use cad_scene::{LinetypePattern, RenderBatch, RenderConic, RenderTopology, SceneDelta};
use std::f64::consts::TAU;

const SIZE: u32 = 256;

fn source(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn conic_batch(conics: Vec<RenderConic>, entity: u128) -> RenderBatch {
    let origin = conics.first().map(|c| c.center).unwrap_or(Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    });
    RenderBatch {
        local_origin: origin,
        topology: RenderTopology::Conics,
        vertices: Vec::new(),
        normals: Vec::new(),
        colors: Vec::new(),
        indices: Vec::new(),
        edges: Vec::new(),
        conics,
        mirrored: false,
        alpha: 1.0,
        // White on the black clear: the encoded channel is a direct coverage read.
        color: [1.0, 1.0, 1.0],
        color_unresolved: false,
        lineweight: 0.0,
        lineweight_unresolved: false,
        linetype: LinetypePattern::continuous(),
        linetype_unresolved: false,
        sources: vec![source(entity)],
        draw_order: 0,
    }
}

fn circle(center: Point3, radius: f64) -> RenderConic {
    RenderConic {
        center,
        major: Point3 {
            x: radius,
            y: 0.0,
            z: 0.0,
        },
        minor: Point3 {
            x: 0.0,
            y: radius,
            z: 0.0,
        },
        start: 0.0,
        sweep: TAU,
        full: true,
    }
}

fn arc(center: Point3, radius: f64, start: f64, sweep: f64) -> RenderConic {
    RenderConic {
        center,
        major: Point3 {
            x: radius,
            y: 0.0,
            z: 0.0,
        },
        minor: Point3 {
            x: 0.0,
            y: radius,
            z: 0.0,
        },
        start,
        sweep,
        full: false,
    }
}

fn gpu() -> Option<HeadlessGpu> {
    match create_headless_gpu(BackendPreference::Auto) {
        Ok(gpu) => Some(gpu),
        Err(error) => {
            eprintln!("skipping SDF arc GPU test: {error}");
            None
        }
    }
}

/// The universal per-run tier: arcs are universal-only this round, so the tier
/// override makes the run deterministic regardless of the device.
fn init(gpu: HeadlessGpu) -> Renderer {
    let HeadlessGpu {
        device,
        queue,
        adapter: _,
    } = gpu;
    let mut renderer = Renderer::new(BackendPreference::Auto);
    renderer.set_storage_buffer_support_override(Some(false));
    renderer
        .initialize_with_device(device, queue)
        .expect("initialize headless renderer");
    renderer
}

fn camera(center: Point3, world_per_px: f64) -> Camera2d {
    Camera2d {
        center,
        world_per_px,
        z_plane: 0.0,
    }
}

/// A camera whose `center` maps to a **pixel centre** (not a pixel corner), so a
/// conic centred there has its axis points on pixel centres and the SDF's peak
/// coverage is observable. Without this half-pixel shift the ring can fall on
/// pixel corners and no pixel centre is closer than ~0.5 px.
fn aligned_camera(center: Point3, world_per_px: f64) -> Camera2d {
    camera(
        Point3 {
            x: center.x + 0.5 * world_per_px,
            y: center.y + 0.5 * world_per_px,
            z: center.z,
        },
        world_per_px,
    )
}

fn scene(batches: Vec<RenderBatch>) -> SceneDelta {
    SceneDelta {
        stamp: TaskStamp::new(DocumentId(1), 0),
        added: batches,
        removed_chunks: Vec::new(),
        images: Vec::new(),
    }
}

/// World position of a pixel **center** for `camera` at the target size.
fn pixel_world(camera: &Camera2d, x: u32, y: u32) -> (f64, f64) {
    let half = SIZE as f64 / 2.0;
    let wx = camera.center.x + (x as f64 + 0.5 - half) * camera.world_per_px;
    let wy = camera.center.y - (y as f64 + 0.5 - half) * camera.world_per_px;
    (wx, wy)
}

/// The pixel whose **centre** is nearest the world point `(wx, wy)`.
fn pixel_at(camera: &Camera2d, wx: f64, wy: f64) -> (u32, u32) {
    let half = SIZE as f64 / 2.0;
    let x = half + (wx - camera.center.x) / camera.world_per_px - 0.5;
    let y = half - (wy - camera.center.y) / camera.world_per_px - 0.5;
    (
        x.round().clamp(0.0, (SIZE - 1) as f64) as u32,
        y.round().clamp(0.0, (SIZE - 1) as f64) as u32,
    )
}

/// The analytic signed distance (world units) from `p` to `conic`'s drawn set,
/// mirroring `shaders/arc.wgsl` exactly (the f64 reference the SDF is fenced to).
fn analytic_distance(conic: &RenderConic, p: (f64, f64)) -> f64 {
    let major_len = (conic.major.x * conic.major.x
        + conic.major.y * conic.major.y
        + conic.major.z * conic.major.z)
        .sqrt();
    let minor_len = (conic.minor.x * conic.minor.x
        + conic.minor.y * conic.minor.y
        + conic.minor.z * conic.minor.z)
        .sqrt();
    let u = (conic.major.x / major_len, conic.major.y / major_len);
    let v = (conic.minor.x / minor_len, conic.minor.y / minor_len);
    let q = (p.0 - conic.center.x, p.1 - conic.center.y);
    let x = q.0 * u.0 + q.1 * u.1;
    let y = q.0 * v.0 + q.1 * v.1;
    let d_ring = (x * x + y * y).sqrt() - major_len;
    let full = conic.full;
    if full {
        return d_ring.abs();
    }
    let t = y.atan2(x);
    let rel = (t - conic.start).rem_euclid(TAU);
    let inside = if conic.sweep >= 0.0 {
        rel <= conic.sweep
    } else {
        TAU - rel <= -conic.sweep
    };
    if inside {
        return d_ring.abs();
    }
    let e0 = (major_len * conic.start.cos(), major_len * conic.start.sin());
    let e1 = (
        major_len * (conic.start + conic.sweep).cos(),
        major_len * (conic.start + conic.sweep).sin(),
    );
    let d0 = ((x - e0.0).powi(2) + (y - e0.1).powi(2)).sqrt();
    let d1 = ((x - e1.0).powi(2) + (y - e1.1).powi(2)).sqrt();
    d0.min(d1)
}

/// Decode an sRGB8 channel back to linear coverage in `[0, 1]`.
fn coverage(channel: u8) -> f64 {
    let c = channel as f64 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// The rendered coverage at a pixel (the red channel; the batch is white).
fn rendered_coverage(image: &RgbaImage, x: u32, y: u32) -> f64 {
    coverage(image.pixel(x, y)[0])
}

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

/// Straight polyline segments as a `Lines` batch (one segment per vertex pair),
/// relative to `origin`.
fn straight_lines(origin: Point3, segments: Vec<[f64; 3]>, entity: u128) -> RenderBatch {
    let mut vertices = Vec::new();
    for pair in segments.chunks_exact(2) {
        for p in pair {
            vertices.push([
                (p[0] - origin.x) as f32,
                (p[1] - origin.y) as f32,
                (p[2] - origin.z) as f32,
            ]);
        }
    }
    RenderBatch {
        local_origin: origin,
        topology: RenderTopology::Lines,
        vertices,
        normals: Vec::new(),
        colors: Vec::new(),
        indices: Vec::new(),
        edges: Vec::new(),
        conics: Vec::new(),
        mirrored: false,
        alpha: 1.0,
        color: [1.0, 1.0, 1.0],
        color_unresolved: false,
        lineweight: 0.0,
        lineweight_unresolved: false,
        linetype: LinetypePattern::continuous(),
        linetype_unresolved: false,
        sources: vec![source(entity)],
        draw_order: 0,
    }
}

/// The peak rendered coverage inside a pixel band (`x_lo..=x_hi`), used to
/// isolate one class's core intensity when several classes share a frame.
fn band_peak(image: &RgbaImage, x_lo: u32, x_hi: u32) -> f64 {
    let mut peak = 0.0f64;
    for y in 0..SIZE {
        for x in x_lo..=x_hi.min(SIZE - 1) {
            peak = peak.max(rendered_coverage(image, x, y));
        }
    }
    peak
}

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

/// The same analytic circles/arcs chord-expanded into a `LineList` batch — the
/// pre-SDF draw, used as the visual-acceptance "before".
fn chord_lines_batch(conics: &[RenderConic], entity: u128) -> RenderBatch {
    let origin = conics.first().map(|c| c.center).unwrap_or(Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    });
    let mut vertices = Vec::new();
    for conic in conics {
        let segment = cad_representation::ConicSegment {
            center: conic.center,
            major: conic.major,
            minor: conic.minor,
            start: conic.start,
            sweep: conic.sweep,
            full: conic.full,
        };
        let points = segment.chord_points();
        for pair in points.windows(2) {
            for p in pair {
                vertices.push([
                    (p.x - origin.x) as f32,
                    (p.y - origin.y) as f32,
                    (p.z - origin.z) as f32,
                ]);
            }
        }
    }
    RenderBatch {
        local_origin: origin,
        topology: RenderTopology::Lines,
        vertices,
        normals: Vec::new(),
        colors: Vec::new(),
        indices: Vec::new(),
        edges: Vec::new(),
        conics: Vec::new(),
        mirrored: false,
        alpha: 1.0,
        color: [1.0, 1.0, 1.0],
        color_unresolved: false,
        lineweight: 0.0,
        lineweight_unresolved: false,
        linetype: LinetypePattern::continuous(),
        linetype_unresolved: false,
        sources: vec![source(entity)],
        draw_order: 0,
    }
}

/// Fence (h): visual acceptance. Render a **mixed arc + line real drawing** twice
/// — once with the conics chord-expanded (`before`, the pre-SDF draw) and once as
/// SDF instances (`after`) — into a NEW output directory. The straight lines are
/// identical in both frames, so the SDF ring can be compared against them. The
/// SDF frame must be continuous (no gaps), reach full core opacity, and carry a
/// weight consistent with the lines; there is **no** byte-identity claim with
/// chords. This is the sanctioned semantic-change evidence
/// (`docs/adr/0008-sdf-conic-rendering.md`).
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
        .unwrap_or_else(|_| "/tmp/opencode/yacr-sdf-arcs-v2".to_string());
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

    // Smoother: the SDF's analytic AA produces at least as many distinct
    // intermediate grey levels as the chord draw.
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
        levels(&after) >= levels(&before),
        "the SDF should be at least as anti-aliased as the chord draw"
    );
}
