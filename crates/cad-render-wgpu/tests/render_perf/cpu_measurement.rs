use super::*;

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
        masks: Vec::new(),
    }
}

/// The view state a measured leg drives the camera in.
///
/// `Full` fits the whole synthetic scene in the target; `Zoom` is
/// [`ZOOM_FACTOR`] times closer, so most of the scene is off-screen. Chunk
/// visibility (backlog §P1) only matters in the zoomed state, so the baseline
/// frame-cost distribution is recorded for both.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Full,
    Zoom,
}

impl ViewMode {
    fn label(self) -> &'static str {
        match self {
            ViewMode::Full => "full",
            ViewMode::Zoom => "zoom",
        }
    }
}

/// The `Full` view fits the scene; `Zoom` divides `world_per_px` by this.
const ZOOM_FACTOR: f64 = 16.0;

/// World-x extent of the synthetic [`scene`]: `line_batch(i)` spans
/// `x in [i*0.25, i*0.25 + 0.1]`, so the last batch ends at
/// `(batches-1)*0.25 + 0.1`.
fn scene_span_x(batches: usize) -> f64 {
    (batches.saturating_sub(1)) as f64 * 0.25 + 0.1
}

/// `world_per_px` that fits the whole synthetic scene into `target` (the `Full`
/// view). Uses the larger of the two axis requirements, so both axes fit.
fn fit_all_world_per_px(batches: usize, target: &RenderTarget) -> f64 {
    let span_x = scene_span_x(batches).max(f64::MIN_POSITIVE);
    let span_y = 0.2_f64;
    (span_x / target.width.max(1) as f64).max(span_y / target.height.max(1) as f64)
}

/// The `world_per_px` a leg measured in `view` drives.
fn view_world_per_px(
    view: ViewMode,
    batches: usize,
    target: &RenderTarget,
    zoom_factor: f64,
) -> f64 {
    let full = fit_all_world_per_px(batches, target);
    match view {
        ViewMode::Full => full,
        ViewMode::Zoom => full / zoom_factor,
    }
}

struct SceneResult {
    label: String,
    batches: usize,
    /// `"full"` or `"zoom"`: the view state this row was measured in.
    view: &'static str,
    /// The `world_per_px` this row's camera used.
    world_per_px: f64,
    cpu: Stats,
    wall: Stats,
    /// `wall - (plan_ms + uniform_ms + encode_ms + submit_ms)` per frame.
    ///
    /// **Not** a GPU measurement: it folds the blocking device poll (which
    /// waits on the GPU), any present and the pre-phase CPU (`ensure_target`
    /// and the flags-table top-up) together with timer noise.
    residual: Stats,
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
    /// The over-budget reason when the frame was a *page* of the scene rather
    /// than the whole resident scene (a budget fallback), else `None`.
    over_budget: Option<String>,
    /// Resident instanced segments after the scene's upload.
    instance_segments: u64,
    /// Instanced stream buffers uploaded for the scene.
    instance_streams: usize,
}

/// One measurement leg: which synthetic scene scale, which draw path and which
/// view state. Kept as a struct so `measure_scene` stays within clippy's
/// argument limit.
struct Leg<'a> {
    label: &'a str,
    batches: usize,
    instanced: bool,
    view: ViewMode,
    world_per_px: f64,
}

fn measure_scene(
    renderer: &mut Renderer,
    target: &RenderTarget,
    leg: &Leg<'_>,
    frames: usize,
    warmup: usize,
) -> SceneResult {
    let samples = run_frames(
        renderer,
        target,
        &scene(leg.batches),
        frames,
        warmup,
        leg.instanced,
        leg.world_per_px,
    );
    let stats = samples.last.expect("at least one timed frame");
    SceneResult {
        label: leg.label.to_string(),
        batches: leg.batches,
        view: leg.view.label(),
        world_per_px: leg.world_per_px,
        cpu: percentiles(&samples.cpu),
        wall: percentiles(&samples.wall),
        residual: percentiles(&samples.residual),
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
        instanced: leg.instanced,
        tier: stats.tier.as_str(),
        accelerated: stats.accelerated,
        over_budget: stats.over_budget.as_ref().map(|reason| {
            format!(
                "{} requested={} limit={} skipped_batches={}",
                reason.report.category,
                reason.report.requested,
                reason.report.limit,
                reason.report.skipped_batches
            )
        }),
        instance_segments: renderer.instance_segment_count(),
        instance_streams: renderer.instance_stream_count(),
    }
}

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

