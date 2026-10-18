//! Private accelerated-tier GPU contracts for the pulled-prefix fast path.
//!
//! These tests live **inside** `cad-render-wgpu` (children of the crate root
//! module), so they can read [`Renderer`]'s private pulled state and compare the
//! **actual `wgpu::Buffer` handles** the retained base geometry is built from.
//! `wgpu::Buffer` compares by handle identity (`wgpu` implements `PartialEq` for
//! every handle type from "some already-unique property", or a generated
//! never-reused identifier — see `wgpu::cmp`), so an equality here means "the very
//! same buffer was retained", which a reuse *counter* can never prove. These tests
//! also **hold clones of the captured handles** for the whole test, so the
//! identity they compare cannot be recycled: distinctness assertions between an
//! old and a rebuilt buffer stay meaningful, and a re-upload cannot masquerade as
//! a retention. The module is test-only:
//! no new public (production) API is added, and the `mod` declaration in `lib.rs`
//! is `#[cfg(test)]`-gated.
//!
//! Covered contracts (hardware; the pure invariants live in
//! `renderer_impl::pulled_prefix_tests`):
//!
//! 1. An admitted suffix commit retains the base pulled segments' **buffer
//!    identities** on a mixed line + merged-mesh scene; the chunk table is still
//!    rebuilt each commit, and repeated suffix commits leave both the base
//!    identity and the resident resource counts bounded (the replaced suffix is
//!    garbage-collected, not accumulated).
//! 2. `clear_batches` invalidates the checkpoint, so the next commit is a full
//!    warm-up, never a reuse of pre-clear geometry.
//! 3. A device recovery — the documented teardown (`Renderer::rebuild_device`,
//!    which clears the derived GPU state and requires the host to hand in a new
//!    device) followed by that hand-in — clears the checkpoint, the pulled storage,
//!    the selection-flags attachments, the id target and the device/queue **before**
//!    the attach; a `PreparedUpload` staged before it is rejected by the
//!    device-generation check (same GPU) or by device identity (a genuinely
//!    different device), and a new base then draws and picks on the new epoch. A
//!    same-device full replacement is **not** a device recovery and must not be
//!    reported as one.
//!
//! Adapter policy: every test prints the adapter it ran on. Without an adapter,
//! or without vertex-stage storage buffers, it prints `SKIP ... NOT a pass` and
//! returns: a skip is never a pass claim, and a software adapter's result is
//! labelled by the printed backend (it is never presented as real-GPU
//! acceptance).
//!
//! Contract 3 needs **two distinct device handles**, and builds them from **one
//! shared `wgpu::Instance`** (two `enumerate_adapters` calls, one device request
//! each): wgpu 30's device equality is hub-local, so devices from separate
//! instances can both carry id 0 and compare equal. That is a fixture requirement
//! for this contract — not a claim about wgpu's equality in general. See
//! `two_devices_from_one_instance`.
//!
//! Run serially (these bind a real device and poll for submissions):
//!
//! ```text
//! cargo test -p cad-render-wgpu --lib --locked -- --test-threads=1 pulled_prefix_gpu_tests
//! ```

use crate::headless::{create_headless_gpu, AdapterInfo, HeadlessGpu};
use crate::{BackendPreference, Camera2d, RenderTarget, RenderTier, Renderer};
use cad_domain::{CadError, DocumentId, EntityId, InstancePath, Point3, SelectionRef};
use cad_scene::{RenderBatch, RenderTopology};
use futures_lite::future::block_on;
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

/// A 64 px target covering exactly the world square `[-1, 1]`, matching the
/// convention the other accelerated-tier tests use.
fn camera_2d() -> Camera2d {
    Camera2d {
        center: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        world_per_px: 2.0 / 64.0,
        z_plane: 0.0,
    }
}

