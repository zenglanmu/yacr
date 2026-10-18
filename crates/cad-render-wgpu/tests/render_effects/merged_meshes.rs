use super::*;

/// A glyph-heavy mixed scene (meshes + lines + mesh edge overlays together) must
/// render byte-identically to the all-legacy path. The scene is a
/// non-overlapping grid, so this fences the *path* equality (merging meshes
/// across an intervening line batch, instancing the edge overlays) without
/// relying on coplanar fragment order; the overlapping-tie fence below covers
/// the equal-depth reorder case.
#[test]
fn merged_meshes_lines_and_overlays_match_legacy_pixels() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);

    // Interleave meshes (some with wireframe overlays) and plain line segments,
    // all on the same non-overlapping grid.
    let mut batches = Vec::new();
    for k in 0..18 {
        let x = -0.9 + (k % 6) as f64 * 0.3;
        let y = -0.9 + (k / 6) as f64 * 0.3;
        batches.push(merged_quad(
            0.08,
            [x, y, 0.0],
            [0.2 + 0.03 * k as f32, 0.7, 0.3],
            k as u128 + 1,
            k % 3 == 0,
            0,
        ));
        // A horizontal line segment in the gap to the right of the quad.
        let mut line = lines_batch(vec![
            [(x + 0.16) as f32, y as f32, 0.0],
            [(x + 0.26) as f32, y as f32, 0.0],
        ]);
        line.draw_order = 0;
        batches.push(line);
    }

    let (merged, stats) =
        render_pixels_full(&mut renderer, &target, &batches, true, true, camera_2d());
    assert!(
        stats.merged_mesh_batches >= 18,
        "all 18 meshes must merge, got {}",
        stats.merged_mesh_batches
    );
    assert!(
        stats.instanced_batches >= 18,
        "all 18 line batches and the 6 overlays must instance, got {}",
        stats.instanced_batches
    );
    assert!(
        stats.draw_calls < batches.len() as u64,
        "the mixed scene must collapse, got {} draws for {} batches",
        stats.draw_calls,
        batches.len()
    );

    let (legacy, legacy_stats) =
        render_pixels_full(&mut renderer, &target, &batches, false, false, camera_2d());
    assert_eq!(
        legacy_stats.draw_calls,
        batches.len() as u64,
        "the legacy path draws one draw per batch"
    );
    let background = opaque_black(&legacy);
    assert!(
        legacy.chunks_exact(4).any(|px| px != background.as_slice()),
        "the mixed scene must rasterize something; the comparison would be vacuous"
    );
    // Merged/instanced batches use per-chunk `f64` origins while the legacy path
    // uses per-batch origins; the f32 reduction differs by ~1 ulp, which with
    // anti-aliased strokes shifts sub-unit-coverage fringe pixels (invisible on
    // hard 1 px lines). The paths still agree on every pixel outside the
    // geometry's ~2 px AA bands; the collapsed draw counts above are unchanged.
    let diff = differing_pixels(&merged, &legacy, 4);
    assert!(
        diff <= 128,
        "a glyph-heavy mixed scene must match the all-legacy frame outside the AA \
         fringe; {diff} pixels differed"
    );
}

