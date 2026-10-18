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
//! (`plan`/`uniform`/`encode`/`submit`/`wait`, plus `cpu_ms`) together with the
//! whole-call `wall_ms` and the `residual_ms = wall_ms − Σ CPU phases` **frame
//! cost distribution** (residual is *not* GPU time) over pan and zoom frames at
//! a mid and a deep zoom, plus a full-view vs deep-zoom **constancy** comparison
//! (does the frame cost track the visible geometry?) and the tier/path each leg
//! actually ran (`accelerated` vs universal per-run, incl. any budget fallback).
//! `instance_stream_uploads` is recorded before
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

mod fixture;
mod fonts;
mod frames;
mod histograms;
mod report;

use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use cad_render_wgpu::headless::create_headless_gpu_with;
use cad_render_wgpu::{BackendPreference, GpuSelection};
use cad_scene::FrameBudget;

use fixture::run_fixture;
use report::write_report;

/// The renderer's default per-frame budget (`Renderer::new`): a single frame is
/// a *page* of the scene, not the whole drawing.
pub(crate) const DEFAULT_FRAME_BUDGET: FrameBudget = FrameBudget {
    max_vertices: 8_000_000,
    max_triangles: 2_000_000,
    max_bytes: usize::MAX,
};

/// Unbounded per-frame budget: one frame accepts the whole resident scene, so
/// the class breakdown and total draw count are for the full drawing.
pub(crate) const FULL_FRAME_BUDGET: FrameBudget = FrameBudget {
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

pub(crate) fn repo_root() -> PathBuf {
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

/// Adapter kind for the run: `YACR_PERF_GPU` = `auto` (default; discrete GPU
/// preferred), `high` or `low`. The default keeps the historical
/// discrete-first behaviour; `low` exists so a large drawing that exhausts a
/// small discrete card's VRAM can still be measured on an integrated adapter —
/// the report always states which adapter actually ran.
fn gpu_selection() -> GpuSelection {
    std::env::var("YACR_PERF_GPU")
        .ok()
        .and_then(|value| GpuSelection::parse(&value))
        .unwrap_or_default()
}

pub(crate) fn short(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
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

    let gpu = match create_headless_gpu_with(BackendPreference::Auto, gpu_selection()) {
        Ok(gpu) => gpu,
        Err(error) => {
            eprintln!("SKIP realscene_render_frame_perf: no wgpu adapter: {error}");
            return;
        }
    };
    eprintln!(
        "realscene_render_frame_perf: gpu_selection={} adapter backend={} name={} device_type={} driver={} driver_info={}",
        gpu_selection().as_str(),
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
