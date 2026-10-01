//! Headless core CLI. Every data operation shares the application/database
//! command path used by the UI; there is no second business implementation.
//!
//! Spec v2.0 §19: operations return structured JSON with object IDs, transaction
//! results and diagnostics — never an opaque success string. Diagnostics are
//! redacted by `cad-diagnostics`; fixed-viewport rendering explicitly requires
//! a GPU environment and otherwise reports "not run".

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cad_app::host::HostController;
use cad_app::{Command, CommandId, CommandPayload};
use cad_domain::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliOperation {
    Scan,
    ProxyReport,
    Measure,
    ImportNotes,
    ExportNotes,
    BuildRepresentation,
    FixedViewportRender,
    Benchmark,
}

impl CliOperation {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "scan" => Some(Self::Scan),
            "proxy-report" => Some(Self::ProxyReport),
            "measure" => Some(Self::Measure),
            "import-notes" => Some(Self::ImportNotes),
            "export-notes" => Some(Self::ExportNotes),
            "build-representation" => Some(Self::BuildRepresentation),
            "render" => Some(Self::FixedViewportRender),
            "benchmark" => Some(Self::Benchmark),
            _ => None,
        }
    }
}

/// A versioned invocation record; re-running it reproduces the command.
#[derive(Debug, Clone)]
pub struct CliInvocation {
    pub schema_version: u32,
    pub operation: CliOperation,
    pub input: PathBuf,
    pub notes: Option<PathBuf>,
    /// Measure input in the input file's coordinate space.
    pub points: Vec<Point3>,
    /// Annotation import: attach despite a fingerprint mismatch.
    pub allow_fingerprint_mismatch: bool,
}

impl CliInvocation {
    pub fn new(operation: CliOperation, input: impl Into<PathBuf>) -> Self {
        CliInvocation {
            schema_version: 1,
            operation,
            input: input.into(),
            notes: None,
            points: Vec::new(),
            allow_fingerprint_mismatch: false,
        }
    }
}

fn read_input(path: &Path) -> CadResult<Arc<[u8]>> {
    let bytes = std::fs::read(path).map_err(|e| CadError::InvalidInput(format!("read failed: {e}")))?;
    Ok(Arc::from(bytes.into_boxed_slice()))
}

fn load_document(invocation: &CliInvocation) -> CadResult<HostController> {
    let bytes = read_input(&invocation.input)?;
    let mut controller = HostController::with_demo_document([1920.0, 1080.0])?;
    controller.open_bytes(bytes, &invocation.input.display().to_string())?;
    Ok(controller)
}

fn completeness_json(completeness: &Completeness) -> serde_json::Value {
    match completeness {
        Completeness::Complete => serde_json::json!({ "status": "complete" }),
        Completeness::Partial(items) => serde_json::json!({ "status": "partial", "items": items }),
        Completeness::Missing(items) => serde_json::json!({ "status": "missing", "items": items }),
        Completeness::Unverified => serde_json::json!({ "status": "unverified" }),
    }
}

fn support_name(status: SupportStatus) -> &'static str {
    match status {
        SupportStatus::NotImplemented => "not_implemented",
        SupportStatus::Unsupported => "unsupported",
        SupportStatus::Unverified => "unverified",
        SupportStatus::Partial => "partial",
        SupportStatus::Verified => "verified",
    }
}

fn run_scan(controller: &HostController) -> CadResult<serde_json::Value> {
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
        "schema_version": 1,
        "operation": "scan",
        "entities": document.drawing.entity_count(),
        "layers": document.drawing.layers().count(),
        "bounds": bounds,
        "units": format!("{:?}", document.units.source),
        "completeness": completeness,
        "diagnostics": diagnostics,
    }))
}

fn run_proxy_report(controller: &HostController) -> CadResult<serde_json::Value> {
    let report = controller
        .last_import_report
        .as_ref()
        .ok_or_else(|| CadError::InvalidInput("no import report; open a DWG first".into()))?;
    let proxy_diagnostics: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.code.starts_with("proxy") || d.code.starts_with("unknown"))
        .map(|d| serde_json::json!({ "code": d.code, "message": cad_diagnostics::redact_text(&d.message) }))
        .collect();
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
        "schema_version": 1,
        "operation": "proxy-report",
        "completeness": completeness_json(&report.completeness),
        "entity_types": types,
        "proxy_diagnostics": proxy_diagnostics,
        "note": "entity types are reported per capability; absence of a type is not a compatibility claim",
    }))
}

fn run_measure(controller: &mut HostController, invocation: &CliInvocation) -> CadResult<serde_json::Value> {
    if invocation.points.len() < 2 {
        return Err(CadError::InvalidInput(
            "measure needs two (distance), three (angle) or more (length) points".into(),
        ));
    }
    let command = Command {
        schema_version: 1,
        id: CommandId::Measure,
        document: controller.document_id.clone(),
        viewport: controller.viewport_id.clone(),
        payload: CommandPayload::Points(invocation.points.clone()),
    };
    let outcome = controller.execute(command)?;
    let units = controller
        .application
        .workspace
        .documents
        .get(&controller.document_id)
        .map(|d| format!("{:?}", d.units.source))
        .unwrap_or_else(|| "unknown".to_string());
    Ok(serde_json::json!({
        "schema_version": 1,
        "operation": "measure",
        "input_points": invocation.points.iter().map(|p| [p.x, p.y, p.z]).collect::<Vec<_>>(),
        "results": outcome.diagnostics.iter().map(|d| serde_json::json!({
            "code": d.code,
            "message": d.message,
        })).collect::<Vec<_>>(),
        "units": units,
    }))
}

