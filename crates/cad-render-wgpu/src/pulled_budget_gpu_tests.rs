//! Private GPU contracts for the pulled **capacity gate** and its explicit
//! fallback (hardware; the pure byte/verdict invariants live in
//! `renderer_impl::pulled_budget_tests`).
//!
//! Why they live inside the crate: they read [`Renderer`]'s private resident
//! classes and `pulled_state` field, and drive the **test-only** overrides
//! (`set_pulled_memory_budget_override`, `set_pulled_oom_injection`).
//!
//! Covered contracts:
//!
//! 1. An **under-budget** scene still builds and uses the pulled organisation
//!    (`PulledState::Active`, `FrameStats::accelerated == true`) — no regression
//!    for small scenes.
//! 2. An **over-budget** requirement is rejected by the pre-flight gate: the
//!    reported `required_bytes` equals the exact payload the resident classes
//!    need (recomputed from the real types), the budget is the injected value,
//!    the pulled classes are absent, and the frame draws through the universal
//!    per-run path with **byte-identical pixels**. An exact fit is accepted.
//! 3. A **captured out-of-memory error** discards the pulled state instead of
//!    panicking, falls back to the universal path, and is reported as
//!    `PulledState::SkippedOutOfMemory`.
//!
//! Honesty boundary on contract 3: the injected message proves the
//! capture → discard → fallback **handler**; it does not prove that this
//! machine's driver routes a real OOM into the scope. No portable test can
//! provoke a real GPU OOM deterministically, so the wgpu-level routing is
//! verified by reading the pinned wgpu 30.0.1 backend source (see
//! `capture_oom_scope`) rather than by faking a driver failure.
//!
//! Adapter policy: every test prints the adapter it ran on. Without an adapter,
//! or without vertex-stage storage buffers, it prints `SKIP ... NOT a pass` and
//! returns: a skip is never a pass claim.
//!
//! Run serially (these bind a real device and poll for submissions):
//!
//! ```text
//! cargo test -p cad-render-wgpu --lib --locked -- --test-threads=1 pulled_budget_gpu_tests
//! ```

use crate::headless::{create_headless_gpu, HeadlessGpu};
use crate::renderer_impl::pulled_payload_bytes;
use crate::{BackendPreference, Camera2d, PulledState, RenderTarget, RenderTier, Renderer};
use cad_domain::{DocumentId, EntityId, InstancePath, Point3, SelectionRef};
use cad_scene::{RenderBatch, RenderTopology};

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