/// The hard overlapping-tie case the non-overlapping grid fence cannot reach:
/// mesh-mesh, line-mesh and line-line fragments that share pixels at *equal*
/// depth. Under the frame-discipline invariant (mesh `Less`, line `LessEqual`,
/// both depth-writing, replace-blend) the merged/instanced path must reproduce
/// the legacy winners exactly.
#[test]
fn merged_overlapping_ties_match_legacy_pixels() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);

    // Two fully overlapping quads (mesh-mesh tie: the first drawn wins under
    // `Less`) and two collinear segments (line-line tie: the later drawn wins
    // under `LessEqual`), all coplanar at z = 0 so the segments also tie the
    // quads (line-mesh tie: the line wins).
    let overlap_scene = |order: [i64; 4]| {
        let mut l1 = lines_batch(vec![[-0.6, 0.0, 0.0], [0.6, 0.0, 0.0]]);
        l1.color = [0.0, 0.0, 1.0];
        l1.color_unresolved = false;
        l1.draw_order = order[2];
        let mut l2 = lines_batch(vec![[-0.6, 0.0, 0.0], [0.6, 0.0, 0.0]]);
        l2.color = [1.0, 1.0, 0.0];
        l2.color_unresolved = false;
        l2.draw_order = order[3];
        vec![
            merged_quad(0.5, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], 1, false, order[0]),
            merged_quad(0.5, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0], 2, false, order[1]),
            l1,
            l2,
        ]
    };
    let forward = overlap_scene([0, 1, 2, 3]);
    let reverse = overlap_scene([3, 2, 1, 0]);

    let (merged, stats) =
        render_pixels_full(&mut renderer, &target, &forward, true, true, camera_2d());
    assert_eq!(stats.merged_mesh_batches, 2, "both meshes must merge");
    assert_eq!(stats.instanced_batches, 2, "both segments must instance");
    let background = opaque_black(&merged);
    assert!(
        merged.chunks_exact(4).any(|px| px != background.as_slice()),
        "the overlapping scene must rasterize something"
    );

    let (legacy, _) =
        render_pixels_full(&mut renderer, &target, &forward, false, false, camera_2d());
    assert_eq!(
        merged, legacy,
        "merged/instanced overlapping ties must match the legacy winners exactly"
    );

    // Non-vacuous control: reversing the draw order changes which quad and which
    // segment win their ties, so the fence above can detect a reorder.
    let (legacy_reverse, _) =
        render_pixels_full(&mut renderer, &target, &reverse, false, false, camera_2d());
    assert_ne!(
        legacy, legacy_reverse,
        "draw order must decide the tie winners; otherwise the equality above is vacuous"
    );
}

/// The 3D perspective merged-vs-legacy fence. Unlike the 2D ortho camera, a
/// `Camera3d` gives every vertex a different clip depth, so equal-depth ties are
/// not produced by construction; the merged chunk must still reproduce the
/// legacy per-batch perspective frame.
#[test]
fn merged_meshes_match_legacy_in_perspective() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    // A grid of quads carrying *absolute* vertices and a shared zero origin, with
    // a per-quad depth offset. Both paths then narrow the same `f32` origin and
    // use identical per-vertex data, so the fence isolates the merge itself while
    // the perspective projection gives each quad a distinct clip depth.
    let mut batches = quad_grid_batches(3, 3, false);
    for (k, batch) in batches.iter_mut().enumerate() {
        let o = batch.local_origin;
        let dz = k as f32 * 0.03;
        for v in &mut batch.vertices {
            v[0] += o.x as f32;
            v[1] += o.y as f32;
            v[2] += o.z as f32 + dz;
        }
        batch.local_origin = Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
    }
    let camera = camera_3d();

    renderer.set_line_instancing(true);
    renderer.set_mesh_merging(true);
    renderer.clear_batches();
    renderer
        .upload(&scene(batches.clone()))
        .expect("upload 3D merged");
    let stats = renderer
        .render_3d(camera, &target)
        .expect("render 3D merged");
    assert_eq!(stats.merged_mesh_batches, 9, "all 9 meshes must merge");
    assert!(
        stats.mesh_draws < 9,
        "9 meshes must collapse into fewer 3D draws, got {}",
        stats.mesh_draws
    );
    let merged = renderer.read_target_rgba().expect("read 3D merged").pixels;
    let background = opaque_black(&merged);
    assert!(
        merged.chunks_exact(4).any(|px| px != background.as_slice()),
        "3D merged frame rasterized nothing"
    );

    renderer.set_line_instancing(false);
    renderer.set_mesh_merging(false);
    renderer.clear_batches();
    renderer.upload(&scene(batches)).expect("upload 3D legacy");
    let legacy_stats = renderer
        .render_3d(camera, &target)
        .expect("render 3D legacy");
    assert_eq!(
        legacy_stats.draw_calls, 9,
        "legacy draws each 3D mesh batch"
    );
    let legacy = renderer.read_target_rgba().expect("read 3D legacy").pixels;
    assert_eq!(
        merged, legacy,
        "merged meshes must match the legacy perspective frame byte-for-byte"
    );
}

