//! Pulled-prefix (retained base) equivalence fences — native GPU contracts.
//!
//! The accelerated tier collapses opaque classes to one draw per class by pulling
//! primitives from read-only vertex-stage storage buffers, and it retains the
//! **base prefix** segments across suffix commits (the annotation overlay) so a
//! pan/overlay update can reuse them instead of rebuilding the whole scene.
//!
//! These fences pin that contract on a real device:
//!  * a full establish followed by a warmup suffix, then a **second suffix commit
//!    must actually reuse** the retained prefix (`pulled_prefix_reuses()` climbs,
//!    `pulled_prefix_rebuilds()` does not) — the regression this file exists for;
//!  * a reused-suffix scene must be **pixel- and pick-identical** to the same
//!    scene committed in one full pass;
//!  * repeated overlay replace / remove / re-add keeps the base and keeps reusing;
//!  * a **full base replacement** invalidates the retained prefix (no reuse, a
//!    later suffix re-establishes);
//!  * a device/override binding limit too small for the pulled line class falls
//!    back to the universal per-run path and never claims a reuse.
//!
//! Each test creates a real wgpu device on the adapter `create_headless_gpu`
//! selects (default Vulkan ICD → the project's Quadro P620; Mesa lavapipe on a
//! GPU-less machine) and prints it. Tests **skip** (never pass) when no adapter
//! exists or the device cannot host the accelerated tier, so a skip is never an
//! evidence claim. The pick comparison uses semantic [`SelectionRef`] identity,
//! never the raw key, because pick keys are allocation-order dependent.
//!
//! Diagnostic-only: no renderer/pick/commit semantics are asserted beyond
//! equivalence, and no storage limit or override is changed in production code.

use cad_domain::{DocumentId, EntityId, InstancePath, Point3, SelectionRef, Transform3};
use cad_render_wgpu::headless::{create_headless_gpu, HeadlessGpu};
use cad_render_wgpu::{
    BackendPreference, Camera2d, PreparedUpload, RenderTarget, RenderTier, Renderer, FLAG_HIDDEN,
    FLAG_SELECTED,
};
use cad_resources::{DecodedImage, DecodedImageCache, ResourceKey, ResourceLimits};
use cad_scene::{ImageBatch, LinetypePattern, RenderBatch, RenderTopology, DEFAULT_BATCH_COLOR};

const TARGET: u32 = 64;

