//! Renderer-level contract: a **suffix-only** overlay commit preserves the
//! adaptive page-budget controller's cost knowledge, while a base-changing
//! commit still resets it.
//!
//! The runtime re-uploads a transient overlay range (the deferred reference-grid
//! refresh) once per camera settle with `keep_prefix == the resident base batch
//! count`. That is a suffix commit: it appends/replaces only the overlay suffix
//! and leaves the base draw-cost profile unchanged. Resetting the controller on
//! every such commit knocked the learned `scale` back to `1.0` every settle, so
//! the controller never converged and pages stayed vertex-heavy (the reported
//! full-view jank). These tests pin the fix at the renderer boundary, using the
//! public `pacing_scale()` / `pacing_cost_ema_ms()` accessors and the `FrameStats`
//! budget fields.
//!
//! A GPU is required; without an adapter every test skips with a clear message
//! rather than pretending to pass. Run serially:
//!
//! ```text
//! cargo test -p cad-render-wgpu --test pacing_commit_suffix --locked -- --test-threads=1
//! ```

use cad_domain::{DocumentId, EntityId, InstancePath, Point3, SelectionRef};
use cad_render_wgpu::headless::{create_headless_gpu, HeadlessGpu};
use cad_render_wgpu::{BackendPreference, Camera2d, RenderTarget, Renderer};
use cad_scene::{RenderBatch, RenderTopology};

fn source() -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(1),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn gpu() -> Option<HeadlessGpu> {
    match create_headless_gpu(BackendPreference::Auto) {
        Ok(gpu) => Some(gpu),
        Err(error) => {
            eprintln!("skipping pacing suffix-commit GPU test: {error}");
            None
        }
    }
}

fn lines_batch(vertices: Vec<[f32; 3]>) -> RenderBatch {
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
        sources: vec![source()],
        draw_order: 0,
    }
}

/// `n` independent one-segment line batches (each charges 6 vertices / 2
/// triangles), so a 6-vertex page holds exactly one batch.
fn line_batches(n: usize) -> Vec<RenderBatch> {
    (0..n)
        .map(|i| {
            let y = -0.8 + i as f32 * 0.1;
            lines_batch(vec![[-0.8, y, 0.0], [0.8, y, 0.0]])
        })
        .collect()
}

fn init(gpu: HeadlessGpu) -> Renderer {
    let HeadlessGpu {
        device,
        queue,
        adapter: _,
    } = gpu;
    let mut renderer = Renderer::new(BackendPreference::Auto);
    // The universal per-run path (progressive pages); the accelerated tier is
    // fenced in `tests/accelerated_tier.rs`.
    renderer.set_storage_buffer_support_override(Some(false));
    renderer
        .initialize_with_device(device, queue)
        .expect("initialize headless renderer");
    renderer
}

fn camera_2d() -> Camera2d {
    Camera2d {
        center: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        // 64 px span 2 world units, so [-1, 1] exactly fills the target.
        world_per_px: 2.0 / 64.0,
        z_plane: 0.0,
    }
}

/// A renderer with an 8-batch base scene, progressive pacing and a target so low
/// that every observed frame is over-target: the controller fast-shrinks and
/// never climbs back up on its own.
fn adapted_renderer(gpu: HeadlessGpu) -> (Renderer, RenderTarget) {
    let mut renderer = init(gpu);
    renderer.frame_budget.max_vertices = 6;
    renderer.frame_budget.max_triangles = 2;
    let staged = renderer
        .prepare_upload_batches(&line_batches(8))
        .expect("stage the base scene");
    renderer
        .commit_upload(staged, 0)
        .expect("commit the base scene");
    renderer.set_progressive_rendering(true);
    renderer.set_frame_budget_target_ms(0.001);

    let target = RenderTarget::new(64, 64);
    let mut frames = 0;
    while renderer.pacing_scale() >= 1.0 && frames < 40 {
        renderer.render(camera_2d(), &target).unwrap();
        frames += 1;
    }
    assert!(
        renderer.pacing_scale() < 1.0 && renderer.pacing_cost_ema_ms() > 0.0,
        "the controller must have adapted before the commit is meaningful"
    );
    (renderer, target)
}

