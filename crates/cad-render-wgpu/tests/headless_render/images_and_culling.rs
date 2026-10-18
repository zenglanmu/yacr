use super::*;

/// An image whose resource is absent from the cache is skipped and reported,
/// never drawn as a placeholder. This is a real software-Vulkan (lavapipe) frame.
#[test]
fn missing_image_resource_draws_nothing_and_is_reported() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    // Empty cache: "absent" cannot be resolved.
    let cache = DecodedImageCache::new();
    let report = renderer
        .upload_images(&[image_batch("absent")], &cache)
        .expect("a missing resource must not panic");
    assert_eq!(report.uploaded, 0);
    assert_eq!(report.unresolved, vec![ResourceKey("absent".into())]);
    let reason = report
        .diagnostic()
        .expect("a missing image must be reported, never faked");
    assert_eq!(reason.code, cad_diagnostics::codes::RESOURCE_MISSING);

    let target = RenderTarget::new(64, 64);
    let stats = renderer
        .render(camera_2d(), &target)
        .expect("render after a skipped image");
    assert_eq!(stats.image_batches, 0, "no image quad may be drawn");
    assert_eq!(stats.draw_calls, 0, "nothing was submitted");
    let image = renderer.read_target_rgba().expect("read frame");
    let background = image.pixel(0, 0);
    assert_eq!(
        image.count_differing_from(background, 0),
        0,
        "a skipped image must leave the frame untouched"
    );
}

/// The texture must not be drawn vertically mirrored: with decoded rows stored
/// top-down and the CAD/DXF origin at the lower-left, the top of the quad shows
/// the image's top row. This is a real software-Vulkan (lavapipe) frame; without
/// the `uv.y = 1 - v` flip the halves swap and this test fails.
#[test]
fn uploaded_image_is_not_vertically_flipped() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let mut cache = DecodedImageCache::new();
    cache
        .insert(
            ResourceKey("two-tone".into()),
            two_tone_image(),
            &ResourceLimits::default(),
        )
        .expect("insert two-tone image");
    renderer
        .upload_images(&[image_batch("two-tone")], &cache)
        .expect("upload two-tone image");

    let target = RenderTarget::new(64, 64);
    renderer
        .render(camera_2d(), &target)
        .expect("render two-tone frame");
    let image = renderer.read_target_rgba().expect("read two-tone frame");
    // The quad spans world y in [-0.8, 0.8]; the target's row 0 is the top, so
    // row 8 samples near the top of the quad and row 56 near the bottom.
    let top = image.pixel(32, 8);
    let bottom = image.pixel(32, 56);
    assert!(
        top[0] > top[2].saturating_add(40) && top[1] < 60,
        "the top of the quad must sample the image's top (red) row, got {top:?}"
    );
    assert!(
        bottom[2] > bottom[0].saturating_add(40) && bottom[1] < 60,
        "the bottom of the quad must sample the image's bottom (blue) row, got {bottom:?}"
    );
}

/// A `clip` polygon must be consumed: the rendered footprint is the polygon, not
/// the whole unit square. Only the left half is clipped in, so the right half of
/// the quad must stay the clear colour. Real software-Vulkan (lavapipe) frame.
#[test]
fn clipped_image_renders_only_the_clip_polygon() {
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
    // World-space left half of the [-0.8, 0.8] square with matching UVs.
    let clip: Arc<[ImageVertex]> = Arc::from(
        vec![
            ImageVertex {
                position: Point3 {
                    x: -0.8,
                    y: -0.8,
                    z: 0.0,
                },
                uv: [0.0, 0.0],
            },
            ImageVertex {
                position: Point3 {
                    x: 0.0,
                    y: -0.8,
                    z: 0.0,
                },
                uv: [0.5, 0.0],
            },
            ImageVertex {
                position: Point3 {
                    x: 0.0,
                    y: 0.8,
                    z: 0.0,
                },
                uv: [0.5, 1.0],
            },
            ImageVertex {
                position: Point3 {
                    x: -0.8,
                    y: 0.8,
                    z: 0.0,
                },
                uv: [0.0, 1.0],
            },
        ]
        .into_boxed_slice(),
    );
    let mut batch = image_batch("red");
    batch.clip = Some(clip);
    renderer
        .upload_images(&[batch], &cache)
        .expect("upload clipped image");

    let target = RenderTarget::new(64, 64);
    let stats = renderer
        .render(camera_2d(), &target)
        .expect("render clipped frame");
    assert_eq!(stats.image_batches, 1, "the clipped image must still draw");

    let image = renderer.read_target_rgba().expect("read clipped frame");
    let left = image.pixel(16, 32);
    let right = image.pixel(48, 32);
    let background = image.pixel(0, 0);
    assert!(
        left[0] > 240 && left[1] < 15 && left[2] < 15,
        "the left (clipped-in) half must show the texture, got {left:?}"
    );
    assert_eq!(
        right, background,
        "the right (clipped-away) half must stay the clear colour, got {right:?}"
    );
}

