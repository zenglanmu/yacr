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

// ---------------------------------------------------------------------------
// 1. Real buffer identity across admitted suffix commits
// ---------------------------------------------------------------------------

#[test]
fn retained_base_geometry_is_the_same_wgpu_buffers_across_repeated_suffix_commits() {
    const WHAT: &str = "retained_base_geometry";
    let Some(gpu) = gpu_or_skip(WHAT) else {
        return;
    };
    if !accelerated_supported(&gpu, WHAT) {
        return;
    }
    let mut renderer = init_accelerated(&gpu, WHAT);
    let target = RenderTarget::new(64, 64);
    let base = base_batches();
    let base_len = base.len();

    // Base commit with no retained prefix: nothing to establish yet.
    commit(&mut renderer, &base, 0);
    assert!(
        renderer.pulled_prefix_checkpoint.is_none(),
        "{WHAT}: a base commit with no prefix must not establish a checkpoint"
    );

    // First suffix commit: the full warm-up that establishes the checkpoint.
    commit(&mut renderer, &suffix_batches(10, 0.75), base_len);
    let checkpoint = renderer
        .pulled_prefix_checkpoint
        .clone()
        .expect("the warm-up must establish a checkpoint");
    assert_eq!(checkpoint.retained_batches, base_len, "{WHAT}");
    assert!(
        checkpoint.instance_streams >= 1 && checkpoint.mesh_chunks >= 1,
        "{WHAT}: the fixture must retain resources of BOTH pulled classes (lines={}, meshes={})",
        checkpoint.instance_streams,
        checkpoint.mesh_chunks
    );
    assert!(
        checkpoint.line_segments >= 1 && checkpoint.mesh_segments >= 1,
        "{WHAT}: the warm-up must record base segments of both pulled classes"
    );
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        0,
        "{WHAT}: the warm-up is a full rebuild, not a reuse"
    );
    assert!(renderer.pulled_prefix_rebuilds() >= 1, "{WHAT}");

    // Capture the real base buffers and the chunk tables of the warm-up.
    let base_line = base_line_buffers(&renderer, checkpoint.line_segments);
    let base_mesh = base_mesh_buffers(&renderer, checkpoint.mesh_segments);
    let line_table_before = renderer
        .pulled_lines
        .as_ref()
        .expect("line storage")
        .chunk_table
        .clone();
    let mesh_table_before = renderer
        .pulled_mesh
        .as_ref()
        .expect("mesh storage")
        .chunk_table
        .clone();
    let streams_after_warmup = renderer.instance_stream_count();
    let chunks_after_warmup = renderer.mesh_chunk_count();

    // Second suffix commit: admitted reuse of the retained base.
    commit(&mut renderer, &suffix_batches(20, -0.75), base_len);
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        1,
        "{WHAT}: the second suffix commit must reuse the retained base"
    );
    assert_eq!(
        renderer.instance_stream_count(),
        streams_after_warmup,
        "{WHAT}: the replaced suffix's streams must be collected, not accumulated"
    );
    assert_eq!(
        renderer.mesh_chunk_count(),
        chunks_after_warmup,
        "{WHAT}: the replaced suffix's mesh chunks must be collected, not accumulated"
    );
    assert_same_line_buffers(&renderer, &base_line, WHAT);
    assert_same_mesh_buffers(&renderer, &base_mesh, WHAT);

    // Identity is discriminating: the chunk table is a *new* buffer every commit,
    // so a same-buffer assertion above cannot be satisfied vacuously.
    assert!(
        renderer
            .pulled_lines
            .as_ref()
            .expect("line storage")
            .chunk_table
            != line_table_before,
        "{WHAT}: the line chunk table must be rebuilt (a fresh buffer) on every commit"
    );
    assert!(
        renderer
            .pulled_mesh
            .as_ref()
            .expect("mesh storage")
            .chunk_table
            != mesh_table_before,
        "{WHAT}: the mesh chunk table must be rebuilt (a fresh buffer) on every commit"
    );

    // The new suffix is fresh geometry, never one of the retained base buffers.
    let all_line_after_reuse = line_buffers(&renderer);
    assert!(
        all_line_after_reuse.len() > base_line.len(),
        "{WHAT}: the reuse must still build a suffix segment"
    );
    let suffix_line = &all_line_after_reuse[base_line.len()..];
    assert!(
        all_distinct(suffix_line, &base_line),
        "{WHAT}: the rebuilt suffix must use fresh buffers, distinct from the retained base"
    );
    let all_mesh_after_reuse = mesh_buffers(&renderer);
    let suffix_mesh = &all_mesh_after_reuse[base_mesh.len()..];
    assert!(
        suffix_mesh
            .iter()
            .all(|suffix| base_mesh.iter().all(|base| suffix != base)),
        "{WHAT}: the rebuilt mesh suffix must use fresh buffers, distinct from the retained base"
    );

    // Repeat: the base identity never changes and the resident resources stay
    // bounded (the previous suffix is collected every time).
    let streams_after_reuse = renderer.instance_stream_count();
    let chunks_after_reuse = renderer.mesh_chunk_count();
    let line_segments_after_reuse = renderer.pulled_line_segment_count();
    let mesh_segments_after_reuse = renderer.pulled_mesh_segment_count();
    for (index, y) in [0.85_f32, -0.45, 0.45].into_iter().enumerate() {
        commit(
            &mut renderer,
            &suffix_batches(30 + (index as u128) * 2, y),
            base_len,
        );
        assert_eq!(
            renderer.pulled_prefix_reuses(),
            2 + index as u64,
            "{WHAT}: repeat {index} must reuse the retained base again"
        );
        assert_eq!(
            renderer.instance_stream_count(),
            streams_after_reuse,
            "{WHAT}: repeat {index} must not accumulate instance streams"
        );
        assert_eq!(
            renderer.mesh_chunk_count(),
            chunks_after_reuse,
            "{WHAT}: repeat {index} must not accumulate mesh chunks"
        );
        assert_eq!(
            renderer.pulled_line_segment_count(),
            line_segments_after_reuse,
            "{WHAT}: repeat {index} must not accumulate pulled line segments"
        );
        assert_eq!(
            renderer.pulled_mesh_segment_count(),
            mesh_segments_after_reuse,
            "{WHAT}: repeat {index} must not accumulate pulled mesh segments"
        );
        assert_same_line_buffers(&renderer, &base_line, WHAT);
        assert_same_mesh_buffers(&renderer, &base_mesh, WHAT);
    }

    // The retained base still submits and draws after all the reuse commits.
    assert_draws_and_picks(&mut renderer, &target, WHAT);
}

