//! Hidden entities are not pickable: the id pass must suppress every draw whose
//! whole-entity key carries `FLAG_HIDDEN`, exactly like the colour pass discards
//! such a fragment in-shader (`shaders/*.wgsl`).
//!
//! These tests create a real wgpu device (on the acceptance machine: the Vulkan
//! P620) and assert observable behaviour only: the id pass resolves whatever is
//! drawn *beneath* a hidden entity, the colour frame is byte-identical to a scene
//! where the hidden entity is absent, and the flags machinery (keys, pick table,
//! pulled-prefix reuse counters) is untouched. When no adapter — or no P620 — is
//! available they print an explicit skip that is **not** a pass.
//!
//! Run (serial):
//!
//! ```text
//! cargo test -p cad-render-wgpu --test pick_hidden_flags --locked -- --test-threads=1 --nocapture
//! ```

use cad_domain::{
    DocumentId, EntityId, InstancePath, Point3, Revision, SelectionRef, SubElementId, TaskStamp,
    Transform3,
};
use cad_render_wgpu::headless::{create_headless_gpu, HeadlessGpu};
use cad_render_wgpu::{
    BackendPreference, Camera2d, RenderTarget, RenderTier, Renderer, FLAG_HIDDEN, FLAG_HOVERED,
    FLAG_SELECTED,
};
use cad_resources::{DecodedImage, DecodedImageCache, ResourceKey, ResourceLimits};
use cad_scene::{ImageBatch, RenderBatch, RenderTopology, SceneDelta};
use std::time::Duration;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn source(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn base_batch(topology: RenderTopology, vertices: Vec<[f32; 3]>, entity: u128) -> RenderBatch {
    RenderBatch {
        conics: Vec::new(),
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

/// A horizontal band of line segments at `y`, spanning `x0..x1`, so a scanline is
/// guaranteed to rasterize (a single zero-width line through a pixel centre is not
/// reliably covered on every rasterizer).
fn line_band(y: f32, rows: u32, entity: u128, draw_order: i64) -> RenderBatch {
    let mut vertices = Vec::new();
    for row in 0..rows {
        let offset = y + row as f32 * 0.5;
        vertices.push([-20.0, offset, 0.0]);
        vertices.push([20.0, offset, 0.0]);
    }
    let mut batch = base_batch(RenderTopology::Lines, vertices, entity);
    batch.draw_order = draw_order;
    batch
}

/// A quad mesh (two triangles) centred at `(cx, cy)` with half-size `half`.
fn quad(cx: f32, cy: f32, half: f32, entity: u128) -> RenderBatch {
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
    // Clockwise in a Y-up world, flipped by the index order so `front_face = Ccw`
    // culling keeps it visible.
    batch.indices = vec![[0, 2, 1], [0, 3, 2]];
    batch
}

/// A quad whose source carries a sub-element (the face-overlay shape): the
/// sub-element overlay must keep picking when its entity is hidden, because
/// entity-level flags only ever match whole-entity sources.
fn quad_with_sub_element(cx: f32, cy: f32, half: f32, entity: u128) -> RenderBatch {
    let mut batch = quad(cx, cy, half, entity);
    batch.sources = vec![SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: Some(SubElementId {
            source_key: "face-a".into(),
            topology_revision: Revision(0),
        }),
    }];
    batch
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

fn scene(batches: Vec<RenderBatch>) -> SceneDelta {
    SceneDelta {
        stamp: TaskStamp::new(DocumentId(1), 0),
        added: batches,
        removed_chunks: Vec::new(),
        images: Vec::new(),
        masks: Vec::new(),
    }
}

/// A solid-colour `width`x`height` RGBA8 image.
fn solid_image(width: u32, height: u32, rgba: [u8; 4]) -> DecodedImage {
    let pixels: Vec<u8> = rgba
        .iter()
        .copied()
        .cycle()
        .take((width * height * 4) as usize)
        .collect();
    DecodedImage {
        width,
        height,
        rgba: pixels.into(),
    }
}

/// An image batch mapping the unit square to a 16x16 world square centred on
/// `center` (at `world_per_px == 1` that is a 16 px square, clear of other
/// geometry).
fn image_batch_at(resource: &str, entity: u128, center: [f64; 2]) -> ImageBatch {
    ImageBatch {
        resource: ResourceKey(resource.to_string()),
        transform: Transform3::from_basis(
            Point3 {
                x: 16.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 0.0,
                y: 16.0,
                z: 0.0,
            },
            Point3 {
                x: center[0] - 8.0,
                y: center[1] - 8.0,
                z: 0.0,
            },
        ),
        clip: None,
        alpha: 1.0,
        draw_order: 0,
        sources: vec![source(entity)],
    }
}

// ---------------------------------------------------------------------------
// Device / renderer setup
// ---------------------------------------------------------------------------

/// The acceptance adapter, printed before the test runs. A missing adapter — or a
/// different GPU — skips explicitly and is **not** a pass.
fn gpu() -> Option<HeadlessGpu> {
    match create_headless_gpu(BackendPreference::Auto) {
        Ok(gpu) => {
            eprintln!(
                "hidden-pick test adapter: {} ({}, {})",
                gpu.adapter.name, gpu.adapter.backend, gpu.adapter.device_type
            );
            if !gpu.adapter.name.contains("P620") {
                eprintln!(
                    "skipping hidden-pick GPU test: adapter {} is not the acceptance P620; NOT a pass",
                    gpu.adapter.name
                );
                return None;
            }
            Some(gpu)
        }
        Err(error) => {
            eprintln!("skipping hidden-pick GPU test: {error}; NOT a pass");
            None
        }
    }
}

fn init(gpu: HeadlessGpu) -> Renderer {
    let HeadlessGpu {
        device,
        queue,
        adapter: _,
    } = gpu;
    let mut renderer = Renderer::new(BackendPreference::Auto);
    // The id/colour parity fences below compare exact pixels; pin the universal
    // tier (the accelerated tier has its own fence at the end of this file).
    renderer.set_storage_buffer_support_override(Some(false));
    renderer
        .initialize_with_device(device, queue)
        .expect("initialize renderer");
    renderer.set_poll_timeout(Duration::from_secs(30));
    renderer
}

fn init_accelerated(gpu: HeadlessGpu) -> Option<Renderer> {
    let limits = gpu.device.limits();
    if limits.max_storage_buffers_per_shader_stage < 3
        || limits.max_storage_buffer_binding_size < 64 * 1024
    {
        eprintln!(
            "skipping hidden-pick accelerated fence: adapter lacks vertex-stage storage buffers; NOT a pass"
        );
        return None;
    }
    let HeadlessGpu {
        device,
        queue,
        adapter: _,
    } = gpu;
    let mut renderer = Renderer::new(BackendPreference::Auto);
    renderer.set_storage_buffer_support_override(Some(true));
    renderer
        .initialize_with_device(device, queue)
        .expect("initialize accelerated renderer");
    renderer.set_poll_timeout(Duration::from_secs(30));
    assert_eq!(
        renderer.render_tier(),
        RenderTier::Accelerated,
        "this fence must run on the accelerated tier or it proves nothing"
    );
    Some(renderer)
}

// ---------------------------------------------------------------------------
// Readback helpers
// ---------------------------------------------------------------------------

fn render(renderer: &mut Renderer, target: &RenderTarget) -> Vec<u8> {
    renderer
        .render(camera_2d(), target)
        .expect("render color frame");
    renderer.read_target_rgba().expect("read frame").pixels
}

fn inked(pixels: &[u8], width: u32, x: u32, y: u32) -> bool {
    let i = (y as usize * width as usize + x as usize) * 4;
    pixels[i] > 8 || pixels[i + 1] > 8 || pixels[i + 2] > 8
}

/// The first pixel the colour pass drew that the id pass also resolves.
fn first_pickable_pixel(
    renderer: &mut Renderer,
    pixels: &[u8],
    width: u32,
    height: u32,
) -> Option<(u32, u32)> {
    first_pickable_pixel_where(renderer, pixels, width, height, |_, _| true)
}

/// The first pixel accepted by `accept` that the colour pass drew and the id pass
/// resolves. Used to locate one entity's own pixels without assuming an exact
/// rasterization row.
fn first_pickable_pixel_where(
    renderer: &mut Renderer,
    pixels: &[u8],
    width: u32,
    height: u32,
    accept: impl Fn(u32, u32) -> bool,
) -> Option<(u32, u32)> {
    for y in 0..height {
        for x in 0..width {
            if accept(x, y)
                && inked(pixels, width, x, y)
                && renderer
                    .pick_at(x, y)
                    .expect("pick while scanning")
                    .is_some()
            {
                return Some((x, y));
            }
        }
    }
    None
}

fn entity_at(renderer: &mut Renderer, x: u32, y: u32) -> Option<u128> {
    renderer
        .pick_at(x, y)
        .expect("pick")
        .and_then(|hit| hit.source.map(|source| source.entity.0))
}

// ---------------------------------------------------------------------------
// 1. Hidden ⇒ not pickable, colour discards, unhide restores
// ---------------------------------------------------------------------------

#[test]
fn hidden_entity_is_not_pickable_and_the_colour_pass_discards_it() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(96, 96);
    renderer
        .upload(&scene(vec![quad(0.0, 0.0, 8.0, 0x55)]))
        .expect("upload");

    let key = renderer.pick_key_for_batch(0).expect("batch key");
    let table_len = renderer.pick_table_len();
    let plain = render(&mut renderer, &target);
    let (x, y) = first_pickable_pixel(&mut renderer, &plain, 96, 96).expect("pickable pixel");
    assert_eq!(entity_at(&mut renderer, x, y), Some(0x55));

    // Hide the entity: the colour pass discards it and the id pass must not
    // resolve it either (nothing is beneath it, so the pick is empty).
    renderer.set_selection_flags_enabled(true);
    renderer.set_pick_key_flags(key, FLAG_HIDDEN).expect("hide");
    let hidden = render(&mut renderer, &target);
    assert!(
        !inked(&hidden, 96, x, y),
        "a hidden entity must be discarded by the colour pass"
    );
    assert_eq!(
        entity_at(&mut renderer, x, y),
        None,
        "a hidden entity must not be pickable"
    );

    // Unhide: both passes come back, and the key allocation never moved.
    renderer.set_pick_key_flags(key, 0).expect("unhide");
    let restored = render(&mut renderer, &target);
    assert!(
        inked(&restored, 96, x, y),
        "unhidden entity must draw again"
    );
    assert_eq!(
        entity_at(&mut renderer, x, y),
        Some(0x55),
        "unhiding must restore picking"
    );
    assert_eq!(renderer.pick_key_for_batch(0), Some(key));
    assert_eq!(renderer.pick_table_len(), table_len);
}

// ---------------------------------------------------------------------------
// 2. Hidden over visible: the pick resolves what is beneath, with colour parity
// ---------------------------------------------------------------------------

#[test]
fn hidden_top_entity_reveals_the_entity_beneath_with_colour_parity() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(96, 96);
    // Two identical bands at the same row and depth; the higher `draw_order` is
    // drawn last, so it wins the pixel (`CompareFunction::LessEqual`). Line
    // instancing is off so each band is its own draw with its own key in *both*
    // passes — instanced runs of one stream may be coalesced into a single colour
    // draw, which would make the frame assertion about the coalesced draw rather
    // than about this batch.
    renderer.set_line_instancing(false);
    let bottom = line_band(0.0, 3, 0xB1, 0);
    let top = line_band(0.0, 3, 0xB2, 1);
    renderer
        .upload(&scene(vec![bottom.clone(), top.clone()]))
        .expect("upload");

    let any = render(&mut renderer, &target);
    let (x, y) = first_pickable_pixel(&mut renderer, &any, 96, 96).expect("pickable pixel");
    assert_eq!(
        entity_at(&mut renderer, x, y),
        Some(0xB2),
        "the fixture must start with the top band winning the pixel"
    );
    let top_key = renderer.pick_key_for_batch(1).expect("top band key");
    let bottom_key = renderer.pick_key_for_batch(0).expect("bottom band key");

    // Hide the top band: the bottom band must be revealed in *both* passes.
    renderer.set_selection_flags_enabled(true);
    renderer
        .set_pick_key_flags(top_key, FLAG_HIDDEN)
        .expect("hide the top band");
    let hidden_top = render(&mut renderer, &target);
    assert_eq!(
        entity_at(&mut renderer, x, y),
        Some(0xB1),
        "the pick must fall through to the visible band beneath"
    );

    // Colour parity: a scene with the hidden entity removed must be
    // byte-identical to the scene with it hidden (a hidden draw contributes
    // nothing to either pass).
    renderer.clear_batches();
    renderer
        .upload(&scene(vec![bottom.clone()]))
        .expect("upload bottom only");
    let bottom_only = render(&mut renderer, &target);
    assert_eq!(
        hidden_top, bottom_only,
        "hiding the top band must be indistinguishable from not having it"
    );
    assert_eq!(entity_at(&mut renderer, x, y), Some(0xB1));

    // Restore: uploading both again with no flags reproduces the original frame
    // and the top band picks again.
    renderer.clear_batches();
    renderer
        .upload(&scene(vec![bottom, top]))
        .expect("upload both again");
    let restored = render(&mut renderer, &target);
    assert_eq!(
        restored, any,
        "clearing the flags must restore the exact original frame"
    );
    assert_eq!(entity_at(&mut renderer, x, y), Some(0xB2));
    assert_eq!(renderer.pick_key_for_batch(0), Some(bottom_key));
    assert_eq!(renderer.pick_key_for_batch(1), Some(top_key));
}

// ---------------------------------------------------------------------------
// 3. The toggle off ⇒ a flagged entity stays pickable (shader short-circuit)
// ---------------------------------------------------------------------------

#[test]
fn disabling_selection_flags_makes_a_hidden_entity_pickable_again() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(96, 96);
    renderer
        .upload(&scene(vec![quad(0.0, 0.0, 8.0, 0x66)]))
        .expect("upload");
    let key = renderer.pick_key_for_batch(0).expect("batch key");
    let plain = render(&mut renderer, &target);
    let (x, y) = first_pickable_pixel(&mut renderer, &plain, 96, 96).expect("pickable pixel");

    // Flag the entity while the toggle is off: the flags path is inert, exactly
    // like the colour shader's `flags_ctl.enabled == 0` short-circuit.
    renderer
        .set_pick_key_flags(key, FLAG_HIDDEN)
        .expect("flag while disabled");
    let disabled = render(&mut renderer, &target);
    assert_eq!(
        disabled, plain,
        "with the toggle off a flag update must not change a single pixel"
    );
    assert_eq!(
        entity_at(&mut renderer, x, y),
        Some(0x66),
        "with the toggle off the entity must stay pickable"
    );

    // Turning the toggle on applies the same bits immediately (no commit).
    renderer.set_selection_flags_enabled(true);
    assert_eq!(
        entity_at(&mut renderer, x, y),
        None,
        "enabling the toggle must apply the already-set HIDDEN bit"
    );
    renderer.set_selection_flags_enabled(false);
    assert_eq!(
        entity_at(&mut renderer, x, y),
        Some(0x66),
        "turning the toggle back off must make it pickable again"
    );
}

