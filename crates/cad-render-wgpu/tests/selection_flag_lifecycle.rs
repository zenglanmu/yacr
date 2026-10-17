//! The entity-flag **request lifecycle**: a transient empty resident set must
//! not drop a live selection, while a real lifecycle reset (device generation
//! change / explicit full teardown) must.
//!
//! These tests drive a real wgpu device (the acceptance machine: the Vulkan
//! P620) and assert observable behaviour: the retained request, the resolved
//! `entity_flag_keys()`, and the rendered tint. When no adapter is available
//! they print an explicit skip that is **not** a pass.
//!
//! Run (serial):
//!
//! ```text
//! cargo test -p cad-render-wgpu --test selection_flag_lifecycle --locked -- --test-threads=1 --nocapture
//! ```

use cad_domain::{DocumentId, EntityId, InstancePath, Point3, SelectionRef, TaskStamp};
use cad_render_wgpu::headless::{create_headless_gpu, HeadlessGpu};
use cad_render_wgpu::{BackendPreference, Camera2d, RenderTarget, Renderer, FLAG_SELECTED};
use cad_scene::{RenderBatch, RenderTopology, SceneDelta};

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

/// A quad mesh (two triangles) centred at the origin, so its centre pixel stays
/// the target centre under any 2D camera.
fn quad(entity: u128) -> RenderBatch {
    RenderBatch {
        conics: Vec::new(),
        local_origin: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        topology: RenderTopology::Mesh,
        vertices: vec![
            [-8.0, -8.0, 0.0],
            [-8.0, 8.0, 0.0],
            [8.0, 8.0, 0.0],
            [8.0, -8.0, 0.0],
        ],
        normals: vec![[0.0, 0.0, 1.0]; 4],
        colors: Vec::new(),
        indices: vec![[0, 2, 1], [0, 3, 2]],
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
    }
}

// ---------------------------------------------------------------------------
// Setup / readback
// ---------------------------------------------------------------------------

/// The acceptance adapter, printed before the test runs. A missing adapter skips
/// explicitly and is **not** a pass.
fn gpu() -> Option<HeadlessGpu> {
    match create_headless_gpu(BackendPreference::Auto) {
        Ok(gpu) => {
            eprintln!(
                "selection-flag lifecycle adapter: {} ({}, {})",
                gpu.adapter.name, gpu.adapter.backend, gpu.adapter.device_type
            );
            Some(gpu)
        }
        Err(error) => {
            eprintln!("skipping selection-flag lifecycle test: {error}; NOT a pass");
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
    renderer
        .initialize_with_device(device, queue)
        .expect("initialize renderer");
    renderer.set_selection_flags_enabled(true);
    renderer
}

fn render(renderer: &mut Renderer, target: &RenderTarget) -> Vec<u8> {
    renderer
        .render(camera_2d(), target)
        .expect("render color frame");
    renderer.read_target_rgba().expect("read frame").pixels
}

/// The centre pixel of a 64x64 target (inside the origin-centred quad).
fn centre_pixel(pixels: &[u8]) -> [u8; 4] {
    let index = (32usize * 64 + 32) * 4;
    [
        pixels[index],
        pixels[index + 1],
        pixels[index + 2],
        pixels[index + 3],
    ]
}

// ---------------------------------------------------------------------------
// 1. A transient empty resident set keeps the retained request (the defect)
// ---------------------------------------------------------------------------

#[test]
fn transient_empty_resident_commit_keeps_the_retained_flag_request() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);

    renderer.upload(&scene(vec![quad(1)])).expect("upload base");
    let key = renderer.pick_key_for_batch(0).expect("batch key");
    let plain = render(&mut renderer, &target);
    let plain_centre = centre_pixel(&plain);
    assert!(
        plain_centre[0] > 8 || plain_centre[1] > 8 || plain_centre[2] > 8,
        "the fixture quad must be drawn at the target centre"
    );

    renderer
        .set_entity_flags(&[(source(1), FLAG_SELECTED)])
        .expect("select");
    assert_eq!(renderer.entity_flag_keys(), vec![key]);
    let tinted = render(&mut renderer, &target);
    let tinted_centre = centre_pixel(&tinted);
    assert_ne!(
        tinted_centre, plain_centre,
        "the selected entity must be tinted"
    );

    // A **transient** empty resident commit (what the 2D navigation fallback
    // does on a zero-batch base): the key space restarts, the request must not.
    let empty = renderer
        .prepare_upload_batches(&[])
        .expect("stage empty suffix");
    renderer.commit_upload(empty, 0).expect("commit empty");
    assert_eq!(renderer.batch_count(), 0);
    assert!(
        renderer.entity_flag_keys().is_empty(),
        "the retired keys must carry no bits"
    );

    // Re-upload the SAME scene WITHOUT re-pushing any flags. If the request
    // survived, the rebuild re-resolves it and the entity is tinted again.
    renderer
        .upload(&scene(vec![quad(1)]))
        .expect("re-upload base");
    let key2 = renderer
        .pick_key_for_batch(0)
        .expect("batch key after reset");
    assert_eq!(
        renderer.entity_flag_keys(),
        vec![key2],
        "the retained request must re-resolve onto the new key"
    );
    assert_eq!(renderer.pick_key_flags(key2), FLAG_SELECTED);
    let restored = render(&mut renderer, &target);
    assert_eq!(
        restored, tinted,
        "the selection tint must survive a transient empty resident set"
    );
}

