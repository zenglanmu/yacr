//! GPU id-readback picking: id-pass correctness per draw class, non-interference
//! with the color pass, and the selection-flags machinery.
//!
//! These tests create a real wgpu device on whatever adapter is visible (on this
//! project's machine a real Vulkan GPU; the report line is the run's own adapter)
//! and assert exact identities, not just "something happened". When no adapter is
//! available they skip with a clear message rather than pretending to pass.

use cad_domain::{DocumentId, EntityId, InstancePath, Point3, SelectionRef, TaskStamp};
use cad_render_wgpu::headless::{create_headless_gpu, HeadlessGpu};
use cad_render_wgpu::{
    BackendPreference, Camera2d, IdTargetFormat, RenderTarget, Renderer, FLAG_HIDDEN, FLAG_SELECTED,
};
use cad_scene::{RenderBatch, RenderTopology, SceneDelta};

fn source(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn gpu() -> Option<HeadlessGpu> {
    match create_headless_gpu(BackendPreference::Auto) {
        Ok(gpu) => Some(gpu),
        Err(error) => {
            eprintln!("skipping headless GPU test: {error}");
            None
        }
    }
}

fn init(gpu: HeadlessGpu, format: Option<IdTargetFormat>) -> Renderer {
    let HeadlessGpu {
        device,
        queue,
        adapter,
    } = gpu;
    eprintln!(
        "id-pass test adapter: {} ({}, {})",
        adapter.name, adapter.backend, adapter.device_type
    );
    let mut renderer = Renderer::new(BackendPreference::Auto);
    renderer.set_id_format_override(format);
    renderer
        .initialize_with_device(device, queue)
        .expect("initialize renderer");
    renderer.set_poll_timeout(std::time::Duration::from_secs(30));
    renderer
}

fn scene(batches: Vec<RenderBatch>) -> SceneDelta {
    SceneDelta {
        stamp: TaskStamp::new(DocumentId(1), 0),
        added: batches,
        removed_chunks: Vec::new(),
        images: Vec::new(),
    }
}

fn camera_2d() -> Camera2d {
    Camera2d {
        center: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        world_per_px: 1.0,
        z_plane: 0.0,
    }
}

fn base_batch(topology: RenderTopology, vertices: Vec<[f32; 3]>, entity: u128) -> RenderBatch {
    RenderBatch {
        local_origin: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        topology,
        vertices,
        normals: Vec::new(),
        colors: Vec::new(),
        indices: Vec::new(),
        edges: Vec::new(),
        mirrored: false,
        alpha: 1.0,
        color: cad_scene::DEFAULT_BATCH_COLOR,
        color_unresolved: true,
        lineweight: 0.0,
        lineweight_unresolved: true,
        linetype: cad_scene::LinetypePattern::continuous(),
        linetype_unresolved: true,
        sources: vec![source(entity)],
        draw_order: 0,
    }
}

/// A `LineList` batch: a horizontal band of `rows` segments at `y`, spanning
/// `x0..x1`, so a scanline is guaranteed to rasterize (a single zero-width line
/// through a pixel centre is not reliably covered on every rasterizer).
fn line_band(y: f32, rows: u32, entity: u128) -> RenderBatch {
    let mut vertices = Vec::new();
    for row in 0..rows {
        let offset = y + row as f32 * 0.5;
        vertices.push([-20.0, offset, 0.0]);
        vertices.push([20.0, offset, 0.0]);
    }
    base_batch(RenderTopology::Lines, vertices, entity)
}

/// A quad mesh (two triangles) centred at `(cx, cy)` with half-size `half`.
/// `edges` adds the wireframe overlay (which the merged path instances).
fn quad(cx: f32, cy: f32, half: f32, entity: u128, edges: bool) -> RenderBatch {
    let mut batch = base_batch(
        RenderTopology::Mesh,
        vec![
            [cx - half, cy - half, 0.0],
            [cx - half, cy + half, 0.0],
            [cx + half, cy + half, 0.0],
            [cx + half, cy - half, 0.0],
        ],
        entity,
    );
    batch.normals = vec![[0.0, 0.0, 1.0]; 4];
    // The vertex order is clockwise in a Y-up world, so the index order flips it
    // to counter-clockwise and the `front_face = Ccw` culling keeps it visible
    // (the same convention the existing merged-mesh tests use).
    batch.indices = vec![[0, 2, 1], [0, 3, 2]];
    if edges {
        batch.edges = vec![[cx - half, cy - half, 0.0], [cx + half, cy - half, 0.0]];
    }
    batch
}

/// The pixel containing world point `(wx, wy)` for `camera`, clamped to the
/// target. Mirrors the renderer's own world→clip mapping.
fn world_to_pixel(camera: &Camera2d, width: u32, height: u32, world: [f64; 2]) -> (u32, u32) {
    let sx = 2.0 / (f64::from(width) * camera.world_per_px);
    let sy = 2.0 / (f64::from(height) * camera.world_per_px);
    let clip_x = sx * (world[0] - camera.center.x);
    let clip_y = sy * (world[1] - camera.center.y);
    let px = ((clip_x + 1.0) * 0.5 * f64::from(width)).floor() as i64;
    let py = ((1.0 - clip_y) * 0.5 * f64::from(height)).floor() as i64;
    (
        px.clamp(0, i64::from(width) - 1) as u32,
        py.clamp(0, i64::from(height) - 1) as u32,
    )
}

fn render(renderer: &mut Renderer, target: &RenderTarget) -> Vec<u8> {
    renderer
        .render(camera_2d(), target)
        .expect("render color frame");
    renderer.read_target_rgba().expect("read frame").pixels
}

/// The first pixel the color pass actually drew (not the clear colour), scanning
/// row-major. Using a pixel the color pass covered makes the id-pass assertion
/// about *the same visible geometry* rather than a guess about rasterization.
fn first_drawn_pixel(pixels: &[u8], width: u32, height: u32) -> Option<(u32, u32)> {
    for y in 0..height {
        for x in 0..width {
            let i = (y as usize * width as usize + x as usize) * 4;
            let drawn = pixels[i] > 8 || pixels[i + 1] > 8 || pixels[i + 2] > 8;
            if drawn {
                return Some((x, y));
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Id-pass correctness per class
// ---------------------------------------------------------------------------

#[test]
fn pick_before_any_frame_has_no_view() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    // No colour frame has been rendered, so there is no projection to pick with.
    assert!(renderer.pick_at(1, 1).expect("pick").is_none());
}

#[test]
fn a_line_pixel_resolves_to_its_entity() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(96, 96);
    renderer
        .upload(&scene(vec![line_band(6.0, 3, 0xABC)]))
        .expect("upload");
    let pixels = render(&mut renderer, &target);
    let (x, y) = first_drawn_pixel(&pixels, 96, 96).expect("the band must rasterize");

    let hit = renderer.pick_at(x, y).expect("pick").expect("a hit");
    assert_eq!(hit.entity_key, renderer.pick_key_for_batch(0).unwrap());
    assert_eq!(
        hit.source.expect("exact").entity,
        EntityId(0xABC),
        "the id pass must resolve the line pixel to its own entity"
    );
    assert!(!hit.key_collision);
    // A line has no sub-element concept; the id pass never claims one.
    assert!(hit.sub_element.is_none());
}

#[test]
fn a_merged_glyph_mesh_pixel_resolves_to_its_entity() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(96, 96);
    // No edges => the glyph class, which is flat-merged by default.
    renderer
        .upload(&scene(vec![quad(0.0, 0.0, 8.0, 0x111, false)]))
        .expect("upload");
    assert_eq!(renderer.mesh_chunk_count(), 1, "the glyph quad is merged");

    let pixels = render(&mut renderer, &target);
    let (x, y) = world_to_pixel(&camera_2d(), 96, 96, [0.0, 0.0]);
    assert!(
        pixels[(y as usize * 96 + x as usize) * 4 + 1] > 0,
        "quad drawn"
    );

    let hit = renderer.pick_at(x, y).expect("pick").expect("a hit");
    assert_eq!(
        hit.source.expect("exact").entity,
        EntityId(0x111),
        "a merged-mesh fragment must carry its own batch's pick key"
    );
}

#[test]
fn a_mesh_with_an_edge_overlay_resolves_to_the_same_entity_on_both_paths() {
    let Some(available) = gpu() else {
        return;
    };
    drop(available);
    let target = RenderTarget::new(96, 96);
    let batches = vec![quad(0.0, 0.0, 8.0, 0x222, true)];
    let (x, y) = world_to_pixel(&camera_2d(), 96, 96, [0.0, 0.0]);

    // Converted (merged mesh + instanced overlay) and legacy (per-batch) must
    // both resolve the same pixel to the same entity.
    for merging in [true, false] {
        let Some(gpu) = gpu() else {
            return;
        };
        let mut renderer = init(gpu, None);
        renderer.set_mesh_merging(merging);
        renderer.set_line_instancing(merging);
        renderer.upload(&scene(batches.clone())).expect("upload");
        let pixels = render(&mut renderer, &target);
        assert!(
            pixels[(y as usize * 96 + x as usize) * 4 + 1] > 0,
            "merging={merging}: the quad must rasterize at its centre"
        );
        let hit = renderer.pick_at(x, y).expect("pick");
        let hit = hit.unwrap_or_else(|| {
            panic!(
                "merging={merging}: no id-pass hit at ({x}, {y}); chunks={} streams={}",
                renderer.mesh_chunk_count(),
                renderer.instance_stream_count()
            )
        });
        assert_eq!(
            hit.source.expect("exact").entity,
            EntityId(0x222),
            "merging={merging} must resolve the overlay mesh to its entity"
        );
    }
}

#[test]
fn separated_entities_pick_to_their_own_pixels() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(128, 128);
    renderer
        .upload(&scene(vec![
            quad(-16.0, 0.0, 6.0, 0x10, false),
            quad(16.0, 0.0, 6.0, 0x20, false),
        ]))
        .expect("upload");
    let _ = render(&mut renderer, &target);

    for (world_x, expected) in [(-16.0, 0x10u128), (16.0, 0x20)] {
        let (x, y) = world_to_pixel(&camera_2d(), 128, 128, [world_x, 0.0]);
        let hit = renderer.pick_at(x, y).expect("pick").expect("a hit");
        assert_eq!(
            hit.source.expect("exact").entity,
            EntityId(expected),
            "pixel at world x={world_x} must resolve to its own quad"
        );
    }
}

#[test]
fn a_background_pixel_picks_nothing() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(64, 64);
    renderer
        .upload(&scene(vec![quad(0.0, 0.0, 3.0, 7, false)]))
        .expect("upload");
    let _ = render(&mut renderer, &target);
    // A corner far from the small quad is the cleared id target.
    assert!(renderer.pick_at(1, 1).expect("pick").is_none());
    // Out-of-range pixels are not hits either.
    assert!(renderer.pick_at(9999, 9999).expect("pick").is_none());
}

