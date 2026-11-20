//! Native GPU contracts for the textured-mesh channel (ADR-0038 D9/D10).
//!
//! These tests live inside `cad-render-wgpu`, so they bind a real headless device,
//! upload a real decoded texture and sample the presented pixels. They auto-skip
//! when no adapter is available (`SKIP ... NOT a pass`), and a software adapter's
//! result is layout/geometry evidence only — never a performance or vendor claim.
//!
//! What is contracted:
//!
//! 1. A **textured** mesh samples its texture: the drawn pixel is the texel
//!    colour (times the batch tint), not the batch colour, and the batch is still
//!    pickable through the unchanged id pass.
//! 2. An **unresolved** texture key is never replaced by a placeholder: the batch
//!    draws with the untextured pipeline (its batch colour) and the key is
//!    reported by [`Renderer::mesh_texture_fallbacks`].
//!
//! Run serially (these bind a real device):
//!
//! ```text
//! cargo test -p cad-render-wgpu --lib --locked -- --test-threads=1 mesh_textured_gpu_tests
//! ```
use crate::headless::{create_headless_gpu, HeadlessGpu};
use crate::{BackendPreference, Camera2d, RenderTarget, Renderer};
use cad_domain::{DocumentId, EntityId, InstancePath, Point3, SelectionRef};
use cad_resources::{DecodedImage, DecodedImageCache, ResourceKey, ResourceLimits};
use cad_scene::{RenderBatch, RenderTopology};
use std::sync::Arc;
use std::time::Duration;

const SIZE: u32 = 64;
const WORLD_PER_PX: f64 = 2.0 / 64.0;
/// A canonical document texture key (`<32 lowercase hex>.png`).
const TEXTURE_KEY: &str = "4c2f2e0a5f3d9b7c1e8a6d4b2c0f9e71.png";

fn gpu_or_skip(what: &str) -> Option<HeadlessGpu> {
    match create_headless_gpu(BackendPreference::Auto) {
        Ok(gpu) => {
            eprintln!(
                "[mesh-textured-gpu] {what}: adapter backend={} name={} device_type={} driver={}",
                gpu.adapter.backend, gpu.adapter.name, gpu.adapter.device_type, gpu.adapter.driver
            );
            Some(gpu)
        }
        Err(error) => {
            eprintln!(
                "[mesh-textured-gpu] {what}: SKIP — no headless adapter ({error}); NOT a pass"
            );
            None
        }
    }
}

fn init(gpu: &HeadlessGpu) -> Renderer {
    let mut renderer = Renderer::new(BackendPreference::Auto);
    renderer.set_poll_timeout(Duration::from_secs(30));
    renderer
        .initialize_with_device(gpu.device.clone(), gpu.queue.clone())
        .expect("initialize the renderer");
    renderer
}

fn camera() -> Camera2d {
    Camera2d {
        center: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        world_per_px: WORLD_PER_PX,
        z_plane: 0.0,
    }
}

