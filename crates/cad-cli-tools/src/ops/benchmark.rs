//! Geometry benchmark operation.

use super::*;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn run_benchmark(
    controller: &HostController,
    invocation: &CliInvocation,
    fonts: Option<&Arc<cad_representation::FontEngine>>,
) -> CadResult<serde_json::Value> {
    use cad_diagnostics::{
        BenchmarkBudgets, BenchmarkReport, MeasuredContext, MeasuredMemory, MeasuredTimings,
    };

    // `file_bytes` is the real on-disk size of the opened sample.
    let file_bytes = std::fs::metadata(&invocation.input).ok().map(|m| m.len());

    // Parse time was measured by the importer on this exact open; it is `None`
    // if this path never ran the importer (for example a caller reusing a
    // pre-loaded document), and we report `null` rather than fake a number.
    let parse_ms = controller
        .last_import_report
        .as_ref()
        .and_then(|report| report.parse_ms);

    let document = controller
        .application
        .workspace
        .documents
        .get(&controller.document_id)
        .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
    let registry = cad_representation::ProviderRegistry::with_default_provider();
    let context = representation_context(controller, fonts);

    // Build the display representation and batch it, timing the geometry build
    // and measuring CPU cache bytes exactly (SceneCache counts every chunk).
    let mut cache = cad_scene::SceneCache::new(cad_scene::SceneBudget::default());
    let mut delta = cad_scene::SceneDelta {
        stamp: context.stamp.clone(),
        added: Vec::new(),
        removed_chunks: Vec::new(),
        images: Vec::new(),
        masks: Vec::new(),
    };
    let build_start = std::time::Instant::now();
    let mut primitives = 0usize;
    let mut failures: Vec<serde_json::Value> = Vec::new();
    for entity in document.drawing.model_space() {
        match registry.build_expanded(&document.drawing, entity, &context) {
            Ok(representation) => {
                primitives += representation.fragments.len();
                let built = cache.build(&representation, context.stamp.clone())?;
                delta.added.extend(built.added);
                delta.images.extend(built.images);
                delta.masks.extend(built.masks);
            }
            Err(error) => failures.push(serde_json::json!({
                "entity": entity.id.0.to_string(),
                "error": error.to_string(),
            })),
        }
    }
    let build_ms = build_start.elapsed().as_secs_f64() * 1000.0;

    // GPU estimate is the exact packed buffer size the renderer would upload for
    // these batches, summed from the scene itself (not a heuristic). Computed
    // before publishing moves the batches into the cache.
    let gpu_estimated_bytes: u64 = delta
        .added
        .iter()
        .map(|batch| batch.upload_size_bytes() as u64)
        .sum();
    let vertices: usize = delta.added.iter().map(|b| b.vertices.len()).sum();
    let triangles: usize = delta
        .added
        .iter()
        .filter(|b| b.topology == cad_scene::RenderTopology::Mesh)
        .map(|b| b.triangle_count())
        .sum();
    let batches = delta.added.len();

    // Publish so the CPU cache accounts the bytes exactly as the live cache
    // would; `total_cpu_bytes` is the real accounted geometry footprint.
    cache.publish(delta, &context.stamp)?;
    let cpu_geometry_bytes = cache.total_cpu_bytes() as u64;

    // This native CLI path does not own a GPU device, so upload and
    // first-usable frame times are not measurable here. They stay `None`; the
    // `benchmark-gpu` (render) path owns those phases. Never a fabricated 0.
    let sample_hash = controller
        .last_import_report
        .as_ref()
        .and_then(sample_hash_of);
    let measured_context = MeasuredContext {
        sample_hash,
        // No GPU device is selected on this geometry path.
        device: None,
        // The CLI is not a browser.
        browser: None,
        release_build: !cfg!(debug_assertions),
        // The controlling session viewport is real and known.
        viewport: Some(controller.viewport_id),
        quality_configuration: Some("scene-budget-default".to_string()),
    };
    let report = BenchmarkReport {
        // Raw sample bytes are not retained here, so the sample identity is the
        // importer's content hash carried on the report, `None` when this path
        // did not import. It is never a path or a file name.
        sample_hash,
        release_build: !cfg!(debug_assertions),
        timings: MeasuredTimings {
            parse_ms,
            build_ms: Some(build_ms),
            upload_ms: None,
            first_usable_ms: None,
            complete_ms: Some(build_ms),
        },
        memory: MeasuredMemory {
            file_bytes,
            domain_bytes: None, // no byte-exact database estimate exists
            cpu_geometry_bytes: Some(cpu_geometry_bytes),
            gpu_estimated_bytes: Some(gpu_estimated_bytes),
            atlas_bytes: None,      // no atlas is built on this path
            attachment_bytes: None, // no render attachments on this path
        },
        budgets: BenchmarkBudgets {
            cpu_bytes: Some(cad_scene::SceneBudget::default().cpu_bytes as u64),
            upload_bytes_per_frame: Some(
                cad_scene::SceneBudget::default().upload_bytes_per_frame as u64,
            ),
            queued_tasks: Some(cad_scene::SceneBudget::default().queued_tasks as u64),
            max_vertices_per_frame: Some(
                cad_scene::SceneBudget::default().max_vertices_per_frame as u64,
            ),
            max_triangles_per_frame: Some(
                cad_scene::SceneBudget::default().max_triangles_per_frame as u64,
            ),
        },
        context: measured_context,
        over_budget: Vec::new(),
    };

    let mut value = report.to_json();
    // Keep the operation/schema envelope the other CLI documents use, and the
    // scene counts as measured facts (not a performance grade). `batches` is
    // captured before the cache took ownership of the delta.
    value["schema_version"] = serde_json::json!(CLI_SCHEMA_VERSION);
    value["operation"] = serde_json::json!(CliOperation::Benchmark.as_str());
    value["scene"] = serde_json::json!({
        "primitives": primitives,
        "batches": batches,
        "vertices": vertices,
        "triangles": triangles,
    });
    value["failures"] = serde_json::json!(failures);
    Ok(value)
}

/// Recover the content hash from an import report's identity, if it carries one.
///
/// `DocumentIdentity::Temporary` carries no content hash, so this returns `None`
/// and the benchmark leaves the hash absent rather than inventing one.
#[cfg(not(target_arch = "wasm32"))]
fn sample_hash_of(report: &cad_import_acadrust::ImportReport) -> Option<[u8; 32]> {
    match &report.identity {
        DocumentIdentity::Sha256(hash) => Some(*hash),
        _ => None,
    }
}
