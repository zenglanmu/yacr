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
//! * `FrameStats::draw_calls` and `FrameStats::uniform_writes`.
//!
//! It opens **no DWG** (`cad-render-wgpu` cannot depend on the importer); the
//! scene is a synthetic set of small line batches whose *count* equals the
//! fixture's entity count, because the per-frame CPU cost scales with batch
//! count, not with geometry content. Every scene is labelled as synthetic.
//!
//! Two legs are measured:
//!
//! * `renderer_frame_cpu_measurement` — the fixture-scale scenes, now also
//!   reporting a `plan`/`uniform`/`encode`/`submit`/`wait` phase table and the
//!   per-frame `set_*` call counters;
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

fn source(index: usize) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(index as u128 + 1),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

/// A small two-segment line batch (`LineList`), one per entity.
fn line_batch(index: usize, transparent: bool) -> RenderBatch {
    let x = index as f32 * 0.25;
    RenderBatch {
        conics: Vec::new(),
        local_origin: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        topology: RenderTopology::Lines,
        vertices: vec![
            [x, 0.0, 0.0],
            [x + 0.1, 0.1, 0.0],
            [x + 0.1, 0.1, 0.0],
            [x, 0.2, 0.0],
        ],
        normals: Vec::new(),
        colors: Vec::new(),
        indices: Vec::new(),
        edges: Vec::new(),
        mirrored: false,
        alpha: if transparent { 0.5 } else { 1.0 },
        color: cad_scene::DEFAULT_BATCH_COLOR,
        color_unresolved: true,
        lineweight: 0.0,
        lineweight_unresolved: true,
        linetype: cad_scene::LinetypePattern::continuous(),
        linetype_unresolved: true,
        sources: vec![source(index)],
        draw_order: index as i64,
    }
}

fn scene(batches: usize) -> SceneDelta {
    // ~1% transparent so the camera-dependent transparent re-sort is exercised
    // without dominating the opaque cost the pan/zoom frames are dominated by.
    let added = (0..batches).map(|i| line_batch(i, i % 100 == 0)).collect();
    SceneDelta {
        stamp: TaskStamp::new(DocumentId(1), 0),
        added,
        removed_chunks: Vec::new(),
        images: Vec::new(),
    }
}