/// Two image batches referencing the same `ResourceKey` must allocate a single
/// GPU texture (and bind group), not one per batch.
#[test]
fn image_batches_sharing_a_key_allocate_one_texture() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let mut cache = DecodedImageCache::new();
    cache
        .insert(
            ResourceKey("shared".into()),
            solid_image(2, 2, [255, 0, 0, 255]),
            &ResourceLimits::default(),
        )
        .expect("insert shared image");
    let first = image_batch("shared");
    let mut second = image_batch("shared");
    second.draw_order = 1;
    let report = renderer
        .upload_images(&[first, second], &cache)
        .expect("upload two batches sharing a key");

    assert_eq!(report.uploaded, 2, "both quads are resident");
    assert_eq!(
        renderer.image_texture_count(),
        1,
        "two batches with the same key must share one texture"
    );
    // The padded 2x2 texture (256-byte row alignment) is charged once; each quad
    // charges its own 4 vertices * 20 bytes + 6 indices * 4 bytes.
    let padded_texture = 256u64 * 2;
    let quad = (4 * 20 + 6 * 4) as u64;
    assert_eq!(
        report.bytes,
        padded_texture + 2 * quad,
        "texture bytes must be charged once, quad bytes per batch"
    );
}

/// The renderer persists across document opens, but each open builds a fresh
/// `DecodedImageCache`. A `ResourceKey` reused by a second drawing with different
/// bytes must rebuild the texture instead of silently sampling the first
/// drawing's image. Real software-Vulkan (lavapipe) frame; without the identity
/// check the second readback would still be red.
#[test]
fn new_cache_with_same_key_rebuilds_texture() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);

    // First document: "logo" names opaque red bytes.
    let mut red_cache = DecodedImageCache::new();
    red_cache
        .insert(
            ResourceKey("logo".into()),
            solid_image(2, 2, [255, 0, 0, 255]),
            &ResourceLimits::default(),
        )
        .expect("insert red logo");
    renderer
        .upload_images(&[image_batch("logo")], &red_cache)
        .expect("upload red logo");
    renderer
        .render(camera_2d(), &target)
        .expect("render red logo frame");
    let red = renderer.read_target_rgba().expect("read red frame");
    let red_center = red.pixel(32, 32);
    assert!(
        red_center[0] > 240 && red_center[1] < 15 && red_center[2] < 15,
        "the first document's texture must render red, got {red_center:?}"
    );

    // Second document: the same key now names opaque blue bytes in a new cache.
    let mut blue_cache = DecodedImageCache::new();
    blue_cache
        .insert(
            ResourceKey("logo".into()),
            solid_image(2, 2, [0, 0, 255, 255]),
            &ResourceLimits::default(),
        )
        .expect("insert blue logo");
    renderer
        .upload_images(&[image_batch("logo")], &blue_cache)
        .expect("upload blue logo");
    renderer
        .render(camera_2d(), &target)
        .expect("render blue logo frame");
    let blue = renderer.read_target_rgba().expect("read blue frame");
    let blue_center = blue.pixel(32, 32);
    assert!(
        blue_center[2] > 240 && blue_center[0] < 15 && blue_center[1] < 15,
        "the second document's bytes must rebuild the texture and render blue, got {blue_center:?}"
    );
}

/// Progressive rendering must composite images only on the page that clears the
/// target. A tiny vertex budget splits the geometry across two pages; the image
/// is charged and drawn on the first page only, never re-composited (which would
/// accumulate an `alpha < 1` image's opacity).
#[test]
fn progressive_images_draw_only_on_the_cleared_page() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    // Two 60-vertex line batches (30 segments each). Anti-aliased line strokes
    // charge 6 vertices per segment, so each batch is 180 vertices: a 200-vertex
    // budget fits the first on page 0 (180) and pushes the second to a
    // continuation page, while leaving room on page 0 for the image's 4 vertices.
    let big_line = || {
        let mut vertices = Vec::new();
        for i in 0..30 {
            let x = -0.9 + i as f32 * 0.06;
            vertices.push([x, -0.5, 0.0]);
            vertices.push([x, 0.5, 0.0]);
        }
        lines_batch(vertices)
    };
    renderer
        .upload(&scene(vec![big_line(), big_line()]))
        .expect("upload two large line batches");
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
        .expect("upload image");

    renderer.frame_budget.max_vertices = 200;
    renderer.set_progressive_rendering(true);

    let target = RenderTarget::new(64, 64);
    let mut image_draws = Vec::new();
    let mut pages = 0;
    loop {
        let stats = renderer.render(camera_2d(), &target).unwrap();
        image_draws.push(stats.image_batches);
        pages += 1;
        assert!(pages <= 4, "progressive frame did not terminate");
        if !renderer.frame_pending() {
            break;
        }
    }
    assert!(pages > 1, "the scene must span more than one page");
    assert_eq!(
        image_draws[0], 1,
        "the image must be drawn on the cleared first page"
    );
    assert!(
        image_draws[1..].iter().all(|&n| n == 0),
        "images must not re-composite on continuation pages, got {image_draws:?}"
    );
    assert_eq!(
        image_draws.iter().sum::<usize>(),
        1,
        "each image is drawn exactly once per frame"
    );
}