fn lines_batch(entity: u128, y: f32) -> RenderBatch {
    RenderBatch {
        conics: Vec::new(),
        local_origin: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        topology: RenderTopology::Lines,
        vertices: vec![[-0.9, y, 0.0], [0.9, y, 0.0]],
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

/// An opaque, mergeable triangle quad.
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

/// Two instanced line batches (both classes are then exercised) and one
/// mergeable mesh.
fn scene() -> Vec<RenderBatch> {
    vec![
        lines_batch(1, -0.4),
        lines_batch(2, 0.4),
        quad_mesh(3, 0.2, 0.0, 0.0),
    ]
}

fn gpu_or_skip(what: &str) -> Option<HeadlessGpu> {
    match create_headless_gpu(BackendPreference::Auto) {
        Ok(gpu) => {
            eprintln!(
                "[pulled-budget-gpu] {what}: adapter backend={} name={} device_type={} driver={}",
                gpu.adapter.backend, gpu.adapter.name, gpu.adapter.device_type, gpu.adapter.driver
            );
            Some(gpu)
        }
        Err(error) => {
            eprintln!(
                "[pulled-budget-gpu] {what}: SKIP — no headless adapter ({error}); NOT a pass"
            );
            None
        }
    }
}

/// Whether this device can host the accelerated tier (vertex-stage storage
/// buffers). A device that cannot is skipped, never failed.
fn accelerated_supported(gpu: &HeadlessGpu, what: &str) -> bool {
    let limits = gpu.device.limits();
    let supported = limits.max_storage_buffers_per_shader_stage >= 3
        && limits.max_storage_buffer_binding_size >= 64 * 1024
        && gpu.adapter.backend != "webgl2";
    if !supported {
        eprintln!(
            "[pulled-budget-gpu] {what}: SKIP — adapter lacks vertex-stage storage buffers; NOT a pass"
        );
    }
    supported
}

fn init_accelerated(gpu: &HeadlessGpu, what: &str) -> Renderer {
    let mut renderer = Renderer::new(BackendPreference::Auto);
    renderer.set_storage_buffer_support_override(Some(true));
    renderer
        .initialize_with_device(gpu.device.clone(), gpu.queue.clone())
        .expect("initialize the accelerated renderer");
    assert_eq!(
        renderer.render_tier(),
        RenderTier::Accelerated,
        "{what}: the forced tier must be accelerated"
    );
    renderer
}

fn init_universal(gpu: &HeadlessGpu, what: &str) -> Renderer {
    let mut renderer = Renderer::new(BackendPreference::Auto);
    renderer.set_storage_buffer_support_override(Some(false));
    renderer
        .initialize_with_device(gpu.device.clone(), gpu.queue.clone())
        .expect("initialize the universal renderer");
    assert_eq!(
        renderer.render_tier(),
        RenderTier::Universal,
        "{what}: the forced tier must be universal"
    );
    renderer
}

fn commit(renderer: &mut Renderer, batches: &[RenderBatch]) {
    let staged = renderer
        .prepare_upload_batches(batches)
        .expect("stage upload");
    renderer.commit_upload(staged, 0).expect("commit upload");
}

fn render_and_read(renderer: &mut Renderer, target: &RenderTarget) -> (Vec<u8>, crate::FrameStats) {
    let stats = renderer.render(camera_2d(), target).expect("render frame");
    let pixels = renderer
        .read_target_rgba()
        .expect("read the rendered frame back")
        .pixels;
    (pixels, stats)
}

/// The exact bytes one rebuild would allocate for the renderer's resident
/// classes, recomputed here from their real record counts (never copied from the
/// value under test) so the reported `required_bytes` is checked, not trusted.
fn resident_required_bytes(renderer: &Renderer) -> u64 {
    let line_instances: Vec<usize> = renderer
        .instance_streams
        .iter()
        .map(|stream| stream.pulled_instances.len())
        .collect();
    let mesh_vertices: Vec<usize> = renderer
        .mesh_chunks
        .iter()
        .map(|chunk| chunk.pulled_vertices.len())
        .collect();
    let mesh_indices: Vec<usize> = renderer
        .mesh_chunks
        .iter()
        .map(|chunk| chunk.pulled_indices.len())
        .collect();
    pulled_payload_bytes(&line_instances, &mesh_vertices, &mesh_indices)
}

#[test]
fn under_budget_scene_still_uses_the_pulled_path() {
    let what = "under-budget";
    let Some(gpu) = gpu_or_skip(what) else {
        return;
    };
    if !accelerated_supported(&gpu, what) {
        return;
    }
    let mut renderer = init_accelerated(&gpu, what);
    commit(&mut renderer, &scene());
    assert_eq!(
        renderer.pulled_state(),
        PulledState::Active,
        "{what}: a small scene must still build the pulled classes"
    );
    assert!(
        renderer.pulled_lines.is_some(),
        "{what}: the line class must be resident"
    );
    assert!(
        renderer.pulled_mesh.is_some(),
        "{what}: the mesh class must be resident"
    );
    let target = RenderTarget::new(64, 64);
    let (pixels, stats) = render_and_read(&mut renderer, &target);
    assert!(
        pixels
            .chunks_exact(4)
            .any(|px| px[0] > 8 || px[1] > 8 || px[2] > 8),
        "{what}: the frame must draw ink"
    );
    assert_eq!(stats.pulled_state, PulledState::Active);
    assert!(
        stats.accelerated,
        "{what}: the accelerated organisation must be used"
    );
}

#[test]
fn over_budget_requirement_is_rejected_and_reported_exactly() {
    let what = "over-budget";
    let Some(gpu) = gpu_or_skip(what) else {
        return;
    };
    if !accelerated_supported(&gpu, what) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let mut renderer = init_accelerated(&gpu, what);
    commit(&mut renderer, &scene());
    assert_eq!(renderer.pulled_state(), PulledState::Active);
    let required = resident_required_bytes(&renderer);
    assert!(required > 0, "{what}: the fixture must convert geometry");

    // One byte below the exact requirement: the gate must reject. The budget is
    // injected, so no real GPU OOM is needed (and none would be reachable with a
    // scene this small).
    let budget = required - 1;
    renderer.set_pulled_memory_budget_override(Some(budget));
    commit(&mut renderer, &scene());
    assert_eq!(
        renderer.pulled_state(),
        PulledState::SkippedInsufficientVram {
            required_bytes: required,
            budget_bytes: budget,
        },
        "{what}: the reported requirement must be the exact payload"
    );
    assert!(
        renderer.pulled_lines.is_none() && renderer.pulled_mesh.is_none(),
        "{what}: a skipped rebuild must leave no pulled class resident"
    );
    let (skipped_px, skipped_stats) = render_and_read(&mut renderer, &target);
    assert!(
        !skipped_stats.accelerated,
        "{what}: a skipped rebuild must not be drawn through the pulled path"
    );
    assert!(
        !skipped_stats.pulled_state.allows_accelerated(),
        "{what}: the reported state must not permit the accelerated path"
    );
    assert_eq!(
        skipped_stats.pulled_state.as_str(),
        "skipped-insufficient-vram"
    );

    // The universal tier must draw the same pixels for the same scene: the
    // fallback is the existing fenced path, not a visual change.
    let mut universal = init_universal(&gpu, what);
    commit(&mut universal, &scene());
    let (universal_px, universal_stats) = render_and_read(&mut universal, &target);
    assert!(!universal_stats.accelerated);
    assert_eq!(
        skipped_px, universal_px,
        "{what}: the fallback frame must be byte-identical to the universal tier"
    );

    // An exact fit is accepted again (the gate is `>`, not `>=`), proving the
    // skip is the budget verdict and not a side effect of the override.
    renderer.set_pulled_memory_budget_override(Some(required));
    commit(&mut renderer, &scene());
    assert_eq!(
        renderer.pulled_state(),
        PulledState::Active,
        "{what}: an exact fit must build the pulled classes"
    );
    let (refit_px, refit_stats) = render_and_read(&mut renderer, &target);
    assert!(refit_stats.accelerated);
    assert_eq!(
        refit_px, universal_px,
        "{what}: the pulled frame must be byte-identical to the universal tier"
    );
}

#[test]
fn captured_out_of_memory_falls_back_instead_of_panicking() {
    let what = "captured-oom";
    let Some(gpu) = gpu_or_skip(what) else {
        return;
    };
    if !accelerated_supported(&gpu, what) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let mut renderer = init_accelerated(&gpu, what);
    let required = {
        commit(&mut renderer, &scene());
        resident_required_bytes(&renderer)
    };
    // Inject the captured error through the same handler the real scope feeds.
    renderer.set_pulled_oom_injection(Some("injected test out-of-memory"));
    commit(&mut renderer, &scene());
    match renderer.pulled_state() {
        PulledState::SkippedOutOfMemory {
            required_bytes,
            budget_bytes,
            message,
        } => {
            assert_eq!(required_bytes, required, "{what}: the estimate is carried");
            assert!(
                budget_bytes >= required,
                "{what}: the injected capture is the out-of-budget case"
            );
            assert!(
                message.contains("injected test out-of-memory"),
                "{what}: the captured message must be reported verbatim ({message})"
            );
        }
        other => panic!("{what}: expected a captured-OOM skip, got {other:?}"),
    }
    assert!(
        renderer.pulled_lines.is_none() && renderer.pulled_mesh.is_none(),
        "{what}: the partial pulled state must be discarded"
    );
    let (fallback_px, fallback_stats) = render_and_read(&mut renderer, &target);
    assert!(
        !fallback_stats.accelerated,
        "{what}: the capture must fall back to the universal path"
    );
    assert_eq!(
        fallback_stats.pulled_state.as_str(),
        "skipped-out-of-memory"
    );
    assert!(
        fallback_px
            .chunks_exact(4)
            .any(|px| px[0] > 8 || px[1] > 8 || px[2] > 8),
        "{what}: the fallback frame must draw real geometry, not a placeholder"
    );

    // The fallback is the universal path's rendering, byte for byte.
    let mut universal = init_universal(&gpu, what);
    commit(&mut universal, &scene());
    let (universal_px, _) = render_and_read(&mut universal, &target);
    assert_eq!(
        fallback_px, universal_px,
        "{what}: the fallback frame must be byte-identical to the universal tier"
    );
}