fn camera(center_x: f64) -> Camera2d {
    Camera2d {
        center: Point3 {
            x: center_x,
            y: 0.0,
            z: 0.0,
        },
        world_per_px: 1.0,
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

struct SceneResult {
    label: String,
    batches: usize,
    cpu: Stats,
    wall: Stats,
    plan: Stats,
    uniform: Stats,
    encode: Stats,
    submit: Stats,
    wait: Stats,
    draw_calls: u64,
    uniform_writes: u64,
    vertices: usize,
    opaque_batches: usize,
    transparent_batches: usize,
    invisible_batches: usize,
    set_pipeline_calls: u64,
    set_bind_group_calls: u64,
    set_vertex_buffer_calls: u64,
    set_index_buffer_calls: u64,
    uniform_slots: usize,
    progressive_restarts: u64,
    /// Per-frame draws issued through the instanced line path (coalesced runs).
    instanced_draws: u64,
    /// Per-frame batches drawn through the instanced line path.
    instanced_batches: usize,
    /// `true` when the measured scene ran on the instanced path.
    instanced: bool,
    /// The live draw tier of the frame (`"universal"`/`"accelerated"`).
    tier: &'static str,
    /// Whether the frame's opaque pass used the accelerated per-class draws.
    accelerated: bool,
    /// Resident instanced segments after the scene's upload.
    instance_segments: u64,
    /// Instanced stream buffers uploaded for the scene.
    instance_streams: usize,
}

/// Per-frame samples and the last frame's constant composition, collected while
/// driving a scene's panning frames. Split out so both the fixture-scale leg and
/// the batch-count sweep measure the same way.
struct FrameSamples {
    cpu: Vec<f64>,
    wall: Vec<f64>,
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
fn run_frames(
    renderer: &mut Renderer,
    target: &RenderTarget,
    delta: &SceneDelta,
    frames: usize,
    warmup: usize,
    instanced: bool,
) -> FrameSamples {
    renderer.set_line_instancing(instanced);
    renderer.clear_batches();
    renderer.upload(delta).expect("upload scene");
    let mut samples = FrameSamples::new(frames);
    for step in 0..(warmup + frames) {
        let cam = camera(step as f64 * 3.0); // a 3 px pan per frame
        let start = Instant::now();
        let stats = renderer.render(cam, target).expect("render frame");
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        if step >= warmup {
            samples.cpu.push(stats.cpu_ms);
            samples.wall.push(elapsed);
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

fn measure_scene(
    renderer: &mut Renderer,
    target: &RenderTarget,
    label: &str,
    batches: usize,
    frames: usize,
    warmup: usize,
    instanced: bool,
) -> SceneResult {
    let samples = run_frames(renderer, target, &scene(batches), frames, warmup, instanced);
    let stats = samples.last.expect("at least one timed frame");
    SceneResult {
        label: label.to_string(),
        batches,
        cpu: percentiles(&samples.cpu),
        wall: percentiles(&samples.wall),
        plan: percentiles(&samples.plan),
        uniform: percentiles(&samples.uniform),
        encode: percentiles(&samples.encode),
        submit: percentiles(&samples.submit),
        wait: percentiles(&samples.wait),
        draw_calls: stats.draw_calls,
        uniform_writes: stats.uniform_writes,
        vertices: stats.vertices,
        opaque_batches: stats.opaque_batches,
        transparent_batches: stats.transparent_batches,
        invisible_batches: stats.invisible_batches,
        set_pipeline_calls: stats.set_pipeline_calls,
        set_bind_group_calls: stats.set_bind_group_calls,
        set_vertex_buffer_calls: stats.set_vertex_buffer_calls,
        set_index_buffer_calls: stats.set_index_buffer_calls,
        uniform_slots: stats.uniform_slots,
        progressive_restarts: stats.progressive_restarts,
        instanced_draws: stats.instanced_draws,
        instanced_batches: stats.instanced_batches,
        instanced,
        tier: stats.tier.as_str(),
        accelerated: stats.accelerated,
        instance_segments: renderer.instance_segment_count(),
        instance_streams: renderer.instance_stream_count(),
    }
}

// --- host summary (for the combined step) ----------------------------------

/// `(entity_count, pan mean_ms, zoom mean_ms)` parsed from a `cad-host`
/// `pan_zoom_cpu_baseline` `summary.json`. Hand-rolled to avoid a JSON
/// dependency; the schema is owned by `crates/cad-host/tests/perf_baseline.rs`.
fn parse_host_summary(text: &str) -> Vec<(usize, f64, f64)> {
    fn number_after(hay: &str, needle: &str) -> Option<f64> {
        let start = hay.find(needle)? + needle.len();
        let rest = hay[start..].trim_start();
        let end = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == 'e' || c == '+'))
            .unwrap_or(rest.len());
        rest[..end].parse().ok()
    }
    let mut out = Vec::new();
    for part in text.split("\"entity_count\":").skip(1) {
        let Some(entities) = number_after(part, "").map(|v| v as usize) else {
            continue;
        };
        let pan = part
            .find("\"pan\":")
            .and_then(|i| number_after(&part[i..], "\"mean_ms\":"));
        let zoom = part
            .find("\"zoom\":")
            .and_then(|i| number_after(&part[i..], "\"mean_ms\":"));
        if let (Some(pan), Some(zoom)) = (pan, zoom) {
            out.push((entities, pan, zoom));
        }
    }
    out
}

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

fn stat_row(label: &str, s: &Stats) -> String {
    format!(
        "| {label} | {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} |",
        s.n, s.min_ms, s.mean_ms, s.median_ms, s.p95_ms, s.max_ms
    )
}

fn write_report(
    dir: &Path,
    tag: &str,
    adapter: &cad_render_wgpu::headless::AdapterInfo,
    results: &[SceneResult],
    frames: usize,
    warmup: usize,
    host: &[(usize, f64, f64)],
) -> String {
    std::fs::create_dir_all(dir).expect("create output dir");
    let mut md = String::new();
    md.push_str("# Renderer per-frame CPU-cost measurement\n\n");
    md.push_str(&format!("Generated: {}\n\n", timestamp_human()));
    md.push_str(
        "Measured path: `Renderer::render(camera, target)` in a loop of panning frames \
         (a 3 px pan per frame) over a synthetic scene whose batch count equals the \
         fixture's entity count. `cpu_ms` is the render thread's CPU path (plan/cache, \
         packing each page's uniforms into one shared dynamically-offset buffer, encode, \
         submit) with the blocking device poll \
         excluded; `wall` is the whole `render()` call.\n\n\
         **No DWG is opened here**: `cad-render-wgpu` cannot depend on the importer, so \
         the scene is synthetic and scale-matched, not the real drawing geometry. \
         Per-frame CPU cost scales with batch count, which is what this leg varies.\n\n",
    );

    md.push_str("## GPU adapter (proof this is real hardware)\n\n");
    md.push_str(&format!(
        "| backend | device_type | name | driver |\n|---|---|---|---|\n| {} | {} | {} | {} |\n\n",
        adapter.backend, adapter.device_type, adapter.name, adapter.driver
    ));
    md.push_str(&format!(
        "Frames per scene: {frames}; warmup frames per scene: {warmup} (untimed).\n\n"
    ));

    md.push_str("## Per-frame CPU time (ms) — `FrameStats::cpu_ms`\n\n");
    md.push_str("| Scene (synthetic) | n | min | mean | median | p95 | max |\n");
    md.push_str("|---|---:|---:|---:|---:|---:|---:|\n");
    for r in results {
        md.push_str(&stat_row(&format!("cpu [{}]", r.label), &r.cpu));
        md.push('\n');
    }
    md.push('\n');

    md.push_str("## Per-frame whole-call wall time (ms)\n\n");
    md.push_str("| Scene (synthetic) | n | min | mean | median | p95 | max |\n");
    md.push_str("|---|---:|---:|---:|---:|---:|---:|\n");
    for r in results {
        md.push_str(&stat_row(&format!("wall [{}]", r.label), &r.wall));
        md.push('\n');
    }
    md.push('\n');

    md.push_str("## Frame composition (constant across timed frames)\n\n");
    md.push_str(
        "| Scene | batches | tier | accelerated | draw_calls | instanced_draws | instanced_batches | instance_streams | instance_segments | uniform_writes | vertices | opaque | transparent | invisible |\n",
    );
    md.push_str("|---|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n");
    for r in results {
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            r.label,
            r.batches,
            r.tier,
            r.accelerated,
            r.draw_calls,
            r.instanced_draws,
            r.instanced_batches,
            r.instance_streams,
            r.instance_segments,
            r.uniform_writes,
            r.vertices,
            r.opaque_batches,
            r.transparent_batches,
            r.invisible_batches,
        ));
    }
    md.push('\n');

    // Draw-call collapse: pair each legacy row with its instanced counterpart.
    md.push_str("## Draw-call collapse (legacy vs instanced, same scene/device)\n\n");
    md.push_str(
        "The instanced path packs every eligible opaque line segment into static \
         per-instance streams at upload; draws collapse to one per coalesced run while \
         colour/width/pick-key ride in the instance (never as a draw split). Transparent \
         batches and mesh overlays stay on the legacy per-batch path, so the draw count is \
         `legacy draws + instanced runs`.\n\n\
         The `tier` column reports the live draw organization: on a storage-buffer-capable \
         device the `instanced` rows run the **accelerated** tier (one draw per class via \
         vertex pulling), while `legacy` rows are the universal per-batch path.\n\n",
    );
    md.push_str("| Scene | path | tier | accelerated | draw_calls | cpu mean ms | submit mean ms | encode mean ms |\n");
    md.push_str("|---|---|---|---:|---:|---:|---:|---:|\n");
    for r in results {
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {:.3} | {:.3} | {:.3} |\n",
            r.label,
            if r.instanced { "instanced" } else { "legacy" },
            r.tier,
            r.accelerated,
            r.draw_calls,
            r.cpu.mean_ms,
            r.submit.mean_ms,
            r.encode.mean_ms,
        ));
    }
    md.push('\n');

    md.push_str("## Per-frame phase breakdown (ms)\n\n");
    md.push_str(
        "`cpu_ms` = `plan_ms` + `uniform_ms` + `encode_ms` + `submit_ms` (plus the O(1) \
         `ensure_target` and handle lookups). `wait_ms` is the blocking device poll and is \
         **excluded** from `cpu_ms`. Each column is mean / p95 over the timed frames.\n\n",
    );
    md.push_str("| Scene | plan mean | plan p95 | uniform mean | uniform p95 | encode mean | encode p95 | submit mean | submit p95 | wait mean | wait p95 |\n");
    md.push_str("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n");
    for r in results {
        md.push_str(&format!(
            "| {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} |\n",
            r.label,
            r.plan.mean_ms,
            r.plan.p95_ms,
            r.uniform.mean_ms,
            r.uniform.p95_ms,
            r.encode.mean_ms,
            r.encode.p95_ms,
            r.submit.mean_ms,
            r.submit.p95_ms,
            r.wait.mean_ms,
            r.wait.p95_ms,
        ));
    }
    md.push('\n');

    md.push_str("## Encode state-change counters (per frame, post-dedup)\n\n");
    md.push_str("| Scene | set_pipeline | set_bind_group | set_vertex_buffer | set_index_buffer | uniform_slots | progressive_restarts |\n");
    md.push_str("|---|---:|---:|---:|---:|---:|---:|\n");
    for r in results {
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            r.label,
            r.set_pipeline_calls,
            r.set_bind_group_calls,
            r.set_vertex_buffer_calls,
            r.set_index_buffer_calls,
            r.uniform_slots,
            r.progressive_restarts,
        ));
    }
    md.push('\n');

    if !host.is_empty() {
        md.push_str("## Combined per-step cost (phase sum)\n\n");
        md.push_str(
            "Combined = one cad-host pan/zoom step (`dispatch` + preparation settle, real \
             DWG import, measured in `pan_zoom_cpu_baseline`) **plus** one `renderer.render()` \
             frame at the matched batch count (above). The two phases are measured in \
             different processes and are summed, not sampled as one continuous block; the \
             render phase uses the scale-matched synthetic scene, not the real drawing's \
             geometry. p95 is the sum of the two per-phase p95s (an upper-bound-style \
             approximation, not a true combined percentile).\n\n",
        );
        md.push_str("| Fixture | entities | pan step mean ms | zoom step mean ms | render frame mean ms | combined pan mean ms | combined zoom mean ms |\n");
        md.push_str("|---|---:|---:|---:|---:|---:|---:|\n");
        for r in results {
            if let Some((_, pan, zoom)) = host.iter().find(|(e, _, _)| *e == r.batches) {
                md.push_str(&format!(
                    "| {} | {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} |\n",
                    r.label,
                    r.batches,
                    pan,
                    zoom,
                    r.cpu.mean_ms,
                    pan + r.cpu.mean_ms,
                    zoom + r.cpu.mean_ms,
                ));
            }
        }
        md.push('\n');
    }

    md.push_str("## Reproduce\n\n```bash\nexport PATH=\"$HOME/.cargo/bin:$PATH\"\n# real hardware GPU (default Vulkan ICD; discrete preferred):\n");
    md.push_str(&format!(
        "YACR_PERF_TAG={tag} YACR_PERF_OUT=\"$PWD/…\" \\\n  cargo test -p cad-render-wgpu --test render_perf -- --ignored --nocapture --test-threads=1\n```\n\n"
    ));
    md.push_str(
        "## Skipped / not run\n\n\
         - No real DWG geometry in this leg (synthetic scale-matched batches); the real \
           import + preparation cost is measured in `cad-host`'s `pan_zoom_cpu_baseline`.\n\
         - `gpu_ms` is not measured (no timestamp queries); `wall` includes the blocking poll.\n\
         - Single machine / single run; no cross-machine or thermal bounds claim.\n",
    );

    let report = dir.join(format!("{tag}.md"));
    std::fs::write(&report, &md).expect("write report");
    report.display().to_string()
}