// ---------------------------------------------------------------------------
// 4. A hidden image is not pickable
// ---------------------------------------------------------------------------

#[test]
fn hidden_image_is_not_pickable() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(96, 96);
    let mut cache = DecodedImageCache::new();
    cache
        .insert(
            ResourceKey("red".into()),
            solid_image(2, 2, [255, 0, 0, 255]),
            &ResourceLimits::default(),
        )
        .expect("insert image");
    renderer
        .upload_images(&[image_batch_at("red", 0x77, [0.0, 0.0])], &cache)
        .expect("upload image");

    let key = renderer.pick_key_for_image(0).expect("image key");
    let plain = render(&mut renderer, &target);
    let (x, y) = first_pickable_pixel(&mut renderer, &plain, 96, 96).expect("pickable pixel");
    assert_eq!(entity_at(&mut renderer, x, y), Some(0x77));

    renderer.set_selection_flags_enabled(true);
    renderer
        .set_pick_key_flags(key, FLAG_HIDDEN)
        .expect("hide the image");
    let hidden = render(&mut renderer, &target);
    assert!(
        !inked(&hidden, 96, x, y),
        "a hidden image must be discarded by the colour pass"
    );
    assert_eq!(
        entity_at(&mut renderer, x, y),
        None,
        "a hidden image must not be pickable"
    );

    renderer.set_pick_key_flags(key, 0).expect("unhide");
    assert_eq!(
        entity_at(&mut renderer, x, y),
        Some(0x77),
        "unhiding the image must restore picking"
    );
    assert_eq!(renderer.pick_key_for_image(0), Some(key));
}