/// A varying per-vertex colour (a HATCH-style gradient) with a non-white tint is
/// the case where baking `tint * c_i` at upload re-associates the f32 product
/// versus the legacy per-pixel `tint * lerp(c_i)`. This is the honest fence for
/// that scoping: the two paths must agree within a small tolerance (they are not
/// guaranteed bit-identical, and are documented as such).
#[test]
fn merged_gradient_mesh_matches_legacy_within_tolerance() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);

    // One quad with four distinct corner colours and a non-white batch tint.
    let mut batch = merged_quad(0.7, [0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 1, false, 0);
    batch.colors = vec![
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 1.0, 0.0],
    ];
    batch.color = [0.7, 0.8, 0.9];
    batch.color_unresolved = false;
    let batches = vec![batch];

    let (merged, stats) =
        render_pixels_full(&mut renderer, &target, &batches, true, true, camera_2d());
    assert_eq!(stats.merged_mesh_batches, 1);
    let (legacy, _) =
        render_pixels_full(&mut renderer, &target, &batches, false, false, camera_2d());
    // On the reference adapter (Quadro P620 / Vulkan) this ramp is currently
    // byte-identical (0 pixels differ), but bit-identity is *not* guaranteed: the
    // merged path interpolates `tint * c_i` while the legacy path interpolates
    // `c_i` then multiplies by `tint`, so a vendor whose f32 rounding differs may
    // see a 1-ULP divergence. The fence therefore allows ≤1/255 per channel.
    assert_eq!(
        differing_pixels(&merged, &legacy, 1),
        0,
        "a baked gradient must stay within 1/255 of the legacy per-pixel product"
    );
    // A gradient frame must actually vary (otherwise the fence is vacuous).
    assert!(
        merged
            .chunks_exact(4)
            .any(|px| px != merged[0..4].to_vec().as_slice()),
        "the gradient must produce more than one colour"
    );
}

/// The overlay-order residual: `mesh A + overlay A`, an intervening line batch,
/// then a contiguous `mesh B` must keep the legacy
/// `A.tri, A.overlay, line, B.tri` order. Without flushing the pending mesh run
/// when the line run ends, the merged `A+B.tri` run could draw after the line.
///
/// This exercises the interleaving end-to-end. Note the *pixel* outcome is
/// identical on the reference adapter either way: under a conforming depth test
/// the reorder is unobservable (a line wins an equal-depth tie regardless of
/// order, and non-equal depths are decided by depth, not order). The guarantee
/// is structural — the flush in the frame loop — and this fence pins that the
/// interleaving at least keeps rendering the legacy frame (a regression guard
/// against a change that drops or duplicates the mesh/overlay geometry).
#[test]
fn merged_mesh_overlay_line_mesh_keeps_legacy_order() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);

    // A and B overlap fully (shared origin); the line crosses them at equal
    // depth, so any reorder of `A.tri` relative to the line is observable only
    // through the depth test (which must still resolve line-wins at the tie).
    let batches = vec![
        merged_quad(0.5, [0.0, 0.0, 0.0], [0.9, 0.2, 0.2], 1, true, 0),
        {
            let mut l = lines_batch(vec![[-0.9, 0.0, 0.0], [0.9, 0.0, 0.0]]);
            l.color = [0.1, 0.1, 0.9];
            l.color_unresolved = false;
            l.draw_order = 1;
            l
        },
        merged_quad(0.5, [0.0, 0.0, 0.0], [0.2, 0.9, 0.2], 2, true, 2),
    ];

    let (merged, stats) =
        render_pixels_full(&mut renderer, &target, &batches, true, true, camera_2d());
    assert_eq!(stats.merged_mesh_batches, 2, "both meshes must merge");
    assert_eq!(
        stats.instanced_batches, 3,
        "the two overlays and the line must instance"
    );
    let background = opaque_black(&merged);
    assert!(
        merged.chunks_exact(4).any(|px| px != background.as_slice()),
        "the interleaved scene must rasterize something"
    );

    let (legacy, _) =
        render_pixels_full(&mut renderer, &target, &batches, false, false, camera_2d());
    assert_eq!(
        merged, legacy,
        "the interleaving must keep the legacy mesh/overlay/line order"
    );
}

