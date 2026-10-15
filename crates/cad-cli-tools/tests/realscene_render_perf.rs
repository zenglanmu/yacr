//! Real-DWG end-to-end render-frame measurement **with fonts loaded**.
//!
//! ## What it measures
//!
//! The same render path `cad-cli-tools render` uses, replicated read-only in a
//! test (the CLI operation itself is `pub(crate)`, so a test cannot call it):
//!
//! 1. import a real DWG through the shared app path
//!    (`HostController::with_demo_document` + `open_bytes`, the acadrust
//!    importer boundary);
//! 2. build the display representation with the **real font engine** (the
//!    committed `fonts/` package next to the repository root, exactly like the
//!    packaged CLI's sibling `fonts/` directory) and the same
//!    `with_packed_line_segments()` context the CLI render uses;
//! 3. batch it through `SceneCache::build_compact` (identical to `run_render`);
//! 4. upload it to a **real headless wgpu device** and render frames.
//!
//! ## Output
//!
//! Per fixture: entity count, a fragment-level (representation) class histogram,
//! a batch-level class histogram, the "BEFORE instancing / AFTER instancing"
//! per-class draw breakdown from `FrameStats` + the scene itself, the total
//! `draw_calls` on both paths, and per-frame phase tables
//! (`plan`/`uniform`/`encode`/`submit`/`wait`, plus `cpu_ms`) over pan and zoom
//! frames at a mid and a deep zoom. `instance_stream_uploads` is recorded before
//! and after every phase to show a camera-only move re-uploads no stream.
//!
//! Markdown + JSON are written to `YACR_PERF_OUT`, or
//! `/tmp/opencode/yacr-perf/<unix>-realscene/` by default.
//!
//! ## Honest scope
//!
//! * Real DWG geometry, real GPU (the default Vulkan ICD; no
//!   `VK_ICD_FILENAMES` is ever set here). The adapter line in the report is the
//!   actual run's adapter — never a synthetic device.
//! * This **is** real DWG geometry, so it removes the "synthetic scale-matched
//!   scene" caveat of the earlier instancing numbers; the scenes are not
//!   synthetic.
//! * The line class includes every tessellated polyline (arcs, circles,
//!   polylines, glyph outlines when text falls back to lines): they are
//!   indistinguishable after representation, so "arc" is not a separate batch
//!   class here.
//! * Fonts load from the committed `fonts/` package (osifont only). The full
//!   mlightcad catalogue assembled by `scripts/fetch-fonts.sh` is NOT fetched
//!   offline, so a drawing font absent from the committed package falls back to
//!   osifont / the system default face. The report states which faces were
//!   registered.
//! * Not part of the default CI gates (`#[ignore]`); run explicitly with a real
//!   GPU.

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cad_app::host::HostController;
use cad_app::render_scene::{model_scene_merge_stats, SceneAssemblyStats};
use cad_domain::{Point3, TaskStamp, TolerancePolicy};
use cad_render_wgpu::geometry::{classify_alpha, AlphaClass};
use cad_render_wgpu::headless::{create_headless_gpu_with, encode_png, HeadlessGpu};
use cad_render_wgpu::{
    BackendPreference, Camera2d, FrameStats, GpuSelection, RenderTarget, Renderer,
};
use cad_representation::{DisplayPrimitive, FontEngine, ProviderRegistry, RepresentationContext};
use cad_scene::{FrameBudget, RenderTopology, SceneBudget, SceneCache, SceneDelta};

/// The renderer's default per-frame budget (`Renderer::new`): a single frame is
/// a *page* of the scene, not the whole drawing.
const DEFAULT_FRAME_BUDGET: FrameBudget = FrameBudget {
    max_vertices: 8_000_000,
    max_triangles: 2_000_000,
    max_bytes: usize::MAX,
};

/// Unbounded per-frame budget: one frame accepts the whole resident scene, so
/// the class breakdown and total draw count are for the full drawing.
const FULL_FRAME_BUDGET: FrameBudget = FrameBudget {
    max_vertices: usize::MAX,
    max_triangles: usize::MAX,
    max_bytes: usize::MAX,
};

