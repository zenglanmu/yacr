use super::*;

#[test]
fn progressive_pages_match_full_scene_without_raising_per_frame_budget() {
    let Some(gpu) = gpu() else {
        return;
    };
    eprintln!("progressive contract adapter: {:?}", gpu.adapter);
    let mut renderer = init(gpu);
    let mut batches: Vec<_> = rectangle_lines()
        .chunks_exact(2)
        .map(|points| lines_batch(points.to_vec()))
        .collect();
    let mut transparent = triangle_mesh();
    transparent.alpha = 0.5;
    transparent.draw_order = -10;
    batches.push(transparent);
    batches.push(triangle_mesh());
    let delta = scene(batches);
    let mut staged = renderer.prepare_upload_batches(&delta.added[..2]).unwrap();
    staged
        .append(renderer.prepare_upload_batches(&delta.added[2..]).unwrap())
        .unwrap();
    assert_eq!(
        renderer.batch_count(),
        0,
        "staging must not publish a prefix"
    );
    renderer.commit_upload(staged, 0).unwrap();
    let target = RenderTarget::new(64, 64);
    renderer.render(camera_2d(), &target).unwrap();
    let reference = renderer.read_target_rgba().unwrap();
    // Anti-aliased line strokes charge 6 vertices + 2 triangles per segment and a
    // triangle mesh charges 3 vertices + 1 triangle; a `6`/`2` budget fits exactly
    // one batch per page.
    renderer.frame_budget.max_vertices = 6;
    renderer.frame_budget.max_triangles = 2;
    renderer.set_progressive_rendering(true);
    // Drive the controller hard down: the paced page then cannot fit a 6-vertex
    // batch, so the progress floor must keep every page advancing while the
    // settled frame stays byte-identical to the single un-paced full scene.
    renderer.set_frame_budget_target_ms(0.001);
    let mut pages = 0;
    let mut saw_paced_page = false;
    let mut saw_floor = false;
    loop {
        let stats = renderer.render(camera_2d(), &target).unwrap();
        assert!(stats.vertices <= 6);
        assert!(stats.triangles <= 2);
        assert_eq!(
            stats.pacing, "active",
            "progressive native pacing is active"
        );
        if stats.budget_scale < 1.0 {
            saw_paced_page = true;
            assert_eq!(stats.budget_target_ms, 0.001);
        }
        if stats
            .over_budget
            .as_ref()
            .is_some_and(|over| over.report.category == "pacing-floor")
        {
            saw_floor = true;
        }
        pages += 1;
        assert!(pages <= delta.added.len());
        if !renderer.frame_pending() {
            break;
        }
    }
    assert!(pages > 1);
    assert!(
        saw_paced_page,
        "the controller must have scaled the page budget down for this trace"
    );
    assert!(
        saw_floor,
        "a paced page smaller than one batch must report the progress floor"
    );
    assert_eq!(
        renderer.frame_progress(),
        Some((delta.added.len(), delta.added.len()))
    );
    assert_eq!(
        renderer.read_target_rgba().unwrap().pixels,
        reference.pixels
    );
    let mut camera = camera_2d();
    camera.center.x = 0.1;
    renderer.render(camera, &target).unwrap();
    assert!(
        renderer.frame_pending(),
        "navigation must restart accumulation, not retain stale pixels"
    );
    renderer.clear_batches();
    assert!(!renderer.frame_pending());
}

#[test]
fn progressive_oversized_batch_is_an_explicit_error_not_empty_completion() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    renderer
        .upload(&scene(vec![lines_batch(rectangle_lines())]))
        .unwrap();
    renderer.frame_budget.max_vertices = 1;
    renderer.set_progressive_rendering(true);
    assert!(renderer
        .render(camera_2d(), &RenderTarget::new(64, 64))
        .is_err());
    assert_eq!(renderer.frame_progress(), Some((0, 1)));
}

