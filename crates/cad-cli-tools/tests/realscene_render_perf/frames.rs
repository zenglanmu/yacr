//! Per-frame phase records and camera helpers.

use std::time::{Duration, Instant};

use cad_domain::Point3;
use cad_render_wgpu::{Camera2d, FrameStats, RenderTarget, Renderer};
use cad_scene::SceneDelta;

#[derive(Clone, Copy, Default)]
pub(crate) struct FrameRecord {
    /// The whole `Renderer::render()` call (CPU + GPU + the blocking poll).
    pub(crate) wall_ms: f64,
    pub(crate) cpu_ms: f64,
    pub(crate) plan_ms: f64,
    pub(crate) uniform_ms: f64,
    pub(crate) encode_ms: f64,
    pub(crate) submit_ms: f64,
    pub(crate) wait_ms: f64,
    /// `wall_ms - (plan_ms + uniform_ms + encode_ms + submit_ms)`.
    ///
    /// **Not** GPU time: it folds in the blocking device poll (`wait_ms`, which
    /// waits on the GPU), any present and the small pre-phase CPU
    /// (`ensure_target` / flags top-up). It is only ever wall time the phase
    /// timers do not cover.
    pub(crate) residual_ms: f64,
    pub(crate) draw_calls: u64,
    pub(crate) uniform_writes: u64,
}

impl FrameRecord {
    /// `wall_ms` is measured by the caller around `Renderer::render`; the phase
    /// fields and the derived residual come from the returned [`FrameStats`].
    fn from_stats(stats: &FrameStats, wall_ms: f64) -> Self {
        let phase_ms = stats.plan_ms + stats.uniform_ms + stats.encode_ms + stats.submit_ms;
        FrameRecord {
            wall_ms,
            cpu_ms: stats.cpu_ms,
            plan_ms: stats.plan_ms,
            uniform_ms: stats.uniform_ms,
            encode_ms: stats.encode_ms,
            submit_ms: stats.submit_ms,
            wait_ms: stats.wait_ms,
            residual_ms: wall_ms - phase_ms,
            draw_calls: stats.draw_calls,
            uniform_writes: stats.uniform_writes,
        }
    }
}

pub(crate) struct PhaseResult {
    pub(crate) name: String,
    pub(crate) records: Vec<FrameRecord>,
    pub(crate) uploads_before: u64,
    pub(crate) uploads_after: u64,
    pub(crate) mesh_uploads_before: u64,
    pub(crate) mesh_uploads_after: u64,
    pub(crate) draw_calls_first: u64,
    /// The live draw tier of this phase's frames (`"universal"`/`"accelerated"`).
    pub(crate) tier: &'static str,
    /// Whether the opaques used the accelerated per-class draws, i.e. whether
    /// this phase ran the accelerated tier or fell back to the universal per-run
    /// organization on the same device.
    pub(crate) accelerated: bool,
    /// The over-budget reason of the first frame, or `None` when the phase's
    /// frames accepted the whole resident scene. A `Some` here is a budget
    /// fallback: the frame is a *page* of the scene, not the whole drawing.
    pub(crate) over_budget: Option<String>,
    /// The camera's `world_per_px` (first frame) — the view scale this phase ran
    /// at, so full-view vs deep-zoom legs are comparable.
    pub(crate) world_per_px: f64,
}

pub(crate) fn run_phase(
    name: &str,
    renderer: &mut Renderer,
    cameras: &[Camera2d],
    target: &RenderTarget,
) -> PhaseResult {
    let uploads_before = renderer.instance_stream_uploads();
    let mesh_uploads_before = renderer.mesh_chunk_uploads();
    let mut records = Vec::with_capacity(cameras.len());
    let mut draw_calls_first = 0;
    let mut tier = "universal";
    let mut accelerated = false;
    let mut over_budget = None;
    let mut world_per_px = f64::NAN;
    for (index, camera) in cameras.iter().enumerate() {
        let started = Instant::now();
        let stats = renderer.render(*camera, target).expect("frame");
        let wall_ms = started.elapsed().as_secs_f64() * 1000.0;
        if index == 0 {
            draw_calls_first = stats.draw_calls;
            tier = stats.tier.as_str();
            accelerated = stats.accelerated;
            over_budget = stats.over_budget.as_ref().map(|reason| {
                format!(
                    "{} requested={} limit={} skipped_batches={}",
                    reason.report.category,
                    reason.report.requested,
                    reason.report.limit,
                    reason.report.skipped_batches
                )
            });
            world_per_px = camera.world_per_px;
        }
        records.push(FrameRecord::from_stats(&stats, wall_ms));
    }
    let uploads_after = renderer.instance_stream_uploads();
    let mesh_uploads_after = renderer.mesh_chunk_uploads();
    PhaseResult {
        name: name.to_string(),
        records,
        uploads_before,
        uploads_after,
        mesh_uploads_before,
        mesh_uploads_after,
        draw_calls_first,
        tier,
        accelerated,
        over_budget,
        world_per_px,
    }
}

/// Force wgpu to process its deferred-destruction queue after a renderer is
/// dropped, so the next path's buffers are allocated against reclaimed memory.
/// `wgpu` defers buffer/texture destruction to the next poll, and this harness
/// runs three paths (legacy / instanced-universal / accelerated) over a real
/// DWG whose geometry is large enough for that deferral to matter.
pub(crate) fn reclaim(device: &cad_render_wgpu::wgpu::Device) {
    let _ = device.poll(cad_render_wgpu::wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(Duration::from_secs(60)),
    });
}

pub(crate) fn pan_cameras(base: Camera2d, frames: usize, px_per_frame: f64) -> Vec<Camera2d> {
    let step = base.world_per_px * px_per_frame;
    (1..=frames)
        .map(|i| {
            let mut camera = base;
            camera.center.x += step * i as f64;
            camera
        })
        .collect()
}

pub(crate) fn zoom_cameras(base: Camera2d, frames: usize, factor: f64) -> Vec<Camera2d> {
    (1..=frames)
        .map(|i| {
            let mut camera = base;
            camera.world_per_px = base.world_per_px * factor.powi(i as i32);
            camera
        })
        .collect()
}

/// Camera fit over what is actually drawn, exactly like `run_render`.
pub(crate) fn fit_camera(delta: &SceneDelta, width: u32, height: u32) -> Option<Camera2d> {
    let mut fit: Option<(f64, f64, f64, f64)> = None;
    for batch in &delta.added {
        let ox = batch.local_origin.x;
        let oy = batch.local_origin.y;
        for vertex in &batch.vertices {
            let x = ox + vertex[0] as f64;
            let y = oy + vertex[1] as f64;
            if !x.is_finite() || !y.is_finite() {
                continue;
            }
            fit = Some(match fit {
                None => (x, y, x, y),
                Some((min_x, min_y, max_x, max_y)) => {
                    (min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y))
                }
            });
        }
    }
    let (min_x, min_y, max_x, max_y) = fit?;
    let span_x = (max_x - min_x).abs();
    let span_y = (max_y - min_y).abs();
    let world_per_px = (span_x / (width as f64 * 0.9))
        .max(span_y / (height as f64 * 0.9))
        .max(1e-9);
    Some(Camera2d {
        center: Point3 {
            x: (min_x + max_x) / 2.0,
            y: (min_y + max_y) / 2.0,
            z: 0.0,
        },
        world_per_px,
        z_plane: 0.0,
    })
}