fn lines_batch(entity: u128, vertices: Vec<[f32; 3]>) -> RenderBatch {
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

/// An opaque, mergeable triangle quad (non-empty index list, `alpha == 1.0`).
fn quad_mesh(entity: u128, half: f32, x: f64, y: f64) -> RenderBatch {
    RenderBatch {
        conics: Vec::new(),
        local_origin: Point3 { x, y, z: 0.0 },
        topology: RenderTopology::Mesh,
        vertices: vec![
            [-half, -half, 0.0],
            [-half, half, 0.0],
            [half, half, 0.0],
            [half, -half, 0.0],
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

/// The retained base prefix: two instanced line batches **and** two mergeable
/// mesh batches, so a single committed prefix exercises **both** pulled classes.
fn base_batches() -> Vec<RenderBatch> {
    vec![
        lines_batch(1, vec![[-0.9, -0.85, 0.0], [0.9, -0.85, 0.0]]),
        lines_batch(2, vec![[-0.9, -0.6, 0.0], [0.9, -0.6, 0.0]]),
        quad_mesh(3, 0.18, -0.55, 0.25),
        quad_mesh(4, 0.18, 0.55, 0.25),
    ]
}

/// A suffix commit's batches (one instanced line batch + one mergeable mesh
/// batch). Every call replaces the previous suffix, keeping the same shape so
/// the resident resource counts are directly comparable across repeats.
fn suffix_batches(first_entity: u128, y: f32) -> Vec<RenderBatch> {
    vec![
        lines_batch(first_entity, vec![[-0.9, y, 0.0], [0.9, y, 0.0]]),
        quad_mesh(first_entity + 1, 0.12, 0.0, -0.75),
    ]
}

// ---------------------------------------------------------------------------
// Device / renderer setup
// ---------------------------------------------------------------------------

fn gpu_or_skip(what: &str) -> Option<HeadlessGpu> {
    match create_headless_gpu(BackendPreference::Auto) {
        Ok(gpu) => {
            eprintln!(
                "[pulled-prefix-gpu] {what}: adapter backend={} name={} device_type={} driver={}",
                gpu.adapter.backend, gpu.adapter.name, gpu.adapter.device_type, gpu.adapter.driver
            );
            Some(gpu)
        }
        Err(error) => {
            eprintln!(
                "[pulled-prefix-gpu] {what}: SKIP — no headless adapter ({error}); NOT a pass"
            );
            None
        }
    }
}

/// Whether this device can host the accelerated tier (vertex-stage storage
/// buffers). A device that cannot is skipped, never failed: the downgrade itself
/// is fenced by the pure tier tests.
fn accelerated_supported(gpu: &HeadlessGpu, what: &str) -> bool {
    let limits = gpu.device.limits();
    let supported = limits.max_storage_buffers_per_shader_stage >= 3
        && limits.max_storage_buffer_binding_size >= 64 * 1024
        && gpu.adapter.backend != "webgl2";
    if !supported {
        eprintln!(
            "[pulled-prefix-gpu] {what}: SKIP — adapter {} lacks vertex-stage storage buffers (max_storage_buffers_per_shader_stage={}, max_storage_buffer_binding_size={}); NOT a pass",
            gpu.adapter.backend,
            limits.max_storage_buffers_per_shader_stage,
            limits.max_storage_buffer_binding_size
        );
    }
    supported
}

/// A renderer on the accelerated tier, forced deterministically (the real device
/// still has to be able to create the pulled pipelines, or the tier downgrades
/// and this test would be vacuous).
fn init_accelerated(gpu: &HeadlessGpu, what: &str) -> Renderer {
    let mut renderer = Renderer::new(BackendPreference::Auto);
    renderer.set_storage_buffer_support_override(Some(true));
    renderer.set_poll_timeout(Duration::from_secs(30));
    renderer
        .initialize_with_device(gpu.device.clone(), gpu.queue.clone())
        .expect("initialize the accelerated renderer");
    assert_eq!(
        renderer.render_tier(),
        RenderTier::Accelerated,
        "{what}: the forced tier must be accelerated; a downgrade would make this contract vacuous"
    );
    renderer
}

fn commit(renderer: &mut Renderer, batches: &[RenderBatch], keep_prefix: usize) {
    let staged = renderer
        .prepare_upload_batches(batches)
        .expect("stage upload");
    renderer
        .commit_upload(staged, keep_prefix)
        .expect("commit upload");
}

// ---------------------------------------------------------------------------
// One-instance / two-device fixture (contract 3 only)
// ---------------------------------------------------------------------------

/// Backends the shared-instance fixture uses, matching the headless helper's
/// `Auto`/`WebGpu` resolution (Vulkan preferred, GL as the fallback).
fn test_backends() -> wgpu::Backends {
    wgpu::Backends::VULKAN | wgpu::Backends::GL
}

/// Stable lowercase device-type string, mirroring the headless helper's mapping
/// (a test-local copy: that mapping is private to the `headless` module).
fn device_type_str(device_type: wgpu::DeviceType) -> &'static str {
    match device_type {
        wgpu::DeviceType::Cpu => "cpu",
        wgpu::DeviceType::IntegratedGpu => "integrated_gpu",
        wgpu::DeviceType::DiscreteGpu => "discrete_gpu",
        wgpu::DeviceType::VirtualGpu => "virtual_gpu",
        wgpu::DeviceType::Other => "other",
    }
}

/// The same owned adapter description the headless helper builds.
fn adapter_info(info: &wgpu::AdapterInfo) -> AdapterInfo {
    AdapterInfo {
        backend: info.backend.to_str().to_string(),
        name: info.name.clone(),
        device_type: device_type_str(info.device_type).to_string(),
        driver: info.driver.clone(),
        driver_info: info.driver_info.clone(),
    }
}

/// The Vulkan adapter reporting `P620` (the acceptance machine's discrete GPU),
/// or `None` — the caller then skips, explicitly not as a pass.
fn select_p620(adapters: &[wgpu::Adapter]) -> Option<usize> {
    adapters.iter().position(|adapter| {
        let info = adapter.get_info();
        info.backend == wgpu::Backend::Vulkan && info.name.contains("P620")
    })
}

/// Request exactly one device from `adapter`. Each adapter handle is requested
/// from once (one adapter -> one device), matching the WebGPU model.
fn request_device_for(adapter: &wgpu::Adapter, what: &str) -> Option<HeadlessGpu> {
    let info = adapter.get_info();
    match block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("cad-headless-device"),
        ..Default::default()
    })) {
        Ok((device, queue)) => Some(HeadlessGpu {
            device,
            queue,
            adapter: adapter_info(&info),
        }),
        Err(error) => {
            eprintln!(
                "[pulled-prefix-gpu] {what}: SKIP — device request failed ({error}); NOT a pass"
            );
            None
        }
    }
}