/// The paced page budget must follow the per-frame ceiling and the cursor must
/// advance by exactly the accepted count — no skip, no duplicate.
#[test]
fn paced_budget_is_adjustable_per_frame_without_skipping_or_duplicating() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let batches = line_batches(8);
    let delta = scene(batches);
    renderer.upload(&delta).expect("upload eight line batches");
    let target = RenderTarget::new(64, 64);
    renderer.render(camera_2d(), &target).unwrap();
    let reference = renderer.read_target_rgba().unwrap();

    renderer.set_progressive_rendering(true);
    // A huge target disables adaptation, so the *ceiling* is the budget and we
    // control it directly frame by frame.
    renderer.set_frame_budget_target_ms(1.0e9);

    let alternate = [12usize, 18];
    let mut accepted_sequence = Vec::new();
    let mut previous_cursor = 0usize;
    let mut frames = 0;
    loop {
        renderer.frame_budget.max_vertices = alternate[frames % alternate.len()];
        let stats = renderer.render(camera_2d(), &target).unwrap();
        let (cursor, total) = renderer.frame_progress().expect("progressive frame");
        assert_eq!(total, 8);
        assert!(
            cursor > previous_cursor,
            "the cursor must advance on every page, {previous_cursor} -> {cursor}"
        );
        assert_eq!(
            cursor - previous_cursor,
            stats.opaque_batches,
            "the cursor must advance by exactly the accepted batch count"
        );
        accepted_sequence.push(stats.opaque_batches);
        previous_cursor = cursor;
        frames += 1;
        assert!(frames <= 8, "paged frame did not terminate");
        if !renderer.frame_pending() {
            break;
        }
    }
    assert_eq!(
        accepted_sequence,
        vec![2, 3, 2, 1],
        "each page must accept exactly what its budget allows"
    );
    assert_eq!(renderer.frame_progress(), Some((8, 8)));
    assert_eq!(
        renderer.read_target_rgba().unwrap().pixels,
        reference.pixels,
        "no skip and no duplicate: the settled frame equals the full scene"
    );
}

/// A pick reads the un-paced ceiling, so shrinking the *active* page budget must
/// not change a pick result.
#[test]
fn pick_is_invariant_under_a_tiny_active_page_budget() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    // The pickable quad is uploaded first (renderer key 1) and drawn last
    // (highest draw order), so both the full-ceiling reference pick and the tiny
    // ceiling pick resolve the centre to it. It charges 4 vertices, more than a
    // scaled 6-vertex page can hold once pacing has stepped down.
    let mut pickable = quad_mesh();
    pickable.draw_order = 100;
    let mut batches = vec![pickable];
    batches.extend(line_batches(6));
    renderer
        .upload(&scene(batches))
        .expect("upload pickable scene");
    let target = RenderTarget::new(64, 64);
    renderer.render(camera_2d(), &target).unwrap();
    let reference_pick = renderer.pick_at(32, 32).expect("reference pick");
    assert!(reference_pick.is_some(), "the centre must be pickable");

    renderer.frame_budget.max_vertices = 6;
    renderer.frame_budget.max_triangles = 2;
    renderer.set_progressive_rendering(true);
    renderer.set_frame_budget_target_ms(0.001);
    let mut frames = 0;
    loop {
        renderer.render(camera_2d(), &target).unwrap();
        frames += 1;
        assert!(frames <= 16, "paced pick scene did not terminate");
        if !renderer.frame_pending() {
            break;
        }
    }
    assert!(
        renderer.pacing_scale() < 1.0,
        "the trace must have shrunk the active page budget, got {}",
        renderer.pacing_scale()
    );
    let paced_pick = renderer.pick_at(32, 32).expect("paced pick");
    assert_eq!(
        paced_pick, reference_pick,
        "picking must be independent of the paced page budget"
    );
}