fn source(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

/// One world-space quad (`[-0.5, 0.5]²` at z = 0) with UV0 covering the unit
/// square, referencing `texture`.
fn textured_quad(color: [f32; 3], texture: &str) -> RenderBatch {
    RenderBatch {
        uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
        texture: Some(ResourceKey(texture.to_owned())),
        conics: Vec::new(),
        local_origin: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        topology: RenderTopology::Mesh,
        vertices: vec![
            [-0.5, -0.5, 0.0],
            [0.5, -0.5, 0.0],
            [0.5, 0.5, 0.0],
            [-0.5, 0.5, 0.0],
        ],
        normals: vec![[0.0, 0.0, 1.0]; 4],
        colors: Vec::new(),
        indices: vec![[0, 1, 2], [0, 2, 3]],
        edges: Vec::new(),
        mirrored: false,
        alpha: 1.0,
        color,
        color_unresolved: false,
        lineweight: 0.0,
        lineweight_unresolved: true,
        linetype: cad_scene::LinetypePattern::continuous(),
        linetype_unresolved: true,
        sources: vec![source(9)],
        draw_order: 0,
    }
}

/// A cache holding one opaque red 1×1 texture under [`TEXTURE_KEY`].
fn red_texture_cache() -> DecodedImageCache {
    let mut cache = DecodedImageCache::new();
    cache
        .insert(
            ResourceKey(TEXTURE_KEY.to_owned()),
            DecodedImage {
                width: 1,
                height: 1,
                rgba: Arc::from([255u8, 0, 0, 255].as_slice()),
            },
            &ResourceLimits::default(),
        )
        .expect("the fixture texture fits the resource budget");
    cache
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

#[test]
fn a_textured_mesh_samples_its_texture_and_is_still_pickable() {
    let Some(gpu) = gpu_or_skip("textured mesh draw + pick") else {
        return;
    };
    let mut renderer = init(&gpu);
    // A white tint isolates the sampled texel: an untextured white quad would come
    // out white, so a red centre can only be the texture the batch references.
    let batch = textured_quad([1.0, 1.0, 1.0], TEXTURE_KEY);
    renderer
        .prepare_upload_batches(std::slice::from_ref(&batch))
        .and_then(|staged| renderer.commit_upload(staged, 0))
        .expect("commit the textured scene");
    let report = renderer
        .upload_mesh_textures(&[ResourceKey(TEXTURE_KEY.to_owned())], &red_texture_cache())
        .expect("upload the mesh texture");
    assert_eq!(report.uploaded, 1);
    assert!(report.unresolved.is_empty(), "the fixture resolves");
    renderer
        .render(camera(), &RenderTarget::new(SIZE, SIZE))
        .expect("render");

    let pixel = centre_pixel(&renderer);
    assert!(
        pixel[0] > 200 && pixel[1] < 60 && pixel[2] < 60,
        "the centre must be the red texel (times the white tint), got {pixel:?}"
    );
    assert_eq!(renderer.mesh_texture_fallbacks(), &[]);

    // Picking is untouched: the textured batch is not merged, so it takes the
    // legacy per-batch id pass exactly like an untextured mesh.
    let hit = renderer
        .pick_at(SIZE / 2, SIZE / 2)
        .expect("run the id pass")
        .expect("the quad covers the centre");
    assert_eq!(hit.entity_key, 1, "one resident batch owns key 1");
    assert_eq!(
        hit.source.expect("exact single-source hit").entity,
        EntityId(9)
    );
}

#[test]
fn an_unresolved_texture_draws_untextured_and_is_reported() {
    let Some(gpu) = gpu_or_skip("unresolved mesh texture") else {
        return;
    };
    let mut renderer = init(&gpu);
    let batch = textured_quad([0.0, 1.0, 0.0], TEXTURE_KEY);
    renderer
        .prepare_upload_batches(std::slice::from_ref(&batch))
        .and_then(|staged| renderer.commit_upload(staged, 0))
        .expect("commit the textured scene");

    // Nothing is uploaded: the key is not resident.
    let report = renderer
        .upload_mesh_textures(
            &[ResourceKey(TEXTURE_KEY.to_owned())],
            &DecodedImageCache::new(),
        )
        .expect("an unresolved key is reported, not an error");
    assert_eq!(report.uploaded, 0);
    assert_eq!(
        report
            .unresolved
            .iter()
            .map(|key| key.as_str())
            .collect::<Vec<_>>(),
        vec![TEXTURE_KEY]
    );
    assert!(report.diagnostic().is_some(), "the drop is observable");

    renderer
        .render(camera(), &RenderTarget::new(SIZE, SIZE))
        .expect("render");
    let pixel = centre_pixel(&renderer);
    assert!(
        pixel[1] > 200 && pixel[0] < 60 && pixel[2] < 60,
        "without a texture the batch keeps its own colour (never a placeholder), got {pixel:?}"
    );
    assert_eq!(
        renderer
            .mesh_texture_fallbacks()
            .iter()
            .map(|key| key.as_str())
            .collect::<Vec<_>>(),
        vec![TEXTURE_KEY],
        "the draw-time fallback names the unresolved key"
    );
}
