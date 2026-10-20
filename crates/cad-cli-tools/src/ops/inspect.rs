//! Drawing inspection and representation-report operations.

use super::*;

pub(crate) fn run_scan(controller: &HostController) -> CadResult<serde_json::Value> {
    let document = controller
        .application
        .workspace
        .documents
        .get(&controller.document_id)
        .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
    let bounds = document.drawing.bounds().map(|(min, max)| {
        serde_json::json!({ "min": [min.x, min.y, min.z], "max": [max.x, max.y, max.z] })
    });
    let (completeness, diagnostics) = match &controller.last_import_report {
        Some(report) => (
            completeness_json(&report.completeness),
            report
                .diagnostics
                .iter()
                .map(|d| {
                    serde_json::json!({
                        "code": d.code,
                        "message": cad_diagnostics::redact_text(&d.message),
                    })
                })
                .collect::<Vec<_>>(),
        ),
        None => (serde_json::json!({ "status": "unverified" }), Vec::new()),
    };
    Ok(serde_json::json!({
        "schema_version": CLI_SCHEMA_VERSION,
        "operation": CliOperation::Scan.as_str(),
        "entities": document.drawing.entity_count(),
        "model_entities": document.drawing.model_space().len(),
        "block_definitions": document.drawing.blocks().count(),
        "layers": document.drawing.layers().count(),
        "bounds": bounds,
        "units": units_name(&document.units),
        "completeness": completeness,
        "diagnostics": diagnostics,
    }))
}

pub(crate) fn run_proxy_report(controller: &HostController) -> CadResult<serde_json::Value> {
    let report = controller
        .last_import_report
        .as_ref()
        .ok_or_else(|| CadError::InvalidInput("no import report; open a DWG first".into()))?;
    // Every proxy/unknown diagnostic is surfaced, not just a subset: a proxy
    // without a cache is a completeness issue, not silent success (audit B30).
    let proxy_diagnostics = import_completeness_issues(report);
    let types: Vec<_> = report
        .capabilities
        .iter()
        .map(|capability| {
            serde_json::json!({
                "type": capability.type_key,
                "read": support_name(capability.read),
                "semantic": support_name(capability.semantic),
                "render": support_name(capability.render),
                "pick": support_name(capability.pick),
                "measure": support_name(capability.measure),
            })
        })
        .collect();
    Ok(serde_json::json!({
        "schema_version": CLI_SCHEMA_VERSION,
        "operation": CliOperation::ProxyReport.as_str(),
        "completeness": completeness_json(&report.completeness),
        "entity_types": types,
        "proxy_diagnostics": proxy_diagnostics,
        "note": "entity types are reported per capability; absence of a type is not a compatibility claim",
    }))
}

pub(crate) fn run_measure(
    controller: &mut HostController,
    invocation: &CliInvocation,
) -> CadResult<serde_json::Value> {
    if invocation.points.len() < 2 {
        return Err(CadError::InvalidInput(
            "measure needs two (distance), three (angle) or more (length) points".into(),
        ));
    }
    let command = Command {
        schema_version: 1,
        id: CommandId::Measure,
        document: controller.document_id,
        viewport: controller.viewport_id,
        payload: CommandPayload::Points(invocation.points.clone()),
    };
    let outcome = controller.execute(command)?;
    let units = controller
        .application
        .workspace
        .documents
        .get(&controller.document_id)
        .map(|d| units_name(&d.units))
        .unwrap_or_else(|| "unknown".to_string());
    let measurement = outcome.measurement.as_ref().map(measurement_json);
    Ok(serde_json::json!({
        "schema_version": CLI_SCHEMA_VERSION,
        "operation": CliOperation::Measure.as_str(),
        "input_points": invocation.points.iter().map(|p| [p.x, p.y, p.z]).collect::<Vec<_>>(),
        "measurement": measurement,
        "results": outcome.diagnostics.iter().map(|d| serde_json::json!({
            "code": d.code,
            "message": d.message,
        })).collect::<Vec<_>>(),
        "units": units,
    }))
}

