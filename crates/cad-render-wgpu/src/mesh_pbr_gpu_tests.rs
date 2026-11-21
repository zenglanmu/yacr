//! Native GPU contracts for the PBR mesh channel (ADR-0039 D6).
//!
//! These tests bind a real headless device, upload a real PBR material (generated
//! tangent frame included) and sample the presented pixels. They auto-skip when
//! no adapter is available (`SKIP ... NOT a pass`), and a software adapter's
//! result is pixel/geometry evidence only — never a performance or vendor claim.
//!
//! What is contracted (ADR-0039 invariant 7): none of these may be replaced by
//! "the shader compiles".
//!
//! 1. A **normal map** changes the presented pixels versus the flat identity
//!    normal (and the tangent frame is actually generated: the counter stays 0).
//! 2. `metallic = 1` differs from `metallic = 0` under the same light.
//! 3. An **emissive** term is visible with the light facing away.
//! 4. An **occlusion** map darkens the surface (diffuse/ambient only).
//! 5. Degenerate UVs disable the normal-map channel instead of inventing a
//!    tangent, and the event is counted.
//!
//! Run serially (these bind a real device):
//!
//! ```text
//! cargo test -p cad-render-wgpu --lib --locked -- --test-threads=1 mesh_pbr_gpu_tests
//! ```
use crate::headless::{create_headless_gpu, HeadlessGpu};
use crate::{BackendPreference, Camera3d, RenderTarget, Renderer};
use cad_domain::{DocumentId, EntityId, InstancePath, Point3, SelectionRef};
use cad_resources::{DecodedImage, DecodedImageCache, ResourceKey, ResourceLimits};
use cad_scene::{RenderBatch, RenderPbr, RenderTopology};
use std::sync::Arc;
use std::time::Duration;

const SIZE: u32 = 64;
/// A canonical document texture key (`<32 lowercase hex>.png`).
pub(crate) const BASE_KEY: &str = "4c2f2e0a5f3d9b7c1e8a6d4b2c0f9e71.png";
const MR_KEY: &str = "11111111111111111111111111111111.png";
const NORMAL_KEY: &str = "22222222222222222222222222222222.png";
const EMISSIVE_KEY: &str = "33333333333333333333333333333333.png";
const OCCLUSION_KEY: &str = "44444444444444444444444444444444.png";

pub(crate) fn gpu_or_skip(what: &str) -> Option<HeadlessGpu> {
    match create_headless_gpu(BackendPreference::Auto) {
        Ok(gpu) => {
            eprintln!(
                "[mesh-pbr-gpu] {what}: adapter backend={} name={} device_type={} driver={}",
                gpu.adapter.backend, gpu.adapter.name, gpu.adapter.device_type, gpu.adapter.driver
            );
            Some(gpu)
        }
        Err(error) => {
            eprintln!("[mesh-pbr-gpu] {what}: SKIP — no headless adapter ({error}); NOT a pass");
            None
        }
    }
}

pub(crate) fn init(gpu: &HeadlessGpu) -> Renderer {
    let mut renderer = Renderer::new(BackendPreference::Auto);
    renderer.set_poll_timeout(Duration::from_secs(30));
    renderer
        .initialize_with_device(gpu.device.clone(), gpu.queue.clone())
        .expect("initialize the renderer");
    renderer
}

/// A 3D camera on `+Z` looking at the origin, so a `+Z`-normal quad faces both the
/// camera and the camera-relative key light.
fn camera() -> Camera3d {
    Camera3d::plan_view(
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 5.0,
        },
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
    )
}

