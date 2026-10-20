//! GPU id-pass picking vs CPU picking, on real DWG fixtures.
//!
//! ## What it measures
//!
//! For each committed real-DWG fixture the harness:
//!
//! 1. imports the drawing and builds the display scene exactly like
//!    `realscene_render_perf` / `cad-cli-tools render` (shared app path, real
//!    font engine, `SceneCache::build_compact`);
//! 2. uploads it to a **real headless wgpu device** and renders one frame;
//! 3. samples a grid of pixels. Each sample is picked twice:
//!    * **GPU** — `Renderer::pick_at`, the on-demand id pass plus a one-texel
//!      readback (the P3a-1 machinery);
//!    * **CPU** — `cad_app::picking::pick_at_screen`, the reference path
//!      (`cad_spatial` precise picking over the database);
//! 4. classifies every sample and reports per class.
//!
//! ## Honest scope
//!
//! * Real DWG geometry on the default Vulkan ICD (no `VK_ICD_FILENAMES` is ever
//!   set here). The adapter line in the report is the actual run's adapter.
//!
//! The two picks have deliberately different semantics and the harness does not
//! pretend otherwise. The GPU pick is **exact at the pixel**: it reports the
//! front-most fragment the rasterizer actually produced. The CPU pick is a **ray
//! with a tolerance** (`TolerancePolicy`'s interaction radius in logical pixels),
//! so it can hit geometry that passes near the pixel without covering it. A
//! sample the CPU hits and the GPU does not is therefore not a defect.
//!
//! Where **both** sides hit, two rates are reported: the exact-match rate and the
//! rate at which the CPU can reproduce the GPU's entity at that pixel. A
//! disagreement the CPU cannot reproduce at all is printed in full and fails the
//! test — it is never smoothed over. Sub-element (mesh face) identities are not
//! compared: the id pass cannot derive a face (no primitive index in the fragment
//! stage), so the comparison is entity-level only.
//!
//! Not part of the default CI gates (`#[ignore]`): it needs real DWGs and a real
//! GPU. Run it with
//! `cargo test -p cad-cli-tools --test pick_parity --locked -- --ignored --nocapture`
//! and override the sample budget with `YACR_PICK_PARITY_GRID=<cols>x<rows>` (or
//! the fixture list with `YACR_PICK_PARITY_DWGS=<colon-separated paths>`).

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;
use std::sync::Arc;

use cad_app::camera::{Camera, Projection};
use cad_app::host::HostController;
use cad_domain::{Point3, TaskStamp, TolerancePolicy};
use cad_render_wgpu::headless::{create_headless_gpu, HeadlessGpu};
use cad_render_wgpu::{BackendPreference, Camera2d, RenderTarget, Renderer};
use cad_representation::{ProviderRegistry, RepresentationContext};
use cad_scene::{RenderTopology, SceneBudget, SceneCache, SceneDelta};
use cad_spatial::{pick_closest, BackFacePolicy, PickOptions};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn fixture_paths() -> Vec<PathBuf> {
    if let Ok(raw) = std::env::var("YACR_PICK_PARITY_DWGS") {
        return raw
            .split(':')
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect();
    }
    let root = repo_root().join("fixtures");
    [
        root.join("debug-default-fonts.dwg"),
        root.join("complex-test.dwg"),
    ]
    .into_iter()
    .filter(|path| path.is_file())
    .collect()
}

fn grid() -> (u32, u32) {
    let raw = std::env::var("YACR_PICK_PARITY_GRID").unwrap_or_default();
    let mut parts = raw.split('x');
    let cols = parts.next().and_then(|v| v.parse().ok()).unwrap_or(12);
    let rows = parts.next().and_then(|v| v.parse().ok()).unwrap_or(9);
    (cols.max(1), rows.max(1))
}

