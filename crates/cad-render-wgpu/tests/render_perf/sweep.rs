use super::*;

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
        masks: Vec::new(),
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
                1.0,
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