// ---------------------------------------------------------------------------
// Inputs / output
// ---------------------------------------------------------------------------

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_u32(key: &str, default: u32) -> u32 {
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

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// Fixture selection: `YACR_PERF_DWGS` (colon-separated) or `YACR_TEST_DWG`
/// (single), else the two committed real-DWG fixtures when they exist.
fn fixture_paths() -> Vec<PathBuf> {
    if let Ok(raw) = std::env::var("YACR_PERF_DWGS") {
        return raw
            .split(':')
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect();
    }
    if let Ok(raw) = std::env::var("YACR_TEST_DWG") {
        return vec![PathBuf::from(raw)];
    }
    let root = repo_root().join("fixtures");
    [
        root.join("debug-default-fonts.dwg"),
        root.join("complex-test.dwg"),
    ]
    .into_iter()
    .filter(|p| p.is_file())
    .collect()
}

fn output_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("YACR_PERF_OUT") {
        return PathBuf::from(dir);
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    PathBuf::from(format!("/tmp/opencode/yacr-perf/{stamp}-realscene"))
}

// ---------------------------------------------------------------------------
// Fonts (production CLI loading shape)
// ---------------------------------------------------------------------------

struct FontProvenance {
    /// Number of font keys the drawing references (per-entity, not deduplicated).
    requested_count: usize,
    /// Unique requested keys, capped for display.
    requested_unique: Vec<String>,
    requested_unique_count: usize,
    dir: Option<String>,
    registered: usize,
    fallback_keys: Vec<String>,
    note: String,
}

/// Load fonts exactly like the native CLI: the sibling `fonts/` package next to
/// the executable, an explicit `--font` set, and a system default face.
///
/// The test binary lives under `target/debug/deps/`, so `resolve_font_dir(None)`
/// finds nothing; the committed repository package (`fonts/` at the repo root)
/// stands in for a packaged CLI's sibling directory and is labelled as such.
fn load_fonts(requested: &[String]) -> (Option<Arc<FontEngine>>, FontProvenance) {
    let exe = std::env::current_exe().unwrap_or_default();
    let committed = repo_root().join("fonts");
    let explicit_dir = committed.is_dir().then_some(committed.as_path());
    let dir = cad_platform::fonts::local::resolve_font_dir(explicit_dir, &exe)
        .ok()
        .flatten()
        .or_else(|| {
            cad_platform::fonts::local::resolve_font_dir(None, &exe)
                .ok()
                .flatten()
        });
    let engine = cad_platform::fonts::local::load_engine(
        dir.as_deref(),
        requested,
        &[],
        cad_platform::fonts::local::cached_system_default_candidates(),
    )
    .expect("font engine load");
    let (registered, fallback_keys) = match &engine {
        Some(engine) => (engine.len(), engine.fallback_keys().to_vec()),
        None => (0, Vec::new()),
    };
    let mut unique: Vec<String> = requested
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<String>>()
        .into_iter()
        .collect();
    let requested_unique_count = unique.len();
    unique.truncate(64);
    let note = if dir.as_deref().is_some_and(|d| d.starts_with(repo_root())) {
        "committed repository fonts/ package (osifont.ttf); the mlightcad full \
         catalogue from scripts/fetch-fonts.sh is NOT fetched offline"
            .to_string()
    } else if dir.is_some() {
        "sibling fonts/ directory next to the test executable".to_string()
    } else {
        "no fonts/ directory resolved; only explicit/system faces".to_string()
    };
    (
        engine,
        FontProvenance {
            requested_count: requested.len(),
            requested_unique: unique,
            requested_unique_count,
            dir: dir.map(|d| d.display().to_string()),
            registered,
            fallback_keys,
            note,
        },
    )
}

// ---------------------------------------------------------------------------
// Class histograms
// ---------------------------------------------------------------------------

/// Fragment-level classes from the display representation (font-aware: a
/// tessellated glyph is `MeshWithoutEdgeOverlay`).
#[derive(Default, Clone)]
struct FragmentHistogram {
    lines: usize,
    line_vertices: usize,
    meshes: usize,
    mesh_triangles: usize,
    glyph_meshes: usize,
    glyph_triangles: usize,
    texts: usize,
    instances: usize,
    images: usize,
}

impl FragmentHistogram {
    fn tally(&mut self, rep: &cad_representation::DisplayRepresentation) {
        for fragment in &rep.fragments {
            match &fragment.primitive {
                DisplayPrimitive::Lines(points) | DisplayPrimitive::LineSegments(points) => {
                    self.lines += 1;
                    self.line_vertices += points.len();
                }
                DisplayPrimitive::Mesh(mesh) => {
                    self.meshes += 1;
                    self.mesh_triangles += mesh.triangles.len();
                }
                DisplayPrimitive::MeshWithoutEdgeOverlay(mesh) => {
                    self.glyph_meshes += 1;
                    self.glyph_triangles += mesh.triangles.len();
                }
                DisplayPrimitive::Text { .. } => self.texts += 1,
                DisplayPrimitive::Instance { .. } => self.instances += 1,
                DisplayPrimitive::Image { .. } => self.images += 1,
            }
        }
    }

    fn total(&self) -> usize {
        self.lines + self.meshes + self.glyph_meshes + self.texts + self.instances + self.images
    }
}

/// Batch-level classes from the built scene. A `Mesh` batch with an edge overlay
/// is a solid mesh (`DisplayPrimitive::Mesh`); one without is a glyph
/// (`DisplayPrimitive::MeshWithoutEdgeOverlay`) — the cache sets the overlay
/// exactly for the former (see `cad_scene::SceneCache::build_internal`).
#[derive(Default, Clone)]
struct BatchHistogram {
    line_batches: usize,
    line_vertices: usize,
    mesh_batches: usize,
    mesh_triangles: usize,
    glyph_batches: usize,
    glyph_triangles: usize,
    mesh_edge_batches: usize,
    /// Extra edge-overlay draw calls a `Mesh` batch issues beyond its triangle
    /// draw (each mesh-wireframe draw is a second `draw_indexed`).
    overlay_draws: usize,
    opaque: usize,
    transparent: usize,
    invisible: usize,
    images: usize,
}

impl BatchHistogram {
    fn tally(&mut self, delta: &SceneDelta) {
        for batch in &delta.added {
            match batch.topology {
                RenderTopology::Lines => {
                    self.line_batches += 1;
                    self.line_vertices += batch.vertices.len();
                }
                RenderTopology::MeshEdges => self.mesh_edge_batches += 1,
                RenderTopology::Mesh => {
                    if batch.edges.is_empty() {
                        self.glyph_batches += 1;
                        self.glyph_triangles += batch.triangle_count();
                    } else {
                        self.mesh_batches += 1;
                        self.mesh_triangles += batch.triangle_count();
                        self.overlay_draws += 1;
                    }
                }
            }
            match classify_alpha(batch.alpha) {
                AlphaClass::Opaque => self.opaque += 1,
                AlphaClass::Transparent => self.transparent += 1,
                AlphaClass::Invisible => self.invisible += 1,
            }
        }
        self.images = delta.images.len();
    }

    fn total_batches(&self) -> usize {
        self.line_batches + self.mesh_batches + self.glyph_batches + self.mesh_edge_batches
    }
}

// ---------------------------------------------------------------------------
// Frame phases
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
struct FrameRecord {
    cpu_ms: f64,
    plan_ms: f64,
    uniform_ms: f64,
    encode_ms: f64,
    submit_ms: f64,
    wait_ms: f64,
    draw_calls: u64,
    uniform_writes: u64,
}

impl FrameRecord {
    fn from_stats(stats: &FrameStats) -> Self {
        FrameRecord {
            cpu_ms: stats.cpu_ms,
            plan_ms: stats.plan_ms,
            uniform_ms: stats.uniform_ms,
            encode_ms: stats.encode_ms,
            submit_ms: stats.submit_ms,
            wait_ms: stats.wait_ms,
            draw_calls: stats.draw_calls,
            uniform_writes: stats.uniform_writes,
        }
    }
}

struct PhaseResult {
    name: String,
    records: Vec<FrameRecord>,
    uploads_before: u64,
    uploads_after: u64,
    mesh_uploads_before: u64,
    mesh_uploads_after: u64,
    draw_calls_first: u64,
}

fn run_phase(
    name: &str,
    renderer: &mut Renderer,
    cameras: &[Camera2d],
    target: &RenderTarget,
) -> PhaseResult {
    let uploads_before = renderer.instance_stream_uploads();
    let mesh_uploads_before = renderer.mesh_chunk_uploads();
    let mut records = Vec::with_capacity(cameras.len());
    let mut draw_calls_first = 0;
    for (index, camera) in cameras.iter().enumerate() {
        let stats = renderer.render(*camera, target).expect("frame");
        if index == 0 {
            draw_calls_first = stats.draw_calls;
        }
        records.push(FrameRecord::from_stats(&stats));
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
    }
}

/// Force wgpu to process its deferred-destruction queue after a renderer is
/// dropped, so the next path's buffers are allocated against reclaimed memory.
/// `wgpu` defers buffer/texture destruction to the next poll, and this harness
/// runs three paths (legacy / instanced-universal / accelerated) over a real
/// DWG whose geometry is large enough for that deferral to matter.
fn reclaim(device: &cad_render_wgpu::wgpu::Device) {
    let _ = device.poll(cad_render_wgpu::wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(Duration::from_secs(60)),
    });
}

