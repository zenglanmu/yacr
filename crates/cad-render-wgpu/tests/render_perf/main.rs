//! Real-GPU per-frame CPU-cost measurement for the pan/zoom render path.
//!
//! ## What it measures
//!
//! For a scene whose batch count matches a fixture's entity count, this leg
//! drives `Renderer::render()` for a run of panning frames and records, per
//! frame:
//!
//! * `FrameStats::cpu_ms` — the render thread's CPU path (plan/cache lookup,
//!   packing the page's uniforms into one shared, dynamically-offset buffer,
//!   command encoding, `queue.submit`), with the blocking device poll excluded;
//! * the outer wall time of one `render()` call (CPU + GPU, including the poll);
//! * the frame's **residual** `wall - (plan_ms + uniform_ms + encode_ms +
//!   submit_ms)`, reported explicitly as *"GPU + present + blocking poll — NOT
//!   a GPU measurement"*. It is never presented as GPU time: no timestamp
//!   query exists (see the report's skipped/not-run section).
//! * `FrameStats::draw_calls` and `FrameStats::uniform_writes`.
//!
//! The fixture-scale leg drives **two view states** of the same synthetic scene:
//! `full` (the whole scene fits the target) and `zoom` (a fixed factor closer,
//! so most of the scene is off-screen). A frame-cost distribution per view state
//! is what a later culling change (backlog §P1) must move, so the baseline is
//! recorded for both rather than for a single camera.
//!
//! It opens **no DWG** (`cad-render-wgpu` cannot depend on the importer); the
//! scene is a synthetic set of small line batches whose *count* equals the
//! fixture's entity count, because the per-frame CPU cost scales with batch
//! count, not with geometry content. Every scene is labelled as synthetic.
//!
//! Two legs are measured:
//!
//! * `renderer_frame_cpu_measurement` — the fixture-scale scenes, now also
//!   reporting a `plan`/`uniform`/`encode`/`submit`/`wait` phase table, the
//!   per-frame `set_*` call counters and the wall/residual frame-cost
//!   distribution per view state;
//! * `renderer_batch_count_sweep` — a fixed 88k-segment scene split into
//!   `B ∈ {1, 100, 1k, 10k, 44.5k, 88k}` line batches, fit as
//!   `cpu_ms = a + b*B` to isolate the per-batch CPU constant `b`, plus the
//!   merged-vs-unmerged byte-identity fence.
//!
//! ## GPU honesty
//!
//! The device is chosen by [`cad_render_wgpu::headless::create_headless_gpu`],
//! which prefers a discrete GPU on Vulkan/GL (`GpuSelection::Auto`). It uses the
//! default Vulkan ICD — **no** `VK_ICD_FILENAMES` override — so on this machine
//! that is the real NVIDIA Quadro P620. The adapter `backend`/`name`/`driver`
//! are written into the report as proof. If no hardware adapter is available the
//! leg skips with a clear message rather than silently measuring a software
//! backend; a caller that must fall back to lavapipe does so explicitly and
//! labels the report as software-rendered (AGENTS.md rule 6).
//!
//! ## Inputs
//!
//! * `YACR_PERF_SCALES` — `label:batches` pairs, comma-separated. Default
//!   `debug-default-fonts:10656,complex-test:44591`.
//! * `YACR_PERF_FRAMES` — timed frames per scene (default 30).
//! * `YACR_PERF_WARMUP` — untimed warmup frames per scene (default 5).
//! * `YACR_PERF_OUT` — output directory.
//! * `YACR_PERF_TAG` — report file stem (default `render`; use `before`/`after`
//!   to drop `before.md`/`after.md`).
//! * `YACR_PERF_HOST_SUMMARY` — optional `summary.json` from
//!   `cad-host`'s `pan_zoom_cpu_baseline`; when present a "combined per-step"
//!   section sums the per-phase numbers (see the honest-scope note there).

use cad_domain::{DocumentId, EntityId, InstancePath, Point3, SelectionRef, TaskStamp};

use cad_render_wgpu::headless::create_headless_gpu;

use cad_render_wgpu::{BackendPreference, Camera2d, RenderTarget, Renderer};

use cad_scene::{RenderBatch, RenderTopology, SceneDelta};

use std::path::{Path, PathBuf};

use std::time::{Instant, SystemTime, UNIX_EPOCH};

mod cpu_measurement;
mod sweep;
fn source(index: usize) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(index as u128 + 1),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