// ---------------------------------------------------------------------------
// Multi-source and collision reporting
// ---------------------------------------------------------------------------

#[test]
fn a_multi_source_batch_reports_every_candidate_and_never_the_first() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(96, 96);
    // Two entities concatenated into one batch, exactly what the scene builder
    // does for adjacent line fragments.
    let mut batch = line_band(6.0, 3, 0);
    batch.sources = vec![source(0x1000), source(0x2000)];
    renderer.upload(&scene(vec![batch])).expect("upload");
    let pixels = render(&mut renderer, &target);
    let (x, y) = first_drawn_pixel(&pixels, 96, 96).expect("drawn");

    let hit = renderer.pick_at(x, y).expect("pick").expect("a hit");
    assert!(
        hit.source.is_none(),
        "a multi-source fragment must not be attributed to one entity"
    );
    assert_eq!(hit.candidates.len(), 2);
    assert_eq!(hit.candidates[0].entity, EntityId(0x1000));
    assert_eq!(hit.candidates[1].entity, EntityId(0x2000));
    assert!(hit.is_ambiguous());
    assert_eq!(hit.reason, Some("pick-multi-source-batch"));
}

#[test]
fn a_u128_truncation_collision_is_reported_and_still_resolves_exactly() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(128, 128);
    // Two distinct entities whose low 32 bits agree.
    let low = 0x0000_4242u128;
    let high = (1u128 << 96) | low;
    renderer
        .upload(&scene(vec![
            quad(-16.0, 0.0, 6.0, low, false),
            quad(16.0, 0.0, 6.0, high, false),
        ]))
        .expect("upload");
    let _ = render(&mut renderer, &target);

    let collisions = renderer.pick_collisions();
    assert_eq!(collisions.len(), 1, "the truncation must be detected");
    assert_eq!(collisions[0].truncated_key, 0x4242);
    assert_eq!(collisions[0].identities.len(), 2);

    // Resolution is still exact (the key is a renderer key), and the hazard is
    // surfaced on the hit instead of causing a silent mis-pick.
    for (world_x, expected) in [(-16.0, low), (16.0, high)] {
        let (x, y) = world_to_pixel(&camera_2d(), 128, 128, [world_x, 0.0]);
        let hit = renderer.pick_at(x, y).expect("pick").expect("a hit");
        assert_eq!(hit.source.expect("exact").entity, EntityId(expected));
        assert!(
            hit.key_collision,
            "the collision must be visible on the hit"
        );
    }
}