// ---------------------------------------------------------------------------
// 2. An explicit full teardown still drops the retained request
// ---------------------------------------------------------------------------

#[test]
fn explicit_clear_drops_the_retained_flag_request() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    let target = RenderTarget::new(64, 64);

    renderer.upload(&scene(vec![quad(1)])).expect("upload base");
    let plain = render(&mut renderer, &target);
    renderer
        .set_entity_flags(&[(source(1), FLAG_SELECTED)])
        .expect("select");
    assert_eq!(renderer.entity_flag_keys().len(), 1);

    // A document switch/close is an explicit full teardown: the request belongs
    // to the retired scene and must not survive it.
    renderer.clear_batches();
    renderer
        .upload(&scene(vec![quad(1)]))
        .expect("re-upload base");
    assert!(
        renderer.entity_flag_keys().is_empty(),
        "an explicit clear must drop the retained request"
    );
    let after = render(&mut renderer, &target);
    assert_eq!(
        after, plain,
        "no entity may be tinted after an explicit clear"
    );
}

// ---------------------------------------------------------------------------
// 3. A device generation change still drops the retained request
// ---------------------------------------------------------------------------

#[test]
fn device_generation_change_drops_the_retained_flag_request() {
    let Some(first) = gpu() else {
        return;
    };
    let mut renderer = init(first);
    let target = RenderTarget::new(64, 64);

    renderer.upload(&scene(vec![quad(1)])).expect("upload base");
    let plain = render(&mut renderer, &target);
    renderer
        .set_entity_flags(&[(source(1), FLAG_SELECTED)])
        .expect("select");
    assert_eq!(renderer.entity_flag_keys().len(), 1);

    // A new device generation: the old keys are gone, so the request must be
    // dropped rather than re-resolved onto a rebuilt scene.
    let Some(second) = gpu() else {
        eprintln!("second adapter unavailable; NOT a pass");
        return;
    };
    let HeadlessGpu {
        device,
        queue,
        adapter: _,
    } = second;
    // The real host flow after a device loss: tear the old generation down, then
    // initialize on the new device.
    let _ = renderer.rebuild_device(&target);
    renderer
        .initialize_with_device(device, queue)
        .expect("re-initialize on a new device generation");
    renderer.set_selection_flags_enabled(true);
    renderer
        .upload(&scene(vec![quad(1)]))
        .expect("re-upload base");
    assert!(
        renderer.entity_flag_keys().is_empty(),
        "a new device generation must drop the retained request"
    );
    let after = render(&mut renderer, &target);
    assert_eq!(
        after, plain,
        "no entity may be tinted after a device generation change"
    );
}