/// Two **genuinely distinct** device handles brought up from **one shared
/// `wgpu::Instance`**.
///
/// Why the instance is shared: wgpu 30's device equality proxies the *hub-local*
/// device id, not the context, so two devices obtained from two different
/// `wgpu::Instance`s can both carry id 0 and compare equal — which is exactly what
/// made an earlier revision of this contract fail on its "genuinely different
/// device" assert (run `04`). Bringing both devices up in one context makes their
/// ids unique, so device inequality is meaningful again. This is a **fixture**
/// requirement for this contract; it is not a claim that wgpu's handle equality is
/// globally broken.
///
/// Two `enumerate_adapters` calls on the shared instance yield two fresh adapter
/// handles for the same physical GPU (fresh hub-local adapter ids), and each
/// adapter is asked for exactly one device. The instance is handed back so the
/// caller keeps the shared hub alive for the whole contract.
fn two_devices_from_one_instance(what: &str) -> Option<(wgpu::Instance, HeadlessGpu, HeadlessGpu)> {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = test_backends();
    let instance = wgpu::Instance::new(desc);

    let first_adapters = block_on(instance.enumerate_adapters(test_backends()));
    let second_adapters = block_on(instance.enumerate_adapters(test_backends()));
    let (Some(first_index), Some(second_index)) =
        (select_p620(&first_adapters), select_p620(&second_adapters))
    else {
        eprintln!(
            "[pulled-prefix-gpu] {what}: SKIP — no Vulkan adapter reporting P620 in both enumerations of one instance; NOT a pass"
        );
        return None;
    };
    let first = request_device_for(&first_adapters[first_index], what)?;
    let second = request_device_for(&second_adapters[second_index], what)?;

    // The fixture's own preconditions, asserted before any contract state exists:
    // the same physical model on both sides, and two distinct device handles in one
    // context (unique hub-local ids).
    assert!(
        first.adapter.name.contains("P620") && second.adapter.name.contains("P620"),
        "{what}: the fixture must run on the P620 on both sides (first={}, second={})",
        first.adapter.name,
        second.adapter.name
    );
    assert!(
        first.device != second.device,
        "{what}: the fixture must produce two distinct device handles in one shared instance"
    );
    eprintln!(
        "[pulled-prefix-gpu] {what}: adapter backend={} name={} device_type={} driver={} (one shared instance, two fresh adapter handles, two distinct device ids)",
        first.adapter.backend, first.adapter.name, first.adapter.device_type, first.adapter.driver
    );
    Some((instance, first, second))
}

// ---------------------------------------------------------------------------
// Readback / pick helpers
// ---------------------------------------------------------------------------

fn ink_count(pixels: &[u8]) -> usize {
    pixels
        .chunks_exact(4)
        .filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8)
        .count()
}