/// Per-frame renderer CPU measurement on the best available adapter (discrete
/// GPU preferred). Writes `<tag>.md` and prints a one-line summary per scene.
#[test]
#[ignore = "real-GPU renderer perf measurement; not a CI gate"]
fn renderer_frame_cpu_measurement() {
    let frames = env_usize("YACR_PERF_FRAMES", 30);
    let warmup = env_usize("YACR_PERF_WARMUP", 5);
    let tag = env_string("YACR_PERF_TAG").unwrap_or_else(|| "render".to_string());
    let dir = output_dir();

    let scales: Vec<(String, usize)> = match env_string("YACR_PERF_SCALES") {
        Some(raw) => raw
            .split(',')
            .filter_map(|pair| {
                let (label, count) = pair.split_once(':')?;
                Some((label.to_string(), count.trim().parse().ok()?))
            })
            .collect(),
        None => vec![
            ("debug-default-fonts".to_string(), 10_656),
            ("complex-test".to_string(), 44_591),
        ],
    };
    if scales.is_empty() {
        eprintln!("SKIP renderer_frame_cpu_measurement: no scales configured");
        return;
    }

    let gpu = match create_headless_gpu(BackendPreference::Auto) {
        Ok(gpu) => gpu,
        Err(error) => {
            eprintln!("SKIP renderer_frame_cpu_measurement: no adapter available: {error}");
            return;
        }
    };
    let adapter = gpu.adapter.clone();
    let cad_render_wgpu::headless::HeadlessGpu {
        device,
        queue,
        adapter: _,
    } = gpu;
    let mut renderer = Renderer::new(BackendPreference::Auto);
    renderer
        .initialize_with_device(device, queue)
        .expect("initialize renderer");
    let target = RenderTarget::new(1280, 800);

    let mut results = Vec::new();
    for (label, batches) in &scales {
        // "before" (legacy per-batch path) and "after" (instanced streams) are
        // measured on the same binary, device and scene, so the delta is the
        // instanced path alone.
        for instanced in [false, true] {
            let mode = if instanced { "instanced" } else { "legacy" };
            let result = measure_scene(
                &mut renderer,
                &target,
                &format!("{label} [{mode}]"),
                *batches,
                frames,
                warmup,
                instanced,
            );
            eprintln!(
                "{}: batches={} mode={mode} tier={} accelerated={} cpu mean={:.3}ms \
                 median={:.3}ms p95={:.3}ms wall mean={:.3}ms draw_calls={} \
                 instanced_draws={} instanced_batches={} streams={} segments={} uniform_writes={}",
                label,
                batches,
                result.tier,
                result.accelerated,
                result.cpu.mean_ms,
                result.cpu.median_ms,
                result.cpu.p95_ms,
                result.wall.mean_ms,
                result.draw_calls,
                result.instanced_draws,
                result.instanced_batches,
                result.instance_streams,
                result.instance_segments,
                result.uniform_writes,
            );
            results.push(result);
        }
    }

    let host = env_string("YACR_PERF_HOST_SUMMARY")
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| parse_host_summary(&text))
        .unwrap_or_default();

    let report = write_report(&dir, &tag, &adapter, &results, frames, warmup, &host);
    eprintln!("wrote {report}");
}

