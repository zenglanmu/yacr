//! Per-fixture run: import → representation → scene build → render.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use cad_app::host::HostController;
use cad_app::render_scene::{model_scene_merge_stats, SceneAssemblyStats};
use cad_domain::{TaskStamp, TolerancePolicy};
use cad_render_wgpu::headless::{encode_png, HeadlessGpu};
use cad_render_wgpu::{BackendPreference, FrameStats, RenderTarget, Renderer};
use cad_representation::{ProviderRegistry, RepresentationContext};
use cad_scene::{SceneBudget, SceneCache, SceneDelta};

use super::fonts::{load_fonts, FontProvenance};
use super::frames::{fit_camera, pan_cameras, reclaim, run_phase, zoom_cameras, PhaseResult};
use super::histograms::{BatchHistogram, FragmentHistogram};
use super::{short, DEFAULT_FRAME_BUDGET, FULL_FRAME_BUDGET};

pub(crate) struct FixtureReport {
    pub(crate) path: String,
    pub(crate) bytes: u64,
    pub(crate) entity_count: usize,
    pub(crate) model_entities: usize,
    pub(crate) import_completeness: String,
    pub(crate) fonts: FontProvenance,
    pub(crate) fragments: FragmentHistogram,
    pub(crate) batches: BatchHistogram,
    pub(crate) merge_stats: Option<SceneAssemblyStats>,
    pub(crate) non_background: usize,
    pub(crate) adapter: String,
    /// Full-scene single frame (unbounded budget): the whole drawing.
    pub(crate) legacy: FrameStatsSummary,
    pub(crate) instanced: FrameStatsSummary,
    /// Converted path with the accelerated tier forced **off**: the universal
    /// per-run organization on the same scene/device, so the tier's effect is
    /// isolated from the conversion's effect.
    pub(crate) instanced_universal: FrameStatsSummary,
    /// Single frame at the renderer's default budget (the CLI's real one-frame
    /// result): a page of the scene, with the over-budget reason.
    pub(crate) legacy_default: FrameStatsSummary,
    pub(crate) instanced_default: FrameStatsSummary,
    pub(crate) instanced_universal_default: FrameStatsSummary,
    pub(crate) phases: Vec<PhaseResult>,
    pub(crate) instance_streams: usize,
    pub(crate) instance_segments: u64,
    pub(crate) uploads_after_upload: u64,
    pub(crate) mesh_chunks: usize,
    pub(crate) mesh_vertices: u64,
    pub(crate) mesh_uploads_after_upload: u64,
    pub(crate) png: Option<String>,
}

pub(crate) struct FrameStatsSummary {
    pub(crate) draw_calls: u64,
    pub(crate) opaque_batches: usize,
    pub(crate) transparent_batches: usize,
    pub(crate) invisible_batches: usize,
    pub(crate) image_batches: usize,
    pub(crate) instanced_draws: u64,
    pub(crate) instanced_batches: usize,
    pub(crate) instance_streams: usize,
    pub(crate) mesh_draws: u64,
    pub(crate) merged_mesh_batches: usize,
    pub(crate) mesh_chunks: usize,
    pub(crate) uniform_writes: u64,
    pub(crate) vertices: usize,
    pub(crate) triangles: usize,
    pub(crate) over_budget: Option<String>,
    /// The live draw tier of the frame (`"universal"`/`"accelerated"`).
    pub(crate) tier: &'static str,
    /// Whether the frame's opaque pass used the accelerated per-class draws.
    pub(crate) accelerated: bool,
    /// Draw calls issued by the accelerated per-class path this frame.
    pub(crate) accelerated_draws: u64,
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
pub(crate) fn run_fixture(
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