// ---------------------------------------------------------------------------
// 5. A flag update is reflected immediately, with no stale id pixels
// ---------------------------------------------------------------------------

#[test]
fn flag_updates_are_reflected_without_a_commit_and_without_re_uploads() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(96, 96);
    renderer
        .upload(&scene(vec![quad(0.0, 0.0, 8.0, 0x88)]))
        .expect("upload");
    let key = renderer.pick_key_for_batch(0).expect("batch key");
    let plain = render(&mut renderer, &target);
    let (x, y) = first_pickable_pixel(&mut renderer, &plain, 96, 96).expect("pickable pixel");

    let uploads = renderer.instance_stream_uploads();
    let table_len = renderer.pick_table_len();
    renderer.set_selection_flags_enabled(true);

    renderer.set_pick_key_flags(key, FLAG_HIDDEN).expect("hide");
    // No commit anywhere: the id pass re-reads the flags mirror every pick, so the
    // very next pick (and a repeated one) must already see the new bit.
    assert_eq!(entity_at(&mut renderer, x, y), None);
    assert_eq!(
        entity_at(&mut renderer, x, y),
        None,
        "a repeated pick must not resurrect a stale id pixel"
    );

    renderer.set_pick_key_flags(key, 0).expect("unhide");
    assert_eq!(entity_at(&mut renderer, x, y), Some(0x88));

    assert_eq!(
        renderer.instance_stream_uploads(),
        uploads,
        "flag updates must never re-upload geometry"
    );
    assert_eq!(renderer.pick_table_len(), table_len);
    assert_eq!(renderer.pick_key_for_batch(0), Some(key));
}