// ---------------------------------------------------------------------------
// Non-interference with the color pass
// ---------------------------------------------------------------------------

#[test]
fn the_id_pass_does_not_disturb_the_color_frame() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(96, 96);
    renderer
        .upload(&scene(vec![
            quad(-12.0, 0.0, 5.0, 0xAA, false),
            line_band(6.0, 3, 0xBB),
        ]))
        .expect("upload");

    let frame_only = render(&mut renderer, &target);
    // Pick several pixels: each one runs a whole id pass.
    for (x, y) in [(48u32, 48u32), (20, 48), (70, 20), (1, 1)] {
        let _ = renderer.pick_at(x, y).expect("pick");
    }
    // The color target must be untouched by the picks...
    let after_picks = renderer.read_target_rgba().expect("read").pixels;
    assert_eq!(
        after_picks, frame_only,
        "an id pass must not modify the colour target"
    );
    // ...and a frame rendered after a pick must be byte-identical to one rendered
    // without any pick at all.
    let frame_again = render(&mut renderer, &target);
    assert_eq!(
        frame_again, frame_only,
        "a pick must not perturb the following colour frame"
    );
}

// ---------------------------------------------------------------------------
// Selection-flags machinery
// ---------------------------------------------------------------------------

#[test]
fn flags_are_off_by_default_and_a_flag_update_leaves_pixels_unchanged() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(96, 96);
    renderer
        .upload(&scene(vec![quad(0.0, 0.0, 8.0, 0x55, false)]))
        .expect("upload");
    assert!(
        !renderer.selection_flags_enabled(),
        "the flags tint is off by default so highlight overlays keep working"
    );
    let before = render(&mut renderer, &target);

    let key = renderer.pick_key_for_batch(0).expect("key");
    renderer
        .set_pick_key_flags(key, FLAG_SELECTED)
        .expect("set flags");
    assert_eq!(renderer.pick_key_flags(key), FLAG_SELECTED);

    let after = render(&mut renderer, &target);
    assert_eq!(
        after, before,
        "with the toggle off, a flag update must not change a single pixel"
    );
}