// ---------------------------------------------------------------------------
// 2. `clear_batches` invalidates, so the next commit is a full warm-up
// ---------------------------------------------------------------------------

#[test]
fn clear_batches_forces_the_next_commit_to_be_a_full_warmup() {
    const WHAT: &str = "clear_invalidates";
    let Some(gpu) = gpu_or_skip(WHAT) else {
        return;
    };
    if !accelerated_supported(&gpu, WHAT) {
        return;
    }
    let mut renderer = init_accelerated(&gpu, WHAT);
    let base = base_batches();
    let base_len = base.len();

    commit(&mut renderer, &base, 0);
    commit(&mut renderer, &suffix_batches(10, 0.75), base_len);
    assert!(
        renderer.pulled_prefix_checkpoint.is_some(),
        "{WHAT}: the warm-up must establish a checkpoint"
    );
    let epoch_before = renderer.pulled_cache_epoch;

    renderer.clear_batches();
    assert!(
        renderer.pulled_prefix_checkpoint.is_none(),
        "{WHAT}: clear_batches must invalidate the retained prefix"
    );
    assert!(
        renderer.pulled_cache_epoch > epoch_before,
        "{WHAT}: clear_batches must bump the pulled cache epoch"
    );
    assert!(
        renderer.pulled_lines.is_none() && renderer.pulled_mesh.is_none(),
        "{WHAT}: clear_batches must drop the pulled storage"
    );

    // A commit after the clear cannot reuse anything: the checkpoint is gone.
    commit(&mut renderer, &base, 0);
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        0,
        "{WHAT}: no commit after clear_batches may count a reuse"
    );
    assert!(
        renderer.pulled_prefix_checkpoint.is_none(),
        "{WHAT}: a prefix-less commit must not establish a checkpoint"
    );

    // The first prefix-bearing commit after the clear is a full warm-up again.
    let rebuilds_before = renderer.pulled_prefix_rebuilds();
    commit(&mut renderer, &suffix_batches(20, -0.75), base_len);
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        0,
        "{WHAT}: the post-clear suffix commit must be a full rebuild, not a reuse"
    );
    assert!(
        renderer.pulled_prefix_rebuilds() > rebuilds_before,
        "{WHAT}: the post-clear suffix commit must re-establish the checkpoint"
    );
    assert!(
        renderer.pulled_prefix_checkpoint.is_some(),
        "{WHAT}: a fresh checkpoint must be established after the full warm-up"
    );

    // ...and only the commit after that reuses.
    commit(&mut renderer, &suffix_batches(30, 0.45), base_len);
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        1,
        "{WHAT}: the commit after the post-clear warm-up is the first that may reuse"
    );
}