fn pan_cameras(base: Camera2d, frames: usize, px_per_frame: f64) -> Vec<Camera2d> {
    let step = base.world_per_px * px_per_frame;
    (1..=frames)
        .map(|i| {
            let mut camera = base;
            camera.center.x += step * i as f64;
            camera
        })
        .collect()
}

fn zoom_cameras(base: Camera2d, frames: usize, factor: f64) -> Vec<Camera2d> {
    (1..=frames)
        .map(|i| {
            let mut camera = base;
            camera.world_per_px = base.world_per_px * factor.powi(i as i32);
            camera
        })
        .collect()
}

/// Camera fit over what is actually drawn, exactly like `run_render`.
fn fit_camera(delta: &SceneDelta, width: u32, height: u32) -> Option<Camera2d> {
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

// ---------------------------------------------------------------------------
// Per-fixture run
// ---------------------------------------------------------------------------

struct FixtureReport {
    path: String,
    bytes: u64,
    entity_count: usize,
    model_entities: usize,
    import_completeness: String,
    fonts: FontProvenance,
    fragments: FragmentHistogram,
    batches: BatchHistogram,
    merge_stats: Option<SceneAssemblyStats>,
    non_background: usize,
    adapter: String,
    /// Full-scene single frame (unbounded budget): the whole drawing.
    legacy: FrameStatsSummary,
    instanced: FrameStatsSummary,
    /// Converted path with the accelerated tier forced **off**: the universal
    /// per-run organization on the same scene/device, so the tier's effect is
    /// isolated from the conversion's effect.
    instanced_universal: FrameStatsSummary,
    /// Single frame at the renderer's default budget (the CLI's real one-frame
    /// result): a page of the scene, with the over-budget reason.
    legacy_default: FrameStatsSummary,
    instanced_default: FrameStatsSummary,
    instanced_universal_default: FrameStatsSummary,
    phases: Vec<PhaseResult>,
    instance_streams: usize,
    instance_segments: u64,
    uploads_after_upload: u64,
    mesh_chunks: usize,
    mesh_vertices: u64,
    mesh_uploads_after_upload: u64,
    png: Option<String>,
}

struct FrameStatsSummary {
    draw_calls: u64,
    opaque_batches: usize,
    transparent_batches: usize,
    invisible_batches: usize,
    image_batches: usize,
    instanced_draws: u64,
    instanced_batches: usize,
    instance_streams: usize,
    mesh_draws: u64,
    merged_mesh_batches: usize,
    mesh_chunks: usize,
    uniform_writes: u64,
    vertices: usize,
    triangles: usize,
    over_budget: Option<String>,
    /// The live draw tier of the frame (`"universal"`/`"accelerated"`).
    tier: &'static str,
    /// Whether the frame's opaque pass used the accelerated per-class draws.
    accelerated: bool,
    /// Draw calls issued by the accelerated per-class path this frame.
    accelerated_draws: u64,
}

impl FrameStatsSummary {
    fn from_stats(stats: &FrameStats) -> Self {
        FrameStatsSummary {
            draw_calls: stats.draw_calls,
            opaque_batches: stats.opaque_batches,
            transparent_batches: stats.transparent_batches,
            invisible_batches: stats.invisible_batches,
            image_batches: stats.image_batches,
            instanced_draws: stats.instanced_draws,
            instanced_batches: stats.instanced_batches,
            instance_streams: stats.instance_streams,
            mesh_draws: stats.mesh_draws,
            merged_mesh_batches: stats.merged_mesh_batches,
            mesh_chunks: stats.mesh_chunks,
            uniform_writes: stats.uniform_writes,
            vertices: stats.vertices,
            triangles: stats.triangles,
            over_budget: stats.over_budget.as_ref().map(|reason| {
                format!(
                    "{} requested={} limit={} skipped_batches={}",
                    reason.report.category,
                    reason.report.requested,
                    reason.report.limit,
                    reason.report.skipped_batches
                )
            }),
            tier: stats.tier.as_str(),
            accelerated: stats.accelerated,
            accelerated_draws: stats.accelerated_draws,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_fixture(
    path: &Path,
    dir: &Path,
    width: u32,
    height: u32,
    frames: usize,
    deep_zoom: f64,
    gpu: &HeadlessGpu,
) -> Result<FixtureReport, String> {
    let bytes: Arc<[u8]> = std::fs::read(path)
        .map_err(|e| format!("read {}: {e}", path.display()))?
        .into();
    let mut controller = HostController::with_demo_document([width as f64, height as f64])
        .map_err(|e| e.to_string())?;
    controller
        .open_bytes(bytes, &path.display().to_string())
        .map_err(|e| format!("import {}: {e}", path.display()))?;
    let drawing = controller
        .drawing()
        .ok_or_else(|| "no drawing after open".to_string())?;
    let entity_count = drawing.entity_count();
    let model_entities = drawing.model_space().len();
    let import_completeness = controller
        .last_import_report
        .as_ref()
        .map(|report| match &report.completeness {
            cad_domain::Completeness::Complete => "complete".to_string(),
            cad_domain::Completeness::Partial(v) => format!("partial ({} issues)", v.len()),
            cad_domain::Completeness::Missing(v) => format!("missing ({} issues)", v.len()),
            cad_domain::Completeness::Unverified => "unverified".to_string(),
        })
        .unwrap_or_else(|| "unverified".to_string());

    let requested = cad_platform::fonts::requested_fonts(drawing.as_ref());
    let (fonts, font_provenance) = load_fonts(&requested);

    let stamp = TaskStamp::new(controller.document_id, controller.session.generation);
    let registry = ProviderRegistry::with_default_provider();
    let mut context = RepresentationContext::new(
        controller.document_id,
        TolerancePolicy::default(),
        stamp.clone(),
    )
    .with_packed_line_segments();
    if let Some(fonts) = &fonts {
        context = context.with_fonts(fonts.clone());
    }

    // Build the scene exactly as `run_render` does, tallying the class histograms.
    let mut cache = SceneCache::new(SceneBudget::default());
    let mut delta = SceneDelta {
        stamp: stamp.clone(),
        added: Vec::new(),
        removed_chunks: Vec::new(),
        images: Vec::new(),
    };
    let mut fragments = FragmentHistogram::default();
    for entity in drawing.model_space() {
        let representation = registry
            .build_expanded(drawing.as_ref(), entity, &context)
            .map_err(|e| format!("representation for {}: {e}", entity.id.0))?;
        fragments.tally(&representation);
        let built = cache
            .build_compact(&representation, stamp.clone())
            .map_err(|e| format!("scene build: {e}"))?;
        delta.added.extend(built.added);
        delta.images.extend(built.images);
    }

    let mut batches = BatchHistogram::default();
    batches.tally(&delta);

    // Read-only, font-less host-path merge counters (a cross-check, not this
    // exact scene): `model_scene_merge_stats` builds its own throwaway scene.
    let merge_stats = model_scene_merge_stats(drawing.as_ref(), stamp.clone()).ok();

    let base_camera = fit_camera(&delta, width, height)
        .ok_or_else(|| "no drawable geometry to frame".to_string())?;
    let target = RenderTarget::new(width, height);

    // --- per-path measurements, one renderer alive at a time ----------------
    //
    // A real DWG's accelerated storage (whole-scene line + merged-mesh buffers,
    // plus the CPU copies kept for rebuild) is large; running all three
    // renderers concurrently OOMs on `complex-test.dwg`. Each path is measured
    // and then dropped before the next is created.
    let mut deep = base_camera;
    deep.world_per_px = base_camera.world_per_px / deep_zoom;
    let pan_legacy = pan_cameras(base_camera, frames, 8.0);
    let pan_mid = pan_cameras(base_camera, frames, 8.0);
    let pan_deep = pan_cameras(deep, frames, 8.0);
    let zoom_mid = zoom_cameras(base_camera, frames, 0.95);
    let mut phases = Vec::new();

    // --- legacy path (BEFORE instancing) ------------------------------------
    //
    // The full-scene frame uses an unbounded budget so the class breakdown and
    // total draw count describe the whole drawing; the default-budget frame is
    // the CLI's real one-frame result (a budget page of the scene).
    let mut legacy = Renderer::new(BackendPreference::Auto);
    legacy.set_poll_timeout(Duration::from_secs(600));
    legacy
        .initialize_with_device(gpu.device.clone(), gpu.queue.clone())
        .map_err(|e| format!("legacy init: {e}"))?;
    legacy.set_line_instancing(false);
    legacy.set_mesh_merging(false);
    legacy
        .upload(&delta)
        .map_err(|e| format!("legacy upload: {e}"))?;
    legacy.frame_budget = FULL_FRAME_BUDGET;
    let legacy_frame = legacy
        .render(base_camera, &target)
        .map_err(|e| format!("legacy render: {e}"))?;
    legacy.frame_budget = DEFAULT_FRAME_BUDGET;
    let legacy_default = legacy
        .render(base_camera, &target)
        .map_err(|e| format!("legacy default-budget render: {e}"))?;
    phases.push(run_phase(
        "pan_mid_legacy",
        &mut legacy,
        &pan_legacy,
        &target,
    ));
    drop(legacy);
    reclaim(&gpu.device);

    // --- instanced path with the accelerated tier forced off ----------------
    //
    // Same conversion (instancing + flat merge) but the universal per-run
    // organization, so the tier's effect is isolated from the conversion's.
    let mut instanced_universal = Renderer::new(BackendPreference::Auto);
    instanced_universal.set_poll_timeout(Duration::from_secs(600));
    instanced_universal.set_storage_buffer_support_override(Some(false));
    instanced_universal
        .initialize_with_device(gpu.device.clone(), gpu.queue.clone())
        .map_err(|e| format!("instanced-universal init: {e}"))?;
    instanced_universal.set_line_instancing(true);
    instanced_universal.set_mesh_merging(true);
    instanced_universal
        .upload(&delta)
        .map_err(|e| format!("instanced-universal upload: {e}"))?;
    instanced_universal.frame_budget = FULL_FRAME_BUDGET;
    let instanced_universal_frame = instanced_universal
        .render(base_camera, &target)
        .map_err(|e| format!("instanced-universal render: {e}"))?;
    instanced_universal.frame_budget = DEFAULT_FRAME_BUDGET;
    let instanced_universal_default = instanced_universal
        .render(base_camera, &target)
        .map_err(|e| format!("instanced-universal default-budget render: {e}"))?;
    phases.push(run_phase(
        "pan_mid_instanced_universal",
        &mut instanced_universal,
        &pan_mid,
        &target,
    ));
    drop(instanced_universal);
    reclaim(&gpu.device);

    // --- instanced path (AFTER instancing, live tier) -----------------------
    let mut instanced = Renderer::new(BackendPreference::Auto);
    instanced.set_poll_timeout(Duration::from_secs(600));
    instanced
        .initialize_with_device(gpu.device.clone(), gpu.queue.clone())
        .map_err(|e| format!("instanced init: {e}"))?;
    instanced.set_line_instancing(true);
    instanced.set_mesh_merging(true);
    instanced
        .upload(&delta)
        .map_err(|e| format!("instanced upload: {e}"))?;
    let uploads_after_upload = instanced.instance_stream_uploads();
    let instance_streams = instanced.instance_stream_count();
    let instance_segments = instanced.instance_segment_count();
    let mesh_uploads_after_upload = instanced.mesh_chunk_uploads();
    let mesh_chunks = instanced.mesh_chunk_count();
    let mesh_vertices = instanced.mesh_vertex_count();

    instanced.frame_budget = FULL_FRAME_BUDGET;
    let instanced_frame = instanced
        .render(base_camera, &target)
        .map_err(|e| format!("instanced render: {e}"))?;
    let image = instanced
        .read_target_rgba()
        .map_err(|e| format!("readback: {e}"))?;
    instanced.frame_budget = DEFAULT_FRAME_BUDGET;
    let instanced_default = instanced
        .render(base_camera, &target)
        .map_err(|e| format!("instanced default-budget render: {e}"))?;
    let background = image.pixel(0, 0);
    let non_background = image.count_differing_from(background, 8);

    // Visual evidence: one instanced frame per fixture.
    let png_path = dir.join(format!("frame-{}.png", short(&path.display().to_string())));
    let png = match encode_png(&image) {
        Ok(bytes) => std::fs::write(&png_path, bytes)
            .ok()
            .map(|_| png_path.display().to_string()),
        Err(_) => None,
    };

    phases.push(run_phase(
        "pan_mid_instanced",
        &mut instanced,
        &pan_mid,
        &target,
    ));
    phases.push(run_phase(
        "pan_deep_instanced",
        &mut instanced,
        &pan_deep,
        &target,
    ));
    phases.push(run_phase(
        "zoom_mid_instanced",
        &mut instanced,
        &zoom_mid,
        &target,
    ));
    drop(instanced);

    Ok(FixtureReport {
        path: path.display().to_string(),
        bytes: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
        entity_count,
        model_entities,
        import_completeness,
        fonts: font_provenance,
        fragments,
        batches,
        merge_stats,
        non_background,
        adapter: format!(
            "{} / {} / {} / driver {} / {}",
            gpu.adapter.backend,
            gpu.adapter.name,
            gpu.adapter.device_type,
            gpu.adapter.driver,
            gpu.adapter.driver_info
        ),
        legacy: FrameStatsSummary::from_stats(&legacy_frame),
        instanced: FrameStatsSummary::from_stats(&instanced_frame),
        instanced_universal: FrameStatsSummary::from_stats(&instanced_universal_frame),
        legacy_default: FrameStatsSummary::from_stats(&legacy_default),
        instanced_default: FrameStatsSummary::from_stats(&instanced_default),
        instanced_universal_default: FrameStatsSummary::from_stats(&instanced_universal_default),
        phases,
        instance_streams,
        instance_segments,
        uploads_after_upload,
        mesh_chunks,
        mesh_vertices,
        mesh_uploads_after_upload,
        png,
    })
}

// ---------------------------------------------------------------------------
// Statistics
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Stats {
    n: usize,
    min: f64,
    mean: f64,
    median: f64,
    p95: f64,
    max: f64,
}

fn stats(samples: &[f64]) -> Stats {
    if samples.is_empty() {
        return Stats {
            n: 0,
            min: f64::NAN,
            mean: f64::NAN,
            median: f64::NAN,
            p95: f64::NAN,
            max: f64::NAN,
        };
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = sorted.len();
    let at = |p: f64| sorted[(((n as f64) * p).ceil() as usize).clamp(1, n) - 1];
    Stats {
        n,
        min: sorted[0],
        mean: sorted.iter().sum::<f64>() / n as f64,
        median: at(0.5),
        p95: at(0.95),
        max: sorted[n - 1],
    }
}

fn field(records: &[FrameRecord], f: impl Fn(&FrameRecord) -> f64) -> Stats {
    stats(&records.iter().map(f).collect::<Vec<_>>())
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

fn short(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

fn phase_table(md: &mut String, result: &PhaseResult) {
    md.push_str(&format!(
        "**{}** ({} frames; `instance_stream_uploads` {} → {}; `mesh_chunk_uploads` {} → {})\n\n",
        result.name,
        result.records.len(),
        result.uploads_before,
        result.uploads_after,
        result.mesh_uploads_before,
        result.mesh_uploads_after,
    ));
    md.push_str(
        "| phase | n | min | mean | median | p95 | max |\n|---|---:|---:|---:|---:|---:|---:|\n",
    );
    type Row = (&'static str, fn(&FrameRecord) -> f64);
    let rows: [Row; 8] = [
        ("cpu_ms", |r| r.cpu_ms),
        ("plan_ms", |r| r.plan_ms),
        ("uniform_ms", |r| r.uniform_ms),
        ("encode_ms", |r| r.encode_ms),
        ("submit_ms", |r| r.submit_ms),
        ("wait_ms", |r| r.wait_ms),
        ("draw_calls", |r| r.draw_calls as f64),
        ("uniform_writes", |r| r.uniform_writes as f64),
    ];
    for (label, getter) in rows {
        let s = field(&result.records, getter);
        md.push_str(&format!(
            "| {label} | {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} |\n",
            s.n, s.min, s.mean, s.median, s.p95, s.max
        ));
    }
    md.push('\n');
}

fn write_report(
    dir: &Path,
    reports: &[FixtureReport],
    frames: usize,
    deep_zoom: f64,
    width: u32,
    height: u32,
) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut md = String::new();
    md.push_str("# Real-DWG render-frame perf (fonts loaded, real GPU)\n\n");
    md.push_str(&format!(
        "Generated: unix {}\n\n",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    ));
    md.push_str(
        "Path: real DWG import (acadrust) → font-aware display representation \
         (`with_packed_line_segments`) → `SceneCache::build_compact` → real headless \
         wgpu device → per-frame `Renderer::render`. Identical to `cad-cli-tools render` \
         (the CLI operation itself is `pub(crate)`, so this replicates it).\n\n\
         **Real DWG geometry** (not a synthetic scene). Real GPU (default Vulkan ICD; \
         `VK_ICD_FILENAMES` is never set here). Same-machine BEFORE/AFTER comparison, \
         not an absolute performance claim.\n\n",
    );
    md.push_str(&format!(
        "Frames per phase: {frames}; render target {width}x{height}; deep zoom \
         = fit / {deep_zoom:.1}.\n\n"
    ));

    let adapter = reports
        .first()
        .map(|r| r.adapter.clone())
        .unwrap_or_default();
    md.push_str(&format!("## Adapter (actual run)\n\n`{adapter}`\n\n"));

    // Fixture / entity / font table.
    md.push_str("## Fixtures and fonts\n\n");
    md.push_str(
        "| Fixture | Bytes | Entities | Model entities | Import | Font faces registered | Font dir |\n\
         |---|---:|---:|---:|---|---:|---|\n",
    );
    for r in reports {
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            short(&r.path),
            r.bytes,
            r.entity_count,
            r.model_entities,
            r.import_completeness,
            r.fonts.registered,
            r.fonts.dir.clone().unwrap_or_else(|| "(none)".into()),
        ));
    }
    md.push('\n');
    for r in reports {
        md.push_str(&format!(
            "Font provenance [{}]: requested={} keys ({} unique); registered={}; fallback={:?}; {}\n\n\
             Requested (first {} unique): {:?}\n\n",
            short(&r.path),
            r.fonts.requested_count,
            r.fonts.requested_unique_count,
            r.fonts.registered,
            r.fonts.fallback_keys,
            r.fonts.note,
            r.fonts.requested_unique.len(),
            r.fonts.requested_unique,
        ));
    }

    for r in reports {
        md.push_str(&format!("## {}\n\n", short(&r.path)));
        md.push_str(&format!(
            "Non-background pixels: {} (drawable frame confirmed).\n\n",
            r.non_background
        ));

        // Fragment-level classes.
        md.push_str("### Fragment classes (representation, fonts loaded)\n\n");
        md.push_str("| class | fragments | vertices / triangles |\n|---|---:|---:|\n");
        md.push_str(&format!(
            "| line (polyline/arc/circle/glyph-outline) | {} | {} v |\n",
            r.fragments.lines, r.fragments.line_vertices
        ));
        md.push_str(&format!(
            "| mesh (solid, edge overlay) | {} | {} tri |\n",
            r.fragments.meshes, r.fragments.mesh_triangles
        ));
        md.push_str(&format!(
            "| glyph mesh (MeshWithoutEdgeOverlay) | {} | {} tri |\n",
            r.fragments.glyph_meshes, r.fragments.glyph_triangles
        ));
        md.push_str(&format!("| text (unshaped) | {} | |\n", r.fragments.texts));
        md.push_str(&format!("| instance | {} | |\n", r.fragments.instances));
        md.push_str(&format!("| image | {} | |\n", r.fragments.images));
        md.push_str(&format!(
            "\nFragments in: {}; batches out: {}; merge ratio: {:.3}\n\n",
            r.fragments.total(),
            r.batches.total_batches() + r.batches.images,
            if r.batches.total_batches() + r.batches.images == 0 {
                0.0
            } else {
                r.fragments.total() as f64 / (r.batches.total_batches() + r.batches.images) as f64
            }
        ));

        // Batch-level classes with BEFORE/AFTER draws.
        md.push_str("### Per-class draws: BEFORE (all-legacy) vs AFTER (instanced + merged)\n\n");
        md.push_str(
            "AFTER enables both the instanced line path and the flat-merged mesh path. The two \
             conversion paths are reported separately: `instanced_draws` coalesces the line-path \
             batches (plain `Lines` batches **and** the instanced edge overlays of merged meshes), \
             and `mesh_draws` coalesces the merged mesh triangle runs. A merged mesh with an edge \
             overlay therefore appears in both `instanced_batches` and `merged_mesh_batches`.\n\n",
        );
        md.push_str(
            "| class | batches | legacy draws (BEFORE) | merged/instanced draws (AFTER) | size |\n\
             |---|---:|---:|---:|---|\n",
        );
        let ib = &r.instanced;
        let line_path_batches = ib.instanced_batches;
        let residual_lines = r
            .batches
            .line_batches
            .saturating_add(r.batches.overlay_draws)
            .saturating_sub(line_path_batches);
        md.push_str(&format!(
            "| line path (`Lines` + edge overlays) | {} | {} | {} = {} coalesced + {} legacy | {} v |\n",
            line_path_batches,
            r.batches.line_batches + r.batches.overlay_draws,
            ib.instanced_draws as usize + residual_lines,
            ib.instanced_draws,
            residual_lines,
            r.batches.line_vertices
        ));
        md.push_str(&format!(
            "| mesh (glyph + solid, flat-merged) | {} | {} | {} | {} tri |\n",
            ib.merged_mesh_batches,
            r.batches.glyph_batches + r.batches.mesh_batches,
            ib.mesh_draws,
            r.batches.glyph_triangles + r.batches.mesh_triangles
        ));
        md.push_str(&format!(
            "| glyph mesh (batch count) | {} | {} | merged above | {} tri |\n",
            r.batches.glyph_batches, r.batches.glyph_batches, r.batches.glyph_triangles
        ));
        md.push_str(&format!(
            "| mesh (solid, batch count) | {} | {} | merged above | {} tri |\n",
            r.batches.mesh_batches, r.batches.mesh_batches, r.batches.mesh_triangles
        ));
        md.push_str(&format!(
            "| mesh-edges | {} | {} | {} (legacy; pre-existing no-op) | |\n",
            r.batches.mesh_edge_batches, r.batches.mesh_edge_batches, r.batches.mesh_edge_batches
        ));
        md.push_str(&format!(
            "| image | {} | {} | {} | |\n",
            r.batches.images, r.batches.images, r.batches.images
        ));
        md.push_str(&format!(
            "| mesh edge-overlay (extra draw) | {} | {} | instanced into the line path | |\n",
            r.batches.overlay_draws, r.batches.overlay_draws
        ));
        md.push('\n');

        md.push_str("Pass split (frame, not class — overlaps the rows above):\n\n");
        md.push_str(&format!(
            "- batches: opaque={}, transparent={}, invisible={}\n",
            r.batches.opaque, r.batches.transparent, r.batches.invisible
        ));

        md.push_str("\n### Totals (full scene, unbounded frame budget)\n\n");
        md.push_str(&format!(
            "- legacy `draw_calls` (FrameStats): **{}** (opaque={}, transparent={}, image={})\n",
            r.legacy.draw_calls,
            r.legacy.opaque_batches,
            r.legacy.transparent_batches,
            r.legacy.image_batches
        ));
        md.push_str(&format!(
            "- instanced+merged `draw_calls` (FrameStats): **{}** (instanced_draws={}, \
             instanced_batches={}, mesh_draws={}, merged_mesh_batches={}, remaining legacy \
             opaque={})\n",
            r.instanced.draw_calls,
            r.instanced.instanced_draws,
            r.instanced.instanced_batches,
            r.instanced.mesh_draws,
            r.instanced.merged_mesh_batches,
            // Exact residual: every non-converted opaque batch is one legacy
            // draw, so it is the frame's draw count minus the coalesced
            // conversion draws and the transparent/image passes.
            // `instanced_batches` and `merged_mesh_batches` overlap on a mesh
            // that carries an edge overlay, so they are not summed.
            r.instanced
                .draw_calls
                .saturating_sub(r.instanced.instanced_draws + r.instanced.mesh_draws)
                .saturating_sub(
                    r.instanced.transparent_batches as u64 + r.instanced.image_batches as u64
                )
        ));
        md.push_str(&format!(
            "- effective draws incl. mesh edge overlays: legacy {} (incl. {} overlays), \
             instanced+merged {} (overlays are instanced, not extra)\n",
            r.legacy.draw_calls + r.batches.overlay_draws as u64,
            r.batches.overlay_draws,
            r.instanced.draw_calls,
        ));
        // Tier isolation: the same conversion on the universal per-run path vs the
        // accelerated per-class path. The `instanced` rows are the live tier
        // (accelerated on a storage-buffer device); the `instanced-universal` rows
        // force the tier off.
        let tier_ratio = if r.instanced.draw_calls == 0 {
            f64::NAN
        } else {
            r.instanced_universal.draw_calls as f64 / r.instanced.draw_calls as f64
        };
        md.push_str(&format!(
            "- **tier**: legacy={} (accelerated={}) | instanced={} (accelerated={}, \
             accelerated_draws={}) | instanced-universal={} (accelerated={})\n",
            r.legacy.tier,
            r.legacy.accelerated,
            r.instanced.tier,
            r.instanced.accelerated,
            r.instanced.accelerated_draws,
            r.instanced_universal.tier,
            r.instanced_universal.accelerated,
        ));
        md.push_str(&format!(
            "- converted-path draws (same scene/device): universal per-run={} vs accelerated \
             per-class={} ({tier_ratio:.2}× fewer)\n",
            r.instanced_universal.draw_calls, r.instanced.draw_calls,
        ));
        md.push_str(&format!(
            "- resident instance streams: {} ({} segments); stream uploads after `upload()`: {}\n",
            r.instance_streams, r.instance_segments, r.uploads_after_upload
        ));
        md.push_str(&format!(
            "- resident merged mesh chunks: {} ({} vertices); chunk uploads after `upload()`: {}\n",
            r.mesh_chunks, r.mesh_vertices, r.mesh_uploads_after_upload
        ));
        md.push_str(&format!(
            "- draw reduction (full scene): {} → {} ({:.2}×)\n",
            r.legacy.draw_calls,
            r.instanced.draw_calls,
            if r.instanced.draw_calls == 0 {
                f64::NAN
            } else {
                r.legacy.draw_calls as f64 / r.instanced.draw_calls as f64
            }
        ));
        if let Some(merge) = &r.merge_stats {
            md.push_str(&format!(
                "- font-less host merge cross-check (`model_scene_merge_stats`, visible model \
                 entities): fragments_in={}, batches_out={}, ratio={:.3}, mergeable line pairs={}\n",
                merge.fragments_in,
                merge.batches_out,
                merge.merge_ratio(),
                merge.adjacent_mergeable_line_pairs
            ));
        }

        md.push_str("\n### Single frame at the renderer's default budget (`cad-cli-tools render` shape)\n\n");
        md.push_str(
            "The default `FrameBudget` (8,000,000 vertices / 2,000,000 triangles) makes one frame \
             a *page* of the scene, not the whole drawing — the CLI reports that page's \
             `draw_calls`. The CLI's `--max-batches`/`--max-vertices` do not change this budget; \
             it lives on the renderer.\n\n",
        );
        md.push_str(&format!(
            "- legacy: draw_calls={} accepted opaque={} over_budget={:?}\n",
            r.legacy_default.draw_calls,
            r.legacy_default.opaque_batches,
            r.legacy_default.over_budget
        ));
        md.push_str(&format!(
            "- instanced: draw_calls={} accepted opaque={} over_budget={:?}\n",
            r.instanced_default.draw_calls,
            r.instanced_default.opaque_batches,
            r.instanced_default.over_budget
        ));
        md.push_str(&format!(
            "- instanced-universal (tier forced off): draw_calls={} accepted opaque={} \
             over_budget={:?}\n",
            r.instanced_universal_default.draw_calls,
            r.instanced_universal_default.opaque_batches,
            r.instanced_universal_default.over_budget
        ));

        // Verdict.
        let line_path_batches = ib.instanced_batches;
        let residual_lines = r
            .batches
            .line_batches
            .saturating_add(r.batches.overlay_draws)
            .saturating_sub(line_path_batches);
        let candidates = [
            (
                "line path (coalesced runs + residual legacy)",
                ib.instanced_draws as usize + residual_lines,
            ),
            ("merged mesh runs", ib.mesh_draws as usize),
            ("image", r.batches.images),
        ];
        let winner = candidates
            .iter()
            .max_by_key(|(_, count)| *count)
            .copied()
            .unwrap_or(("none", 0));
        md.push_str(&format!(
            "\n**Remaining-draw verdict:** `{}` dominates the instanced+merged frame ({} of {} draws).\n\n",
            winner.0, winner.1, ib.draw_calls
        ));

        // Phase tables.
        md.push_str("### Per-frame phases\n\n");
        for phase in &r.phases {
            phase_table(&mut md, phase);
        }
    }

    md.push_str(
        "## Reproduce\n\n```bash\nexport PATH=\"$HOME/.cargo/bin:$PATH\"\n\
         cargo test -p cad-cli-tools --test realscene_render_perf -- --ignored --nocapture --test-threads=1\n\
         ```\n\n\
         (Override fixtures with `YACR_PERF_DWGS=\"a.dwg:b.dwg\"` or `YACR_TEST_DWG=a.dwg`; \
         frame count with `YACR_PERF_FRAMES`; output with `YACR_PERF_OUT`.)\n\n",
    );

    md.push_str(
        "## Skipped / not run\n\n\
         - The full mlightcad font catalogue (`scripts/fetch-fonts.sh`) is not fetched; only the \
         committed `fonts/` package (osifont) plus a system default face is registered. A drawing \
         font absent from the committed package falls back to osifont.\n\
         - Arcs/circles/polylines are tessellated to lines before batching, so \"arc\" is not a \
         separate batch class here.\n\
         - No selection/highlight or tool overlay: only the base model scene is measured.\n\
         - Raster images are not decoded/uploaded (no `--images-dir`); the image class is the \
         batch count only.\n\
         - Single machine / single run; not a cross-machine, vendor, or thermal claim.\n",
    );

    std::fs::write(dir.join("realscene.md"), &md)?;

    // JSON.
    let mut fixtures_json = Vec::new();
    for r in reports {
        let phases_json: Vec<serde_json::Value> = r
            .phases
            .iter()
            .map(|p| {
                serde_json::json!({
                    "name": p.name,
                    "frames": p.records.len(),
                    "instance_stream_uploads_before": p.uploads_before,
                    "instance_stream_uploads_after": p.uploads_after,
                    "mesh_chunk_uploads_before": p.mesh_uploads_before,
                    "mesh_chunk_uploads_after": p.mesh_uploads_after,
                    "draw_calls_first_frame": p.draw_calls_first,
                    "cpu_ms": stats_json(&field(&p.records, |r| r.cpu_ms)),
                    "plan_ms": stats_json(&field(&p.records, |r| r.plan_ms)),
                    "uniform_ms": stats_json(&field(&p.records, |r| r.uniform_ms)),
                    "encode_ms": stats_json(&field(&p.records, |r| r.encode_ms)),
                    "submit_ms": stats_json(&field(&p.records, |r| r.submit_ms)),
                    "wait_ms": stats_json(&field(&p.records, |r| r.wait_ms)),
                })
            })
            .collect();
        fixtures_json.push(serde_json::json!({
            "path": r.path,
            "bytes": r.bytes,
            "entity_count": r.entity_count,
            "model_entities": r.model_entities,
            "import_completeness": r.import_completeness,
            "fonts": {
                "requested_count": r.fonts.requested_count,
                "requested_unique_count": r.fonts.requested_unique_count,
                "requested_unique": r.fonts.requested_unique,
                "registered": r.fonts.registered,
                "fallback_keys": r.fonts.fallback_keys,
                "dir": r.fonts.dir,
                "note": r.fonts.note,
            },
            "fragment_classes": {
                "lines": r.fragments.lines,
                "line_vertices": r.fragments.line_vertices,
                "meshes": r.fragments.meshes,
                "mesh_triangles": r.fragments.mesh_triangles,
                "glyph_meshes": r.fragments.glyph_meshes,
                "glyph_triangles": r.fragments.glyph_triangles,
                "texts": r.fragments.texts,
                "instances": r.fragments.instances,
                "images": r.fragments.images,
            },
            "batch_classes": {
                "line_batches": r.batches.line_batches,
                "line_vertices": r.batches.line_vertices,
                "mesh_batches": r.batches.mesh_batches,
                "mesh_triangles": r.batches.mesh_triangles,
                "glyph_batches": r.batches.glyph_batches,
                "glyph_triangles": r.batches.glyph_triangles,
                "mesh_edge_batches": r.batches.mesh_edge_batches,
                "overlay_draws": r.batches.overlay_draws,
                "opaque": r.batches.opaque,
                "transparent": r.batches.transparent,
                "invisible": r.batches.invisible,
                "images": r.batches.images,
            },
            "merge_stats_fontless": r.merge_stats.as_ref().map(|m| serde_json::json!({
                "fragments_in": m.fragments_in,
                "batches_out": m.batches_out,
                "merge_ratio": m.merge_ratio(),
                "adjacent_same_style": m.adjacent_same_style,
                "adjacent_mergeable_line_pairs": m.adjacent_mergeable_line_pairs,
            })),
            "full_scene_legacy_frame": frame_summary_json(&r.legacy),
            "full_scene_instanced_frame": frame_summary_json(&r.instanced),
            "full_scene_instanced_universal_frame": frame_summary_json(&r.instanced_universal),
            "default_budget_legacy_frame": frame_summary_json(&r.legacy_default),
            "default_budget_instanced_frame": frame_summary_json(&r.instanced_default),
            "default_budget_instanced_universal_frame": frame_summary_json(
                &r.instanced_universal_default
            ),
            "instance_streams": r.instance_streams,
            "instance_segments": r.instance_segments,
            "uploads_after_upload": r.uploads_after_upload,
            "mesh_chunks": r.mesh_chunks,
            "mesh_vertices": r.mesh_vertices,
            "mesh_uploads_after_upload": r.mesh_uploads_after_upload,
            "non_background": r.non_background,
            "png": r.png,
            "phases": phases_json,
        }));
    }
    let document = serde_json::json!({
        "schema_version": 1,
        "kind": "realscene_render_frame_perf",
        "generated_unix": SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        "adapter": adapter,
        "width": width,
        "height": height,
        "frames_per_phase": frames,
        "deep_zoom_factor": deep_zoom,
        "fixtures": fixtures_json,
    });
    std::fs::write(
        dir.join("realscene.json"),
        serde_json::to_string_pretty(&document).unwrap(),
    )?;
    Ok(())
}

fn stats_json(s: &Stats) -> serde_json::Value {
    serde_json::json!({
        "n": s.n,
        "min": s.min,
        "mean": s.mean,
        "median": s.median,
        "p95": s.p95,
        "max": s.max,
    })
}

fn frame_summary_json(s: &FrameStatsSummary) -> serde_json::Value {
    serde_json::json!({
        "draw_calls": s.draw_calls,
        "opaque_batches": s.opaque_batches,
        "transparent_batches": s.transparent_batches,
        "invisible_batches": s.invisible_batches,
        "image_batches": s.image_batches,
        "instanced_draws": s.instanced_draws,
        "instanced_batches": s.instanced_batches,
        "instance_streams": s.instance_streams,
        "mesh_draws": s.mesh_draws,
        "merged_mesh_batches": s.merged_mesh_batches,
        "mesh_chunks": s.mesh_chunks,
        "uniform_writes": s.uniform_writes,
        "vertices": s.vertices,
        "triangles": s.triangles,
        "over_budget": s.over_budget,
        "tier": s.tier,
        "accelerated": s.accelerated,
        "accelerated_draws": s.accelerated_draws,
    })
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[test]
#[ignore = "real-DWG render-frame perf with fonts, real GPU; run explicitly, not a CI gate"]
fn realscene_render_frame_perf() {
    let fixtures = fixture_paths();
    if fixtures.is_empty() {
        eprintln!(
            "SKIP realscene_render_frame_perf: no fixture (set YACR_PERF_DWGS/YACR_TEST_DWG, or \
             provide the committed fixtures)"
        );
        return;
    }
    for path in &fixtures {
        if !path.is_file() {
            eprintln!(
                "SKIP realscene_render_frame_perf: fixture {} does not exist",
                path.display()
            );
            return;
        }
    }

    let width = env_u32("YACR_PERF_WIDTH", 1600);
    let height = env_u32("YACR_PERF_HEIGHT", 1000);
    let frames = env_usize("YACR_PERF_FRAMES", 24);
    let deep_zoom = env_f64("YACR_PERF_DEEP_ZOOM", 20.0);
    let dir = output_dir();
    if let Err(error) = std::fs::create_dir_all(&dir) {
        eprintln!(
            "SKIP realscene_render_frame_perf: cannot create output dir {}: {error}",
            dir.display()
        );
        return;
    }

    let gpu = match create_headless_gpu_with(BackendPreference::Auto, GpuSelection::Auto) {
        Ok(gpu) => gpu,
        Err(error) => {
            eprintln!("SKIP realscene_render_frame_perf: no wgpu adapter: {error}");
            return;
        }
    };
    eprintln!(
        "realscene_render_frame_perf: adapter backend={} name={} device_type={} driver={} driver_info={}",
        gpu.adapter.backend,
        gpu.adapter.name,
        gpu.adapter.device_type,
        gpu.adapter.driver,
        gpu.adapter.driver_info
    );

    let mut reports = Vec::new();
    for path in &fixtures {
        let started = Instant::now();
        match run_fixture(path, &dir, width, height, frames, deep_zoom, &gpu) {
            Ok(report) => {
                eprintln!(
                    "{}: entities={} legacy_draws={} instanced_draws={} glyph_batches={} \
                     mesh_batches={} line_batches={} streams={} elapsed={:.1}s",
                    short(&report.path),
                    report.entity_count,
                    report.legacy.draw_calls,
                    report.instanced.draw_calls,
                    report.batches.glyph_batches,
                    report.batches.mesh_batches,
                    report.batches.line_batches,
                    report.instance_streams,
                    started.elapsed().as_secs_f64(),
                );
                reports.push(report);
            }
            Err(error) => eprintln!("fixture {} failed: {error}", path.display()),
        }
    }

    if reports.is_empty() {
        eprintln!("SKIP realscene_render_frame_perf: no fixture produced a frame");
        return;
    }

    match write_report(&dir, &reports, frames, deep_zoom, width, height) {
        Ok(()) => {
            eprintln!("wrote {}/realscene.md", dir.display());
            eprintln!("wrote {}/realscene.json", dir.display());
        }
        Err(error) => panic!("report write failed: {error}"),
    }
}