// ---------------------------------------------------------------------------
// 6. Only HIDDEN suppresses: SELECTED/HOVERED stay pickable
// ---------------------------------------------------------------------------

#[test]
fn selected_and_hovered_entities_stay_pickable() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(96, 96);
    renderer
        .upload(&scene(vec![quad(0.0, 0.0, 8.0, 0x99)]))
        .expect("upload");
    let key = renderer.pick_key_for_batch(0).expect("batch key");
    let plain = render(&mut renderer, &target);
    let (x, y) = first_pickable_pixel(&mut renderer, &plain, 96, 96).expect("pickable pixel");
    renderer.set_selection_flags_enabled(true);

    for bits in [FLAG_SELECTED, FLAG_HOVERED, FLAG_SELECTED | FLAG_HOVERED] {
        renderer.set_pick_key_flags(key, bits).expect("tint");
        assert_eq!(
            entity_at(&mut renderer, x, y),
            Some(0x99),
            "a {bits:#x}-flagged entity must stay pickable"
        );
    }
    // HIDDEN wins even when it shares the key with a tint bit.
    renderer
        .set_pick_key_flags(key, FLAG_HIDDEN | FLAG_SELECTED)
        .expect("hide");
    assert_eq!(entity_at(&mut renderer, x, y), None);
}