/// (e)/(h) A settled frame with culling is byte-identical to the unculled frame,
/// and exactly the culled chunks' draws and vertex charge disappear.
#[test]
fn culled_frame_is_byte_identical_and_drops_exactly_the_culled_runs() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    renderer.set_max_instances_per_stream(1);
    renderer.upload(&scene(cull_scene())).unwrap();
    let target = RenderTarget::new(64, 64);

    renderer.set_view_culling(false);
    let unculled = renderer.render(camera_2d(), &target).unwrap();
    let reference = renderer.read_target_rgba().unwrap();
    assert_eq!(unculled.cull_chunks_total, 5);
    assert_eq!(
        unculled.cull_chunks_kept, 5,
        "culling disabled: nothing removed"
    );
    assert_eq!(unculled.instanced_draws, 5, "one coalesced run per chunk");

    renderer.set_view_culling(true);
    let culled = renderer.render(camera_2d(), &target).unwrap();
    assert_eq!(culled.cull_chunks_total, 5);
    assert_eq!(culled.cull_chunks_kept, 3, "the two far chunks are culled");
    assert_eq!(culled.instanced_draws, 3);
    assert_eq!(
        unculled.draw_calls - culled.draw_calls,
        2,
        "exactly the culled chunks' draws must disappear"
    );
    assert_eq!(
        unculled.vertices - culled.vertices,
        2 * 6,
        "a culled 1-segment chunk drops its 6-quad charge"
    );
    assert_eq!(
        renderer.read_target_rgba().unwrap().pixels,
        reference.pixels,
        "culling must be pixel-identical to the unculled render"
    );
}

/// (f) A large finite lineweight at the viewport edge must not change pixels:
/// lineweight is carried, not rasterized, and contributes no cull pad.
#[test]
fn large_lineweight_at_the_viewport_edge_is_pixel_identical() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    renderer.set_max_instances_per_stream(1);
    let weighted = |origin_x: f32, y: f32| {
        let mut batch = line_at(origin_x, y);
        batch.lineweight = 1000.0;
        batch.lineweight_unresolved = false;
        batch
    };
    // A chunk just outside the right edge (world x in [1.02, 1.5]) whose stroke
    // band (3·wpp = 0.09375) reaches back into the view, plus a far chunk and an
    // on-screen chunk, all with a large lineweight.
    renderer
        .upload(&scene(vec![
            weighted(1.02, 0.0),
            weighted(1000.0, 0.0),
            weighted(0.0, 0.0),
        ]))
        .unwrap();
    let target = RenderTarget::new(64, 64);

    renderer.set_view_culling(false);
    renderer.render(camera_2d(), &target).unwrap();
    let reference = renderer.read_target_rgba().unwrap();

    renderer.set_view_culling(true);
    let culled = renderer.render(camera_2d(), &target).unwrap();
    assert_eq!(culled.cull_chunks_total, 3);
    assert_eq!(
        culled.cull_chunks_kept, 2,
        "the edge chunk's stroke band keeps it; only the far chunk is culled"
    );
    assert_eq!(
        renderer.read_target_rgba().unwrap().pixels,
        reference.pixels,
        "a large finite lineweight must not change the culled frame"
    );
}

/// (g) A pick reads the un-paced, **unculled** scene: toggling culling must not
/// change a pick result.
#[test]
fn pick_is_invariant_under_chunk_culling() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    renderer.set_max_instances_per_stream(1);
    let mut pickable = quad_mesh();
    pickable.draw_order = 100;
    let mut batches = vec![pickable];
    batches.extend(line_batches(3));
    batches.push(line_at(1000.0, 0.0));
    renderer.upload(&scene(batches)).unwrap();
    let target = RenderTarget::new(64, 64);

    renderer.set_view_culling(false);
    renderer.render(camera_2d(), &target).unwrap();
    let reference = renderer.pick_at(32, 32).expect("unculled pick");
    assert!(reference.is_some(), "the centre must be pickable");

    renderer.set_view_culling(true);
    let stats = renderer.render(camera_2d(), &target).unwrap();
    assert!(
        stats.cull_chunks_kept < stats.cull_chunks_total,
        "the far chunk must actually be culled for this fence to be meaningful"
    );
    let culled = renderer.pick_at(32, 32).expect("culled pick");
    assert_eq!(culled, reference, "picking must be independent of culling");
}