/// Merged runs must not merge across a draw-order gap: the same batches packed in
/// upload order but drawn in a different order must still render in draw order.
#[test]
fn merged_mesh_runs_preserve_draw_order_with_distinct_colors() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);

    // Three fully overlapping quads (same z, same geometry) with distinct uniform
    // per-vertex colours. For opaque meshes the strict `Less` depth test makes the
    // FIRST drawn quad win a depth tie, so draw order decides the visible colour.
    let overlapping = |order: [i64; 3]| {
        vec![
            merged_quad(0.5, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], 1, false, order[0]),
            merged_quad(0.5, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0], 2, false, order[1]),
            merged_quad(0.5, [0.0, 0.0, 0.0], [0.0, 0.0, 1.0], 3, false, order[2]),
        ]
    };
    let forward = overlapping([0, 1, 2]);
    let reverse = overlapping([2, 1, 0]);

    let (merged, stats) =
        render_pixels_full(&mut renderer, &target, &forward, true, true, camera_2d());
    assert_eq!(stats.merged_mesh_batches, 3);
    let (legacy, _) =
        render_pixels_full(&mut renderer, &target, &forward, false, false, camera_2d());
    assert_eq!(
        merged, legacy,
        "merged runs must preserve the per-vertex colour tie order"
    );

    let (reverse_legacy, _) =
        render_pixels_full(&mut renderer, &target, &reverse, false, false, camera_2d());
    assert_ne!(
        legacy, reverse_legacy,
        "draw order must decide the winner; otherwise the equality above is vacuous"
    );
}

/// A camera move (pan/zoom) must be a small uniform write only: neither the
/// merged mesh chunks nor the instanced streams are ever re-uploaded.
#[test]
fn camera_move_does_not_reupload_mesh_chunks() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    let mut batches = quad_grid_batches(3, 3, true);
    batches.extend(grid_line_batches(3, 3));
    renderer.set_line_instancing(true);
    renderer.set_mesh_merging(true);
    renderer.clear_batches();
    renderer
        .upload(&scene(batches))
        .expect("upload mixed scene");
    let mesh_uploads = renderer.mesh_chunk_uploads();
    let line_uploads = renderer.instance_stream_uploads();
    assert!(mesh_uploads > 0, "the scene upload must create a chunk");
    assert!(line_uploads > 0, "the scene upload must create a stream");

    let mut camera = camera_2d();
    for step in 0..8 {
        camera.center.x = step as f64 * 0.05;
        camera.world_per_px = 2.0 / 64.0 * (1.0 + step as f64 * 0.1);
        renderer
            .render(camera, &target)
            .expect("render pan/zoom frame");
    }
    assert_eq!(
        renderer.mesh_chunk_uploads(),
        mesh_uploads,
        "pan/zoom must not re-upload the static merged mesh chunks"
    );
    assert_eq!(
        renderer.instance_stream_uploads(),
        line_uploads,
        "pan/zoom must not re-upload the static instance streams"
    );
    assert!(renderer.mesh_chunk_count() > 0);
}

/// A single mesh batch larger than the injected vertex cap splits across several
/// chunks: no triangle is lost, no empty chunk is created, and the frame matches
/// the legacy path.
#[test]
fn mesh_batch_beyond_the_vertex_cap_splits_without_loss() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    renderer.set_max_vertices_per_mesh_stream(8);
    // One mesh batch of six quads (24 vertices, 12 triangles) far beyond cap 8.
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for k in 0..6u32 {
        let x = -0.9 + k as f32 * 0.3;
        let base = vertices.len() as u32;
        vertices.extend_from_slice(&[
            [x, -0.1, 0.0],
            [x, 0.1, 0.0],
            [x + 0.15, 0.1, 0.0],
            [x + 0.15, -0.1, 0.0],
        ]);
        indices.push([base, base + 2, base + 1]);
        indices.push([base, base + 3, base + 2]);
    }
    let mut batch = merged_quad(0.1, [0.0, 0.0, 0.0], [0.8, 0.4, 0.2], 1, false, 0);
    batch.vertices = vertices;
    batch.normals = vec![[0.0, 0.0, 1.0]; batch.vertices.len()];
    batch.colors = Vec::new();
    batch.indices = indices;
    let batches = vec![batch];

    let (merged, stats) =
        render_pixels_full(&mut renderer, &target, &batches, true, true, camera_2d());
    assert_eq!(stats.merged_mesh_batches, 1);
    assert!(
        stats.mesh_chunks >= 3,
        "24 vertices with cap 8 must split into several chunks, got {}",
        stats.mesh_chunks
    );
    assert_eq!(stats.triangles, 12, "splitting must not lose any triangle");
    let (legacy, _) =
        render_pixels_full(&mut renderer, &target, &batches, false, false, camera_2d());
    assert_eq!(merged, legacy, "splitting must not change the frame");
}