// ---------------------------------------------------------------------------
// 3. Device recovery: documented `rebuild_device` teardown + host re-attach
//    (stale uploads rejected, new generation/device rebuilds, draws + picks)
// ---------------------------------------------------------------------------

#[test]
fn device_reinit_rejects_a_stale_upload_and_rebuilds_on_the_new_generation() {
    const WHAT: &str = "device_reinit";
    // One shared `wgpu::Instance`, two distinct device handles (see the fixture's
    // doc comment: wgpu 30's device equality is hub-local, so a shared hub is what
    // makes "a genuinely different device" assertable).
    let Some((instance, first_device, second_device)) = two_devices_from_one_instance(WHAT) else {
        return;
    };
    // The shared hub must stay alive for the whole contract: both device ids are
    // hub-local and the handles below are compared across the recoveries.
    let _shared_instance = instance;
    if !accelerated_supported(&first_device, WHAT) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let base = base_batches();
    let base_len = base.len();

    let mut renderer = init_accelerated(&first_device, WHAT);
    let generation_a = renderer.device_generation;
    commit(&mut renderer, &base, 0);
    commit(&mut renderer, &suffix_batches(10, 0.75), base_len);
    assert!(
        renderer.pulled_prefix_checkpoint.is_some(),
        "{WHAT}: the warm-up must establish a checkpoint before the reset"
    );
    let line_before_reset = line_buffers(&renderer);
    let mesh_before_reset = mesh_buffers(&renderer);
    assert_draws_and_picks(&mut renderer, &target, WHAT);

    // The attachments the documented teardown must clear actually exist here.
    assert!(
        renderer.flags_ctl_buffer.is_some()
            && renderer.flags_texture.is_some()
            && renderer.flags_bind_group.is_some(),
        "{WHAT}: the selection-flags attachments must exist before the teardown"
    );
    assert!(
        renderer.pick_target.is_some() && renderer.pick_view.is_some(),
        "{WHAT}: the id target must exist before the teardown"
    );

    // An upload staged on generation A, committed after the recovery.
    let stale_same_gpu = renderer
        .prepare_upload_batches(&suffix_batches(20, -0.75))
        .expect("stage an upload before the recovery");

    // Documented recovery sequence (`Renderer::rebuild_device` docs, renderer_impl.rs
    // 3576-3643): the host tears the derived GPU state down first and only then
    // hands in a device; re-initializing while still attached is **not** supported:
    // it leaves the previous attachment's flags control buffer (and id target)
    // behind, and `ensure_flags` then rebuilds its bind group from that stale buffer
    // — observed as a fatal wgpu-core panic ("Cannot get non-existent resource
    // BufferId(2,1)") in the pre-correction run (logs 01/02). That path is a product
    // gap recorded outside this contract and deliberately **not** fixed here.
    let epoch_before_teardown = renderer.pulled_cache_epoch;
    let rebuild = renderer.rebuild_device(&target);
    assert_eq!(
        rebuild,
        Err(CadError::GpuFailure(
            "device rebuild requires a new shared device from the host".into()
        )),
        "{WHAT}: the documented teardown must require the host to hand in a new device"
    );
    assert!(
        renderer.device_lost,
        "{WHAT}: the teardown must mark the device as lost"
    );
    // Every resource `rebuild_device` clears (renderer_impl.rs 3581-3643) is
    // verified before the new device is handed in: the checkpoint (invalidated),
    // both pulled classes, the selection-flags attachments, the id target, the
    // resident batches/streams, and the device/queue themselves. Counters that
    // `rebuild_device` does NOT touch (the monotonic reuse/rebuild diagnostics and
    // the device generation) are deliberately not asserted here.
    assert!(
        renderer.pulled_prefix_checkpoint.is_none()
            && renderer.pulled_lines.is_none()
            && renderer.pulled_mesh.is_none(),
        "{WHAT}: the teardown must drop the checkpoint and the pulled storage"
    );
    assert!(
        renderer.flags_ctl_buffer.is_none()
            && renderer.flags_texture.is_none()
            && renderer.flags_view.is_none()
            && renderer.flags_bind_group.is_none(),
        "{WHAT}: the teardown must drop the selection-flags attachments"
    );
    assert!(
        renderer.pick_target.is_none()
            && renderer.pick_view.is_none()
            && renderer.pick_depth_view.is_none(),
        "{WHAT}: the teardown must drop the id target"
    );
    assert_eq!(
        renderer.pick_size,
        (0, 0),
        "{WHAT}: the teardown must reset the id target size"
    );
    assert_eq!(
        renderer.batch_count(),
        0,
        "{WHAT}: the teardown must drop every resident batch"
    );
    assert_eq!(renderer.instance_stream_count(), 0, "{WHAT}");
    assert_eq!(renderer.mesh_chunk_count(), 0, "{WHAT}");
    assert!(
        renderer.device.is_none() && renderer.queue.is_none(),
        "{WHAT}: the teardown must drop the device/queue until the host hands a new one in"
    );
    assert!(
        renderer.pulled_cache_epoch > epoch_before_teardown,
        "{WHAT}: the teardown must invalidate the pulled cache epoch"
    );

    // Re-attach the *same* `wgpu::Device`: the device identity is unchanged, so
    // this isolates the device-**generation** rejection.
    let epoch_before = renderer.pulled_cache_epoch;
    renderer
        .initialize_with_device(first_device.device.clone(), first_device.queue.clone())
        .expect("re-attach the same device after the teardown");
    assert!(
        !renderer.device_lost,
        "{WHAT}: a successful attach must clear the device-lost state (so the stale upload cannot be rejected by it)"
    );
    assert!(
        renderer.device.as_ref() == Some(&first_device.device),
        "{WHAT}: the re-attach must hold the same device handle"
    );
    assert_eq!(
        renderer.device_generation,
        generation_a + 1,
        "{WHAT}: a successful re-attach establishes a new device generation"
    );
    assert!(
        renderer.pulled_cache_epoch > epoch_before,
        "{WHAT}: the re-attach must invalidate the pulled cache epoch"
    );
    assert!(
        renderer.pulled_prefix_checkpoint.is_none(),
        "{WHAT}: a new generation must not keep the previous checkpoint"
    );
    assert!(
        renderer.pulled_lines.is_none() && renderer.pulled_mesh.is_none(),
        "{WHAT}: the re-attach must drop the pulled storage"
    );
    assert_eq!(
        renderer.batch_count(),
        0,
        "{WHAT}: the re-attach must drop every resident batch"
    );
    assert!(
        matches!(
            renderer.commit_upload(stale_same_gpu, base_len),
            Err(CadError::StaleResult)
        ),
        "{WHAT}: an upload staged before the re-attach must be rejected by the generation check"
    );

    // A fresh base on the new generation: fresh buffers, valid draws + picks.
    commit(&mut renderer, &base, 0);
    let line_after_same_gpu_reinit = line_buffers(&renderer);
    assert!(
        all_distinct(&line_after_same_gpu_reinit, &line_before_reset),
        "{WHAT}: the re-initialized renderer must build a fresh base, never reuse the pre-re-init buffers"
    );
    assert!(
        all_distinct(&mesh_buffers(&renderer), &mesh_before_reset),
        "{WHAT}: the re-initialized renderer must build fresh mesh buffers"
    );
    assert_draws_and_picks(&mut renderer, &target, WHAT);

    // Re-establish a pulled prefix (and keep the flags/id-target attachments the
    // `assert_draws_and_picks` above created) so the second teardown has real state
    // to clear, then stage an upload on this device's generation.
    commit(&mut renderer, &suffix_batches(30, -0.85), base_len);
    assert!(
        renderer.pulled_prefix_checkpoint.is_some(),
        "{WHAT}: a checkpoint must exist before the second teardown"
    );
    assert!(
        renderer.flags_ctl_buffer.is_some() && renderer.pick_target.is_some(),
        "{WHAT}: the flags attachments and the id target must exist before the second teardown"
    );
    let stale_across_devices = renderer
        .prepare_upload_batches(&suffix_batches(40, -0.45))
        .expect("stage an upload before the device swap");
    let generation_on_first_device = renderer.device_generation;

    // A genuinely distinct device handle from the same shared instance: its
    // hub-local id differs from the first device's (asserted in the fixture), so the
    // identity difference below is meaningful rather than a cross-instance id clash.
    if !accelerated_supported(&second_device, WHAT) {
        return;
    }
    // Same documented recovery sequence before the other device is handed in.
    let rebuild_before_swap = renderer.rebuild_device(&target);
    assert_eq!(
        rebuild_before_swap,
        Err(CadError::GpuFailure(
            "device rebuild requires a new shared device from the host".into()
        )),
        "{WHAT}: the second teardown must require the host to hand in a new device"
    );
    assert!(
        renderer.pulled_prefix_checkpoint.is_none()
            && renderer.pulled_lines.is_none()
            && renderer.pulled_mesh.is_none(),
        "{WHAT}: the second teardown must drop the checkpoint and the pulled storage"
    );
    assert!(
        renderer.flags_ctl_buffer.is_none()
            && renderer.flags_texture.is_none()
            && renderer.flags_bind_group.is_none(),
        "{WHAT}: the second teardown must drop the selection-flags attachments"
    );
    assert!(
        renderer.pick_target.is_none()
            && renderer.pick_view.is_none()
            && renderer.pick_size == (0, 0),
        "{WHAT}: the second teardown must drop the id target"
    );
    assert!(
        renderer.device.is_none() && renderer.queue.is_none(),
        "{WHAT}: the second teardown must drop the device/queue until the host hands a new one in"
    );
    let epoch_before_swap = renderer.pulled_cache_epoch;
    renderer
        .initialize_with_device(second_device.device.clone(), second_device.queue.clone())
        .expect("attach the second device after the teardown");
    assert!(
        renderer.device.as_ref() == Some(&second_device.device),
        "{WHAT}: the renderer must hold the second device's handle"
    );
    assert!(
        renderer.device.as_ref() != Some(&first_device.device),
        "{WHAT}: the second attach must hold a genuinely different device handle (distinct hub-local id in the shared instance), not the first one"
    );
    assert!(
        !renderer.device_lost,
        "{WHAT}: a successful attach must clear the device-lost state (so the stale upload cannot be rejected by it)"
    );
    assert_eq!(
        renderer.device_generation,
        generation_on_first_device + 1,
        "{WHAT}: attaching another device must advance the generation"
    );
    assert!(
        renderer.pulled_cache_epoch > epoch_before_swap,
        "{WHAT}: attaching another device must invalidate the pulled cache epoch"
    );
    assert!(
        renderer.pulled_prefix_checkpoint.is_none(),
        "{WHAT}: attaching another device must not keep the previous checkpoint"
    );
    assert_eq!(renderer.batch_count(), 0, "{WHAT}");
    assert!(
        matches!(
            renderer.commit_upload(stale_across_devices, base_len),
            Err(CadError::StaleResult)
        ),
        "{WHAT}: an upload staged on the previous device must be rejected"
    );

    // A valid base on the new device: fresh buffers, and it draws **and picks**.
    // The documented teardown also cleared the id target, so `ensure_pick_target`
    // rebuilds it on this device — the previous attachment's id target can no
    // longer be reached (this is why the pre-correction "draw-only" limitation is
    // gone).
    commit(&mut renderer, &base, 0);
    let line_on_second_device = line_buffers(&renderer);
    assert!(
        all_distinct(&line_on_second_device, &line_after_same_gpu_reinit),
        "{WHAT}: the second device must build its own base buffers"
    );
    assert_draws_and_picks(&mut renderer, &target, WHAT);

    // Distinguish a same-device full replacement: it invalidates the checkpoint
    // and rebuilds the geometry, but the device generation must NOT move (it is
    // not a device recovery, and must not be recorded as one).
    commit(&mut renderer, &suffix_batches(40, 0.45), base_len);
    assert!(
        renderer.pulled_prefix_checkpoint.is_some(),
        "{WHAT}: a checkpoint must exist before the full replacement"
    );
    let line_before_replace = line_buffers(&renderer);
    let generation_before_replace = renderer.device_generation;
    commit(&mut renderer, &base, 0);
    assert_eq!(
        renderer.device_generation,
        generation_before_replace,
        "{WHAT}: a full replacement on the same device is NOT a device recovery; the generation must not move"
    );
    assert!(
        renderer.pulled_prefix_checkpoint.is_none(),
        "{WHAT}: a full replacement invalidates the retained prefix"
    );
    assert!(
        all_distinct(&line_buffers(&renderer), &line_before_replace),
        "{WHAT}: a full replacement rebuilds the pulled geometry"
    );
}