// --- batch-count sweep ------------------------------------------------------

/// Total line segments in the sweep scenes. The split into `B` batches varies;
/// the vertex count (2 per segment) is fixed, so any change in `cpu_ms` is the
/// per-batch cost, not geometry.
const SWEEP_SEGMENTS: usize = 88_000;
/// Grid columns: `SWEEP_SEGMENTS` fills an exact 320 x 275 grid.
const SWEEP_COLS: usize = 320;
const SWEEP_CELL_W: f32 = 4.0;
const SWEEP_CELL_H: f32 = 2.9;

/// One `LineList` batch of `count` segments starting at global segment `first`.
///
/// Each segment lives inside its own 4 x 2.9 cell with a >= 1 px margin, so no
/// two segments overlap in rasterized pixels. That makes merged-vs-unmerged
/// byte-identity a real fence: coplanar overlapping lines within one draw have
/// an undefined fragment order, so the geometry deliberately avoids overlap.
fn grid_batch(first: usize, count: usize, draw_order: i64) -> RenderBatch {
    let mut vertices = Vec::with_capacity(count * 2);
    for s in first..first + count {
        let col = (s % SWEEP_COLS) as f32;
        let row = (s / SWEEP_COLS) as f32;
        let x0 = col * SWEEP_CELL_W + 1.0;
        let y0 = row * SWEEP_CELL_H + 0.5;
        vertices.push([x0, y0, 0.0]);
        vertices.push([x0 + 2.0, y0 + 1.5, 0.0]);
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
        alpha: 1.0,
        color: cad_scene::DEFAULT_BATCH_COLOR,
        color_unresolved: true,
        lineweight: 0.0,
        lineweight_unresolved: true,
        linetype: cad_scene::LinetypePattern::continuous(),
        linetype_unresolved: true,
        sources: vec![source(first)],
        draw_order,
    }
}