/// A suffix-only commit (`keep_prefix == the resident base batch count`) must
/// preserve both the learned scale and the cost EMA; a base change
/// (`keep_prefix == 0`) must still reset them to the fresh state.
#[test]
fn suffix_only_commit_preserves_pacing_knowledge_while_a_base_change_resets_it() {
    let Some(gpu) = gpu() else {
        return;
    };
    let (mut renderer, _target) = adapted_renderer(gpu);
    let scale_before = renderer.pacing_scale();
    let ema_before = renderer.pacing_cost_ema_ms();

    // The overlay suffix: append two batches onto the resident base (the same
    // shape as the runtime's deferred reference-grid refresh).
    let resident = renderer.batch_count();
    assert_eq!(resident, 8, "the base scene must be resident");
    let overlay = renderer
        .prepare_upload_batches(&line_batches(2))
        .expect("stage the overlay suffix");
    renderer
        .commit_upload(overlay, resident)
        .expect("commit the overlay suffix");
    assert_eq!(
        renderer.pacing_scale(),
        scale_before,
        "a suffix-only commit must not reset the learned scale"
    );
    assert_eq!(
        renderer.pacing_cost_ema_ms(),
        ema_before,
        "a suffix-only commit must not reset the cost EMA"
    );
    assert_eq!(renderer.batch_count(), 10, "the suffix was appended");

    // A base change replaces the scene, so the cost knowledge is stale and must
    // reset.
    let replacement = renderer
        .prepare_upload_batches(&line_batches(8))
        .expect("stage the base replacement");
    renderer
        .commit_upload(replacement, 0)
        .expect("commit the base replacement");
    assert_eq!(
        renderer.pacing_scale(),
        1.0,
        "a base change must reset the learned scale"
    );
    assert_eq!(
        renderer.pacing_cost_ema_ms(),
        0.0,
        "a base change must reset the cost EMA"
    );
}

/// Interleaving suffix commits with cost observations must not return the scale
/// to `1.0`: the controller keeps its learned value across the settles.
#[test]
fn repeated_suffix_commits_do_not_return_the_scale_to_one() {
    let Some(gpu) = gpu() else {
        return;
    };
    let (mut renderer, target) = adapted_renderer(gpu);
    let learned = renderer.pacing_scale();
    assert!(learned < 1.0);

    for round in 0..6 {
        // A few observed frames between settles.
        for _ in 0..3 {
            renderer.render(camera_2d(), &target).unwrap();
        }
        // A settle: re-upload the overlay suffix (keep the whole resident prefix).
        let resident = renderer.batch_count();
        let overlay = renderer
            .prepare_upload_batches(&line_batches(1))
            .expect("stage the overlay suffix");
        renderer
            .commit_upload(overlay, resident)
            .expect("commit the overlay suffix");
        assert!(
            renderer.pacing_scale() < 1.0,
            "round {round}: a suffix commit returned the scale to 1.0 ({}), \
             the controller did not keep its learned value",
            renderer.pacing_scale()
        );
        assert!(
            renderer.pacing_cost_ema_ms() > 0.0,
            "round {round}: a suffix commit dropped the cost EMA"
        );
    }
}

/// The end-to-end consequence: after the controller has shrunk below `1.0`, a
/// suffix commit must not raise the *effective* page budget back to the un-paced
/// ceiling on the next frame.
#[test]
fn suffix_commit_after_an_over_target_observation_keeps_the_shrunk_budget() {
    let Some(gpu) = gpu() else {
        return;
    };
    let (mut renderer, target) = adapted_renderer(gpu);
    let shrunk = renderer.pacing_scale();
    assert!(shrunk < 1.0);
    let ceiling = renderer.frame_budget.max_vertices;

    let resident = renderer.batch_count();
    let overlay = renderer
        .prepare_upload_batches(&line_batches(1))
        .expect("stage the overlay suffix");
    renderer
        .commit_upload(overlay, resident)
        .expect("commit the overlay suffix");

    let stats = renderer.render(camera_2d(), &target).unwrap();
    assert_eq!(
        stats.pacing, "active",
        "the pacing regime must stay active across a suffix commit"
    );
    assert!(
        stats.budget_scale < 1.0,
        "the effective budget must not jump back to the ceiling, got scale {}",
        stats.budget_scale
    );
    assert!(
        stats.budget_vertices < ceiling,
        "the effective vertex budget {} must stay below the un-paced ceiling {ceiling}",
        stats.budget_vertices
    );
}
