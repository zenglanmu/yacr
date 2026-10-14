//! CPU-cost baseline harness for pan/zoom viewport changes.
//!
//! ## What it measures
//!
//! Per pan step / zoom step, the wall time of the *real* host path:
//!
//! 1. an [`InteractionRequest::Pointer`] (pan drag) or
//!    [`InteractionRequest::Scroll`] (zoom) dispatched through
//!    [`EngineClient::dispatch`], exactly as the React host does via
//!    `session.handle_input` → `HostCore::dispatch_interaction`; the host
//!    internally runs `HostCore::immediate_command(CommandId::Pan | Zoom, …)`
//!    and the post-business `sync_render`;
//! 2. settling the CPU scene-preparation pipeline (`poll_preparation` until the
//!    worker is idle), which includes the O(#entities) overlay rebuild
//!    (`DrawingDatabase::bounds()` inside the transient-overlay build) triggered
//!    by every `viewport_bounds` change.
//!
//! The drawing is opened through the real import pipeline
//! (`HostCore::deliver_external_open` → `HostController::open_bytes` /*acadrust*/).
//!
//! ## Honest scope
//!
//! * CPU only. No wgpu frame is drawn and no GPU is touched; this is the
//!   command + preparation cost, not a render/GPU cost.
//! * Synthetic/office machine timing, for a BEFORE/AFTER comparison of the same
//!   machine and build profile — not an absolute performance claim.
//! * Not part of the default CI gates (`#[ignore]`). Run explicitly, see the
//!   reproduction commands written into the output report.
//!
//! ## Inputs
//!
//! * `YACR_PERF_DWGS` — colon-separated DWG paths (preferred; used to run both
//!   fixtures). Falls back to `YACR_TEST_DWG` (single path). Absent → clean skip.
//! * `YACR_PERF_STEPS` — steps per motion (default 30).
//! * `YACR_PERF_WARMUP` — warmup steps per motion (default 5).
//! * `YACR_PERF_OUT` — output directory (default `/tmp/opencode/yacr-perf/<unix>`).

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cad_app::interaction::{
    InteractionRequest, Modifiers, PointInput, PointerButton, PointerId, PointerKind, PointerPhase,
};
use cad_app::render_scene::{model_scene_merge_stats, SceneAssemblyStats};
use cad_domain::{Point3, TaskStamp};
use cad_engine::protocol::{EngineClient, EngineRequest};
use cad_engine::render::RenderSessionHandle;
use cad_host::{HostCore, HostOptions, HostPlatform, UnsupportedResources};
use cad_platform::{
    ConfigLoad, ConfigStore, FileEvent, FileTarget, HostCapabilities, PlatformFiles, SavePurpose,
    SurfaceMetrics, SurfaceSource,
};

// --- test-local platform fakes ---------------------------------------------

/// A platform with no file interaction: the harness delivers bytes directly.
struct NoFiles;

impl PlatformFiles for NoFiles {
    fn begin_open(&self) -> cad_domain::CadResult<()> {
        Ok(())
    }
    fn begin_open_target(&self, _target: &FileTarget) -> cad_domain::CadResult<()> {
        Ok(())
    }
    fn begin_save(&self, _purpose: SavePurpose, _default_name: &str) -> cad_domain::CadResult<()> {
        Ok(())
    }
    fn write_atomic(&self, _target: &FileTarget, _bytes: &[u8]) -> cad_domain::CadResult<()> {
        Ok(())
    }
    fn refuses_source_overwrite(&self, _target: &FileTarget) -> bool {
        false
    }
    fn set_source(&self, _target: Option<&FileTarget>) {}
    fn poll(&self) -> Vec<FileEvent> {
        Vec::new()
    }
}

struct NoConfig;

impl ConfigStore for NoConfig {
    fn load_config(&self) -> ConfigLoad {
        ConfigLoad::Absent
    }
    fn load_preference(&self) -> ConfigLoad {
        ConfigLoad::Absent
    }
    fn persist_preference(&self, _text: &str) -> cad_domain::CadResult<()> {
        Ok(())
    }
}

struct TestSurface;