fn stat_row(label: &str, s: &Stats) -> String {
    format!(
        "| {label} | {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} |",
        s.n, s.min_ms, s.mean_ms, s.median_ms, s.p95_ms, s.max_ms
    )
}

/// The device facts the report must state: which adapter actually ran the
/// frames, and which features are enabled on that device.
///
/// The features list is the honest proof that **no** feature was requested
/// (both device-creation sites use `DeviceDescriptor { ..Default::default() }`),
/// which is why there is no `TIMESTAMP_QUERY` and therefore no GPU time in this
/// report.
struct DeviceFacts {
    adapter: cad_render_wgpu::headless::AdapterInfo,
    /// `Renderer::device_features()` rendered with `Debug`.
    enabled_features: String,
    /// Whether `TIMESTAMP_QUERY` is enabled on the live device.
    timestamp_query_enabled: bool,
}

/// The frames/zoom shape a report describes.
struct RunShape {
    frames: usize,
    warmup: usize,
    zoom_factor: f64,
}

fn write_report(
    dir: &Path,
    tag: &str,
    facts: &DeviceFacts,
    results: &[SceneResult],
    run: RunShape,
    host: &[(usize, f64, f64)],
) -> String {
    let RunShape {
        frames,
        warmup,
        zoom_factor,
    } = run;
    let adapter = &facts.adapter;
    std::fs::create_dir_all(dir).expect("create output dir");
    let mut md = String::new();
    md.push_str("# Renderer per-frame cost measurement (CPU phases + wall/residual)\n\n");
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
    md.push_str(&format!(
        "Each scene is measured in two view states of the same scene: `full` fits the \
         whole scene in the 1280×800 target, `zoom` is {zoom_factor}× closer \
         (`world_per_px = full / {zoom_factor}`), so most of the scene is off-screen — \
         the view state a chunk-visibility change must move. The per-frame **frame-cost \
         distribution** is `wall`, `cpu` and **`residual = wall − (plan_ms + uniform_ms + \
         encode_ms + submit_ms)`**.\n\n\
         **`residual` is NOT a GPU measurement.** It is the wall time the CPU phase timers \
         do not cover: the blocking device poll (`wait_ms`, which waits on the GPU), any \
         present, the pre-phase CPU (`ensure_target` + the flags-table top-up) and timer \
         noise. There is no `TIMESTAMP_QUERY` on this device (see the adapter section), so \
         **no number in this report is GPU execution time**.\n\n",
    ));

    md.push_str("## GPU adapter (proof this is real hardware)\n\n");
    md.push_str(&format!(
        "| backend | device_type | name | driver |\n|---|---|---|---|\n| {} | {} | {} | {} |\n\n",
        adapter.backend, adapter.device_type, adapter.name, adapter.driver
    ));
    md.push_str(&format!(
        "Enabled device features: `{}` — `TIMESTAMP_QUERY` enabled: **{}**. The list is the \
         proof that no feature was requested (`DeviceDescriptor {{ ..Default::default() }}`); \
         no cross-machine, vendor or software-vs-hardware claim is made here beyond the \
         adapter row above.\n\nFrames per scene/state: {frames}; warmup frames: {warmup} \
         (untimed).\n\n",
        facts.enabled_features, facts.timestamp_query_enabled
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

    // The frame-cost distribution the backlog §P0 asks for: wall, the CPU
    // phase sum, and the residual (which is NOT GPU time).
    md.push_str("## Frame-cost distribution per view state (ms/frame)\n\n");
    md.push_str(
        "`residual = wall − (plan_ms + uniform_ms + encode_ms + submit_ms)`, aggregated \
         mean / median / p95 over the timed frames. **`residual` is not GPU time**: it \
         contains the blocking poll (see `wait mean`), any present and the small \
         pre-phase CPU (`ensure_target` / flags top-up), so it is only ever an upper \
         bound on non-CPU frame cost, not a GPU measurement.\n\n",
    );
    md.push_str("| Scene | view | world_per_px | n | wall mean | wall median | wall p95 | cpu mean | cpu median | cpu p95 | residual mean | residual median | residual p95 | wait mean |\n");
    md.push_str("|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n");
    for r in results {
        md.push_str(&format!(
            "| {} | {} | {:e} | {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} |\n",
            r.label,
            r.view,
            r.world_per_px,
            r.wall.n,
            r.wall.mean_ms,
            r.wall.median_ms,
            r.wall.p95_ms,
            r.cpu.mean_ms,
            r.cpu.median_ms,
            r.cpu.p95_ms,
            r.residual.mean_ms,
            r.residual.median_ms,
            r.residual.p95_ms,
            r.wait.mean_ms,
        ));
    }
    md.push('\n');

    md.push_str("## Frame composition (constant across timed frames)\n\n");
    md.push_str(
        "| Scene | batches | tier | accelerated | over_budget | draw_calls | instanced_draws | instanced_batches | instance_streams | instance_segments | uniform_writes | vertices | opaque | transparent | invisible |\n",
    );
    md.push_str("|---|---:|---|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n");
    for r in results {
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            r.label,
            r.batches,
            r.tier,
            r.accelerated,
            r.over_budget.as_deref().unwrap_or("none"),
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
        "YACR_PERF_TAG={tag} YACR_PERF_OUT=\"$PWD/…\" \\\n  cargo test -p cad-render-wgpu --test render_perf renderer_frame_cpu_measurement -- --ignored --nocapture --test-threads=1\n```\n\n"
    ));
    md.push_str(
        "## Skipped / not run\n\n\
         - No real DWG geometry in this leg (synthetic scale-matched batches); the real \
           import + preparation cost is measured in `cad-host`'s `pan_zoom_cpu_baseline`.\n\
         - **No GPU time is measured.** `FrameStats::gpu_ms` stays `None`; there is no \
           `TIMESTAMP_QUERY` (no feature is ever requested — see the adapter section) and \
           the crate has no timestamp/`QuerySet` plumbing at all. The **residual** \
           (`wall − Σ CPU phases`) is *not* GPU time: it folds in the blocking poll, any \
           present and the small pre-phase CPU. `wall` likewise includes the blocking poll.\n\
         - Single machine / single run; no cross-machine, vendor, thermal or \
           release-profile claim.\n",
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
    let zoom_factor = env_f64("YACR_PERF_ZOOM_FACTOR", ZOOM_FACTOR);
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

    // Which adapter actually ran these frames is reported, never assumed. The
    // enabled features prove that no feature (notably `TIMESTAMP_QUERY`) was
    // requested, which is why this report holds no GPU time.
    let features = renderer.device_features();
    let timestamp_query_enabled =
        features.contains(cad_render_wgpu::wgpu::Features::TIMESTAMP_QUERY);
    eprintln!(
        "adapter: api={} device_type={} name={} driver={} features={:?} timestamp_query_enabled={}",
        adapter.backend,
        adapter.device_type,
        adapter.name,
        adapter.driver,
        features,
        timestamp_query_enabled,
    );

    let mut results = Vec::new();
    for (label, batches) in &scales {
        // The same scene measured at two view states: `full` fits the scene in
        // the target, `zoom` is `zoom_factor` times closer (most of the scene
        // off-screen). Chunk visibility (backlog §P1) only bites in `zoom`, so
        // both are recorded as the baseline distribution.
        for view in [ViewMode::Full, ViewMode::Zoom] {
            let world_per_px = view_world_per_px(view, *batches, &target, zoom_factor);
            // "before" (legacy per-batch path) and "after" (instanced streams)
            // are measured on the same binary, device, scene and view, so the
            // delta is the instanced path alone.
            for instanced in [false, true] {
                let mode = if instanced { "instanced" } else { "legacy" };
                let label = format!("{label} [{mode}] [{}]", view.label());
                let leg = Leg {
                    label: &label,
                    batches: *batches,
                    instanced,
                    view,
                    world_per_px,
                };
                let result = measure_scene(&mut renderer, &target, &leg, frames, warmup);
                eprintln!(
                    "{}: batches={} mode={mode} view={} wpp={:.6} tier={} accelerated={} \
                     cpu mean={:.3}ms median={:.3}ms p95={:.3}ms wall mean={:.3}ms \
                     residual mean={:.3}ms wait mean={:.3}ms draw_calls={} \
                     instanced_draws={} instanced_batches={} streams={} segments={} uniform_writes={}",
                    label,
                    batches,
                    result.view,
                    result.world_per_px,
                    result.tier,
                    result.accelerated,
                    result.cpu.mean_ms,
                    result.cpu.median_ms,
                    result.cpu.p95_ms,
                    result.wall.mean_ms,
                    result.residual.mean_ms,
                    result.wait.mean_ms,
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
    }

    let host = env_string("YACR_PERF_HOST_SUMMARY")
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| parse_host_summary(&text))
        .unwrap_or_default();

    let facts = DeviceFacts {
        adapter,
        enabled_features: format!("{features:?}"),
        timestamp_query_enabled,
    };
    let report = write_report(
        &dir,
        &tag,
        &facts,
        &results,
        RunShape {
            frames,
            warmup,
            zoom_factor,
        },
        &host,
    );
    eprintln!("wrote {report}");
}