fn source(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

/// Create a headless device, or `None` (skip) with a printed reason.
fn gpu() -> Option<HeadlessGpu> {
    match create_headless_gpu(BackendPreference::Auto) {
        Ok(gpu) => Some(gpu),
        Err(error) => {
            eprintln!("skipping pulled-prefix GPU contract: {error}");
            None
        }
    }
}

/// Whether this device can host the accelerated tier (vertex-stage storage
/// buffers). A device that cannot is skipped, not failed.
fn supports_storage(gpu: &HeadlessGpu) -> bool {
    let limits = gpu.device.limits();
    limits.max_storage_buffers_per_shader_stage >= 3
        && limits.max_storage_buffer_binding_size >= 64 * 1024
        && gpu.adapter.backend != "webgl2"
}

/// Initialize an **accelerated-tier** renderer (forced, so the fence is
/// deterministic on any capable adapter), printing the adapter once.
fn init_accelerated(gpu: &HeadlessGpu) -> Renderer {
    eprintln!(
        "pulled-prefix contract adapter: {} ({}, {})",
        gpu.adapter.name, gpu.adapter.backend, gpu.adapter.device_type
    );
    let mut renderer = Renderer::new(BackendPreference::Auto);
    renderer.set_storage_buffer_support_override(Some(true));
    renderer
        .initialize_with_device(gpu.device.clone(), gpu.queue.clone())
        .expect("initialize renderer");
    renderer
}

fn camera_2d() -> Camera2d {
    Camera2d {
        center: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        world_per_px: 2.0 / TARGET as f64,
        z_plane: 0.0,
    }
}

/// A `Lines` batch with an explicit number of stacked segments (`rows`) at a
/// given row `spacing` in world units. One batch becomes one pulled instance
/// stream, so `rows` controls the stream size; `spacing` controls how solid the
/// band is (a band thinner than a pixel can be entirely pick-fringe).
fn line_band_rows(y: f32, entity: u128, alpha: f32, rows: u32, spacing: f32) -> RenderBatch {
    line_span_rows(-0.9, 0.9, y, entity, alpha, rows, spacing)
}

/// [`line_band_rows`] with an explicit horizontal world span `x0..x1` (used to
/// make an overlay cross only part of the resident image).
fn line_span_rows(
    x0: f32,
    x1: f32,
    y: f32,
    entity: u128,
    alpha: f32,
    rows: u32,
    spacing: f32,
) -> RenderBatch {
    let mut vertices = Vec::new();
    for row in 0..rows.max(1) {
        let offset = y + row as f32 * spacing;
        vertices.push([x0, offset, 0.0]);
        vertices.push([x1, offset, 0.0]);
    }
    RenderBatch {
        conics: Vec::new(),
        local_origin: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        topology: RenderTopology::Lines,
        vertices,
        normals: Vec::new(),
        colors: Vec::new(),
        indices: Vec::new(),
        edges: Vec::new(),
        mirrored: false,
        alpha,
        color: DEFAULT_BATCH_COLOR,
        color_unresolved: true,
        lineweight: 0.0,
        lineweight_unresolved: true,
        linetype: LinetypePattern::continuous(),
        linetype_unresolved: true,
        sources: vec![source(entity)],
        draw_order: 0,
    }
}

/// A `Lines` batch: a solid band centred at `y` (see [`BAND_ROWS`]).
fn line_band(y: f32, entity: u128, alpha: f32) -> RenderBatch {
    line_band_rows(y, entity, alpha, BAND_ROWS, BAND_SPACING)
}

/// The retained base prefix: several distinct line bands (distinct entities) so
/// the pulled line class owns real base segments.
fn base_batches() -> Vec<RenderBatch> {
    (0..4)
        .map(|i| {
            let y = -0.8 + i as f32 * 0.5;
            line_band(y, (i + 1) as u128, 1.0)
        })
        .collect()
}

/// A single overlay band (the annotation suffix) at a distinct entity and y.
///
/// Same solid basis as [`line_band`] so an interior pixel is fully covered; a
/// sub-pixel-thin band can be entirely pick-fringe and would make the fence
/// vacuous.
fn overlay_batch(entity: u128, y: f32) -> Vec<RenderBatch> {
    vec![line_band_rows(y, entity, 1.0, BAND_ROWS, BAND_SPACING)]
}

fn stage(renderer: &Renderer, batches: &[RenderBatch]) -> PreparedUpload {
    renderer
        .prepare_upload_batches(batches)
        .expect("stage batches")
}

/// Commit `batches` as the suffix after `keep_prefix` (0 replaces the scene).
fn commit(renderer: &mut Renderer, batches: &[RenderBatch], keep_prefix: usize) {
    let prepared = stage(renderer, batches);
    renderer
        .commit_upload(prepared, keep_prefix)
        .expect("commit upload");
}

fn render_pixels(renderer: &mut Renderer) -> Vec<u8> {
    let target = RenderTarget::new(TARGET, TARGET);
    renderer.render(camera_2d(), &target).expect("render frame");
    renderer.read_target_rgba().expect("read frame").pixels
}

fn is_blank(pixels: &[u8]) -> bool {
    match pixels.chunks_exact(4).next() {
        Some(first) => pixels.chunks_exact(4).all(|px| px == first),
        None => true,
    }
}

/// Known fixture geometry (world Y of each band). The base bands are the
/// `base_batches()` rows; the two overlay bands are kept ≥8 px away from every
/// base band and from each other so each chosen pixel belongs to exactly one
/// entity.
const BASE_PICK_Y: f32 = 0.2;
const WARMUP_OVERLAY_Y: f32 = -0.55;
const COMPARED_OVERLAY_Y: f32 = 0.45;

/// Every band is a **solid** stack of `BAND_ROWS` segments spaced exactly one
/// pixel apart (`BAND_SPACING`), so an interior pixel is fully covered in both
/// the colour and id passes. A sub-pixel-thin band is entirely pick-fringe.
const BAND_ROWS: u32 = 5;
const BAND_SPACING: f32 = 2.0 / TARGET as f32; // 1 px per row, matches `camera_2d`

/// The interior (fully covered) pixel of the solid band at `world_y`, derived
/// **analytically from the camera's own parameters** and the band geometry
/// (`BAND_ROWS`/`BAND_SPACING`) — never from "nearest non-black pixel", which is
/// meaningless here because the CAD background is a non-zero colour.
///
/// Screen-Y convention (verified against the device in the gate-3 diagnostic):
/// `screen_y = TARGET/2 - world_y / world_per_px` (y-down, world-up). The chosen
/// row's pixel **centre** sits at the band's vertical centre, comfortably away
/// from the anti-aliased edges; x is the band's horizontal centre.
fn band_interior_pixel(world_y: f32) -> (u32, u32) {
    band_interior_pixel_rows(world_y, BAND_ROWS, BAND_SPACING)
}

/// [`band_interior_pixel`] for an explicit row count/spacing (used by the
/// oversized fallback band, whose geometry basis differs).
fn band_interior_pixel_rows(world_y: f32, rows: u32, spacing: f32) -> (u32, u32) {
    let per_px = 2.0 / TARGET as f32; // matches `camera_2d` world_per_px
    let world_max = world_y + (rows.max(1) - 1) as f32 * spacing;
    let top = TARGET as f32 / 2.0 - world_max / per_px;
    let bottom = TARGET as f32 / 2.0 - world_y / per_px;
    let y = ((top + bottom) / 2.0)
        .round()
        .clamp(0.0, (TARGET - 1) as f32) as u32;
    (TARGET / 2, y)
}

/// The RGBA at `(x, y)` in a packed readback.
fn rgb(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * TARGET + x) * 4) as usize;
    [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
}

/// Assert the analytically chosen interior pixel is actually drawn in `pixels`
/// (guards against a future geometry/sign drift making the pick fence vacuous).
fn assert_drawn(pixels: &[u8], xy: (u32, u32), what: &str) {
    let [r, g, b, _] = rgb(pixels, xy.0, xy.1);
    assert!(
        u32::from(r) + u32::from(g) + u32::from(b) > 0,
        "{what}: the chosen interior pixel {xy:?} must be drawn"
    );
}

/// One `pick_at` per coordinate (bounded: the caller passes only the handful of
/// chosen core pixels), keeping misses visible as `None`.
fn pick_map(
    renderer: &mut Renderer,
    coords: &[(u32, u32)],
) -> Vec<((u32, u32), Option<SelectionRef>)> {
    coords
        .iter()
        .map(|&(x, y)| {
            let hit = renderer.pick_at(x, y).expect("pick");
            ((x, y), hit.and_then(|hit| hit.source))
        })
        .collect()
}

/// The resolved identity at `xy` in a `pick_map` result (semantic, not the key).
fn source_at(map: &[((u32, u32), Option<SelectionRef>)], xy: (u32, u32)) -> Option<SelectionRef> {
    map.iter()
        .find(|(coord, _)| *coord == xy)
        .and_then(|(_, source)| source.clone())
}