impl SurfaceSource for TestSurface {
    fn surface(&self) -> Option<SurfaceMetrics> {
        Some(SurfaceMetrics {
            logical: [1280.0, 800.0],
            scale: 1.0,
            safe_insets: [0.0; 4],
        })
    }
}

fn build_core() -> HostCore {
    let platform = HostPlatform {
        files: Rc::new(NoFiles),
        config_store: Rc::new(NoConfig),
        surface: Rc::new(TestSurface),
        resources: Rc::new(UnsupportedResources),
        capabilities: HostCapabilities {
            open: true,
            new: true,
            save: true,
            plot: true,
            import_worker: false,
        },
    };
    let options = HostOptions {
        fonts: Vec::new(),
        fonts_dir: None,
        images_dir: None,
        headless: false,
        drawing: None,
        size: [1280.0, 800.0],
    };
    HostCore::new(options, platform, RenderSessionHandle::default()).expect("host core")
}

/// Open a real DWG through the real import pipeline and settle preparation.
fn open_dwg(core: &HostCore, path: &Path) -> usize {
    let bytes: Arc<[u8]> = std::fs::read(path).expect("read dwg").into();
    let target = FileTarget::from_path(&path.to_string_lossy());
    core.deliver_external_open(target, bytes)
        .expect("deliver external open");
    settle(core);
    let drawing = core
        .controller
        .borrow()
        .drawing()
        .expect("drawing installed");
    drawing.entity_count()
}

/// Settle the pipeline: drain completions, then wait for the CPU preparation
/// worker (the O(#entities) overlay rebuild) to go idle.
fn settle(core: &HostCore) {
    let _ = core.poll_completions(usize::MAX);
    let deadline = Instant::now() + Duration::from_secs(60);
    while core.render.poll_preparation().expect("poll preparation") {
        assert!(
            Instant::now() < deadline,
            "preparation pipeline did not settle within 60s"
        );
        std::thread::yield_now();
    }
}

/// Re-fit the camera and republish the render inputs, then settle.
fn refit(core: &HostCore) {
    core.controller.borrow_mut().fit().expect("fit");
    core.sync_view().expect("sync view");
    settle(core);
}

fn drawing_center(core: &HostCore) -> (Point3, f64) {
    let drawing = core.controller.borrow().drawing().expect("drawing");
    let (min, max) = drawing.bounds().expect("drawing has model-space bounds");
    let center = Point3 {
        x: (min.x + max.x) * 0.5,
        y: (min.y + max.y) * 0.5,
        z: 0.0,
    };
    let span = (max.x - min.x).max(max.y - min.y).max(1.0);
    (center, span)
}

fn pointer(core: &HostCore, id: PointerId, phase: PointerPhase, position: Point3) {
    core.dispatch(EngineRequest::Interaction(InteractionRequest::Pointer {
        id,
        kind: PointerKind::Mouse,
        phase,
        button: Some(PointerButton::Primary),
        position: PointInput::World(position),
        modifiers: Modifiers::default(),
    }))
    .map(|_| ())
    .expect("pointer dispatch");
}

fn scroll(core: &HostCore, position: Point3) {
    core.dispatch(EngineRequest::Interaction(InteractionRequest::Scroll {
        delta_logical: [0.0, 20.0],
        position: PointInput::World(position),
        modifiers: Modifiers::default(),
    }))
    .map(|_| ())
    .expect("scroll dispatch");
}

/// Time one pan step (a single drag move) including preparation settle.
fn timed_pan(core: &HostCore, id: PointerId, position: Point3) -> f64 {
    let start = Instant::now();
    pointer(core, id, PointerPhase::Move, position);
    settle(core);
    start.elapsed().as_secs_f64() * 1000.0
}