#[test]
fn enabling_the_toggle_applies_the_selected_tint() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(96, 96);
    renderer
        .upload(&scene(vec![quad(0.0, 0.0, 8.0, 0x66, false)]))
        .expect("upload");
    let plain = render(&mut renderer, &target);
    let key = renderer.pick_key_for_batch(0).expect("key");
    renderer
        .set_pick_key_flags(key, FLAG_SELECTED)
        .expect("set flags");
    renderer.set_selection_flags_enabled(true);
    let tinted = render(&mut renderer, &target);
    assert_ne!(
        tinted, plain,
        "with the toggle on, a selected batch must be tinted"
    );

    // HIDDEN must actually hide the batch again.
    renderer
        .set_pick_key_flags(key, FLAG_HIDDEN)
        .expect("set flags");
    let hidden = render(&mut renderer, &target);
    let (x, y) = world_to_pixel(&camera_2d(), 96, 96, [0.0, 0.0]);
    let i = (y as usize * 96 + x as usize) * 4;
    assert!(
        hidden[i] <= 8 && hidden[i + 1] <= 8 && hidden[i + 2] <= 8,
        "a HIDDEN batch must be discarded, got {:?}",
        &hidden[i..i + 4]
    );
}

#[test]
fn a_flag_update_uploads_only_the_flags_table() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(96, 96);
    renderer
        .upload(&scene(vec![
            quad(-12.0, 0.0, 5.0, 1, false),
            line_band(6.0, 3, 2),
        ]))
        .expect("upload");
    let _ = render(&mut renderer, &target);

    let streams_before = renderer.instance_stream_uploads();
    let chunks_before = renderer.mesh_chunk_uploads();
    let bytes_before = renderer.flags_upload_bytes();

    for (index, key) in [(0usize, 1u32), (1, 2)] {
        assert_eq!(renderer.pick_key_for_batch(index), Some(key));
        renderer
            .set_pick_key_flags(key, FLAG_SELECTED)
            .expect("set flags");
    }
    assert!(
        renderer.flags_upload_bytes() > bytes_before,
        "a flags update must upload the flags table"
    );
    assert_eq!(
        renderer.instance_stream_uploads(),
        streams_before,
        "a flags update must not re-upload the static instance streams"
    );
    assert_eq!(
        renderer.mesh_chunk_uploads(),
        chunks_before,
        "a flags update must not re-upload the static merged chunks"
    );
}