/// A distinct base entity id used for the base-core pick assertions.
const BASE_PICK_ENTITY: u128 = 3;

// ---------------------------------------------------------------------------
// Contract 1 (primary regression): full establish -> warmup -> second suffix
// MUST reuse the retained base prefix.
// ---------------------------------------------------------------------------

#[test]
fn accelerated_prefix_full_establish_then_second_suffix_reuses() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        eprintln!("skipping: device cannot host the accelerated tier");
        return;
    }
    let mut renderer = init_accelerated(&gpu);
    assert_eq!(renderer.render_tier(), RenderTier::Accelerated);

    let base = base_batches();
    let base_len = base.len();
    const OVERLAY_1: u128 = 900;
    const OVERLAY_2: u128 = 901;
    const OVERLAY_3: u128 = 902;

    // 1) Full base commit: no prefix is retained yet (keep_prefix == 0).
    commit(&mut renderer, &base, 0);
    assert_eq!(renderer.pulled_prefix_reuses(), 0);
    assert_eq!(
        renderer.pulled_prefix_rebuilds(),
        0,
        "a keep_prefix==0 commit must not establish a retained prefix"
    );

    // 2) Warmup: the first suffix commit at the base boundary is a full rebuild
    //    that RETAINS the base prefix.
    commit(&mut renderer, &overlay_batch(OVERLAY_1, 0.7), base_len);
    assert_eq!(renderer.pulled_prefix_reuses(), 0);
    assert_eq!(
        renderer.pulled_prefix_rebuilds(),
        1,
        "the warmup suffix must establish the retained base prefix"
    );
    let line_segments_after_warmup = renderer.pulled_line_segment_count();
    assert!(
        line_segments_after_warmup > 0,
        "the accelerated line class must own real pulled segments for this fence"
    );

    // 3) Second suffix commit at the SAME base boundary MUST reuse, not rebuild.
    //    This is the regression the file exists for: before the fix the fast path
    //    is unreachable, so the reuse counter never advances.
    let reuses = renderer.pulled_prefix_reuses();
    let rebuilds = renderer.pulled_prefix_rebuilds();
    commit(&mut renderer, &overlay_batch(OVERLAY_2, 0.75), base_len);
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        reuses + 1,
        "the second suffix commit must reuse the retained base prefix"
    );
    assert_eq!(
        renderer.pulled_prefix_rebuilds(),
        rebuilds,
        "a reused suffix commit must not rebuild the pulled base"
    );

    // A third suffix keeps reusing.
    commit(&mut renderer, &overlay_batch(OVERLAY_3, 0.8), base_len);
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        reuses + 2,
        "every further suffix at the same boundary keeps reusing"
    );
    assert_eq!(renderer.pulled_prefix_rebuilds(), rebuilds);
}

// ---------------------------------------------------------------------------
// Contract 2: a reused-suffix scene is pixel- and pick-identical to the same
// scene committed in one full pass.
// ---------------------------------------------------------------------------

/// Build a renderer whose final scene is `base + overlay` reached through a
/// warmup then a reusing suffix commit (returns the renderer and reuse count).
fn build_reused(
    gpu: &HeadlessGpu,
    base: &[RenderBatch],
    overlay: &[RenderBatch],
) -> (Renderer, u64) {
    let mut renderer = init_accelerated(gpu);
    commit(&mut renderer, base, 0);
    // Warmup suffix (establishes the prefix), at its own band so its pixels never
    // collide with the compared overlay's.
    commit(
        &mut renderer,
        &overlay_batch(7, WARMUP_OVERLAY_Y),
        base.len(),
    );
    // The suffix we actually compare (reuses).
    let before = renderer.pulled_prefix_reuses();
    commit(&mut renderer, overlay, base.len());
    let after = renderer.pulled_prefix_reuses();
    assert_eq!(
        after,
        before + 1,
        "the compared suffix must have reused the retained base"
    );
    (renderer, after)
}

fn build_full(gpu: &HeadlessGpu, batches: &[RenderBatch]) -> Renderer {
    let mut renderer = init_accelerated(gpu);
    commit(&mut renderer, batches, 0);
    renderer
}

/// A single full commit under the **same** pulled segment cap, so a degraded
/// (fallback) scene is compared like-for-like against a forced full commit.
fn build_full_with_cap(gpu: &HeadlessGpu, batches: &[RenderBatch], cap: usize) -> Renderer {
    let mut renderer = init_accelerated(gpu);
    renderer.set_pulled_line_segment_elems_override(Some(cap));
    commit(&mut renderer, batches, 0);
    renderer
}