/// The progress floor: batches that fit the un-paced ceiling must never produce
/// the "one GPU batch exceeds..." error, and the cursor must strictly advance.
#[test]
fn progress_floor_keeps_the_cursor_monotone_for_ceiling_fitting_batches() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    // One batch per un-paced page (6 vertices), so a scaled page cannot fit a
    // batch once the controller has stepped down.
    renderer.frame_budget.max_vertices = 6;
    renderer.frame_budget.max_triangles = 2;
    renderer
        .upload(&scene(line_batches(8)))
        .expect("upload eight line batches");
    renderer.set_progressive_rendering(true);
    renderer.set_frame_budget_target_ms(0.001);

    let target = RenderTarget::new(64, 64);
    let mut previous_cursor = 0usize;
    let mut saw_floor = false;
    let mut pages = 0;
    loop {
        let stats = renderer
            .render(camera_2d(), &target)
            .expect("a batch that fits the ceiling must not be an error");
        let (cursor, total) = renderer.frame_progress().expect("progressive frame");
        assert!(cursor > previous_cursor, "cursor must strictly advance");
        assert!(total == 8);
        if stats
            .over_budget
            .as_ref()
            .is_some_and(|over| over.report.category == "pacing-floor")
        {
            saw_floor = true;
        }
        previous_cursor = cursor;
        pages += 1;
        assert!(pages <= 8, "paced frame did not terminate");
        if !renderer.frame_pending() {
            break;
        }
    }
    assert_eq!(previous_cursor, 8);
    assert!(
        saw_floor,
        "a paced page smaller than one batch must take the progress floor"
    );
}

/// An image-heavy scene must keep its images in the settled frame under small
/// paced pages: the full image charge is reserved on the cleared page before
/// geometry fills the remainder.
#[test]
fn paced_pages_keep_images_in_the_settled_frame() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    // Five 30-segment line batches (180 vertices each), drawn outside the image
    // box so the image centre stays assertable. A 183-vertex ceiling fits one
    // batch; adding the image's 4 vertices pushes a geometry-first plan over it,
    // which is exactly the latent drop this change fixes.
    let big_line = || {
        let mut vertices = Vec::new();
        for i in 0..30 {
            let x = 0.85 + i as f32 * 0.001;
            vertices.push([x, -0.5, 0.0]);
            vertices.push([x, 0.5, 0.0]);
        }
        lines_batch(vertices)
    };
    let batches: Vec<RenderBatch> = (0..5).map(|_| big_line()).collect();
    renderer
        .upload(&scene(batches))
        .expect("upload line batches");
    let mut cache = DecodedImageCache::new();
    cache
        .insert(
            ResourceKey("red".into()),
            solid_image(2, 2, [255, 0, 0, 255]),
            &ResourceLimits::default(),
        )
        .expect("insert red image");
    renderer
        .upload_images(&[image_batch("red")], &cache)
        .expect("upload red image");

    renderer.frame_budget.max_vertices = 183;
    renderer.set_progressive_rendering(true);
    renderer.set_frame_budget_target_ms(0.001);

    let target = RenderTarget::new(64, 64);
    let mut image_draws = 0usize;
    let mut saw_paced_page = false;
    let mut pages = 0;
    loop {
        let stats = renderer.render(camera_2d(), &target).unwrap();
        image_draws += stats.image_batches;
        if stats.budget_scale < 1.0 {
            saw_paced_page = true;
        }
        pages += 1;
        assert!(pages <= 16, "paced image scene did not terminate");
        if !renderer.frame_pending() {
            break;
        }
    }
    assert!(saw_paced_page, "the trace must exercise a paced page");
    assert_eq!(
        image_draws, 1,
        "the resident image must be drawn exactly once, on the cleared page"
    );
    let frame = renderer.read_target_rgba().expect("read settled frame");
    let centre = frame.pixel(32, 32);
    assert!(
        centre[0] > 240 && centre[1] < 15 && centre[2] < 15,
        "the image centre must be opaque red in the settled frame, got {centre:?}"
    );
}