#[test]
fn a_flag_update_outside_the_key_space_is_rejected() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    assert!(renderer.set_pick_key_flags(0, FLAG_SELECTED).is_err());
    assert!(renderer
        .set_pick_key_flags(u32::MAX, FLAG_SELECTED)
        .is_err());
}

// ---------------------------------------------------------------------------
// Id-target format tier
// ---------------------------------------------------------------------------

#[test]
fn r32uint_is_the_default_id_target_when_the_adapter_reports_it_renderable() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let format = renderer.id_target_format().expect("device initialized");
    let expected = Renderer::id_format_for_features(renderer.device_features());
    assert_eq!(
        format, expected,
        "the reported format must match the adapter probe"
    );
    // Whichever tier the adapter lands on, a pick must work through it.
    let target = RenderTarget::new(64, 64);
    renderer
        .upload(&scene(vec![quad(0.0, 0.0, 6.0, 9, false)]))
        .expect("upload");
    let _ = render(&mut renderer, &target);
    let (x, y) = world_to_pixel(&camera_2d(), 64, 64, [0.0, 0.0]);
    let hit = renderer.pick_at(x, y).expect("pick").expect("a hit");
    assert_eq!(hit.source.expect("exact").entity, EntityId(9));
}

#[test]
fn the_packed_rgba8_fallback_picks_identically() {
    let Some(available) = gpu() else {
        return;
    };
    drop(available);
    let target = RenderTarget::new(96, 96);
    let batches = vec![quad(-12.0, 0.0, 5.0, 0x31, false), line_band(6.0, 3, 0x32)];
    let (qx, qy) = world_to_pixel(&camera_2d(), 96, 96, [-12.0, 0.0]);

    let mut picked = Vec::new();
    for format in [IdTargetFormat::R32Uint, IdTargetFormat::PackedRgba8] {
        let Some(gpu) = gpu() else {
            return;
        };
        let mut renderer = init(gpu, Some(format));
        assert_eq!(
            renderer.id_target_format(),
            Some(format),
            "the override must select the requested tier"
        );
        renderer.upload(&scene(batches.clone())).expect("upload");
        let pixels = render(&mut renderer, &target);
        let (lx, ly) = first_drawn_pixel(&pixels, 96, 96).expect("drawn");
        let quad_hit = renderer.pick_at(qx, qy).expect("pick").expect("a hit");
        let line_hit = renderer.pick_at(lx, ly).expect("pick").expect("a hit");
        picked.push((
            quad_hit.source.expect("exact").entity,
            line_hit.source.expect("exact").entity,
        ));
    }
    assert_eq!(
        picked[0], picked[1],
        "the packed fallback must resolve the same identities as R32Uint"
    );
    assert_eq!(picked[0].0, EntityId(0x31));
    assert_eq!(picked[0].1, EntityId(0x32));
}

#[test]
fn a_scene_reset_clears_the_flag_table() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(96, 96);
    renderer
        .upload(&scene(vec![quad(0.0, 0.0, 8.0, 0x77, false)]))
        .expect("upload");
    let key = renderer.pick_key_for_batch(0).expect("key");
    renderer
        .set_pick_key_flags(key, FLAG_SELECTED)
        .expect("set flags");
    renderer.set_selection_flags_enabled(true);
    let tinted = render(&mut renderer, &target);

    // Retire the scene and upload another one. The key space restarts, so the new
    // scene reuses the same key: its bits must be gone, not inherited.
    renderer.clear_batches();
    renderer
        .upload(&scene(vec![quad(0.0, 0.0, 8.0, 0x77, false)]))
        .expect("re-upload");
    assert_eq!(
        renderer.pick_key_for_batch(0),
        Some(key),
        "the key space restarts after a clear"
    );
    assert_eq!(
        renderer.pick_key_flags(key),
        0,
        "a retired key must not keep its flag bits"
    );
    let after = render(&mut renderer, &target);
    assert_ne!(
        after, tinted,
        "the re-uploaded scene must not inherit the retired selection"
    );

    // Re-selecting it in the new scene tints it again, proving the table is live.
    renderer
        .set_pick_key_flags(key, FLAG_SELECTED)
        .expect("set flags");
    let reselected = render(&mut renderer, &target);
    assert_eq!(
        reselected, tinted,
        "re-selecting the same key in the new scene must tint it identically"
    );
}