#[test]
fn suffix_reuse_matches_a_single_full_commit_pixels_and_picks() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        eprintln!("skipping: device cannot host the accelerated tier");
        return;
    }
    let base = base_batches();
    const OVERLAY: u128 = 910;
    let overlay = overlay_batch(OVERLAY, COMPARED_OVERLAY_Y);

    let (mut reused, reuse_count) = build_reused(&gpu, &base, &overlay);
    assert!(reuse_count >= 1, "the reused path must be exercised");

    let mut composed = base.clone();
    composed.extend(overlay.clone());
    let mut full = build_full(&gpu, &composed);

    let reused_px = render_pixels(&mut reused);
    let full_px = render_pixels(&mut full);
    assert!(!is_blank(&reused_px), "the reused frame must not be blank");
    assert_eq!(
        reused_px, full_px,
        "a reused-suffix scene must be byte-identical to the same scene committed in one full pass \
         (same device, so exact pixels are justified)"
    );

    // Choose ONE interior (fully covered) pixel per entity from the camera-derived
    // band geometry — no per-pixel GPU pick calls — then pick exactly those.
    let overlay_xy = band_interior_pixel(COMPARED_OVERLAY_Y);
    let base_xy = band_interior_pixel(BASE_PICK_Y);
    assert_ne!(
        overlay_xy, base_xy,
        "the two chosen interior pixels must be distinct coordinates"
    );
    assert_drawn(&reused_px, overlay_xy, "overlay band");
    assert_drawn(&reused_px, base_xy, "base band");

    let coords = [overlay_xy, base_xy];
    let reused_picks = pick_map(&mut reused, &coords);
    let full_picks = pick_map(&mut full, &coords);
    assert_eq!(
        reused_picks, full_picks,
        "reused and full commits must resolve the SAME chosen pixels to the same identities \
         (semantic SelectionRef, never the raw key)"
    );
    assert_eq!(
        source_at(&reused_picks, overlay_xy).map(|s| s.entity),
        Some(EntityId(OVERLAY)),
        "the overlay suffix entity must be picked at its core pixel"
    );
    assert_eq!(
        source_at(&reused_picks, base_xy).map(|s| s.entity),
        Some(EntityId(BASE_PICK_ENTITY)),
        "the retained base entity must be picked at its core pixel"
    );
}

// ---------------------------------------------------------------------------
// Contract 3: repeated overlay replace / remove / re-add keeps the base and
// keeps reusing.
// ---------------------------------------------------------------------------

#[test]
fn repeated_overlay_replace_remove_readd_keeps_base_and_reuses() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        eprintln!("skipping: device cannot host the accelerated tier");
        return;
    }
    let mut renderer = init_accelerated(&gpu);
    let base = base_batches();
    let base_len = base.len();
    const OV_A: u128 = 920;
    const OV_B: u128 = 921;

    commit(&mut renderer, &base, 0);
    commit(
        &mut renderer,
        &overlay_batch(OV_A, COMPARED_OVERLAY_Y),
        base_len,
    ); // establish
    assert_eq!(renderer.pulled_prefix_rebuilds(), 1);

    // replace A -> B
    commit(
        &mut renderer,
        &overlay_batch(OV_B, COMPARED_OVERLAY_Y),
        base_len,
    );
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        1,
        "the replace must reuse the retained base prefix"
    );

    // Chosen interior pixels before the removal (base geometry is unchanged by it).
    let before_px = render_pixels(&mut renderer);
    let overlay_xy = band_interior_pixel(COMPARED_OVERLAY_Y);
    let base_xy = band_interior_pixel(BASE_PICK_Y);
    assert_drawn(&before_px, overlay_xy, "overlay band before removal");
    assert_drawn(&before_px, base_xy, "base band before removal");
    let coords = [overlay_xy, base_xy];
    let before_picks = pick_map(&mut renderer, &coords);
    assert_eq!(
        source_at(&before_picks, overlay_xy).map(|s| s.entity),
        Some(EntityId(OV_B)),
        "the overlay must pick before the removal"
    );

    // remove (empty suffix) — assert IMMEDIATELY after the removal, before re-adding.
    commit(&mut renderer, &[], base_len);
    let _removed_px = render_pixels(&mut renderer);
    let after_remove = pick_map(&mut renderer, &coords);
    assert_ne!(
        source_at(&after_remove, overlay_xy).map(|s| s.entity),
        Some(EntityId(OV_B)),
        "the removed overlay must no longer pick at its former core pixel"
    );
    assert_eq!(
        source_at(&after_remove, base_xy).map(|s| s.entity),
        Some(EntityId(BASE_PICK_ENTITY)),
        "a retained base entity must still pick after the overlay removal"
    );

    // re-add B — it picks again, and the counters are unchanged (still reusing).
    commit(
        &mut renderer,
        &overlay_batch(OV_B, COMPARED_OVERLAY_Y),
        base_len,
    );
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        3,
        "replace, remove and re-add must each reuse the retained base prefix"
    );
    assert_eq!(
        renderer.pulled_prefix_rebuilds(),
        1,
        "no suffix step may rebuild the retained base"
    );
    let _readded_px = render_pixels(&mut renderer);
    let after_readd = pick_map(&mut renderer, &coords);
    assert_eq!(
        source_at(&after_readd, overlay_xy).map(|s| s.entity),
        Some(EntityId(OV_B)),
        "the re-added overlay must pick again"
    );
    assert_eq!(
        source_at(&after_readd, base_xy).map(|s| s.entity),
        Some(EntityId(BASE_PICK_ENTITY)),
        "the retained base entity must keep picking through every step"
    );
}

// ---------------------------------------------------------------------------
// Contract 4: a full base replacement invalidates the retained prefix.
// ---------------------------------------------------------------------------

#[test]
fn full_base_replacement_invalidates_the_retained_prefix() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        eprintln!("skipping: device cannot host the accelerated tier");
        return;
    }
    let mut renderer = init_accelerated(&gpu);
    let base = base_batches();
    let base_len = base.len();

    commit(&mut renderer, &base, 0);
    commit(&mut renderer, &overlay_batch(930, 0.7), base_len); // establish
    let reuses = renderer.pulled_prefix_reuses();
    let rebuilds = renderer.pulled_prefix_rebuilds();
    assert_eq!(rebuilds, 1);

    // A new full base (keep_prefix == 0) replaces the scene and must invalidate.
    let new_base = vec![
        line_band(-0.5, 40, 1.0),
        line_band(0.0, 41, 1.0),
        line_band(0.5, 42, 1.0),
    ];
    commit(&mut renderer, &new_base, 0);
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        reuses,
        "a full base replacement must never be reported as a reuse"
    );
    assert_eq!(
        renderer.pulled_prefix_rebuilds(),
        rebuilds,
        "a keep_prefix==0 commit must not establish a prefix"
    );

    // A suffix at the NEW boundary re-establishes (not reuses the old base).
    commit(&mut renderer, &overlay_batch(931, 0.7), new_base.len());
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        reuses,
        "the old retained prefix must be gone"
    );
    assert_eq!(
        renderer.pulled_prefix_rebuilds(),
        rebuilds + 1,
        "the suffix at the new boundary must establish a fresh prefix"
    );
}