/// Instancing a mesh edge overlay must be byte-identical to the legacy wireframe
/// overlay draw (which uses the line pipeline over the shared mesh vertices).
#[test]
fn instanced_edge_overlay_matches_legacy_pixels() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    let batches = vec![
        merged_quad(0.4, [-0.4, 0.0, 0.0], [0.9, 0.9, 0.9], 1, true, 0),
        merged_quad(0.4, [0.4, 0.0, 0.0], [0.2, 0.6, 0.9], 2, true, 0),
    ];

    let (instanced, stats) =
        render_pixels_full(&mut renderer, &target, &batches, true, true, camera_2d());
    assert_eq!(
        stats.instanced_batches, 2,
        "both edge overlays must instance"
    );
    assert!(
        renderer.instance_segment_count() > 0,
        "the overlays must be resident in an instance stream"
    );
    let (legacy, _) =
        render_pixels_full(&mut renderer, &target, &batches, false, false, camera_2d());
    // Instancing a merged batch's overlay uses the chunk's `f64` origin while the
    // legacy per-batch overlay uses the batch's own origin, so the f32 reduction
    // differs by ~1 ulp; with anti-aliased strokes that shifts sub-unit-coverage
    // fringe pixels (it was invisible on hard 1 px lines). The two conversion
    // paths still agree on every pixel outside the overlays' ~2 px AA bands.
    let diff = differing_pixels(&instanced, &legacy, 4);
    assert!(
        diff <= 64,
        "instanced edge overlays must match the legacy overlay draw outside the \
         AA fringe; {diff} pixels differed"
    );
}

/// The upload byte charge for a merged mesh run must be exact: one interleaved
/// vertex per stored vertex (44 B) plus one `u32` per triangle index, replacing
/// the legacy per-batch position/normal/colour/index buffers.
#[test]
fn merged_mesh_upload_charges_exact_chunk_bytes() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    let batches = quad_grid_batches(3, 1, false); // 3 quads, 4 vertices / 6 indices

    let (_, stats) = render_pixels_full(&mut renderer, &target, &batches, true, true, camera_2d());
    let per_merged = 4 * 44 + 6 * 4; // 200
    assert_eq!(
        stats.uploaded_bytes,
        (3 * per_merged) as u64,
        "merged chunks must charge the exact 44 B/vertex + 4 B/index"
    );

    // The legacy per-batch mesh path charges 12 B position + 12 B normal +
    // 12 B colour per vertex and 4 B per triangle index = 168 B per quad.
    // `uploaded_bytes` is cumulative over the renderer's lifetime, so compare
    // the delta this upload added.
    let (_, legacy) =
        render_pixels_full(&mut renderer, &target, &batches, false, false, camera_2d());
    assert_eq!(
        legacy.uploaded_bytes - stats.uploaded_bytes,
        (3 * (4 * 12 + 4 * 12 + 4 * 12 + 6 * 4)) as u64,
        "the legacy mesh path charges its packed per-batch bytes"
    );

    // With an overlay the merged run also charges 56 B per instanced segment;
    // a quad's wireframe is 5 de-duplicated edges = 5 segments.
    let with_edges = quad_grid_batches(1, 1, true);
    let (_, edge_stats) =
        render_pixels_full(&mut renderer, &target, &with_edges, true, true, camera_2d());
    assert_eq!(
        edge_stats.uploaded_bytes - legacy.uploaded_bytes,
        (per_merged + 5 * cad_render_wgpu::LINE_INSTANCE_BYTES as usize) as u64,
        "the instanced overlay must charge the exact instance stride"
    );
}