/// With no completion signal the pacing is disabled by state: the static ceiling
/// is used and no cost knowledge is accumulated.
#[test]
fn disabled_pacing_uses_the_static_default_budget() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    renderer
        .upload(&scene(line_batches(4)))
        .expect("upload line batches");
    renderer.frame_budget.max_vertices = 6;
    renderer.frame_budget.max_triangles = 2;
    renderer.set_progressive_rendering(true);
    // The wasm non-blocking poll has no completion signal; force that decision.
    renderer.set_pacing_signal_override(Some(false));
    renderer.set_frame_budget_target_ms(0.001);

    let target = RenderTarget::new(64, 64);
    let mut pages = 0;
    loop {
        let stats = renderer.render(camera_2d(), &target).unwrap();
        assert_eq!(stats.pacing, "disabled-no-completion-signal");
        assert_eq!(stats.budget_scale, 1.0, "the static budget is not scaled");
        assert_eq!(
            stats.budget_vertices, 6,
            "the static vertex ceiling is used"
        );
        assert_eq!(
            stats.budget_triangles, 2,
            "the static triangle ceiling is used"
        );
        assert_eq!(
            stats.budget_cost_ema_ms, 0.0,
            "a disabled controller must not accumulate cost knowledge"
        );
        pages += 1;
        assert!(pages <= 4, "paced frame did not terminate");
        if !renderer.frame_pending() {
            break;
        }
    }
    assert!(pages > 1);
    assert_eq!(renderer.pacing_scale(), 1.0);
    assert_eq!(renderer.pacing_cost_ema_ms(), 0.0);
}

/// A non-progressive frame is planned against the un-paced ceiling and says so:
/// there is no page to pace.
#[test]
fn non_progressive_frames_report_the_un_paced_ceiling() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    renderer
        .upload(&scene(line_batches(3)))
        .expect("upload line batches");
    let stats = renderer
        .render(camera_2d(), &RenderTarget::new(64, 64))
        .unwrap();
    assert_eq!(stats.pacing, "at-ceiling");
    assert_eq!(stats.budget_scale, 1.0);
    assert_eq!(stats.budget_vertices, renderer.frame_budget.max_vertices);
    assert_eq!(stats.budget_triangles, renderer.frame_budget.max_triangles);
}

/// A **base-changing** scene mutation / `clear_batches` drop the scene-specific
/// cost knowledge; a camera move does not (covered by the parity test), and
/// neither does a suffix-only overlay commit (see `tests/pacing_commit_suffix.rs`).
#[test]
fn scene_mutation_resets_pacing_knowledge() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    renderer.frame_budget.max_vertices = 6;
    renderer.frame_budget.max_triangles = 2;
    renderer
        .upload(&scene(line_batches(8)))
        .expect("upload line batches");
    renderer.set_progressive_rendering(true);
    renderer.set_frame_budget_target_ms(0.001);

    let target = RenderTarget::new(64, 64);
    let mut frames = 0;
    while renderer.pacing_scale() >= 1.0 && frames < 40 {
        renderer.render(camera_2d(), &target).unwrap();
        frames += 1;
    }
    assert!(
        renderer.pacing_scale() < 1.0 && renderer.pacing_cost_ema_ms() > 0.0,
        "the controller must have adapted before the reset is meaningful"
    );

    // A **base-changing** scene mutation (a commit with `keep_prefix == 0`)
    // forgets the cost knowledge immediately. A suffix-only commit does **not**
    // reset it — that is the separate `pacing_commit_suffix` contract.
    let staged = renderer
        .prepare_upload_batches(&line_batches(8))
        .expect("stage a base replacement");
    renderer
        .commit_upload(staged, 0)
        .expect("mutate the base scene");
    assert_eq!(renderer.pacing_scale(), 1.0);
    assert_eq!(renderer.pacing_cost_ema_ms(), 0.0);
    let stats = renderer.render(camera_2d(), &target).unwrap();
    assert_eq!(stats.pacing, "active");
    assert_eq!(
        stats.budget_scale, 1.0,
        "the first post-mutation frame is un-paced"
    );
    assert_eq!(
        stats.budget_cost_ema_ms, 0.0,
        "the seed frame has no prior EMA"
    );

    // Adapt again, then clear: `clear_batches` is also a reset.
    let mut frames = 0;
    while renderer.pacing_scale() >= 1.0 && frames < 40 {
        renderer.render(camera_2d(), &target).unwrap();
        frames += 1;
    }
    assert!(renderer.pacing_scale() < 1.0);
    renderer.clear_batches();
    assert_eq!(renderer.pacing_scale(), 1.0);
    assert_eq!(renderer.pacing_cost_ema_ms(), 0.0);
}