/// Time one zoom step (a single scroll) including preparation settle.
fn timed_zoom(core: &HostCore, position: Point3) -> f64 {
    let start = Instant::now();
    scroll(core, position);
    settle(core);
    start.elapsed().as_secs_f64() * 1000.0
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
    let total: f64 = sorted.iter().sum();
    Stats {
        n,
        min_ms: sorted.first().copied().unwrap_or(f64::NAN),
        max_ms: sorted.last().copied().unwrap_or(f64::NAN),
        mean_ms: total / n as f64,
        median_ms: at(0.5),
        p95_ms: at(0.95),
        total_ms: total,
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
    total_ms: f64,
}

struct FixtureResult {
    path: String,
    bytes: u64,
    entity_count: usize,
    /// Read-only merge-ratio counters for the same model-space scene build the
    /// host draws (fragments in vs. draw items out).
    merge: SceneAssemblyStats,
    /// Direct one-shot cost of the suspected O(#entities) call (diagnostic).
    bounds: Stats,
    pan: Stats,
    zoom: Stats,
}

/// One-shot `DrawingDatabase::bounds()` cost, called directly (not through the
/// pipeline) as an attribution diagnostic for the per-step numbers.
fn measure_bounds(core: &HostCore, repeats: usize) -> Stats {
    let drawing = core.controller.borrow().drawing().expect("drawing");
    let mut samples = Vec::with_capacity(repeats);
    for _ in 0..repeats {
        let start = Instant::now();
        let bounds = drawing.bounds();
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
        assert!(bounds.is_some(), "drawing has model-space bounds");
    }
    percentiles(&samples)
}

/// Read-only merge-ratio counters for the drawing the host has open.
///
/// Runs the same model-space compaction the host open uses, on a throwaway
/// cache, and never touches live scene state. This is a measurement, not a
/// change to what the harness draws.
fn measure_merge(core: &HostCore) -> SceneAssemblyStats {
    let controller = core.controller.borrow();
    let document = controller.document_id;
    let drawing = controller.drawing().expect("drawing installed");
    model_scene_merge_stats(&drawing, TaskStamp::new(document, 0)).expect("merge stats")
}

fn run_fixture(core: &HostCore, path: &Path, steps: usize, warmup: usize) -> FixtureResult {
    let entity_count = open_dwg(core, path);
    let merge = measure_merge(core);
    let bounds = measure_bounds(core, 3);
    let (center, span) = drawing_center(core);
    let step_world = span * 0.02;
    let id = PointerId(1);

    // Warmup: page cache, worker spin-up, tool transitions.
    pointer(core, id, PointerPhase::Down, center);
    for i in 0..warmup {
        let position = Point3 {
            x: center.x + step_world * (i + 1) as f64,
            y: center.y,
            z: 0.0,
        };
        let _ = timed_pan(core, id, position);
    }
    for _ in 0..warmup {
        let _ = timed_zoom(core, center);
    }
    pointer(core, id, PointerPhase::Up, center);

    refit(core);
    pointer(core, id, PointerPhase::Down, center);
    let mut pan = Vec::with_capacity(steps);
    for i in 0..steps {
        let position = Point3 {
            x: center.x + step_world * (i + 1) as f64,
            y: center.y,
            z: 0.0,
        };
        pan.push(timed_pan(core, id, position));
    }
    pointer(core, id, PointerPhase::Up, center);

    refit(core);
    let mut zoom = Vec::with_capacity(steps);
    for _ in 0..steps {
        zoom.push(timed_zoom(core, center));
    }

    FixtureResult {
        path: path.display().to_string(),
        bytes: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
        entity_count,
        merge,
        bounds,
        pan: percentiles(&pan),
        zoom: percentiles(&zoom),
    }
}

// --- report -----------------------------------------------------------------

fn stats_row(label: &str, s: &Stats) -> String {
    format!(
        "| {label} | {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.1} |",
        s.n, s.min_ms, s.mean_ms, s.median_ms, s.p95_ms, s.max_ms, s.total_ms
    )
}

fn write_report(dir: &Path, results: &[FixtureResult], steps: usize, warmup: usize) -> String {
    std::fs::create_dir_all(dir).expect("create output dir");
    let mut md = String::new();
    md.push_str("# Pan/Zoom CPU-cost measurement\n\n");
    md.push_str(&format!("Generated: {}\n\n", timestamp_human()));
    md.push_str(
        "Measured path: real `deliver_external_open` (acadrust import) → per-step \
         `InteractionRequest` dispatch (pan drag / scroll zoom, the React host's \
         `handle_input` → `dispatch_interaction` path, running `immediate_command` + \
         `sync_render`) → preparation settle (CPU overlay rebuild incl. \
         `DrawingDatabase::bounds()`).\n\n\
         **CPU only: no wgpu frame is drawn, no GPU is touched.** Numbers are for a \
         same-machine BEFORE/AFTER comparison, not an absolute claim.\n\n",
    );
    md.push_str(&format!(
        "Steps per motion: {steps}; warmup steps per motion: {warmup} (untimed).\n\n"
    ));

    md.push_str("## Entity counts\n\n| Fixture | Bytes | Entities |\n|---|---:|---:|\n");
    for r in results {
        md.push_str(&format!(
            "| {} | {} | {} |\n",
            r.path, r.bytes, r.entity_count
        ));
    }
    md.push('\n');

    md.push_str(
        "## Merge ratio (display fragments → draw items)\n\n\
         Model-space scene build (the same compaction the host open draws): `fragments_in` \
         is the display-fragment count before merging, `batches_out` is the draw-item count \
         after (render batches + image draws). `ratio` = fragments per draw item. `same-style \
         adj` counts adjacent emitted batches sharing every style attribute. `order-only` / \
         `topology-only` split only by paint order / by line-vs-mesh topology. `mergeable \
         lines` counts adjacent line batches that pass every merge gate (style, order, vertex \
         cap, 8,192-unit extent) but stayed separate at a representation boundary — each is \
         one draw item a representation-spanning merge would remove.\n\n",
    );
    md.push_str(
        "| Fixture | Entities | fragments_in | batches_out | ratio | same-style adj | order-only | topology-only | mergeable lines |\n\
         |---|---:|---:|---:|---:|---:|---:|---:|---:|\n",
    );
    for r in results {
        md.push_str(&format!(
            "| {} | {} | {} | {} | {:.3} | {} | {} | {} | {} |\n",
            short(&r.path),
            r.entity_count,
            r.merge.fragments_in,
            r.merge.batches_out,
            r.merge.merge_ratio(),
            r.merge.adjacent_same_style,
            r.merge.adjacent_differing_by_draw_order,
            r.merge.adjacent_differing_by_topology,
            r.merge.adjacent_mergeable_line_pairs,
        ));
    }
    md.push('\n');

    md.push_str("## Per-step wall time (ms)\n\n");
    md.push_str("| Motion (fixture) | n | min | mean | median | p95 | max | total |\n");
    md.push_str("|---|---:|---:|---:|---:|---:|---:|---:|\n");
    for r in results {
        let pan_label = format!("pan [{}]", short(&r.path));
        let zoom_label = format!("zoom [{}]", short(&r.path));
        md.push_str(&stats_row(&pan_label, &r.pan));
        md.push('\n');
        md.push_str(&stats_row(&zoom_label, &r.zoom));
        md.push('\n');
    }
    md.push('\n');

    // Direct bounds() attribution (median of 3 calls, one-shot diagnostic).
    md.push_str("## `DrawingDatabase::bounds()` diagnostic (median of 3 calls)\n\n");
    md.push_str("| Fixture | Entities | bounds median ms | ms/entity |\n|---|---:|---:|---:|\n");
    for r in results {
        md.push_str(&format!(
            "| {} | {} | {:.3} | {:.6} |\n",
            short(&r.path),
            r.entity_count,
            r.bounds.median_ms,
            per_entity(r.bounds.median_ms, r.entity_count)
        ));
    }
    md.push_str(
        "\nDirect `bounds()` cost is a lower bound on the per-step preparation cost; it is \
         called each step the transient overlay rebuilds (every `viewport_bounds` change).\n\n",
    );

    // Scaling ratio between fixtures (O(#entities) evidence).
    if results.len() >= 2 {
        let (a, b) = (&results[0], &results[1]);
        md.push_str("## Scaling between fixtures (mean per step)\n\n");
        md.push_str("| Fixture | Entities | pan mean ms | zoom mean ms |\n|---|---:|---:|---:|\n");
        for r in results {
            md.push_str(&format!(
                "| {} | {} | {:.3} | {:.3} |\n",
                short(&r.path),
                r.entity_count,
                r.pan.mean_ms,
                r.zoom.mean_ms
            ));
        }
        md.push('\n');
        let entity_ratio = if a.entity_count == 0 {
            f64::NAN
        } else {
            b.entity_count as f64 / a.entity_count as f64
        };
        md.push_str(&format!(
            "Entity-count ratio {}/{} = {:.2}x\n\n",
            short(&b.path),
            short(&a.path),
            entity_ratio
        ));
        md.push_str(&format!(
            "Mean pan-step ratio = {:.2}x; mean zoom-step ratio = {:.2}x; \
             direct `bounds()` median ratio = {:.2}x\n\n",
            ratio(b.pan.mean_ms, a.pan.mean_ms),
            ratio(b.zoom.mean_ms, a.zoom.mean_ms),
            ratio(b.bounds.median_ms, a.bounds.median_ms),
        ));
        md.push_str(&format!(
            "Per-entity pan mean = {:.6} ms/entity ({}), {:.6} ms/entity ({})\n\n",
            per_entity(a.pan.mean_ms, a.entity_count),
            short(&a.path),
            per_entity(b.pan.mean_ms, b.entity_count),
            short(&b.path),
        ));
    }

    md.push_str(
        "## Reproduce\n\n```bash\nexport PATH=\"$HOME/.cargo/bin:$PATH\"\n\
         YACR_PERF_DWGS=\"$PWD/fixtures/debug-default-fonts.dwg:$PWD/fixtures/complex-test.dwg\" \\\n\
         cargo test -p cad-host --test perf_baseline -- --ignored --nocapture --test-threads=1\n\
         ```\n\n\
         (Release profile for a lower-overhead comparison: append `--release` before `--`.)\n\n",
    );

    md.push_str(
        "## Skipped / not run\n\n\
         - No renderer/GPU frame: this is command + CPU preparation only (no wgpu, no vendor GPU).\n\
         - Merge counters are static scene-build counts (`fragments_in`/`batches_out`), not a GPU draw-call measurement.\n\
         - No selection present: `drawing_pick_items` full-entity clone is not exercised here.\n\
         - Single machine / single run; no cross-machine or thermal bounds claim.\n",
    );

    let report = dir.join("baseline.md");
    std::fs::write(&report, &md).expect("write baseline.md");
    report.display().to_string()
}

fn write_summary(dir: &Path, results: &[FixtureResult], steps: usize) {
    let mut json = String::from("{\n");
    json.push_str("  \"schema_version\": 1,\n");
    json.push_str("  \"kind\": \"pan_zoom_cpu_baseline\",\n");
    json.push_str(&format!("  \"steps\": {steps},\n"));
    json.push_str("  \"fixtures\": [\n");
    for (i, r) in results.iter().enumerate() {
        let comma = if i + 1 == results.len() { "" } else { "," };
        json.push_str("    {\n");
        json.push_str(&format!("      \"path\": {:?},\n", r.path));
        json.push_str(&format!("      \"bytes\": {},\n", r.bytes));
        json.push_str(&format!("      \"entity_count\": {},\n", r.entity_count));
        json.push_str(&format!(
            "      \"fragments_in\": {},\n",
            r.merge.fragments_in
        ));
        json.push_str(&format!(
            "      \"batches_out\": {},\n",
            r.merge.batches_out
        ));
        json.push_str(&format!(
            "      \"merge_ratio\": {:.4},\n",
            r.merge.merge_ratio()
        ));
        json.push_str(&format!(
            "      \"adjacent_same_style\": {},\n",
            r.merge.adjacent_same_style
        ));
        json.push_str(&format!(
            "      \"adjacent_differing_by_draw_order\": {},\n",
            r.merge.adjacent_differing_by_draw_order
        ));
        json.push_str(&format!(
            "      \"adjacent_differing_by_topology\": {},\n",
            r.merge.adjacent_differing_by_topology
        ));
        json.push_str(&format!(
            "      \"adjacent_mergeable_line_pairs\": {},\n",
            r.merge.adjacent_mergeable_line_pairs
        ));
        json.push_str(&format!("      \"bounds\": {},\n", stats_json(&r.bounds)));
        json.push_str(&format!("      \"pan\": {},\n", stats_json(&r.pan)));
        json.push_str(&format!("      \"zoom\": {}\n", stats_json(&r.zoom)));
        json.push_str(&format!("    }}{comma}\n"));
    }
    json.push_str("  ]\n}\n");
    std::fs::write(dir.join("summary.json"), json).expect("write summary.json");
}

fn stats_json(s: &Stats) -> String {
    format!(
        "{{\"n\":{},\"min_ms\":{:.4},\"mean_ms\":{:.4},\"median_ms\":{:.4},\"p95_ms\":{:.4},\"max_ms\":{:.4},\"total_ms\":{:.4}}}",
        s.n, s.min_ms, s.mean_ms, s.median_ms, s.p95_ms, s.max_ms, s.total_ms
    )
}

fn ratio(a: f64, b: f64) -> f64 {
    if b == 0.0 {
        f64::NAN
    } else {
        a / b
    }
}

fn per_entity(ms: f64, entities: usize) -> f64 {
    if entities == 0 {
        f64::NAN
    } else {
        ms / entities as f64
    }
}

fn short(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

fn timestamp_human() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix {secs}")
}

fn output_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("YACR_PERF_OUT") {
        return PathBuf::from(dir);
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    PathBuf::from(format!("/tmp/opencode/yacr-perf/{stamp}"))
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Baseline: entity count, per-step mean/median/p95, total, and the scaling
/// ratio between fixtures. Prints the report path; writes `baseline.md` and
/// `summary.json`.
#[test]
#[ignore = "real-DWG perf baseline; requires YACR_TEST_DWG/YACR_PERF_DWGS, not a CI gate"]
fn pan_zoom_cpu_baseline() {
    let raw = std::env::var("YACR_PERF_DWGS")
        .ok()
        .or_else(|| std::env::var("YACR_TEST_DWG").ok());
    let Some(raw) = raw else {
        eprintln!(
            "SKIP pan_zoom_cpu_baseline: set YACR_PERF_DWGS (colon-separated) or YACR_TEST_DWG"
        );
        return;
    };
    let paths: Vec<PathBuf> = raw
        .split(':')
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect();

    let steps = env_usize("YACR_PERF_STEPS", 30);
    let warmup = env_usize("YACR_PERF_WARMUP", 5);
    let dir = output_dir();

    let mut results = Vec::new();
    for path in &paths {
        if !path.is_file() {
            eprintln!(
                "SKIP fixture: {} does not exist (YACR_PERF_DWGS/YACR_TEST_DWG)",
                path.display()
            );
            continue;
        }
        // A fresh host per fixture: no camera/tool state leaks across fixtures.
        let core = build_core();
        let result = run_fixture(&core, path, steps, warmup);
        eprintln!(
            "{}: entities={} fragments_in={} batches_out={} merge_ratio={:.3} bounds median={:.3}ms \
             pan mean={:.3}ms median={:.3}ms p95={:.3}ms total={:.1}ms zoom mean={:.3}ms \
             median={:.3}ms p95={:.3}ms total={:.1}ms",
            short(&result.path),
            result.entity_count,
            result.merge.fragments_in,
            result.merge.batches_out,
            result.merge.merge_ratio(),
            result.bounds.median_ms,
            result.pan.mean_ms,
            result.pan.median_ms,
            result.pan.p95_ms,
            result.pan.total_ms,
            result.zoom.mean_ms,
            result.zoom.median_ms,
            result.zoom.p95_ms,
            result.zoom.total_ms,
        );
        results.push(result);
    }

    if results.is_empty() {
        eprintln!("SKIP pan_zoom_cpu_baseline: no readable fixture path was provided");
        return;
    }

    let report = write_report(&dir, &results, steps, warmup);
    write_summary(&dir, &results, steps);
    eprintln!("wrote {report}");
    eprintln!("wrote {}/summary.json", dir.display());
}