/// A pan camera: `center_x` is in world units, `world_per_px` sets the view
/// scale. The fixture-scale leg pans the camera by 3 *screen pixels* per frame
/// at every view state (the caller scales the step by `world_per_px`), so the
/// `full` and `zoom` legs drive the same motion in screen space.
fn camera(center_x: f64, world_per_px: f64) -> Camera2d {
    Camera2d {
        center: Point3 {
            x: center_x,
            y: 0.0,
            z: 0.0,
        },
        world_per_px,
        z_plane: 0.0,
    }
}

#[derive(Clone, Copy)]
struct Stats {
    n: usize,
    min_ms: f64,
    max_ms: f64,
    mean_ms: f64,
    median_ms: f64,
    p95_ms: f64,
}

fn percentiles(samples: &[f64]) -> Stats {
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = sorted.len();
    let at = |p: f64| -> f64 {
        if n == 0 {
            return f64::NAN;
        }
        let idx = ((n as f64) * p).ceil() as usize;
        sorted[idx.clamp(1, n) - 1]
    };
    Stats {
        n,
        min_ms: sorted.first().copied().unwrap_or(f64::NAN),
        max_ms: sorted.last().copied().unwrap_or(f64::NAN),
        mean_ms: sorted.iter().sum::<f64>() / n as f64,
        median_ms: at(0.5),
        p95_ms: at(0.95),
    }
}

/// Per-frame samples and the last frame's constant composition, collected while
/// driving a scene's panning frames. Split out so both the fixture-scale leg and
/// the batch-count sweep measure the same way.
struct FrameSamples {
    cpu: Vec<f64>,
    wall: Vec<f64>,
    /// `wall - (plan + uniform + encode + submit)` per timed frame.
    residual: Vec<f64>,
    plan: Vec<f64>,
    uniform: Vec<f64>,
    encode: Vec<f64>,
    submit: Vec<f64>,
    wait: Vec<f64>,
    last: Option<cad_render_wgpu::FrameStats>,
}

impl FrameSamples {
    fn new(frames: usize) -> Self {
        FrameSamples {
            cpu: Vec::with_capacity(frames),
            wall: Vec::with_capacity(frames),
            residual: Vec::with_capacity(frames),
            plan: Vec::with_capacity(frames),
            uniform: Vec::with_capacity(frames),
            encode: Vec::with_capacity(frames),
            submit: Vec::with_capacity(frames),
            wait: Vec::with_capacity(frames),
            last: None,
        }
    }
}

/// Upload `delta`, then drive `warmup + frames` panning frames and record the
/// phase spans of the timed ones.
///
/// The camera pans 3 *screen pixels* per frame at `world_per_px`, so the pan is
/// identical in screen space across view states.
fn run_frames(
    renderer: &mut Renderer,
    target: &RenderTarget,
    delta: &SceneDelta,
    frames: usize,
    warmup: usize,
    instanced: bool,
    world_per_px: f64,
) -> FrameSamples {
    renderer.set_line_instancing(instanced);
    renderer.clear_batches();
    renderer.upload(delta).expect("upload scene");
    let mut samples = FrameSamples::new(frames);
    for step in 0..(warmup + frames) {
        // a 3 px pan per frame
        let cam = camera(step as f64 * 3.0 * world_per_px, world_per_px);
        let start = Instant::now();
        let stats = renderer.render(cam, target).expect("render frame");
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        if step >= warmup {
            let phase_ms = stats.plan_ms + stats.uniform_ms + stats.encode_ms + stats.submit_ms;
            samples.cpu.push(stats.cpu_ms);
            samples.wall.push(elapsed);
            samples.residual.push(elapsed - phase_ms);
            samples.plan.push(stats.plan_ms);
            samples.uniform.push(stats.uniform_ms);
            samples.encode.push(stats.encode_ms);
            samples.submit.push(stats.submit_ms);
            samples.wait.push(stats.wait_ms);
            samples.last = Some(stats);
        }
    }
    samples
}

// --- host summary (for the combined step) ----------------------------------

fn timestamp_human() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix {secs}")
}

fn env_string(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn output_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("YACR_PERF_OUT") {
        return PathBuf::from(dir);
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    PathBuf::from(format!("/tmp/opencode/yacr-perf/{stamp}-renderer-phases"))
}

// --- batch-count sweep ------------------------------------------------------