// ---------------------------------------------------------------------------
// Contract 5: a binding limit too small for the pulled line class falls back to
// the universal per-run path and never claims a reuse.
// ---------------------------------------------------------------------------

#[test]
fn tiny_binding_limit_falls_back_without_reuse_and_still_draws() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        eprintln!("skipping: device cannot host the accelerated tier");
        return;
    }
    let mut renderer = init_accelerated(&gpu);
    // A limit too small for even the smallest pulled line segment: the line class
    // must fall back to the universal per-run path (existing override API).
    renderer.set_pulled_binding_limit_override(Some(1));

    let base = base_batches();
    let base_len = base.len();
    commit(&mut renderer, &base, 0);
    commit(&mut renderer, &overlay_batch(940, 0.7), base_len);
    commit(&mut renderer, &overlay_batch(941, 0.75), base_len);

    assert_eq!(
        renderer.pulled_line_segment_count(),
        0,
        "the pulled line class cannot be built under a 1-byte binding limit"
    );
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        0,
        "a class with no pulled segments must never claim a retained-prefix reuse"
    );

    // The geometry must still draw on the universal per-run fallback.
    let pixels = render_pixels(&mut renderer);
    assert!(
        !is_blank(&pixels),
        "the universal fallback must still draw the real line geometry"
    );
}

// ---------------------------------------------------------------------------
// Contract 6 (bounded transparency): a half-alpha overlay compares identically
// between the reused suffix and the single full commit.
// ---------------------------------------------------------------------------

#[test]
fn half_alpha_overlay_reuse_matches_full_commit() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        eprintln!("skipping: device cannot host the accelerated tier");
        return;
    }
    let base = base_batches();
    let mut overlay = line_band(0.75, 950, 0.5);
    overlay.alpha = 0.5;

    let (mut reused, _) = build_reused(&gpu, &base, std::slice::from_ref(&overlay));
    let mut composed = base.clone();
    composed.push(overlay.clone());
    let mut full = build_full(&gpu, &composed);

    let reused_px = render_pixels(&mut reused);
    let full_px = render_pixels(&mut full);
    assert!(!is_blank(&reused_px));
    assert_eq!(
        reused_px, full_px,
        "a half-alpha overlay must composite identically through the reuse and full paths"
    );
}

// ---------------------------------------------------------------------------
// Contract 7 (gate-2 regression): a warmup-established checkpoint followed by a
// suffix whose independently prepared stream exceeds the pulled segment cap must
// NOT be counted as a reuse. The line class degrades to the universal path
// (re-uploading the base geometry), so the checkpoint must be invalidated and the
// reuse counter must not advance; the rendered frame must still match a single
// full commit.
//
// The cap override is configured BEFORE the warmup: `set_pulled_line_segment_elems_override`
// invalidates the retained prefix, so setting it after the warmup would invalidate
// the checkpoint and miss the very path this fence exists to catch.
// ---------------------------------------------------------------------------