/// (i) Panning a camera across a chunk border must not pop: at every camera the
/// culled frame equals the unculled frame.
#[test]
fn culling_does_not_pop_across_a_chunk_border() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    renderer.set_max_instances_per_stream(1);
    // Chunks spread across world x so panning moves them in and out of the view.
    let batches: Vec<RenderBatch> = [-2.0f32, -1.0, 0.0, 1.0, 2.0]
        .into_iter()
        .map(|x| line_at(x, 0.0))
        .collect();
    renderer.upload(&scene(batches)).unwrap();
    let target = RenderTarget::new(64, 64);

    for step in 0..=40 {
        let mut camera = camera_2d();
        camera.center.x = -1.5 + step as f64 * 0.075;
        renderer.set_view_culling(false);
        renderer.render(camera, &target).unwrap();
        let reference = renderer.read_target_rgba().unwrap().pixels;
        renderer.set_view_culling(true);
        renderer.render(camera, &target).unwrap();
        assert_eq!(
            renderer.read_target_rgba().unwrap().pixels,
            reference,
            "culling popped at camera center.x = {}",
            camera.center.x
        );
    }
}

/// (j) The perspective path: culling is byte-identical to the unculled 3D frame.
#[test]
fn culling_is_byte_identical_on_the_3d_path() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    renderer.set_max_instances_per_stream(1);
    // An on-screen chunk near the origin plus a chunk far off to the side, in
    // front of the camera but outside the frustum.
    renderer
        .upload(&scene(vec![line_at(0.0, 0.0), line_at(1000.0, 0.0)]))
        .unwrap();
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

    renderer.set_view_culling(false);
    renderer.render_3d(camera, &target).unwrap();
    let reference = renderer.read_target_rgba().unwrap();

    renderer.set_view_culling(true);
    let culled = renderer.render_3d(camera, &target).unwrap();
    assert_eq!(culled.cull_chunks_total, 2);
    assert_eq!(culled.cull_chunks_kept, 1, "the far chunk is off-frustum");
    assert_eq!(
        renderer.read_target_rgba().unwrap().pixels,
        reference.pixels,
        "culling must be pixel-identical on the 3D path"
    );
}

/// (k) Culling must not break the progressive cursor/pacing hygiene fence: with
/// off-screen chunks culled the cursor still advances and the settled frame
/// equals the unculled settled frame.
#[test]
fn culling_does_not_break_the_progressive_cursor() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    renderer.set_max_instances_per_stream(1);
    let mut batches = line_batches(8);
    // Two far chunks that are culled on every page.
    batches.push(line_at(1000.0, 0.0));
    batches.push(line_at(-1000.0, 0.0));
    renderer.upload(&scene(batches)).unwrap();
    let target = RenderTarget::new(64, 64);

    // Unculled settled reference (full-ceiling, non-progressive).
    renderer.set_view_culling(false);
    renderer.render(camera_2d(), &target).unwrap();
    let reference = renderer.read_target_rgba().unwrap().pixels;

    // Progressive, paced, culling on.
    renderer.set_view_culling(true);
    renderer.frame_budget.max_vertices = 6;
    renderer.frame_budget.max_triangles = 2;
    renderer.set_progressive_rendering(true);
    renderer.set_frame_budget_target_ms(0.001);

    let mut previous_cursor = 0usize;
    let mut pages = 0;
    loop {
        let stats = renderer
            .render(camera_2d(), &target)
            .expect("a ceiling-fitting batch must not be an error");
        let (cursor, total) = renderer.frame_progress().expect("progressive frame");
        assert!(cursor > previous_cursor, "cursor must strictly advance");
        assert_eq!(total, 10);
        assert_eq!(stats.cull_chunks_total, 10);
        assert_eq!(stats.cull_chunks_kept, 8, "the two far chunks stay culled");
        previous_cursor = cursor;
        pages += 1;
        assert!(pages <= 10, "paced frame did not terminate");
        if !renderer.frame_pending() {
            break;
        }
    }
    assert_eq!(previous_cursor, 10);
    assert_eq!(
        renderer.read_target_rgba().unwrap().pixels,
        reference,
        "culling must not change the settled progressive frame"
    );
}