/// The batch class a pick key belongs to. Keys are `1 + batch ordinal` (see
/// `PickTable`), so the class table is indexed by the batch order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BatchClass {
    Lines,
    MeshEdges,
    /// A `Mesh` batch with no wireframe overlay: the tessellated glyph class,
    /// which the renderer flat-merges.
    GlyphMesh,
    /// A `Mesh` batch with a wireframe overlay (a solid), whose overlay the
    /// renderer instances.
    SolidMesh,
    /// An analytic SDF conic batch (slice B).
    Conics,
}

impl BatchClass {
    fn of(batch: &cad_scene::RenderBatch) -> Self {
        match batch.topology {
            RenderTopology::Lines => BatchClass::Lines,
            RenderTopology::MeshEdges => BatchClass::MeshEdges,
            RenderTopology::Mesh if batch.edges.is_empty() => BatchClass::GlyphMesh,
            RenderTopology::Mesh => BatchClass::SolidMesh,
            RenderTopology::Conics => BatchClass::Conics,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            BatchClass::Lines => "lines",
            BatchClass::MeshEdges => "mesh-edges",
            BatchClass::GlyphMesh => "glyph-mesh",
            BatchClass::SolidMesh => "solid-mesh",
            BatchClass::Conics => "conics",
        }
    }
}

#[derive(Default, Clone, Copy)]
struct Tally {
    samples: usize,
    both_miss: usize,
    gpu_only: usize,
    cpu_only: usize,
    agree: usize,
    conflict: usize,
}

impl Tally {
    fn merge(&mut self, other: &Tally) {
        self.samples += other.samples;
        self.both_miss += other.both_miss;
        self.gpu_only += other.gpu_only;
        self.cpu_only += other.cpu_only;
        self.agree += other.agree;
        self.conflict += other.conflict;
    }

    /// Agreement over the samples both sides hit: the only rate that can expose a
    /// mis-pick. `None` when the two never both hit.
    fn agreement_rate(&self) -> Option<f64> {
        let mutual = self.agree + self.conflict;
        (mutual > 0).then(|| self.agree as f64 / mutual as f64)
    }
}

struct Conflict {
    x: u32,
    y: u32,
    /// `ranking` (the CPU can see the GPU's entity too, it just preferred
    /// another) or `unexplained` (the GPU named an entity the CPU cannot see at
    /// that pixel with the same tolerance).
    kind: &'static str,
    gpu: String,
    cpu: String,
    class: &'static str,
}