#[test]
fn oversized_suffix_fallback_is_not_a_reuse_and_invalidates_the_checkpoint() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        eprintln!("skipping: device cannot host the accelerated tier");
        return;
    }
    let mut renderer = init_accelerated(&gpu);

    let base = base_batches();
    let base_len = base.len();

    // 1) Full base commit with NO cap override: the base builds normally, and the
    //    measured pulled instance total tells us the cap the base actually needs.
    commit(&mut renderer, &base, 0);
    let base_instances = renderer.pulled_line_instance_count();
    assert!(
        base_instances > 0,
        "shape precondition: the base commit must build pulled line instances to derive the cap"
    );
    let cap = base_instances as usize;

    // The cap override is configured BEFORE the warmup (setting it later would
    // invalidate the checkpoint and miss the path this fence exists for). Every
    // base stream is <= the base total, so the base still fits.
    renderer.set_pulled_line_segment_elems_override(Some(cap));

    let warmup = overlay_batch(960, WARMUP_OVERLAY_Y);
    // One batch becomes one instance stream; `oversized_rows` is derived from the
    // fixture's own row count (the base is 4 batches of `BAND_ROWS`), not
    // hard-coded, so its stream is strictly larger than the base total = `cap`.
    let base_rows: u32 = 4 * BAND_ROWS;
    let oversized_rows = base_rows * 3 + 4;
    let oversized = vec![line_band_rows(
        WARMUP_OVERLAY_Y,
        961,
        1.0,
        oversized_rows,
        0.01,
    )];

    // 2) Warmup suffix: fits the cap, so it is a full rebuild that RETAINS the base.
    commit(&mut renderer, &warmup, base_len);
    assert!(
        renderer.pulled_line_segment_count() > 0,
        "shape precondition: the warmup must actually build the pulled line class under the cap"
    );
    assert_eq!(renderer.pulled_prefix_rebuilds(), 1);
    assert_eq!(renderer.pulled_prefix_reuses(), 0);

    // 3) Oversized suffix: one stream larger than the cap. The line class must
    //    degrade to the universal path, and that must NOT be reported as a reuse.
    let reuses = renderer.pulled_prefix_reuses();
    let rebuilds = renderer.pulled_prefix_rebuilds();
    commit(&mut renderer, &oversized, base_len);
    assert_eq!(
        renderer.pulled_line_segment_count(),
        0,
        "shape precondition: the oversized suffix stream must degrade the line class to the \
         universal path (a non-zero count would mean the fence never triggered)"
    );
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        reuses,
        "a fallback that re-uploads the base geometry must NOT be counted as a reuse"
    );
    assert_eq!(
        renderer.pulled_prefix_rebuilds(),
        rebuilds,
        "the fallback must invalidate the checkpoint, not establish a fresh one"
    );

    // The fallback frame must still match a single full commit of the same scene
    // under the SAME cap (colour and pick), so the degradation is not silently wrong.
    let mut composed = base.clone();
    composed.extend(oversized.clone());
    let mut full = build_full_with_cap(&gpu, &composed, cap);
    assert_eq!(
        full.pulled_line_segment_count(),
        0,
        "the forced-full comparison must degrade identically under the same cap"
    );
    let fallback_px = render_pixels(&mut renderer);
    let full_px = render_pixels(&mut full);
    assert!(
        !is_blank(&fallback_px),
        "the universal fallback must still draw the real geometry"
    );
    assert_eq!(
        fallback_px, full_px,
        "the fallback frame must be byte-identical to a single full commit of the same scene"
    );
    // Chosen interior pixels (camera-derived, then one pick each) for the
    // same-pixel semantic comparison.
    let base_xy = band_interior_pixel(BASE_PICK_Y);
    let overlay_xy = band_interior_pixel_rows(WARMUP_OVERLAY_Y, oversized_rows, 0.01);
    assert_drawn(&fallback_px, base_xy, "base band");
    assert_drawn(&fallback_px, overlay_xy, "oversized band");
    let coords = [base_xy, overlay_xy];
    assert_eq!(
        pick_map(&mut renderer, &coords),
        pick_map(&mut full, &coords),
        "the fallback and full commits must resolve the SAME chosen pixels to the same identities"
    );

    // 4) The checkpoint is gone: a follow-up normal suffix must REBUILD first
    //    (re-establish) and only the next one may reuse.
    commit(&mut renderer, &warmup, base_len);
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        reuses,
        "after invalidation the follow-up suffix must not reuse"
    );
    assert_eq!(
        renderer.pulled_prefix_rebuilds(),
        rebuilds + 1,
        "the follow-up suffix must re-establish the checkpoint"
    );
    commit(&mut renderer, &warmup, base_len);
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        reuses + 1,
        "only after re-establishing does the next suffix reuse again"
    );
}

// ---------------------------------------------------------------------------
// Resident-image + flags fixtures (existing public APIs only)
// ---------------------------------------------------------------------------

/// Project a world point to a pixel using the renderer's own mapping for
/// `camera_2d()` (centre `(0,0)`, `world_per_px = 2/TARGET`), matching the
/// formula the repo's image tests use (`world_to_pixel`).
fn world_pixel(world_x: f32, world_y: f32) -> (u32, u32) {
    let per_px = 2.0 / TARGET as f32;
    let px = TARGET as f32 / 2.0 + world_x / per_px;
    let py = TARGET as f32 / 2.0 - world_y / per_px;
    (
        px.floor().clamp(0.0, (TARGET - 1) as f32) as u32,
        py.floor().clamp(0.0, (TARGET - 1) as f32) as u32,
    )
}

/// A solid-colour RGBA8 decoded image (the repo's image-test fixture).
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

/// A cache holding one opaque red image under [`IMAGE_RESOURCE`].
fn red_cache() -> DecodedImageCache {
    let mut cache = DecodedImageCache::new();
    cache
        .insert(
            ResourceKey(IMAGE_RESOURCE.into()),
            solid_image(2, 2, [255, 0, 0, 255]),
            &ResourceLimits::default(),
        )
        .expect("insert red image");
    cache
}

/// The resident image quad: `IMAGE_HALF_W` x `IMAGE_HALF_H` world (16x8 px on
/// `camera_2d()`), centred at `IMAGE_CENTRE`, in the free row gap between base
/// bands `e3` and `e2` so image-only and base-only interior pixels are disjoint.
const IMAGE_HALF_W: f32 = 0.25;
const IMAGE_HALF_H: f32 = 0.125;
const IMAGE_CENTRE: (f32, f32) = (0.0, -0.1125);
const IMAGE_ENTITY: u128 = 0xf1;
const IMAGE_RESOURCE: &str = "flags-red";

fn image_quad(draw_order: i64) -> ImageBatch {
    ImageBatch {
        resource: ResourceKey(IMAGE_RESOURCE.into()),
        transform: Transform3::from_basis(
            Point3 {
                x: f64::from(2.0 * IMAGE_HALF_W),
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 0.0,
                y: f64::from(2.0 * IMAGE_HALF_H),
                z: 0.0,
            },
            Point3 {
                x: f64::from(IMAGE_CENTRE.0 - IMAGE_HALF_W),
                y: f64::from(IMAGE_CENTRE.1 - IMAGE_HALF_H),
                z: 0.0,
            },
        ),
        clip: None,
        alpha: 1.0,
        draw_order,
        sources: vec![source(IMAGE_ENTITY)],
    }
}

/// Render and return `(resident image quads drawn this frame, packed RGBA)`.
fn render_stats_pixels(renderer: &mut Renderer) -> (usize, Vec<u8>) {
    let target = RenderTarget::new(TARGET, TARGET);
    let stats = renderer.render(camera_2d(), &target).expect("render frame");
    let pixels = renderer.read_target_rgba().expect("read frame").pixels;
    (stats.image_batches, pixels)
}