fn run_export_notes(controller: &mut HostController, invocation: &CliInvocation) -> CadResult<serde_json::Value> {
    let target = invocation
        .notes
        .clone()
        .unwrap_or_else(|| invocation.input.with_extension("cadnotes.json"));
    let json = controller.export_annotations_json()?;
    std::fs::write(&target, json.as_bytes())
        .map_err(|e| CadError::InvalidInput(format!("annotation write failed: {e}")))?;
    // Only mark saved after the bytes are durably written.
    controller.mark_annotations_saved()?;
    Ok(serde_json::json!({
        "schema_version": 1,
        "operation": "export-notes",
        "annotations": controller
            .application
            .workspace
            .documents
            .get(&controller.document_id)
            .map(|d| d.annotations.len())
            .unwrap_or(0),
        "bytes": json.len(),
        "saved": true,
    }))
}

fn run_import_notes(controller: &mut HostController, invocation: &CliInvocation) -> CadResult<serde_json::Value> {
    let source = invocation
        .notes
        .clone()
        .ok_or_else(|| CadError::InvalidInput("import-notes needs --notes <file>".into()))?;
    let text = std::fs::read_to_string(&source)
        .map_err(|e| CadError::InvalidInput(format!("annotation read failed: {e}")))?;
    let policy = if invocation.allow_fingerprint_mismatch {
        cad_annotations::FingerprintPolicy::ImportUnanchored
    } else {
        cad_annotations::FingerprintPolicy::RejectMismatch
    };
    let count = controller.import_annotations_json(&text, policy)?;
    Ok(serde_json::json!({
        "schema_version": 1,
        "operation": "import-notes",
        "imported": count,
        "undo_recorded": controller.application.can_undo(&controller.document_id),
    }))
}

fn run_build_representation(controller: &HostController) -> CadResult<serde_json::Value> {
    let document = controller
        .application
        .workspace
        .documents
        .get(&controller.document_id)
        .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
    let registry = cad_representation::ProviderRegistry::with_default_provider();
    let context = cad_representation::RepresentationContext::new(
        controller.document_id.clone(),
        TolerancePolicy::default(),
        TaskStamp::new(controller.document_id.clone(), controller.session.generation),
    );
    let mut batches = 0usize;
    let mut vertices = 0usize;
    let mut failures: Vec<serde_json::Value> = Vec::new();
    for entity in document.drawing.model_space() {
        match registry.build(entity, &context) {
            Ok(representation) => {
                for fragment in &representation.fragments {
                    if let cad_representation::DisplayPrimitive::Lines(points) = &fragment.primitive {
                        batches += 1;
                        vertices += points.len();
                    }
                }
            }
            Err(error) => failures.push(serde_json::json!({
                "entity": entity.id.0.to_string(),
                "error": error.to_string(),
            })),
        }
    }
    Ok(serde_json::json!({
        "schema_version": 1,
        "operation": "build-representation",
        "primitives": batches,
        "vertices": vertices,
        "failures": failures,
    }))
}

fn run_benchmark(controller: &HostController, invocation: &CliInvocation) -> CadResult<serde_json::Value> {
    let bytes = std::fs::metadata(&invocation.input)
        .map(|m| m.len())
        .unwrap_or(0);
    let start = std::time::Instant::now();
    let built = run_build_representation(controller)?;
    let build_ms = start.elapsed().as_secs_f64() * 1000.0;
    Ok(serde_json::json!({
        "schema_version": 1,
        "operation": "benchmark",
        "file_bytes": bytes,
        "representation_build_ms": build_ms,
        "representation": built,
        "environment": {
            "release_build": !cfg!(debug_assertions),
            "gpu": "not required for geometry benchmark",
        },
    }))
}

/// Execute one CLI operation and return structured JSON.
pub fn run(invocation: &CliInvocation) -> CadResult<String> {
    if invocation.operation == CliOperation::FixedViewportRender {
        // Explicitly not run: a fixed-viewport GPU frame needs a device the CLI
        // does not own. Use the platform host or a future GPU runner.
        return Err(CadError::Unsupported(
            "fixed-viewport rendering requires a GPU environment; not run".into(),
        ));
    }
    let mut controller = load_document(invocation)?;
    let value = match invocation.operation {
        CliOperation::Scan => run_scan(&controller)?,
        CliOperation::ProxyReport => run_proxy_report(&controller)?,
        CliOperation::Measure => run_measure(&mut controller, invocation)?,
        CliOperation::ImportNotes => run_import_notes(&mut controller, invocation)?,
        CliOperation::ExportNotes => run_export_notes(&mut controller, invocation)?,
        CliOperation::BuildRepresentation => run_build_representation(&controller)?,
        CliOperation::Benchmark => run_benchmark(&controller, invocation)?,
        CliOperation::FixedViewportRender => unreachable!(),
    };
    serde_json::to_string_pretty(&value)
        .map_err(|e| CadError::Invariant(format!("cli encode failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_operation_names_parse() {
        for name in ["scan", "proxy-report", "measure", "import-notes", "export-notes", "build-representation", "render", "benchmark"] {
            assert!(CliOperation::parse(name).is_some(), "{name}");
        }
        assert!(CliOperation::parse("nope").is_none());
    }

    #[test]
    fn render_operation_reports_not_run() {
        let invocation = CliInvocation::new(CliOperation::FixedViewportRender, "missing.dwg");
        let error = run(&invocation).unwrap_err();
        assert!(matches!(error, CadError::Unsupported(_)));
    }

    #[test]
    fn scan_of_missing_file_is_an_input_error() {
        let invocation = CliInvocation::new(CliOperation::Scan, "definitely-missing.dwg");
        let error = run(&invocation).unwrap_err();
        assert!(matches!(error, CadError::InvalidInput(_)));
    }
}