#[test]
fn headless_adapter_is_reported_as_software_vulkan() {
    let Some(gpu) = gpu() else {
        return;
    };
    assert!(
        !gpu.adapter.backend.is_empty() && !gpu.adapter.name.is_empty(),
        "adapter must report backend and name: {:?}",
        gpu.adapter
    );
    // Forcing the lavapipe ICD must yield the Vulkan backend on a CPU device.
    if std::env::var("VK_ICD_FILENAMES")
        .map(|v| v.contains("lvp"))
        .unwrap_or(false)
    {
        assert_eq!(gpu.adapter.backend, "vulkan");
        assert_eq!(gpu.adapter.device_type, "cpu");
        assert_eq!(gpu.adapter.driver, "llvmpipe");
    }
}

#[test]
fn lines_render_onto_target() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    renderer
        .upload(&scene(vec![lines_batch(rectangle_lines())]))
        .expect("upload line batch");

    let target = RenderTarget::new(64, 64);
    let stats = renderer
        .render(camera_2d(), &target)
        .expect("render line frame");
    assert!(stats.draw_calls > 0, "frame submitted no draw calls");
    // Anti-aliased line strokes charge 6 vertices (two triangles) per 2-vertex
    // segment: the rectangle outline is four segments.
    assert_eq!(stats.vertices, 6 * (rectangle_lines().len() / 2));

    let image = renderer.read_target_rgba().expect("read back frame");
    assert_eq!((image.width, image.height), (64, 64));
    assert_eq!(image.pixels.len(), 64 * 64 * 4);
    let background = image.pixel(0, 0);
    let differing = image.count_differing_from(background, 8);
    assert!(
        differing > 0,
        "line geometry rasterized nothing (background {background:?})"
    );
    assert!(
        differing < 64 * 64,
        "the whole target differs from the corner; camera fit is wrong"
    );
    // The center of the rectangle outline is empty, so the exact middle pixel
    // must remain background — a sanity check that we are not painting a blob.
    let center = image.pixel(32, 32);
    assert_eq!(
        center, background,
        "rectangle interior should be background, got {center:?}"
    );
}

#[test]
fn positive_world_y_renders_above_center_after_pan() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    // Asymmetric geometry, nonzero camera and batch origins: symmetric smoke
    // scenes cannot detect an upside-down projection or a translation sign bug.
    let mut batch = lines_batch(vec![[-0.5, 0.5, 0.0], [0.5, 0.5, 0.0]]);
    batch.local_origin = Point3 {
        x: 10.0,
        y: 20.0,
        z: 0.0,
    };
    renderer.upload(&scene(vec![batch])).unwrap();
    let mut camera = camera_2d();
    camera.center = Point3 {
        x: 10.0,
        y: 20.0,
        z: 0.0,
    };
    let target = RenderTarget::new(64, 64);
    renderer.render(camera, &target).unwrap();
    let image = renderer.read_target_rgba().unwrap();
    let background = image.pixel(0, 0);
    let mut rows = Vec::new();
    for y in 0..64 {
        for x in 0..64 {
            if image.pixel(x, y) != background {
                rows.push(y);
            }
        }
    }
    assert!(!rows.is_empty());
    assert!(rows.iter().all(|y| (15..=16).contains(y)), "rows: {rows:?}");
    // Pan upward by 0.5 world units: the same line now reaches screen center.
    camera.center.y += 0.5;
    renderer.render(camera, &target).unwrap();
    let image = renderer.read_target_rgba().unwrap();
    assert!((31..=32).any(|y| image.pixel(32, y) != background));
}