// ---------------------------------------------------------------------------
// Contract 8: a resident raster image + a reused suffix must be byte-identical
// and pick-identical to the same scene committed in one full pass; the image
// must stay resident across the reused suffix commit.
// ---------------------------------------------------------------------------

#[test]
fn suffix_reuse_with_a_resident_image_matches_full_commit() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        eprintln!("skipping: device cannot host the accelerated tier");
        return;
    }
    const OVERLAY: u128 = 0xf0;
    let base = base_batches();
    let base_len = base.len();
    let overlay = overlay_batch(OVERLAY, COMPARED_OVERLAY_Y);
    let cache = red_cache();

    // Reused path: base geometry + resident image uploaded once, then a warmup
    // suffix and a reusing suffix.
    let mut reused = init_accelerated(&gpu);
    commit(&mut reused, &base, 0);
    let report = reused
        .upload_images(&[image_quad(0)], &cache)
        .expect("upload image");
    assert_eq!(report.uploaded, 1, "the resident image must upload");
    assert!(
        report.unresolved.is_empty(),
        "the image resource must resolve"
    );
    let textures = reused.image_texture_count();
    assert_eq!(textures, 1, "one resident image texture");

    commit(&mut reused, &overlay_batch(7, WARMUP_OVERLAY_Y), base_len);
    let before = reused.pulled_prefix_reuses();
    commit(&mut reused, &overlay, base_len);
    assert_eq!(
        reused.pulled_prefix_reuses(),
        before + 1,
        "the suffix must reuse the retained base with an image resident"
    );
    assert_eq!(
        reused.image_texture_count(),
        textures,
        "a reused suffix commit must not disturb the resident image"
    );

    // Full counterpart: the same batches and the same image in one commit.
    let mut composed = base.clone();
    composed.extend(overlay.clone());
    let mut full = init_accelerated(&gpu);
    commit(&mut full, &composed, 0);
    let full_report = full
        .upload_images(&[image_quad(0)], &cache)
        .expect("upload image");
    assert_eq!(full_report.uploaded, 1);

    let (reused_images, reused_px) = render_stats_pixels(&mut reused);
    let (full_images, full_px) = render_stats_pixels(&mut full);
    assert_eq!(
        reused_images, 1,
        "the resident image must be drawn (reused)"
    );
    assert_eq!(full_images, 1, "the resident image must be drawn (full)");
    assert!(!is_blank(&reused_px));
    assert_eq!(
        reused_px, full_px,
        "a resident image plus a reused suffix must be byte-identical to the full commit"
    );

    // The image's own opaque red must actually be on screen (non-vacuous).
    let image_xy = world_pixel(IMAGE_CENTRE.0, IMAGE_CENTRE.1);
    let [r, g, b, a] = rgb(&reused_px, image_xy.0, image_xy.1);
    assert!(
        r > 240 && g < 15 && b < 15 && a > 240,
        "the resident image centre {image_xy:?} must be opaque red, got {:?}",
        [r, g, b, a]
    );

    // Same chosen interior pixels on both: semantic identity equality.
    let base_xy = band_interior_pixel(BASE_PICK_Y);
    let overlay_xy = band_interior_pixel(COMPARED_OVERLAY_Y);
    assert_drawn(&reused_px, base_xy, "base band");
    assert_drawn(&reused_px, overlay_xy, "overlay band");
    let coords = [base_xy, overlay_xy, image_xy];
    let reused_picks = pick_map(&mut reused, &coords);
    let full_picks = pick_map(&mut full, &coords);
    assert_eq!(
        reused_picks, full_picks,
        "reused and full must resolve the same pixels to the same identities with an image resident"
    );
    assert_eq!(
        source_at(&reused_picks, base_xy).map(|s| s.entity),
        Some(EntityId(BASE_PICK_ENTITY)),
        "the retained base entity must pick with an image resident"
    );
    assert_eq!(
        source_at(&reused_picks, overlay_xy).map(|s| s.entity),
        Some(EntityId(OVERLAY)),
        "the suffix entity must pick with an image resident"
    );
    assert_eq!(
        source_at(&reused_picks, image_xy).map(|s| s.entity),
        Some(EntityId(IMAGE_ENTITY)),
        "the resident image must resolve to its own source"
    );
}

// ---------------------------------------------------------------------------
// Contract 9: an overlay that crosses the resident image, plus the selection /
// visibility flags, must stay exact under a reused suffix (flags never change
// pick identity; hiding must remove only the hidden entity).
// ---------------------------------------------------------------------------

