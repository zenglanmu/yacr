//! Statistics and the markdown/JSON report writers.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use super::fixture::{FixtureReport, FrameStatsSummary};
use super::frames::{FrameRecord, PhaseResult};
use super::short;

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

fn phase_table(md: &mut String, result: &PhaseResult) {
    md.push_str(&format!(
        "**{}** ({} frames; `world_per_px` {:.6e}; tier={} accelerated={}; over_budget={}; \
         `instance_stream_uploads` {} → {}; `mesh_chunk_uploads` {} → {})\n\n",
        result.name,
        result.records.len(),
        result.world_per_px,
        result.tier,
        result.accelerated,
        result
            .over_budget
            .as_deref()
            .unwrap_or("none (whole resident scene accepted)"),
        result.uploads_before,
        result.uploads_after,
        result.mesh_uploads_before,
        result.mesh_uploads_after,
    ));
    md.push_str(
        "| phase | n | min | mean | median | p95 | max |\n|---|---:|---:|---:|---:|---:|---:|\n",
    );
    type Row = (&'static str, fn(&FrameRecord) -> f64);
    let rows: [Row; 10] = [
        ("wall_ms", |r| r.wall_ms),
        ("cpu_ms", |r| r.cpu_ms),
        ("residual_ms", |r| r.residual_ms),
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
    md.push_str(
        "\n`residual_ms = wall_ms − (plan_ms + uniform_ms + encode_ms + submit_ms)`. \
         **Not GPU time**: it folds in the blocking poll (`wait_ms`), any present and the \
         small pre-phase CPU; no `TIMESTAMP_QUERY` exists on this device.\n\n",
    );
}

pub(crate) fn write_report(
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
    md.push_str(
        "Per-frame `wall_ms` is the whole `Renderer::render()` call; \
         `residual_ms = wall_ms − (plan_ms + uniform_ms + encode_ms + submit_ms)`. \
         **`residual_ms` is NOT GPU time**: it folds in the blocking device poll \
         (`wait_ms`, which waits on the GPU), the pre-phase CPU and timer noise. No feature \
         is ever requested (`DeviceDescriptor { ..Default::default() }`), so there is no \
         `TIMESTAMP_QUERY` and **no GPU execution time appears in these tables**.\n\n",
    );

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

        // Full view vs deep zoom: does the frame cost track the visible
        // geometry? Both legs use the same renderer, budget and tier, so a ratio
        // near 1.0 is the honest evidence that this frame's cost is ~constant in
        // the visible geometry (there is no visibility culling — backlog §P1).
        if let (Some(mid), Some(deep)) = (
            r.phases.iter().find(|p| p.name == "pan_mid_instanced"),
            r.phases.iter().find(|p| p.name == "pan_deep_instanced"),
        ) {
            md.push_str("### Full view vs deep zoom (frame cost vs visible geometry)\n\n");
            md.push_str(&format!(
                "Both legs run the same renderer, frame budget and tier; the deep leg is the \
                 fit view's `world_per_px` / {deep_zoom:.1} (so the visible world area is \
                 ≈1/{} of the fit view), panned the same 8 px/frame. A ratio near 1.0 means \
                 the frame cost does **not** track the visible geometry — the expected result \
                 while no chunk-visibility culling exists (backlog §P1).\n\n",
                deep_zoom * deep_zoom
            ));
            md.push_str("| leg | world_per_px | path | tier | accelerated | over_budget | wall mean ms | cpu mean ms | residual mean ms | wait mean ms | draw_calls (first) |\n");
            md.push_str("|---|---:|---|---|---:|---|---:|---:|---:|---:|---:|\n");
            for (tag, phase) in [("full (pan_mid)", mid), ("deep (pan_deep)", deep)] {
                md.push_str(&format!(
                    "| {} | {:.6e} | instanced+merged | {} | {} | {} | {:.3} | {:.3} | {:.3} | {:.3} | {} |\n",
                    tag,
                    phase.world_per_px,
                    phase.tier,
                    phase.accelerated,
                    phase.over_budget.as_deref().unwrap_or("none"),
                    field(&phase.records, |r| r.wall_ms).mean,
                    field(&phase.records, |r| r.cpu_ms).mean,
                    field(&phase.records, |r| r.residual_ms).mean,
                    field(&phase.records, |r| r.wait_ms).mean,
                    phase.draw_calls_first,
                ));
            }
            let ratio = |f: fn(&FrameRecord) -> f64| {
                let m = field(&mid.records, f).mean;
                let d = field(&deep.records, f).mean;
                if m.abs() < f64::EPSILON {
                    f64::NAN
                } else {
                    d / m
                }
            };
            md.push_str(&format!(
                "\nDeep/full mean ratios: wall **{:.2}×**, cpu {:.2}×, residual {:.2}×, wait \
                 {:.2}×. A mean ratio ≈1 (within run-to-run noise) is measured *constancy*: \
                 the frame does the same amount of work at the fit view and deep inside the \
                 drawing. The structural reason is that the renderer submits every resident \
                 draw regardless of the view (no chunk-visibility culling exists), so \
                 `draw_calls` and the plan/encode work are view-independent. This is **not** a \
                 GPU measurement (see the residual note above) and it does not by itself say \
                 the cost is GPU-bound.\n\n",
                ratio(|r| r.wall_ms),
                ratio(|r| r.cpu_ms),
                ratio(|r| r.residual_ms),
                ratio(|r| r.wait_ms),
            ));
        }
    }

    md.push_str(
        "## Reproduce\n\n```bash\nexport PATH=\"$HOME/.cargo/bin:$PATH\"\n\
         cargo test -p cad-cli-tools --test realscene_render_perf -- --ignored --nocapture --test-threads=1\n\
         ```\n\n\
         (Override fixtures with `YACR_PERF_DWGS=\"a.dwg:b.dwg\"` or `YACR_TEST_DWG=a.dwg`; \
         frame count with `YACR_PERF_FRAMES`; deep zoom with `YACR_PERF_DEEP_ZOOM`; adapter \
         kind with `YACR_PERF_GPU=auto|high|low` (default `auto`, discrete preferred); output \
         with `YACR_PERF_OUT`.)\n\n",
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
         - **No GPU time is measured**: `FrameStats::gpu_ms` stays `None`, no `TIMESTAMP_QUERY` \
         is requested or available, and `residual_ms` is *not* a GPU measurement (it contains \
         the blocking poll and the pre-phase CPU). `wall_ms` includes the blocking poll.\n\
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
                    "tier": p.tier,
                    "accelerated": p.accelerated,
                    "over_budget": p.over_budget,
                    "world_per_px": p.world_per_px,
                    "wall_ms": stats_json(&field(&p.records, |r| r.wall_ms)),
                    "cpu_ms": stats_json(&field(&p.records, |r| r.cpu_ms)),
                    "residual_ms": stats_json(&field(&p.records, |r| r.residual_ms)),
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