/// `SWEEP_SEGMENTS` segments partitioned into `batches` near-equal line batches
/// (the remainder is distributed to the first batches). `batches == 1` is the
/// fully merged scene; larger values split the same geometry.
fn grid_scene(batches: usize) -> SceneDelta {
    assert!((1..=SWEEP_SEGMENTS).contains(&batches));
    let base = SWEEP_SEGMENTS / batches;
    let rem = SWEEP_SEGMENTS % batches;
    let mut added = Vec::with_capacity(batches);
    let mut first = 0usize;
    for b in 0..batches {
        let count = base + usize::from(b < rem);
        added.push(grid_batch(first, count, b as i64));
        first += count;
    }
    SceneDelta {
        stamp: TaskStamp::new(DocumentId(1), 0),
        added,
        removed_chunks: Vec::new(),
        images: Vec::new(),
    }
}

/// Least-squares fit `y = a + b*x`; returns `(a, b, r2)`.
fn linear_fit(points: &[(f64, f64)]) -> (f64, f64, f64) {
    let n = points.len() as f64;
    let mean_x = points.iter().map(|p| p.0).sum::<f64>() / n;
    let mean_y = points.iter().map(|p| p.1).sum::<f64>() / n;
    let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
    for (x, y) in points {
        let dx = x - mean_x;
        let dy = y - mean_y;
        sxx += dx * dx;
        sxy += dx * dy;
        syy += dy * dy;
    }
    let b = if sxx == 0.0 { 0.0 } else { sxy / sxx };
    let a = mean_y - b * mean_x;
    let r2 = if syy == 0.0 {
        1.0
    } else {
        (sxy * sxy) / (sxx * syy)
    };
    (a, b, r2)
}