/// The representation context shared by `build-representation` and `render`.
///
/// Both must build from the same provider registry, document id, tolerance
/// policy and task stamp; only this constructor is allowed to define that, so
/// the two paths cannot silently drift apart.
pub(crate) fn representation_context(
    controller: &HostController,
    fonts: Option<&Arc<cad_representation::FontEngine>>,
) -> cad_representation::RepresentationContext {
    let mut context = cad_representation::RepresentationContext::new(
        controller.document_id,
        TolerancePolicy::default(),
        TaskStamp::new(controller.document_id, controller.session.generation),
    );
    if let Some(fonts) = fonts {
        context = context.with_fonts(fonts.clone());
    }
    context
}

/// Memory-protection guard for headless scene accumulation.
///
/// A block-reference explosion can expand a small DWG into tens of millions of
/// render batches/vertices. Rather than exhausting host memory and being
/// OOM-killed, the CLI fails explicitly once the accumulated scene exceeds
/// `--max-batches` / `--max-vertices`. A limit of `0` (or `None`) disables the
/// cap for trusted large drawings.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn enforce_scene_budget(
    batches: u64,
    vertices: u64,
    invocation: &CliInvocation,
) -> CadResult<()> {
    if let Some(limit) = invocation.max_batches {
        if limit > 0 && batches > limit {
            return Err(CadError::InvalidInput(format!(
                "render scene exceeds the batch budget: {batches} > {limit} (raise with --max-batches, or 0 to disable)"
            )));
        }
    }
    if let Some(limit) = invocation.max_vertices {
        if limit > 0 && vertices > limit {
            return Err(CadError::InvalidInput(format!(
                "render scene exceeds the vertex budget: {vertices} > {limit} (raise with --max-vertices, or 0 to disable)"
            )));
        }
    }
    Ok(())
}

pub(crate) fn run_build_representation(
    controller: &HostController,
    fonts: Option<&Arc<cad_representation::FontEngine>>,
) -> CadResult<serde_json::Value> {
    let document = controller
        .application
        .workspace
        .documents
        .get(&controller.document_id)
        .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
    let registry = cad_representation::ProviderRegistry::with_default_provider();
    let context = representation_context(controller, fonts);
    let (mut lines, mut meshes, mut texts, mut instances, mut images, mut masks) =
        (0usize, 0, 0, 0, 0, 0);
    let mut vertices = 0usize;
    let mut failures: Vec<serde_json::Value> = Vec::new();
    for entity in document.drawing.model_space() {
        match registry.build_expanded(&document.drawing, entity, &context) {
            Ok(representation) => {
                for fragment in &representation.fragments {
                    match &fragment.primitive {
                        cad_representation::DisplayPrimitive::Lines(points)
                        | cad_representation::DisplayPrimitive::LineSegments(points) => {
                            lines += 1;
                            vertices += points.len();
                        }
                        cad_representation::DisplayPrimitive::Mesh(mesh)
                        | cad_representation::DisplayPrimitive::MeshWithoutEdgeOverlay(mesh) => {
                            meshes += 1;
                            vertices += mesh.vertices.len();
                        }
                        cad_representation::DisplayPrimitive::Conic(conic) => {
                            // Chord-expanded downstream today; count its line form.
                            lines += 1;
                            vertices += conic.chord_points().len();
                        }
                        cad_representation::DisplayPrimitive::Text { .. } => texts += 1,
                        cad_representation::DisplayPrimitive::Instance { .. } => instances += 1,
                        cad_representation::DisplayPrimitive::Image { .. } => images += 1,
                        cad_representation::DisplayPrimitive::Mask(boundary) => {
                            masks += 1;
                            vertices += boundary.len();
                        }
                    }
                }
            }
            Err(error) => failures.push(serde_json::json!({
                "entity": entity.id.0.to_string(),
                "error": error.to_string(),
            })),
        }
    }
    let primitives = lines + meshes + texts + instances + images + masks;
    Ok(serde_json::json!({
        "schema_version": CLI_SCHEMA_VERSION,
        "operation": CliOperation::BuildRepresentation.as_str(),
        "primitives": primitives,
        "vertices": vertices,
        "kind_counts": {
            "lines": lines,
            "meshes": meshes,
            "texts": texts,
            "instances": instances,
            "images": images,
            "masks": masks,
        },
        "failures": failures,
    }))
}