#[test]
fn resident_image_and_flags_stay_exact_under_suffix_reuse() {
    let Some(gpu) = gpu() else {
        return;
    };
    if !supports_storage(&gpu) {
        eprintln!("skipping: device cannot host the accelerated tier");
        return;
    }
    const OVERLAY: u128 = 0xf2;
    let base = base_batches();
    let base_len = base.len();
    // The overlay band sits on the image's rows and spans only the image's right
    // half (`x 0.0..0.9`), so an image-only pixel, an overlay-only pixel and a
    // crossing pixel are all disjoint and individually assertable.
    let crossing = vec![line_span_rows(
        0.0,
        0.9,
        IMAGE_CENTRE.1,
        OVERLAY,
        1.0,
        BAND_ROWS,
        BAND_SPACING,
    )];
    let cache = red_cache();

    let mut renderer = init_accelerated(&gpu);
    commit(&mut renderer, &base, 0);
    let report = renderer
        .upload_images(&[image_quad(1)], &cache)
        .expect("upload image");
    assert_eq!(report.uploaded, 1);
    let textures = renderer.image_texture_count();

    commit(
        &mut renderer,
        &overlay_batch(0xf3, WARMUP_OVERLAY_Y),
        base_len,
    );
    let before = renderer.pulled_prefix_reuses();
    commit(&mut renderer, &crossing, base_len);
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        before + 1,
        "the crossing suffix must reuse the retained base"
    );
    assert_eq!(
        renderer.image_texture_count(),
        textures,
        "the resident image must survive the reused crossing suffix"
    );

    // Full counterpart: same batches, same image, one commit.
    let mut composed = base.clone();
    composed.extend(crossing.clone());
    let mut full = init_accelerated(&gpu);
    commit(&mut full, &composed, 0);
    assert_eq!(
        full.upload_images(&[image_quad(1)], &cache)
            .expect("upload image")
            .uploaded,
        1
    );

    let (images, reused_px) = render_stats_pixels(&mut renderer);
    let (full_images, full_px) = render_stats_pixels(&mut full);
    assert_eq!((images, full_images), (1, 1), "the image must be drawn");
    assert_eq!(
        reused_px, full_px,
        "a crossing overlay + resident image must match the full commit byte-for-byte"
    );

    // Disjoint interiors: image-only (left half), overlay-only (right, outside the
    // image), and the crossing pixel (inside both). The overlay band's own
    // interior row is used (its y span is narrower than the image's), so no chosen
    // pixel is band-fringe.
    let band_y = band_interior_pixel(IMAGE_CENTRE.1).1;
    let image_only = (
        world_pixel(IMAGE_CENTRE.0 - 0.125, IMAGE_CENTRE.1).0,
        band_y,
    );
    let overlay_only = (world_pixel(IMAGE_CENTRE.0 + 0.5, IMAGE_CENTRE.1).0, band_y);
    let crossing_xy = (
        world_pixel(IMAGE_CENTRE.0 + 0.125, IMAGE_CENTRE.1).0,
        band_y,
    );
    let coords = [image_only, overlay_only, crossing_xy];
    for xy in coords {
        assert_drawn(&reused_px, xy, "resident image / crossing overlay region");
    }

    let picks = pick_map(&mut renderer, &coords);
    let full_picks = pick_map(&mut full, &coords);
    assert_eq!(
        picks, full_picks,
        "the crossing overlay must resolve identically in the reused and full commits"
    );
    assert_eq!(
        source_at(&picks, image_only).map(|s| s.entity),
        Some(EntityId(IMAGE_ENTITY)),
        "an image-only pixel must resolve to the image"
    );
    assert_eq!(
        source_at(&picks, overlay_only).map(|s| s.entity),
        Some(EntityId(OVERLAY)),
        "an overlay-only pixel must resolve to the overlay"
    );
    assert!(
        source_at(&picks, crossing_xy).is_some(),
        "the crossing pixel must resolve to exactly one of the two declared entities"
    );

    // --- selection flags: tint is visible but never changes pick identity ------
    let overlay_key = renderer
        .pick_key_for_batch(renderer.batch_count() - 1)
        .expect("the crossing overlay must own a pick key");
    renderer.set_selection_flags_enabled(true);
    renderer
        .set_pick_key_flags(overlay_key, FLAG_SELECTED)
        .expect("set selection flag");
    assert_eq!(renderer.pick_key_flags(overlay_key), FLAG_SELECTED);
    let (_, tinted_px) = render_stats_pixels(&mut renderer);
    assert_ne!(
        tinted_px, reused_px,
        "an enabled selection flag on the overlay must tint the frame (non-vacuous)"
    );
    let tinted_picks = pick_map(&mut renderer, &coords);
    assert_eq!(
        tinted_picks, picks,
        "a selection flag must not change any pick identity"
    );

    // Clearing the flag and the toggle restores the exact frame.
    renderer
        .set_pick_key_flags(overlay_key, 0)
        .expect("clear flags");
    renderer.set_selection_flags_enabled(false);
    let (_, restored_px) = render_stats_pixels(&mut renderer);
    assert_eq!(
        restored_px, reused_px,
        "clearing the flag must restore the exact frame"
    );

    // --- visibility: HIDDEN removes only the overlay, nothing else -------------
    // `flags_enabled` is the flags control word (`write_flags_ctl`), so it gates
    // ALL flag behaviour including the FLAG_HIDDEN discard; the toggle must be
    // enabled while hiding (the repo's own visibility fence keeps it enabled
    // through FLAG_HIDDEN). The tint/clear restore gate above is a separate step.
    renderer.set_selection_flags_enabled(true);
    renderer
        .set_pick_key_flags(overlay_key, FLAG_HIDDEN)
        .expect("hide the overlay");
    let (_, hidden_px) = render_stats_pixels(&mut renderer);
    assert_ne!(
        hidden_px, reused_px,
        "hiding the overlay must change the frame"
    );
    let hidden_picks = pick_map(&mut renderer, &coords);
    assert_ne!(
        source_at(&hidden_picks, overlay_only).map(|s| s.entity),
        Some(EntityId(OVERLAY)),
        "the hidden overlay must no longer pick"
    );
    assert_eq!(
        source_at(&hidden_picks, image_only).map(|s| s.entity),
        Some(EntityId(IMAGE_ENTITY)),
        "the resident image must still pick while the overlay is hidden"
    );
    assert_eq!(
        source_at(&hidden_picks, crossing_xy).map(|s| s.entity),
        Some(EntityId(IMAGE_ENTITY)),
        "the crossing pixel must fall back to the still-visible image"
    );
    renderer
        .set_pick_key_flags(overlay_key, 0)
        .expect("clear flags");
}