/// Render the grid scene at `batches` and return its pixels, at a fixed camera
/// that frames the whole 320 x 275 grid.
fn grid_pixels(renderer: &mut Renderer, target: &RenderTarget, batches: usize) -> Vec<u8> {
    renderer.set_line_instancing(true);
    renderer.clear_batches();
    renderer
        .upload(&grid_scene(batches))
        .expect("upload grid scene");
    let cam = Camera2d {
        center: Point3 {
            x: 640.0,
            y: 400.0,
            z: 0.0,
        },
        world_per_px: 1.0,
        z_plane: 0.0,
    };
    renderer.render(cam, target).expect("render grid frame");
    renderer.read_target_rgba().expect("read grid frame").pixels
}

/// One sweep point: batch count, its CPU time distribution and the last frame's
/// constant encode composition.
struct SweepRow {
    batches: usize,
    /// `true` for the instanced path, `false` for the legacy per-batch path.
    instanced: bool,
    cpu: Stats,
    draw_calls: u64,
    instanced_draws: u64,
    tier: &'static str,
    accelerated: bool,
    set_pipeline: u64,
    set_bind_group: u64,
    set_vertex_buffer: u64,
    set_index_buffer: u64,
    /// Encode mean scaled by 1000 (kept integer to avoid a float in the struct).
    encode_x1000: u64,
    uniform_slots: usize,
}