#[test]
fn mesh_render_uses_triangle_pipeline() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    renderer
        .upload(&scene(vec![triangle_mesh()]))
        .expect("upload mesh batch");

    let target = RenderTarget::new(64, 64);
    let stats = renderer
        .render(camera_2d(), &target)
        .expect("render mesh frame");
    assert!(stats.triangles >= 1, "mesh batch reported no triangles");
    assert_eq!(stats.opaque_batches, 1);

    let image = renderer.read_target_rgba().expect("read back mesh frame");
    let background = image.pixel(0, 0);
    let differing = image.count_differing_from(background, 8);
    assert!(
        differing > 0,
        "mesh geometry rasterized nothing (background {background:?})"
    );
}

/// A per-vertex gradient must reach the fragment shader: the two ends of a
/// red→blue ramp rasterize as different, channel-dominant colours. This is a
/// real software-Vulkan (lavapipe) frame.
#[test]
fn gradient_mesh_renders_a_visible_ramp() {
    let Some(_gpu) = gpu() else {
        return;
    };
    let (left, right) = render_gradient_probes([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
    assert_ne!(left, right, "gradient endpoints rendered identically");
    assert!(
        left[0] > left[2],
        "left end should be red-dominant, got {left:?}"
    );
    assert!(
        right[2] > right[0],
        "right end should be blue-dominant, got {right:?}"
    );
}

/// Two gradients with the same stops but swapped directions must produce
/// different frames, proving the vertex colours (not just a constant tint)
/// drive the raster.
#[test]
fn gradient_direction_changes_the_frame() {
    let Some(_gpu) = gpu() else {
        return;
    };
    let forward = render_gradient_probes([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
    let reversed = render_gradient_probes([0.0, 0.0, 1.0], [1.0, 0.0, 0.0]);
    assert_ne!(
        forward, reversed,
        "reversing the gradient must change the rendered frame"
    );
    // The first probe swaps dominance.
    assert!(forward.0[0] > forward.0[2] && reversed.0[2] > reversed.0[0]);
}

/// A translucent mesh batch must composite over the background differently from
/// an opaque one, proving `RenderBatch::alpha` reaches the blend state. This is
/// a real GPU test (software Vulkan/lavapipe under `VK_ICD_FILENAMES`), not a
/// plan-level assertion.
#[test]
fn translucent_batch_composites_differently_from_opaque() {
    let opaque_pixel = {
        let Some(gpu) = gpu() else {
            return;
        };
        let mut renderer = init(gpu);
        let mut batch = triangle_mesh();
        batch.alpha = 1.0;
        renderer
            .upload(&scene(vec![batch]))
            .expect("upload opaque mesh");
        let target = RenderTarget::new(64, 64);
        let stats = renderer
            .render(camera_2d(), &target)
            .expect("render opaque mesh");
        assert_eq!(stats.opaque_batches, 1);
        assert_eq!(stats.transparent_batches, 0);
        renderer
            .read_target_rgba()
            .expect("read opaque frame")
            .pixel(32, 32)
    };

    // A fresh renderer (new device) so the opaque batch is not still resident.
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let mut batch = triangle_mesh();
    batch.alpha = 0.5;
    renderer
        .upload(&scene(vec![batch]))
        .expect("upload translucent mesh");
    let target = RenderTarget::new(64, 64);
    let stats = renderer
        .render(camera_2d(), &target)
        .expect("render translucent mesh");
    assert_eq!(
        stats.opaque_batches, 0,
        "0.5 alpha must not be drawn in the opaque pass"
    );
    assert_eq!(stats.transparent_batches, 1);
    let image = renderer.read_target_rgba().expect("read translucent frame");
    let translucent_pixel = image.pixel(32, 32);
    let background = image.pixel(0, 0);

    assert_ne!(
        translucent_pixel, background,
        "the translucent triangle did not rasterize the center pixel"
    );
    assert_ne!(
        opaque_pixel, translucent_pixel,
        "alpha=0.5 must composite differently from an opaque batch (opaque {opaque_pixel:?}, \
         translucent {translucent_pixel:?})"
    );
}

#[test]
fn render_3d_is_not_blank() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    renderer
        .upload(&scene(vec![lines_batch(rectangle_lines())]))
        .expect("upload 3d batch");

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
    let target = RenderTarget::new(64, 64);
    renderer
        .render_3d(camera, &target)
        .expect("render 3d frame");

    let image = renderer.read_target_rgba().expect("read back 3d frame");
    let background = image.pixel(0, 0);
    assert!(
        image.count_differing_from(background, 8) > 0,
        "3D frame is blank (background {background:?})"
    );
}

#[test]
fn png_round_trips_dimensions_and_signature() {
    let image = RgbaImage {
        width: 3,
        height: 2,
        pixels: (0..(3 * 2 * 4)).map(|i| (i * 7) as u8).collect(),
    };
    let png = encode_png(&image).expect("encode png");
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "PNG signature");
    // First chunk is IHDR: 4-byte length, "IHDR", width u32, height u32.
    assert_eq!(&png[12..16], b"IHDR");
    let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
    assert_eq!((width, height), (3, 2));

    let wrong = RgbaImage {
        width: 3,
        height: 2,
        pixels: vec![0; 7],
    };
    assert!(encode_png(&wrong).is_err(), "short buffer must be rejected");
}

#[test]
fn readback_before_render_is_not_initialized() {
    let renderer = Renderer::new(BackendPreference::Auto);
    assert!(matches!(
        renderer.read_target_rgba(),
        Err(RenderError::NotInitialized(_))
    ));
}

/// A resident image must actually rasterize its texture colour. This is a real
/// software-Vulkan (lavapipe) frame, not a plan-level assertion.
#[test]
fn uploaded_image_renders_its_texture_colour() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let mut cache = DecodedImageCache::new();
    cache
        .insert(
            ResourceKey("red".into()),
            solid_image(2, 2, [255, 0, 0, 255]),
            &ResourceLimits::default(),
        )
        .expect("insert red image");

    let report = renderer
        .upload_images(&[image_batch("red")], &cache)
        .expect("upload image");
    assert_eq!(report.uploaded, 1);
    assert!(report.unresolved.is_empty());
    assert!(
        report.diagnostic().is_none(),
        "a resolved image must not report a gap"
    );

    let target = RenderTarget::new(64, 64);
    let stats = renderer
        .render(camera_2d(), &target)
        .expect("render image frame");
    assert_eq!(stats.image_batches, 1, "the resident image must be drawn");
    assert!(stats.draw_calls >= 1);

    let image = renderer.read_target_rgba().expect("read image frame");
    let center = image.pixel(32, 32);
    // The source is opaque sRGB red and the target is `Rgba8UnormSrgb`, so the
    // encoded channel value round-trips.
    assert!(
        center[0] > 240 && center[1] < 15 && center[2] < 15,
        "centre pixel should be opaque red, got {center:?}"
    );
    // Outside the quad stays the clear colour (a bluish background), so the red
    // channel is strictly dominated by the green/blue channels there.
    let corner = image.pixel(0, 0);
    assert!(
        corner[0] < center[0].saturating_sub(100) && corner[2] >= corner[0],
        "outside the quad must stay the bluish clear colour, got {corner:?}"
    );
}