// ---------------------------------------------------------------------------
// 7. Entity-level hidden must keep sub-element overlay keys picking
// ---------------------------------------------------------------------------

#[test]
fn hidden_entity_keeps_its_sub_element_overlay_pickable() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(96, 96);
    // The whole-entity batch sits left, its face overlay right, so one pixel each.
    // Mesh merging is off so each batch is its own draw with its own key in both
    // passes (a merged chunk may coalesce contiguous runs into a single colour
    // draw, which would make the frame assertion about the chunk, not this batch).
    renderer.set_mesh_merging(false);
    renderer
        .upload(&scene(vec![
            quad(-20.0, 0.0, 6.0, 0xE1),
            quad_with_sub_element(20.0, 0.0, 6.0, 0xE1),
        ]))
        .expect("upload");
    let base_key = renderer.pick_key_for_batch(0).expect("base key");
    let face_key = renderer.pick_key_for_batch(1).expect("face key");

    let plain = render(&mut renderer, &target);
    let (bx, by) = first_pickable_pixel_where(&mut renderer, &plain, 96, 96, |x, _| x < 48)
        .expect("the whole-entity quad must cover its own pickable pixels");
    let (fx, fy) = first_pickable_pixel_where(&mut renderer, &plain, 96, 96, |x, _| x > 48)
        .expect("the face overlay must cover its own pickable pixels");
    assert_eq!(entity_at(&mut renderer, bx, by), Some(0xE1));
    assert_eq!(entity_at(&mut renderer, fx, fy), Some(0xE1));

    renderer.set_selection_flags_enabled(true);
    renderer
        .set_entity_flags(&[(source(0xE1), FLAG_HIDDEN)])
        .expect("hide the entity");
    assert_eq!(
        renderer.pick_key_flags(base_key),
        FLAG_HIDDEN,
        "the whole-entity batch must carry the entity flag"
    );
    assert_eq!(
        renderer.pick_key_flags(face_key),
        0,
        "an entity-level flag must not reach the sub-element overlay key"
    );

    let hidden = render(&mut renderer, &target);
    assert!(
        !inked(&hidden, 96, bx, by),
        "the hidden whole-entity draw must be discarded"
    );
    assert!(
        inked(&hidden, 96, fx, fy),
        "the sub-element overlay must still be drawn"
    );
    assert_eq!(
        entity_at(&mut renderer, bx, by),
        None,
        "the hidden whole-entity batch must not be pickable"
    );
    let overlay_hit = renderer
        .pick_at(fx, fy)
        .expect("pick")
        .expect("the sub-element overlay must keep picking under a hidden entity");
    assert_eq!(overlay_hit.entity_key, face_key);
    assert!(
        overlay_hit
            .source
            .is_some_and(|source| source.sub_element.is_some()),
        "the overlay must still resolve as the sub-element source"
    );
}