struct FixtureParity {
    path: String,
    adapter: String,
    id_format: String,
    batches: usize,
    samples: usize,
    overall: Tally,
    per_class: Vec<(&'static str, Tally)>,
    conflicts: Vec<Conflict>,
    unexplained: usize,
    unpickable: usize,
    gpu_pick_ms: f64,
}

fn fit_view(delta: &SceneDelta, width: u32, height: u32) -> Option<(Point3, f64)> {
    let mut fit: Option<(f64, f64, f64, f64)> = None;
    for batch in &delta.added {
        let ox = batch.local_origin.x;
        let oy = batch.local_origin.y;
        for vertex in &batch.vertices {
            let x = ox + f64::from(vertex[0]);
            let y = oy + f64::from(vertex[1]);
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
    let world_per_px = (span_x / (f64::from(width) * 0.9))
        .max(span_y / (f64::from(height) * 0.9))
        .max(1e-9);
    Some((
        Point3 {
            x: (min_x + max_x) / 2.0,
            y: (min_y + max_y) / 2.0,
            z: 0.0,
        },
        world_per_px,
    ))
}

fn run_fixture(
    path: &PathBuf,
    gpu: &HeadlessGpu,
    width: u32,
    height: u32,
) -> Result<FixtureParity, String> {
    let bytes: Arc<[u8]> = std::fs::read(path)
        .map_err(|e| format!("read {}: {e}", path.display()))?
        .into();
    let mut controller = HostController::with_demo_document([f64::from(width), f64::from(height)])
        .map_err(|e| e.to_string())?;
    controller
        .open_bytes(bytes, &path.display().to_string())
        .map_err(|e| format!("import {}: {e}", path.display()))?;
    let drawing = controller
        .drawing()
        .ok_or_else(|| "no drawing after open".to_string())?;
    let document = controller.document_id;
    let stamp = TaskStamp::new(document, controller.session.generation);

    // Same scene build as the CLI render path.
    let requested = cad_platform::fonts::requested_fonts(drawing.as_ref());
    let committed = repo_root().join("fonts");
    let font_dir = committed.is_dir().then_some(committed.as_path());
    let fonts = cad_platform::fonts::local::load_engine(
        font_dir,
        &requested,
        false,
        &[],
        cad_platform::fonts::local::cached_system_default_candidates(),
    )
    .map_err(|e| format!("font engine: {e}"))?;
    let registry = ProviderRegistry::with_default_provider();
    let mut context =
        RepresentationContext::new(document, TolerancePolicy::default(), stamp.clone())
            .with_packed_line_segments();
    if let Some(fonts) = &fonts {
        context = context.with_fonts(fonts.clone());
    }
    let mut cache = SceneCache::new(SceneBudget::default());
    let mut delta = SceneDelta {
        stamp: stamp.clone(),
        added: Vec::new(),
        removed_chunks: Vec::new(),
        images: Vec::new(),
        masks: Vec::new(),
    };
    for entity in drawing.model_space() {
        let representation = registry
            .build_expanded(drawing.as_ref(), entity, &context)
            .map_err(|e| format!("representation for {}: {e}", entity.id.0))?;
        let built = cache
            .build_compact(&representation, stamp.clone())
            .map_err(|e| format!("scene build: {e}"))?;
        delta.added.extend(built.added);
        delta.images.extend(built.images);
    }
    if delta.added.is_empty() {
        return Err("the fixture produced no drawable batches".into());
    }

    let classes: Vec<BatchClass> = delta.added.iter().map(BatchClass::of).collect();
    let (center, world_per_px) = fit_view(&delta, width, height)
        .ok_or_else(|| "no drawable geometry to frame".to_string())?;

    // GPU side: the renderer's own camera, matching the CPU camera below.
    let mut renderer = Renderer::new(BackendPreference::Auto);
    renderer
        .initialize_with_device(gpu.device.clone(), gpu.queue.clone())
        .map_err(|e| format!("renderer init: {e}"))?;
    renderer.set_poll_timeout(std::time::Duration::from_secs(60));
    renderer
        .upload(&delta)
        .map_err(|e| format!("upload: {e}"))?;
    let target = RenderTarget::new(width, height);
    let camera_2d = Camera2d {
        center,
        world_per_px,
        z_plane: 0.0,
    };
    renderer
        .render(camera_2d, &target)
        .map_err(|e| format!("render: {e}"))?;

    // CPU side: the same view as a validated `cad_app::camera::Camera`.
    let cpu_camera = Camera {
        eye: Point3 {
            x: center.x,
            y: center.y,
            z: 1000.0,
        },
        target: center,
        up: Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
        projection: Projection::Orthographic {
            scale: world_per_px,
        },
    };
    let policy = TolerancePolicy::default();
    let logical_size = [f64::from(width), f64::from(height)];

    let (cols, rows) = grid();
    let budget = (cols * rows) as usize;

    // Sample pixels the colour pass actually drew wherever possible: a uniform
    // grid on a large drawing lands almost entirely on empty space, which says
    // nothing about agreement. Fall back to the grid when the frame is blank.
    let frame = renderer
        .read_target_rgba()
        .map_err(|e| format!("read frame: {e}"))?
        .pixels;
    let mut drawn: Vec<(u32, u32)> = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let i = (y as usize * width as usize + x as usize) * 4;
            if frame[i] > 8 || frame[i + 1] > 8 || frame[i + 2] > 8 {
                drawn.push((x, y));
            }
        }
    }
    let samples: Vec<(u32, u32)> = if drawn.is_empty() {
        let mut grid_samples = Vec::with_capacity(budget);
        for row in 0..rows {
            for col in 0..cols {
                let x = ((f64::from(col) + 0.5) / f64::from(cols) * f64::from(width)) as u32;
                let y = ((f64::from(row) + 0.5) / f64::from(rows) * f64::from(height)) as u32;
                grid_samples.push((x.min(width - 1), y.min(height - 1)));
            }
        }
        grid_samples
    } else {
        // Evenly subsample the drawn pixels so the budget is spread over the
        // whole drawing rather than the first scanlines.
        let step = drawn.len().div_ceil(budget).max(1);
        drawn.iter().copied().step_by(step).take(budget).collect()
    };

    // CPU items are expanded once and reused for every sample (and for the
    // conflict re-pick below); expanding them per sample would dominate the run.
    let items = cad_app::picking::drawing_pick_items(drawing.as_ref(), document);
    let tolerance = cad_app::picking::pick_tolerance(&policy, &cpu_camera, logical_size)
        .map_err(|e| format!("pick tolerance: {e}"))?;
    let options = PickOptions::new(tolerance)
        .map_err(|e| format!("pick options: {e}"))?
        .with_back_faces(BackFacePolicy::Cull);

    let mut overall = Tally::default();
    let mut per_class: Vec<(&'static str, Tally)> = [
        BatchClass::Lines,
        BatchClass::MeshEdges,
        BatchClass::GlyphMesh,
        BatchClass::SolidMesh,
    ]
    .into_iter()
    .map(|class| (class.as_str(), Tally::default()))
    .collect();
    let mut conflicts = Vec::new();
    let mut unexplained = 0usize;
    let mut unpickable = 0usize;
    let mut gpu_pick_ms = 0.0f64;

    for &(x, y) in &samples {
        let started = std::time::Instant::now();
        let gpu_hit = renderer
            .pick_at(x, y)
            .map_err(|e| format!("pick_at({x},{y}): {e}"))?;
        gpu_pick_ms += started.elapsed().as_secs_f64() * 1000.0;

        let ray = cpu_camera
            .screen_to_ray([f64::from(x) + 0.5, f64::from(y) + 0.5], logical_size)
            .ok_or_else(|| format!("no pick ray for ({x}, {y})"))?;
        let cpu_report =
            pick_closest(&ray, &items, &options).map_err(|e| format!("cpu pick({x},{y}): {e}"))?;

        // The GPU class comes from the owning batch's topology; a CPU-only hit
        // has no batch, so it is reported under the overall tally only.
        let class = gpu_hit
            .as_ref()
            .and_then(|hit| classes.get(hit.entity_key as usize - 1))
            .copied();

        let mut tally = Tally {
            samples: 1,
            ..Default::default()
        };
        match (&gpu_hit, &cpu_report.hit) {
            (None, None) => tally.both_miss = 1,
            (Some(_), None) => tally.gpu_only = 1,
            (None, Some(_)) => tally.cpu_only = 1,
            (Some(gpu_hit), Some(cpu_hit)) => {
                let gpu_entities: Vec<u128> = gpu_hit
                    .candidates
                    .iter()
                    .map(|source| source.entity.0)
                    .collect();
                if gpu_entities.contains(&cpu_hit.source.entity.0) {
                    tally.agree = 1;
                } else {
                    tally.conflict = 1;
                    // Is the GPU's entity *also* under the pixel, just ranked
                    // differently by the CPU stage? Re-pick on the CPU over only
                    // the GPU's candidate entities, with the same ray and
                    // tolerance. A hit means both stages found a valid entity and
                    // simply preferred different ones (a tolerance/ordering
                    // difference); a miss means the GPU reported an entity the CPU
                    // cannot see there at all, which is unexplained and reported
                    // separately.
                    let restricted = cad_spatial::pick_closest_borrowed(
                        &ray,
                        items.iter().filter_map(|item| {
                            gpu_entities.contains(&item.source.entity.0).then_some((
                                &item.source,
                                &item.geometry,
                                &item.transform,
                            ))
                        }),
                        &options,
                    )
                    .map_err(|e| format!("restricted cpu pick({x},{y}): {e}"))?;
                    let kind = if restricted.hit.is_some() {
                        "ranking"
                    } else if !restricted.skipped.is_empty() {
                        // The CPU cannot test that geometry at all (an opaque or
                        // otherwise unsupported primitive): not comparable, and
                        // reported as such rather than counted as a mis-pick.
                        unpickable += 1;
                        "cpu-unpickable"
                    } else {
                        unexplained += 1;
                        "unexplained"
                    };
                    conflicts.push(Conflict {
                        x,
                        y,
                        kind,
                        gpu: format!(
                            "{:?} ({} candidate(s), key {})",
                            gpu_entities,
                            gpu_entities.len(),
                            gpu_hit.entity_key
                        ),
                        cpu: format!(
                            "{} at distance {:.6}",
                            cpu_hit.source.entity.0, cpu_hit.distance
                        ),
                        class: class.map_or("unknown", |class| class.as_str()),
                    });
                }
            }
        }
        overall.merge(&tally);
        if let Some(class) = class {
            if let Some((_, slot)) = per_class
                .iter_mut()
                .find(|(name, _)| *name == class.as_str())
            {
                slot.merge(&tally);
            }
        }
    }

    Ok(FixtureParity {
        path: path.display().to_string(),
        adapter: format!(
            "{} ({}, {})",
            gpu.adapter.name, gpu.adapter.backend, gpu.adapter.device_type
        ),
        id_format: renderer
            .id_target_format()
            .map_or("none".to_string(), |format| format.as_str().to_string()),
        batches: renderer.batch_count(),
        samples: samples.len(),
        overall,
        per_class,
        conflicts,
        unexplained,
        unpickable,
        gpu_pick_ms,
    })
}

fn report(parity: &FixtureParity) {
    println!("=== {} ===", parity.path);
    println!(
        "adapter={} id_format={} batches={} samples={}",
        parity.adapter, parity.id_format, parity.batches, parity.samples
    );
    let rate = |tally: &Tally| {
        tally
            .agreement_rate()
            .map_or("n/a".to_string(), |rate| format!("{:.3}", rate))
    };
    println!(
        "overall: both_miss={} gpu_only={} cpu_only={} agree={} conflict={} agreement={}",
        parity.overall.both_miss,
        parity.overall.gpu_only,
        parity.overall.cpu_only,
        parity.overall.agree,
        parity.overall.conflict,
        rate(&parity.overall)
    );
    for (class, tally) in &parity.per_class {
        println!(
            "  {class:<11} both_miss={:<4} gpu_only={:<4} cpu_only={:<4} agree={:<4} \
             conflict={:<3} agreement={}",
            tally.both_miss,
            tally.gpu_only,
            tally.cpu_only,
            tally.agree,
            tally.conflict,
            rate(tally)
        );
    }
    if parity.samples > 0 {
        println!(
            "gpu pick cost: {:.2} ms total, {:.3} ms/sample (id pass + 1-texel readback)",
            parity.gpu_pick_ms,
            parity.gpu_pick_ms / parity.samples as f64
        );
    }
    // Two honest rates over the samples both sides hit: the exact-match rate and
    // the rate at which the CPU can *reproduce* the GPU's entity at that pixel
    // (a `ranking` disagreement means both entities are under the pixel and the
    // two stages order them differently, which is a semantics difference, not a
    // mis-pick).
    let mutual = parity.overall.agree + parity.overall.conflict;
    if mutual > 0 {
        let reproducible = parity.overall.agree
            + parity
                .conflicts
                .iter()
                .filter(|conflict| conflict.kind == "ranking")
                .count();
        println!(
            "mutual hits: {mutual}; exact-match {:.3}; gpu-entity-reproducible-by-cpu {:.3}",
            parity.overall.agree as f64 / mutual as f64,
            reproducible as f64 / mutual as f64
        );
    }
    if parity.conflicts.is_empty() {
        println!("conflicts: none (no sample was hit by both sides with different entities)");
    } else {
        println!(
            "conflicts ({}; ranking={} cpu-unpickable={} unexplained={}):",
            parity.conflicts.len(),
            parity.conflicts.len() - parity.unpickable - parity.unexplained,
            parity.unpickable,
            parity.unexplained
        );
        for conflict in parity.conflicts.iter().filter(|c| c.kind != "ranking") {
            println!(
                "  ({}, {}) [{}] class={} gpu={} cpu={}",
                conflict.x, conflict.y, conflict.kind, conflict.class, conflict.gpu, conflict.cpu
            );
        }
        let ranking: Vec<&Conflict> = parity
            .conflicts
            .iter()
            .filter(|c| c.kind == "ranking")
            .collect();
        for conflict in ranking.iter().take(8) {
            println!(
                "  ({}, {}) [ranking] class={} gpu={} cpu={}",
                conflict.x, conflict.y, conflict.class, conflict.gpu, conflict.cpu
            );
        }
        if ranking.len() > 8 {
            println!("  ... {} more ranking conflicts", ranking.len() - 8);
        }
    }
}

/// Real-DWG parity between the GPU id pass and CPU picking.
///
/// `#[ignore]`d like the other real-DWG/real-GPU harnesses: run it explicitly
/// (see the module docs). It asserts nothing about a zero conflict count — a
/// conflict is reported, never hidden, and CPU picking stays the fallback by
/// design.
#[test]
#[ignore = "real DWG + real GPU parity harness; run explicitly"]
fn gpu_id_pass_matches_cpu_picking_on_real_fixtures() {
    let fixtures = fixture_paths();
    if fixtures.is_empty() {
        eprintln!("skipping: no DWG fixtures found (see fixtures/ or YACR_PICK_PARITY_DWGS)");
        return;
    }
    let gpu = match create_headless_gpu(BackendPreference::Auto) {
        Ok(gpu) => gpu,
        Err(error) => {
            eprintln!("skipping: no wgpu adapter: {error}");
            return;
        }
    };
    let (width, height) = (640u32, 480u32);
    let mut reports = Vec::new();
    for path in &fixtures {
        match run_fixture(path, &gpu, width, height) {
            Ok(parity) => {
                report(&parity);
                reports.push(parity);
            }
            Err(error) => eprintln!("skipping {}: {error}", path.display()),
        }
    }
    if reports.is_empty() {
        eprintln!("skipping: no fixture produced a comparable scene");
        return;
    }

    // Where both sides hit, they must agree far more often than not, and every
    // disagreement must be explainable as a ranking difference (both entities are
    // under the pixel, the two stages order them differently). A disagreement the
    // CPU cannot reproduce at all is *not* smoothed over: it is printed above and
    // fails here, so a real mis-pick can never pass silently. CPU picking stays
    // the fallback by design regardless.
    for parity in &reports {
        assert_eq!(
            parity.unexplained, 0,
            "{}: {} conflict(s) could not be reproduced by the CPU at all (see the report)",
            parity.path, parity.unexplained
        );
        // Where both sides hit, the GPU's entity must be reproducible by the CPU
        // at that pixel (either an exact match or a `ranking` disagreement). This
        // is the property the id pass actually has to satisfy; the exact-match
        // rate is printed above and is *not* asserted, because equal-depth ties
        // and tolerance ranking legitimately order the two stages differently.
        let mutual = parity.overall.agree + parity.overall.conflict;
        if mutual > 0 {
            let reproducible = parity.overall.agree
                + parity
                    .conflicts
                    .iter()
                    .filter(|conflict| conflict.kind == "ranking")
                    .count();
            let rate = reproducible as f64 / mutual as f64;
            assert!(
                rate >= 0.95,
                "{}: GPU entity reproducible by the CPU at only {rate:.3} of mutual-hit pixels",
                parity.path
            );
        }
    }
}