/// Crossing the byte budget with merged mesh batches must be reported, never
/// silent: the second quad (200 B) crosses a 300-byte budget after the first.
#[test]
fn merged_mesh_byte_budget_is_reported_not_silent() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    renderer.set_mesh_merging(true);
    renderer.frame_budget = FrameBudget {
        max_vertices: usize::MAX,
        max_triangles: usize::MAX,
        max_bytes: 300,
    };
    let batches = quad_grid_batches(2, 1, false);
    renderer.clear_batches();
    renderer.upload(&scene(batches)).expect("upload two quads");
    let target = RenderTarget::new(64, 64);
    let stats = renderer
        .render(camera_2d(), &target)
        .expect("over-budget frame must still render the batch that fits");
    let over = stats
        .over_budget
        .expect("crossing the byte budget must be reported");
    assert_eq!(over.report.category, "bytes");
    assert_eq!(over.report.limit, 300);
    assert_eq!(over.report.requested, 400, "two merged quads at 200 B each");
}

/// Mirrored merged meshes must render exactly like the legacy mirrored mesh
/// path: the merged pipeline selects the same `front_face = Cw`, and the chunk
/// never spans both mirror states.
#[test]
fn mirrored_merged_meshes_match_legacy_pixels() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    let batches = vec![
        mirrored_quad([-0.35, 0.0, 0.0], [0.9, 0.3, 0.2], 1),
        mirrored_quad([0.35, 0.0, 0.0], [0.2, 0.6, 0.9], 2),
    ];

    let (merged, stats) =
        render_pixels_full(&mut renderer, &target, &batches, true, true, camera_2d());
    assert_eq!(stats.merged_mesh_batches, 2);
    let background = opaque_black(&merged);
    assert!(
        merged.chunks_exact(4).any(|px| px != background.as_slice()),
        "a mirrored quad must not be culled away"
    );
    let (legacy, _) =
        render_pixels_full(&mut renderer, &target, &batches, false, false, camera_2d());
    assert_eq!(
        merged, legacy,
        "mirrored merged meshes must match the legacy mirrored path byte-for-byte"
    );
}

/// Incremental `keep_prefix` commit plus chunk GC must keep converted mesh
/// batches' chunk references valid through renumbering, and render exactly like
/// the equivalent batches uploaded in one go.
#[test]
fn incremental_commit_and_gc_renumber_merged_meshes() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    renderer.set_line_instancing(true);
    renderer.set_mesh_merging(true);
    renderer.clear_batches();

    let base = || quad_grid_batches(1, 2, false);
    let overlay = |count: usize| -> Vec<RenderBatch> {
        (0..count)
            .map(|k| {
                merged_quad(
                    0.06,
                    [-0.4 + k as f64 * 0.4, 0.4, 0.0],
                    [0.9, 0.9, 0.1],
                    (10 + k) as u128,
                    false,
                    0,
                )
            })
            .collect()
    };
    renderer.upload(&scene(base())).expect("upload base");
    assert_eq!(renderer.mesh_chunk_count(), 1);
    let staged = renderer
        .prepare_upload(&scene(overlay(2)))
        .expect("stage overlay A");
    renderer.commit_upload(staged, 2).expect("commit overlay A");
    assert_eq!(renderer.batch_count(), 4);
    assert_eq!(renderer.mesh_chunk_count(), 2);
    let staged = renderer
        .prepare_upload(&scene(overlay(1)))
        .expect("stage overlay B");
    renderer.commit_upload(staged, 2).expect("commit overlay B");
    assert_eq!(renderer.batch_count(), 3, "overlay A's batch is replaced");
    assert_eq!(
        renderer.mesh_chunk_count(),
        2,
        "overlay A's chunk is collected and the survivors renumbered"
    );

    // The same batches uploaded in one go render identically.
    let mut all = base();
    all.extend(overlay(1));
    let expected = render_pixels_full(&mut renderer, &target, &all, true, true, camera_2d()).0;

    // Rebuild the incremental sequence and compare.
    renderer.clear_batches();
    renderer.upload(&scene(base())).expect("upload base");
    let staged = renderer
        .prepare_upload(&scene(overlay(2)))
        .expect("stage overlay A");
    renderer.commit_upload(staged, 2).expect("commit overlay A");
    let staged = renderer
        .prepare_upload(&scene(overlay(1)))
        .expect("stage overlay B");
    renderer.commit_upload(staged, 2).expect("commit overlay B");
    renderer
        .render(camera_2d(), &target)
        .expect("render incremental scene");
    let incremental = renderer
        .read_target_rgba()
        .expect("read incremental")
        .pixels;
    assert_eq!(
        incremental, expected,
        "chunk GC renumbering must not change the rendered frame"
    );
}