// ---------------------------------------------------------------------------
// 8. The same suppression under an active pulled-prefix reuse
// ---------------------------------------------------------------------------

#[test]
fn hidden_entity_suppression_keeps_pulled_prefix_reuse_untouched() {
    let Some(gpu) = gpu() else {
        return;
    };
    let Some(mut renderer) = init_accelerated(gpu) else {
        return;
    };
    let target = RenderTarget::new(96, 96);
    let base_band = line_band(-10.0, 3, 0xC1, 0);
    let suffix_band = line_band(10.0, 3, 0xC2, 0);

    // Base, then one suffix commit (the full warm-up that establishes the
    // retained prefix).
    let staged = renderer
        .prepare_upload_batches(std::slice::from_ref(&base_band))
        .expect("stage base");
    renderer.commit_upload(staged, 0).expect("commit base");
    let staged = renderer
        .prepare_upload_batches(std::slice::from_ref(&suffix_band))
        .expect("stage suffix");
    renderer.commit_upload(staged, 1).expect("commit suffix");
    let reuses = renderer.pulled_prefix_reuses();
    let rebuilds = renderer.pulled_prefix_rebuilds();
    assert_eq!(reuses, 0, "the warm-up must not count as a reuse");
    assert!(rebuilds >= 1);

    let base_key = renderer.pick_key_for_batch(0).expect("base key");
    let suffix_key = renderer.pick_key_for_batch(1).expect("suffix key");
    let plain = render(&mut renderer, &target);
    // The bands sit at world y = -10 and +10; the screen y axis is flipped, so scan
    // each half of the target rather than assuming an exact rasterization row.
    let (bx, by) = first_pickable_pixel_where(&mut renderer, &plain, 96, 96, |_, y| y > 48)
        .expect("the base band must cover pickable pixels");
    let (sx, sy) = first_pickable_pixel_where(&mut renderer, &plain, 96, 96, |_, y| y < 48)
        .expect("the suffix band must cover pickable pixels");
    assert_eq!(entity_at(&mut renderer, bx, by), Some(0xC1));
    assert_eq!(entity_at(&mut renderer, sx, sy), Some(0xC2));

    // Hide the suffix entity, then the base entity: both stop picking, the other
    // keeps picking meanwhile, and nothing in the flags path touches the pulled
    // prefix machinery.
    renderer.set_selection_flags_enabled(true);
    renderer
        .set_pick_key_flags(suffix_key, FLAG_HIDDEN)
        .expect("hide suffix");
    assert_eq!(entity_at(&mut renderer, sx, sy), None);
    assert_eq!(entity_at(&mut renderer, bx, by), Some(0xC1));
    renderer
        .set_pick_key_flags(base_key, FLAG_HIDDEN)
        .expect("hide base");
    assert_eq!(entity_at(&mut renderer, bx, by), None);
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        reuses,
        "a flag update must not count as a reuse"
    );
    assert_eq!(
        renderer.pulled_prefix_rebuilds(),
        rebuilds,
        "a flag update must not rebuild the pulled prefix"
    );

    // A real suffix commit still reuses the retained base (the flags path did not
    // disturb the checkpoint machinery).
    let staged = renderer
        .prepare_upload_batches(&[line_band(14.0, 3, 0xC3, 0)])
        .expect("stage second suffix");
    renderer
        .commit_upload(staged, 1)
        .expect("commit second suffix");
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        reuses + 1,
        "the next suffix commit must still reuse the retained base"
    );
}