/// Everything `write_sweep_report` needs, so the writer stays under clippy's
/// argument limit.
struct SweepSummary<'a> {
    rows: &'a [SweepRow],
    merge_identical: bool,
    merge_nonblank: bool,
    frames: usize,
    warmup: usize,
}

fn write_sweep_report(
    dir: &Path,
    tag: &str,
    adapter: &cad_render_wgpu::headless::AdapterInfo,
    summary: &SweepSummary<'_>,
) -> String {
    let SweepSummary {
        rows,
        merge_identical,
        merge_nonblank,
        frames,
        warmup,
    } = *summary;
    std::fs::create_dir_all(dir).expect("create output dir");
    let mut md = String::new();
    md.push_str("# Renderer per-batch CPU-cost sweep\n\n");
    md.push_str(&format!("Generated: {}\n\n", timestamp_human()));
    md.push_str(
        "Fixed geometry (88k line segments = 176k vertices) split into `B` line batches; \
         `cpu_ms` is the render thread's CPU path. Fitting `cpu_ms = a + b*B` isolates the \
         per-batch CPU constant `b` (µs/batch) from the fixed frame cost `a`. Each point is \
         measured twice: the legacy per-batch path and the instanced line-stream path. \
         Synthetic scale-matched scene, single machine, single run.\n\n",
    );
    md.push_str("## GPU adapter (proof this is real hardware)\n\n");
    md.push_str(&format!(
        "| backend | device_type | name | driver |\n|---|---|---|---|\n| {} | {} | {} | {} |\n\n",
        adapter.backend, adapter.device_type, adapter.name, adapter.driver
    ));
    md.push_str(&format!(
        "Frames per point: {frames}; warmup frames per point: {warmup} (untimed).\n\n"
    ));
    md.push_str("| B batches | path | tier | accelerated | cpu mean ms | cpu p95 ms | draw_calls | instanced_draws | encode mean ms | set_pipeline | set_bind_group | set_vertex_buffer | set_index_buffer | uniform_slots |\n");
    md.push_str("|---:|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n");
    for row in rows {
        md.push_str(&format!(
            "| {} | {} | {} | {} | {:.3} | {:.3} | {} | {} | {:.3} | {} | {} | {} | {} | {} |\n",
            row.batches,
            if row.instanced { "instanced" } else { "legacy" },
            row.tier,
            row.accelerated,
            row.cpu.mean_ms,
            row.cpu.p95_ms,
            row.draw_calls,
            row.instanced_draws,
            row.encode_x1000 as f64 / 1000.0,
            row.set_pipeline,
            row.set_bind_group,
            row.set_vertex_buffer,
            row.set_index_buffer,
            row.uniform_slots
        ));
    }
    md.push('\n');

    for instanced in [false, true] {
        let points: Vec<(f64, f64)> = rows
            .iter()
            .filter(|row| row.instanced == instanced)
            .map(|row| (row.batches as f64, row.cpu.mean_ms))
            .collect();
        if points.len() < 2 {
            continue;
        }
        let (a, b, r2) = linear_fit(&points);
        let path = if instanced { "instanced" } else { "legacy" };
        md.push_str(&format!(
            "**{path}: fitted `cpu_ms = {a:.3} ms + {:.3} µs/batch * B`** (r² = {r2:.4}).\n\n",
            b * 1000.0
        ));
    }
    md.push_str(&format!(
        "Merge fence: merged (B=1) vs unmerged (B={SWEEP_SEGMENTS}) scenes render \
         **byte-identical: {merge_identical}**; the compared frame is non-blank: \
         {merge_nonblank}. This is the fence any future batch-merging round must keep.\n\n",
    ));
    md.push_str(
        "## Skipped / not run\n\n\
         - Synthetic grid scene; no real DWG geometry.\n\
         - Single machine / single run; no cross-machine or thermal bounds claim.\n\
         - Draw order is unchanged by the dedup (no batches were re-sorted).\n",
    );
    let report = dir.join(format!("{tag}-sweep.md"));
    std::fs::write(&report, &md).expect("write sweep report");
    report.display().to_string()
}