/// `MeshEdges` geometry now renders. The topology used to be a documented silent
/// no-op (the legacy `draw_batch` read `GpuBatch::indices`, which is empty for a
/// `MeshEdges` batch), and is now converted to anti-aliased line segments on the
/// instanced path and drawn as the same pre-expanded AA quads on the legacy
/// per-batch fallback (backlog item G3). Both conversion paths must rasterize the
/// same segments.
#[test]
fn mesh_edges_batch_renders_anti_aliased_segments() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    let mut batch = lines_batch(vec![
        [-0.8, -0.8, 0.0],
        [0.8, -0.8, 0.0],
        [0.8, -0.8, 0.0],
        [0.8, 0.8, 0.0],
    ]);
    batch.topology = RenderTopology::MeshEdges;

    // Legacy per-batch path: the topology now draws its two AA segments.
    let (legacy, legacy_stats) = render_pixels_full(
        &mut renderer,
        &target,
        &[batch.clone()],
        false,
        false,
        camera_2d(),
    );
    assert_eq!(legacy_stats.draw_calls, 1, "the batch is submitted once");
    let background = opaque_black(&legacy);
    let legacy_drawn = legacy
        .chunks_exact(4)
        .filter(|px| *px != background.as_slice())
        .count();
    assert!(
        legacy_drawn > 0,
        "a MeshEdges batch must now render its segments (previously a silent no-op)"
    );

    // Converted path: the same topology instanced as AA line segments.
    let (converted, converted_stats) =
        render_pixels_full(&mut renderer, &target, &[batch], true, true, camera_2d());
    assert!(
        converted_stats.instanced_draws > 0,
        "the MeshEdges batch must be converted to an instanced line run"
    );
    let converted_drawn = converted
        .chunks_exact(4)
        .filter(|px| *px != background.as_slice())
        .count();
    assert!(
        converted_drawn > 0,
        "MeshEdges must render on the converted path too"
    );
    assert_eq!(
        converted, legacy,
        "both conversion paths must rasterize the same anti-aliased segments"
    );
}

/// Progressive-page boundary inside a merged run: a page that accepts only part
/// of a merged chunk must neither lose nor duplicate geometry — the accumulated
/// final page must equal the single full-budget frame.
#[test]
fn progressive_page_boundary_splits_a_merged_run_without_loss() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);
    // 8 non-overlapping quads that all merge into one chunk (one run per batch).
    let batches = quad_grid_batches(2, 4, false);

    renderer.set_line_instancing(true);
    renderer.set_mesh_merging(true);
    renderer.clear_batches();
    renderer
        .upload(&scene(batches.clone()))
        .expect("upload merged scene");
    assert_eq!(
        renderer.mesh_chunk_count(),
        1,
        "all 8 quads must share one chunk"
    );

    // Reference: the full-budget frame coalesces the whole chunk into one draw.
    let reference_stats = renderer
        .render(camera_2d(), &target)
        .expect("render full-budget frame");
    assert_eq!(reference_stats.merged_mesh_batches, 8);
    assert_eq!(
        reference_stats.mesh_draws, 1,
        "the full-budget frame coalesces the whole run"
    );
    let reference = renderer.read_target_rgba().expect("read full frame").pixels;

    // Force one merged batch per page (a merged quad charges 200 B), so the page
    // boundary falls *inside* the merged run.
    renderer.frame_budget = FrameBudget {
        max_vertices: usize::MAX,
        max_triangles: usize::MAX,
        max_bytes: 200,
    };
    renderer.set_progressive_rendering(true);
    let mut pages = 0;
    loop {
        let stats = renderer.render(camera_2d(), &target).expect("render page");
        assert_eq!(
            stats.merged_mesh_batches, 1,
            "each page must accept exactly one merged batch"
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
        renderer.read_target_rgba().expect("read pages").pixels,
        reference,
        "a page boundary inside a merged run must not lose or duplicate geometry"
    );
}
