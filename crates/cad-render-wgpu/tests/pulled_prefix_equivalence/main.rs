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

mod image_flags;
mod suffix_reuse;
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

// ---------------------------------------------------------------------------
// Contract 3: repeated overlay replace / remove / re-add keeps the base and
// keeps reusing.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Contract 4: a full base replacement invalidates the retained prefix.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Contract 5: a binding limit too small for the pulled line class falls back to
// the universal per-run path and never claims a reuse.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Contract 6 (bounded transparency): a half-alpha overlay compares identically
// between the reused suffix and the single full commit.
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Contract 9: an overlay that crosses the resident image, plus the selection /
// visibility flags, must stay exact under a reused suffix (flags never change
// pick identity; hiding must remove only the hidden entity).
// ---------------------------------------------------------------------------