/// The first pixel the colour pass drew that the id pass also resolves. A plain
/// "first inked pixel" scan can land on an anti-aliased fringe below the id
/// pass's coverage threshold, so the pick is confirmed while scanning.
fn first_pickable_pixel(
    renderer: &mut Renderer,
    pixels: &[u8],
    width: u32,
    height: u32,
) -> Option<(u32, u32)> {
    assert!(
        pixels.len() >= (width as usize) * (height as usize) * 4,
        "the readback must cover the whole target"
    );
    for y in 0..height {
        for x in 0..width {
            let i = (y as usize * width as usize + x as usize) * 4;
            let drawn = pixels[i] > 8 || pixels[i + 1] > 8 || pixels[i + 2] > 8;
            if drawn
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

/// Render the resident scene and read the target back, asserting it drew ink — a
/// real submit + readback, not a claim about the CPU-side state.
fn render_and_read(renderer: &mut Renderer, target: &RenderTarget, what: &str) -> Vec<u8> {
    renderer.render(camera_2d(), target).expect("render frame");
    let pixels = renderer
        .read_target_rgba()
        .expect("read the rendered frame back")
        .pixels;
    assert!(
        ink_count(&pixels) > 0,
        "{what}: the rendered frame must draw ink"
    );
    pixels
}

/// Render, read back, and confirm the frame also resolves a pick to a resident
/// batch key (the colour pass and the id pass agree).
fn assert_draws_and_picks(renderer: &mut Renderer, target: &RenderTarget, what: &str) {
    let pixels = render_and_read(renderer, target, what);
    let (x, y) = first_pickable_pixel(renderer, &pixels, target.width, target.height)
        .unwrap_or_else(|| {
            panic!("{what}: the colour pass must cover a pixel the id pass resolves")
        });
    let hit = renderer
        .pick_at(x, y)
        .expect("pick the covered pixel")
        .unwrap_or_else(|| panic!("{what}: the covered pixel must resolve to a hit"));
    let keys: Vec<u32> = (0..renderer.batch_count())
        .filter_map(|index| renderer.pick_key_for_batch(index))
        .collect();
    assert!(
        keys.contains(&hit.entity_key),
        "{what}: the pick must resolve to a resident batch's key ({})",
        hit.entity_key
    );
}

// ---------------------------------------------------------------------------
// Pulled-state helpers (private-field access)
// ---------------------------------------------------------------------------

fn line_buffers(renderer: &Renderer) -> Vec<wgpu::Buffer> {
    renderer
        .pulled_lines
        .as_ref()
        .expect("the accelerated line storage must exist")
        .segments
        .iter()
        .map(|segment| segment.geometry_buffer.clone())
        .collect()
}

fn mesh_buffers(renderer: &Renderer) -> Vec<(wgpu::Buffer, wgpu::Buffer)> {
    renderer
        .pulled_mesh
        .as_ref()
        .expect("the accelerated mesh storage must exist")
        .segments
        .iter()
        .map(|segment| (segment.vertex_buffer.clone(), segment.index_buffer.clone()))
        .collect()
}

/// The retained base line segments as their real buffer handles.
fn base_line_buffers(renderer: &Renderer, count: usize) -> Vec<wgpu::Buffer> {
    let all = line_buffers(renderer);
    assert!(
        all.len() >= count,
        "expected at least {count} pulled line segments, found {}",
        all.len()
    );
    all[..count].to_vec()
}

/// The retained base mesh segments as their real (vertex, index) buffer handles.
fn base_mesh_buffers(renderer: &Renderer, count: usize) -> Vec<(wgpu::Buffer, wgpu::Buffer)> {
    let all = mesh_buffers(renderer);
    assert!(
        all.len() >= count,
        "expected at least {count} pulled mesh segments, found {}",
        all.len()
    );
    all[..count].to_vec()
}

fn assert_same_line_buffers(renderer: &Renderer, expected: &[wgpu::Buffer], what: &str) {
    let now = base_line_buffers(renderer, expected.len());
    for (index, (retained, before)) in now.iter().zip(expected).enumerate() {
        assert!(
            retained == before,
            "{what}: retained base line segment {index} is a different wgpu::Buffer — the base geometry was re-uploaded instead of reused"
        );
    }
}

fn assert_same_mesh_buffers(
    renderer: &Renderer,
    expected: &[(wgpu::Buffer, wgpu::Buffer)],
    what: &str,
) {
    let now = base_mesh_buffers(renderer, expected.len());
    for (index, ((v_now, i_now), (v_before, i_before))) in now.iter().zip(expected).enumerate() {
        assert!(
            v_now == v_before,
            "{what}: retained base mesh segment {index} vertex buffer is a different wgpu::Buffer"
        );
        assert!(
            i_now == i_before,
            "{what}: retained base mesh segment {index} index buffer is a different wgpu::Buffer"
        );
    }
}

/// `true` when no buffer in `left` is handle-identical to any buffer in `right`.
fn all_distinct<T: PartialEq>(left: &[T], right: &[T]) -> bool {
    left.iter().all(|a| right.iter().all(|b| a != b))
}

mod tests;