fn source(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

/// A world-space quad (`[-2, 2]²` at z = 0) with UV0 covering the unit square.
///
/// `normal` is a vertex attribute, not a geometric fact: a `-Z` normal is a
/// surface the light cannot reach while its winding stays visible.
pub(crate) fn pbr_quad(normal: [f32; 3], uvs: Vec<[f32; 2]>, pbr: RenderPbr) -> RenderBatch {
    RenderBatch {
        uvs,
        texture: None,
        pbr: Some(pbr),
        conics: Vec::new(),
        local_origin: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        topology: RenderTopology::Mesh,
        vertices: vec![
            [-2.0, -2.0, 0.0],
            [2.0, -2.0, 0.0],
            [2.0, 2.0, 0.0],
            [-2.0, 2.0, 0.0],
        ],
        normals: vec![normal; 4],
        colors: Vec::new(),
        indices: vec![[0, 1, 2], [0, 2, 3]],
        edges: Vec::new(),
        mirrored: false,
        alpha: 1.0,
        color: [1.0, 1.0, 1.0],
        color_unresolved: false,
        lineweight: 0.0,
        lineweight_unresolved: true,
        linetype: cad_scene::LinetypePattern::continuous(),
        linetype_unresolved: true,
        sources: vec![source(9)],
        draw_order: 0,
    }
}

pub(crate) fn unit_uvs() -> Vec<[f32; 2]> {
    vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
}

/// A scalar-only material: no texture channel is active.
pub(crate) fn scalar(metallic: f32, roughness: f32, emissive: [f32; 3]) -> RenderPbr {
    RenderPbr {
        metallic,
        roughness,
        emissive,
        occlusion_strength: 0.0,
        metallic_roughness: None,
        normal: None,
        emissive_texture: None,
        occlusion: None,
    }
}

/// A cache holding one opaque 1x1 texture per entry.
fn texture_cache(entries: &[(&str, [u8; 4])]) -> DecodedImageCache {
    let mut cache = DecodedImageCache::new();
    for (key, rgba) in entries {
        cache
            .insert(
                ResourceKey((*key).to_owned()),
                DecodedImage {
                    width: 1,
                    height: 1,
                    rgba: Arc::from(rgba.as_slice()),
                },
                &ResourceLimits::default(),
            )
            .expect("the fixture texture fits the resource budget");
    }
    cache
}

/// Commit `batch`, resolve its texture keys and render one frame.
fn draw(renderer: &mut Renderer, batch: &RenderBatch, cache: &DecodedImageCache) -> [u8; 4] {
    renderer
        .prepare_upload_batches(std::slice::from_ref(batch))
        .and_then(|staged| renderer.commit_upload(staged, 0))
        .expect("commit the PBR scene");
    let keys = cad_scene::mesh_texture_keys(std::iter::once(batch));
    let report = renderer
        .upload_mesh_textures(&keys, cache)
        .expect("upload the PBR material textures");
    assert!(
        report.unresolved.is_empty(),
        "the fixtures resolve, got {:?}",
        report.unresolved
    );
    renderer
        .render_3d(camera(), &RenderTarget::new(SIZE, SIZE))
        .expect("render");
    centre_pixel(renderer)
}

fn centre_pixel(renderer: &Renderer) -> [u8; 4] {
    let image = renderer
        .read_target_rgba()
        .expect("read the presented frame");
    assert_eq!((image.width, image.height), (SIZE, SIZE));
    let at = (((SIZE / 2) * SIZE + SIZE / 2) * 4) as usize;
    [
        image.pixels[at],
        image.pixels[at + 1],
        image.pixels[at + 2],
        image.pixels[at + 3],
    ]
}

fn luma(pixel: [u8; 4]) -> f32 {
    0.2126 * f32::from(pixel[0]) + 0.7152 * f32::from(pixel[1]) + 0.0722 * f32::from(pixel[2])
}

#[test]
fn the_base_colour_map_tints_the_pbr_response() {
    let Some(gpu) = gpu_or_skip("base colour map") else {
        return;
    };
    let mut renderer = init(&gpu);
    let mut batch = pbr_quad([0.0, 0.0, 1.0], unit_uvs(), scalar(0.0, 0.5, [0.0; 3]));
    // `Mesh.texture` is the base-colour key; the PBR channel samples it at slot 0.
    batch.texture = Some(ResourceKey(BASE_KEY.to_owned()));
    let pixel = draw(
        &mut renderer,
        &batch,
        &texture_cache(&[(BASE_KEY, [255, 0, 0, 255])]),
    );
    assert!(
        pixel[0] > 180 && pixel[1] < 100 && pixel[2] < 100 && pixel[0] > pixel[1] + 100,
        "the base-colour map must tint the shaded surface (the small neutral \
         component is the dielectric specular lobe), got {pixel:?}"
    );
}

#[test]
fn the_metallic_roughness_map_modulates_the_scalar_factors() {
    let Some(gpu) = gpu_or_skip("metallic-roughness map") else {
        return;
    };
    let mut renderer = init(&gpu);
    // The scalar says "metal", but `metallic = scalar * mr.b`: a map with b = 0
    // pulls it back to a dielectric, which is visibly brighter under this light.
    let mut material = scalar(1.0, 1.0, [0.0; 3]);
    material.metallic_roughness = Some(ResourceKey(MR_KEY.to_owned()));
    let dielectric = draw(
        &mut renderer,
        &pbr_quad([0.0, 0.0, 1.0], unit_uvs(), material.clone()),
        &texture_cache(&[(MR_KEY, [255, 255, 0, 255])]),
    );
    let metal = draw(
        &mut renderer,
        &pbr_quad([0.0, 0.0, 1.0], unit_uvs(), material),
        &texture_cache(&[(MR_KEY, [255, 255, 255, 255])]),
    );
    assert!(
        luma(dielectric) > luma(metal) + 30.0,
        "the mr map's blue channel must scale metallic: dielectric={dielectric:?} \
         metal={metal:?}"
    );
    // Picking is unchanged: a PBR batch is a legacy per-batch draw, so the id
    // pass resolves it through the same pick key as an untextured mesh. (The key
    // is allocated per commit, so a second scene on one renderer owns a later
    // key: assert the *source*, not the ordinal.)
    let hit = renderer
        .pick_at(SIZE / 2, SIZE / 2)
        .expect("run the id pass")
        .expect("the quad covers the centre");
    assert!(hit.entity_key >= 1, "the resident batch owns a pick key");
    assert_eq!(
        hit.source.expect("exact single-source hit").entity,
        EntityId(9)
    );
}

#[test]
fn a_normal_map_changes_pixels_versus_the_flat_identity_normal() {
    let Some(gpu) = gpu_or_skip("normal map") else {
        return;
    };
    let mut renderer = init(&gpu);
    let flat = draw(
        &mut renderer,
        &pbr_quad([0.0, 0.0, 1.0], unit_uvs(), scalar(0.0, 0.5, [0.0; 3])),
        &DecodedImageCache::new(),
    );

    // A normal map that tilts the surface strongly toward +X (tangent space
    // `(1, 0, 0)`): the light is mostly +Z, so the mapped surface is much darker.
    let mut material = scalar(0.0, 0.5, [0.0; 3]);
    material.normal = Some(ResourceKey(NORMAL_KEY.to_owned()));
    let mapped = draw(
        &mut renderer,
        &pbr_quad([0.0, 0.0, 1.0], unit_uvs(), material),
        &texture_cache(&[(NORMAL_KEY, [255, 128, 128, 255])]),
    );

    assert!(
        (luma(flat) - luma(mapped)).abs() > 20.0,
        "a normal map must change the shaded pixels: flat={flat:?} mapped={mapped:?}"
    );
    assert_eq!(
        renderer.pbr_tangent_unavailable(),
        0,
        "a valid UV quad must produce a tangent frame (none is invented when it fails)"
    );
}

#[test]
fn metallic_one_differs_from_metallic_zero_under_the_same_light() {
    let Some(gpu) = gpu_or_skip("metallic response") else {
        return;
    };
    let mut renderer = init(&gpu);
    let dielectric = draw(
        &mut renderer,
        &pbr_quad([0.0, 0.0, 1.0], unit_uvs(), scalar(0.0, 0.5, [0.0; 3])),
        &DecodedImageCache::new(),
    );
    let metal = draw(
        &mut renderer,
        &pbr_quad([0.0, 0.0, 1.0], unit_uvs(), scalar(1.0, 0.5, [0.0; 3])),
        &DecodedImageCache::new(),
    );
    assert!(
        (luma(dielectric) - luma(metal)).abs() > 30.0,
        "metallic must change the response (diffuse removed, F0 = base): \
         dielectric={dielectric:?} metal={metal:?}"
    );
    assert!(
        luma(dielectric) > luma(metal),
        "a pure metal keeps only the specular lobe, so it is darker here: \
         dielectric={dielectric:?} metal={metal:?}"
    );
}

#[test]
fn emissive_is_visible_with_the_light_facing_away() {
    let Some(gpu) = gpu_or_skip("emissive") else {
        return;
    };
    let mut renderer = init(&gpu);
    // A `-Z` vertex normal cannot face the key light, and `metallic = 1` removes
    // the diffuse and ambient terms: without emission the surface is black.
    let dark = draw(
        &mut renderer,
        &pbr_quad([0.0, 0.0, -1.0], unit_uvs(), scalar(1.0, 1.0, [0.0; 3])),
        &DecodedImageCache::new(),
    );
    let mut material = scalar(1.0, 1.0, [1.0, 1.0, 0.0]);
    material.emissive_texture = Some(ResourceKey(EMISSIVE_KEY.to_owned()));
    let glowing = draw(
        &mut renderer,
        &pbr_quad([0.0, 0.0, -1.0], unit_uvs(), material),
        // The emissive map is a colour texture (sRGB), so white is full emission.
        &texture_cache(&[(EMISSIVE_KEY, [255, 255, 255, 255])]),
    );
    assert!(
        luma(dark) < 10.0,
        "an unlit metal with no emission must be black, got {dark:?}"
    );
    assert!(
        glowing[0] > 180 && glowing[1] > 180 && glowing[2] < 60,
        "emissive (yellow) must be visible without any incoming light, got {glowing:?}"
    );
}

#[test]
fn an_occlusion_map_darkens_the_surface() {
    let Some(gpu) = gpu_or_skip("occlusion") else {
        return;
    };
    let mut renderer = init(&gpu);
    let open = draw(
        &mut renderer,
        &pbr_quad([0.0, 0.0, 1.0], unit_uvs(), scalar(0.0, 0.5, [0.0; 3])),
        &DecodedImageCache::new(),
    );
    let mut material = scalar(0.0, 0.5, [0.0; 3]);
    material.occlusion_strength = 1.0;
    material.occlusion = Some(ResourceKey(OCCLUSION_KEY.to_owned()));
    let occluded = draw(
        &mut renderer,
        &pbr_quad([0.0, 0.0, 1.0], unit_uvs(), material),
        // `r = 0` with `strength = 1` gives `ao = 0`: diffuse and ambient vanish.
        &texture_cache(&[(OCCLUSION_KEY, [0, 0, 0, 255])]),
    );
    assert!(
        luma(open) - luma(occluded) > 40.0,
        "occlusion must darken the diffuse/ambient terms: open={open:?} occluded={occluded:?}"
    );
}

#[test]
fn degenerate_uvs_disable_the_normal_map_instead_of_inventing_a_tangent() {
    let Some(gpu) = gpu_or_skip("degenerate UV frame") else {
        return;
    };
    let mut renderer = init(&gpu);
    // All UVs identical: every triangle has zero UV area, so no tangent can be
    // derived. The batch still draws (the scalar factors are valid).
    let mut material = scalar(0.0, 0.5, [0.0; 3]);
    material.normal = Some(ResourceKey(NORMAL_KEY.to_owned()));
    let batch = pbr_quad([0.0, 0.0, 1.0], vec![[0.5, 0.5]; 4], material);
    let pixel = draw(
        &mut renderer,
        &batch,
        &texture_cache(&[(NORMAL_KEY, [255, 128, 128, 255])]),
    );
    assert!(
        pixel[3] > 0,
        "the batch must still be drawn (scalars are valid), got {pixel:?}"
    );
    assert_eq!(
        renderer.pbr_tangent_unavailable(),
        1,
        "the missing tangent frame is counted as `render.pbr.tangent_unavailable`"
    );
}

// --- procedural IBL (ADR-0044) ---------------------------------------------

/// The legacy `AMBIENT` floor, mirroring `mesh_pbr.wgsl`'s constant. Duplicated
/// deliberately: this is the ADR-0039 value the IBL-off branch must preserve.
const AMBIENT: f32 = 0.25;

/// The surface response with no incoming direct light and no environment.
///
/// A `-Z` vertex normal (the same fixture the emissive test uses) makes
/// `n_dot_l = 0`, so the diffuse and GGX lobes vanish; with `metallic = 1` the
/// legacy ambient term vanishes too (it is albedo-scaled). That isolates "what
/// does the environment contribute" from the direct-light maths — the reported
/// defect is exactly that an unlit metal is black without one.
fn unlit_metal() -> RenderBatch {
    pbr_quad([0.0, 0.0, -1.0], unit_uvs(), scalar(1.0, 0.35, [0.0; 3]))
}

/// sRGB-encode one linear channel the way the `Rgba8UnormSrgb` target does, so a
/// legacy expectation is stated in the readback's own units.
pub(crate) fn srgb_byte(linear: f32) -> f32 {
    let encoded = if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round()
}

#[test]
fn metal_is_not_black_with_ibl_on() {
    let Some(gpu) = gpu_or_skip("IBL metal") else {
        return;
    };
    let mut renderer = init(&gpu);

    renderer.set_ibl_enabled(false);
    let legacy = draw(&mut renderer, &unlit_metal(), &DecodedImageCache::new());

    renderer.set_ibl_enabled(true);
    let ibl = draw(&mut renderer, &unlit_metal(), &DecodedImageCache::new());

    // `luma` is 0..=255; the bar is a normalised luminance.
    let legacy_luma = luma(legacy) / 255.0;
    let ibl_luma = luma(ibl) / 255.0;
    assert!(
        legacy_luma < 0.01,
        "the current defect: an unlit metal has no environment to reflect, so \
         the legacy lighting is black, got {legacy:?}"
    );
    assert!(
        ibl_luma > 0.05,
        "procedural IBL must give the metal an environment reflection, \
         got {ibl:?} (normalised luma {ibl_luma:.3})"
    );
    assert!(
        ibl[3] > 0,
        "the IBL frame must still be the drawn surface, got {ibl:?}"
    );
}

#[test]
fn ibl_roughness_selects_the_specular_lobe() {
    let Some(gpu) = gpu_or_skip("IBL roughness") else {
        return;
    };
    let mut renderer = init(&gpu);
    renderer.set_ibl_enabled(true);

    let sharp = draw(
        &mut renderer,
        &pbr_quad([0.0, 0.0, -1.0], unit_uvs(), scalar(1.0, 0.05, [0.0; 3])),
        &DecodedImageCache::new(),
    );
    let diffuse_like = draw(
        &mut renderer,
        &pbr_quad([0.0, 0.0, -1.0], unit_uvs(), scalar(1.0, 0.9, [0.0; 3])),
        &DecodedImageCache::new(),
    );

    // A mirror lobe samples the horizon band; a rough lobe is pulled `r²` toward
    // the cosine-weighted irradiance, which is the (much darker) ground mix for a
    // `-Z` normal. The two must not collapse to the same reflect.
    assert!(
        (luma(sharp) - luma(diffuse_like)).abs() > 20.0,
        "roughness must select the environment lobe: sharp={sharp:?} \
         rough={diffuse_like:?}"
    );
}

#[test]
fn ibl_occlusion_scales_the_indirect_term() {
    let Some(gpu) = gpu_or_skip("IBL occlusion") else {
        return;
    };
    let mut renderer = init(&gpu);
    renderer.set_ibl_enabled(true);

    let mut occluded_material = scalar(1.0, 0.35, [0.0; 3]);
    occluded_material.occlusion_strength = 1.0;
    occluded_material.occlusion = Some(ResourceKey(OCCLUSION_KEY.to_owned()));
    // Open: `ao = 1`, so the environment term is unscaled.
    let mut open = occluded_material.clone();
    open.occlusion_strength = 0.0;
    open.occlusion = None;

    let open_pixel = draw(
        &mut renderer,
        &pbr_quad([0.0, 0.0, -1.0], unit_uvs(), open),
        &DecodedImageCache::new(),
    );
    let occluded_pixel = draw(
        &mut renderer,
        &pbr_quad([0.0, 0.0, -1.0], unit_uvs(), occluded_material),
        // `r = 0` with `strength = 1` gives `ao = 0`.
        &texture_cache(&[(OCCLUSION_KEY, [0, 0, 0, 255])]),
    );

    assert!(
        luma(open_pixel) - luma(occluded_pixel) > 40.0,
        "occlusion must scale the indirect terms: open={open_pixel:?} \
         occluded={occluded_pixel:?}"
    );
}

#[test]
fn ibl_off_matches_the_legacy_lighting() {
    let Some(gpu) = gpu_or_skip("IBL off legacy parity") else {
        return;
    };
    let mut renderer = init(&gpu);
    // The default is off, but state it explicitly: this test is the fence.
    renderer.set_ibl_enabled(false);

    // A dielectric facing away from the key light: the diffuse and GGX lobes are
    // zero, so the legacy ADR-0039 result is exactly the ambient floor
    // `base_colour * (1 - metallic) * AMBIENT * occlusion = 0.25` for a white
    // scalar albedo. That is a closed-form value, not a recorded one.
    let batch = pbr_quad([0.0, 0.0, -1.0], unit_uvs(), scalar(0.0, 0.5, [0.0; 3]));
    let first = draw(&mut renderer, &batch, &DecodedImageCache::new());
    let second = draw(&mut renderer, &batch, &DecodedImageCache::new());
    assert_eq!(
        first, second,
        "the IBL-off frame must be deterministic (self-consistency)"
    );

    let expected = srgb_byte(AMBIENT);
    for (channel, value) in ["r", "g", "b"].iter().zip(&first[..3]) {
        assert!(
            (f32::from(*value) - expected).abs() <= 4.0,
            "the legacy ambient floor must be unchanged by ADR-0044: \
             channel {channel} = {value}, expected {expected:.1} \
             (AMBIENT = {AMBIENT} linear), frame {first:?}"
        );
    }
    assert_eq!(
        first[3], 255,
        "the drawn surface stays opaque, got {first:?}"
    );

    // ... and the flag is live: the same surface with IBL on is not this value.
    renderer.set_ibl_enabled(true);
    let ibl = draw(&mut renderer, &batch, &DecodedImageCache::new());
    assert!(
        (luma(ibl) - luma(first)).abs() > 5.0,
        "enabling IBL must change the shaded result: legacy={first:?} ibl={ibl:?}"
    );
}

/// Commit `batch` **once** so the following renders exercise the retained
/// progressive plan. [`draw`] re-commits per call, which drops that plan (a base
/// change clears `progressive_frame`), so it can never prove a live toggle.
fn upload_only(renderer: &mut Renderer, batch: &RenderBatch) {
    renderer
        .prepare_upload_batches(std::slice::from_ref(batch))
        .and_then(|staged| renderer.commit_upload(staged, 0))
        .expect("commit the PBR scene");
}

fn render_frame(renderer: &mut Renderer) -> [u8; 4] {
    renderer
        .render_3d(camera(), &RenderTarget::new(SIZE, SIZE))
        .expect("render");
    centre_pixel(renderer)
}

/// ADR-0044: the IBL switch is per-frame uniform data, so toggling it must
/// re-shade the **next** presented frame on one renderer with one resident scene.
///
/// The production session enables progressive rendering, and a completed page
/// walk keeps `progressive_frame` at `cursor == ordered.len()`. A frame key that
/// ignores the IBL flag would therefore accept no page and keep the previous
/// shading: the toggle would be invisible. This is the regression fence (the
/// fixture is an unlit metal, black under the legacy lighting).
#[test]
fn ibl_toggle_reshades_the_next_frame_after_the_progressive_walk_completed() {
    let Some(gpu) = gpu_or_skip("IBL live toggle") else {
        return;
    };
    let mut renderer = init(&gpu);
    // The production configuration (`cad-engine`'s `attach_with_gpu_time`).
    renderer.set_progressive_rendering(true);
    upload_only(&mut renderer, &unlit_metal());

    renderer.set_ibl_enabled(false);
    let legacy = render_frame(&mut renderer);
    // An idle frame must not change the presented pixels (the walk has no page
    // left to draw), so this is the settled legacy image the toggle must replace.
    let idle = render_frame(&mut renderer);
    assert_eq!(
        legacy, idle,
        "an idle frame with no new view input keeps the presented pixels"
    );
    assert!(
        !renderer.frame_pending(),
        "the single-page walk must be complete before the toggle"
    );

    renderer.set_ibl_enabled(true);
    let ibl = render_frame(&mut renderer);
    assert!(
        luma(ibl) > luma(legacy) + 5.0,
        "toggling IBL on must re-shade the next frame (legacy={legacy:?} \
         ibl={ibl:?}); an unchanged frame means the walk was never restarted"
    );
    assert!(
        ibl[3] > 0,
        "the toggled frame must still show the drawn surface, got {ibl:?}"
    );
}

/// ADR-0044 + ADR-0028: the engine passes `pause_after_page = true` for a frame
/// that lands during a camera gesture, which reuses the retained base page. A
/// toggle that arrives while such a pause is armed must still be visible: the
/// retained page holds the **old** shading, so the walk has to redraw page 1
/// with the new uniform (geometry and camera are unchanged, so the colour target
/// is still valid).
#[test]
fn ibl_toggle_reshades_a_frame_that_reuses_the_retained_page() {
    let Some(gpu) = gpu_or_skip("IBL retained-page toggle") else {
        return;
    };
    let mut renderer = init(&gpu);
    renderer.set_progressive_rendering(true);
    upload_only(&mut renderer, &unlit_metal());

    renderer.set_ibl_enabled(false);
    let legacy = render_frame(&mut renderer);

    renderer.set_ibl_enabled(true);
    renderer
        .render_3d_navigation(camera(), &RenderTarget::new(SIZE, SIZE), true)
        .expect("paused frame");
    let ibl = centre_pixel(&renderer);
    assert!(
        luma(ibl) > luma(legacy) + 5.0,
        "a paused frame must not keep the pre-toggle retained page \
         (legacy={legacy:?} ibl={ibl:?})"
    );
}