/// Split-into-B-batches sweep at a fixed vertex count, plus the merged-vs-
/// unmerged byte-identity fence. Real GPU only.
#[test]
#[ignore = "real-GPU renderer perf measurement; not a CI gate"]
fn renderer_batch_count_sweep() {
    let frames = env_usize("YACR_PERF_FRAMES", 30);
    let warmup = env_usize("YACR_PERF_WARMUP", 5);
    let tag = env_string("YACR_PERF_TAG").unwrap_or_else(|| "render".to_string());
    let dir = output_dir();

    let gpu = match create_headless_gpu(BackendPreference::Auto) {
        Ok(gpu) => gpu,
        Err(error) => {
            eprintln!("SKIP renderer_batch_count_sweep: no adapter available: {error}");
            return;
        }
    };
    let adapter = gpu.adapter.clone();
    let cad_render_wgpu::headless::HeadlessGpu {
        device,
        queue,
        adapter: _,
    } = gpu;
    let mut renderer = Renderer::new(BackendPreference::Auto);
    renderer
        .initialize_with_device(device, queue)
        .expect("initialize renderer");
    let target = RenderTarget::new(1280, 800);

    let batch_counts = [1usize, 100, 1_000, 10_000, 44_500, SWEEP_SEGMENTS];
    let mut rows = Vec::new();
    for &b in &batch_counts {
        for instanced in [false, true] {
            let samples = run_frames(
                &mut renderer,
                &target,
                &grid_scene(b),
                frames,
                warmup,
                instanced,
            );
            let cpu = percentiles(&samples.cpu);
            let stats = samples.last.expect("at least one timed frame");
            let mode = if instanced { "instanced" } else { "legacy" };
            eprintln!(
                "sweep B={b} mode={mode} tier={} accelerated={}: cpu mean={:.3}ms p95={:.3}ms \
                 draw_calls={} instanced_draws={} encode mean={:.3}ms slots={}",
                stats.tier.as_str(),
                stats.accelerated,
                cpu.mean_ms,
                cpu.p95_ms,
                stats.draw_calls,
                stats.instanced_draws,
                percentiles(&samples.encode).mean_ms,
                stats.uniform_slots,
            );
            rows.push(SweepRow {
                batches: b,
                instanced,
                cpu,
                draw_calls: stats.draw_calls,
                instanced_draws: stats.instanced_draws,
                tier: stats.tier.as_str(),
                accelerated: stats.accelerated,
                set_pipeline: stats.set_pipeline_calls,
                set_bind_group: stats.set_bind_group_calls,
                set_vertex_buffer: stats.set_vertex_buffer_calls,
                set_index_buffer: stats.set_index_buffer_calls,
                encode_x1000: (percentiles(&samples.encode).mean_ms * 1000.0).round() as u64,
                uniform_slots: stats.uniform_slots,
            });
        }
    }

    // Merge fence: identical geometry, one batch vs many. Both sides run the
    // instanced path (the default), so this fences the collapse itself.
    let merged = grid_pixels(&mut renderer, &target, 1);
    let unmerged = grid_pixels(&mut renderer, &target, SWEEP_SEGMENTS);
    let background = [merged[0], merged[1], merged[2], merged[3]];
    let merge_nonblank = merged.chunks_exact(4).any(|px| px != background.as_slice());
    assert!(
        merge_nonblank,
        "merge fence frame is blank; the comparison would be vacuous"
    );
    assert_eq!(
        merged, unmerged,
        "merged (B=1) and unmerged (B={SWEEP_SEGMENTS}) scenes must render byte-identical"
    );

    let summary = SweepSummary {
        rows: &rows,
        merge_identical: true,
        merge_nonblank,
        frames,
        warmup,
    };
    let report = write_sweep_report(&dir, &tag, &adapter, &summary);
    eprintln!("wrote {report}");
}
