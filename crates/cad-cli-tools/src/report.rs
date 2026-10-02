//! report module.

use super::*;

pub(crate) fn completeness_json(completeness: &Completeness) -> serde_json::Value {
    match completeness {
        Completeness::Complete => serde_json::json!({ "status": "complete" }),
        Completeness::Partial(items) => serde_json::json!({ "status": "partial", "items": items }),
        Completeness::Missing(items) => serde_json::json!({ "status": "missing", "items": items }),
        Completeness::Unverified => serde_json::json!({ "status": "unverified" }),
    }
}

pub(crate) fn support_name(status: SupportStatus) -> &'static str {
    match status {
        SupportStatus::NotImplemented => "not_implemented",
        SupportStatus::Unsupported => "unsupported",
        SupportStatus::Unverified => "unverified",
        SupportStatus::Partial => "partial",
        SupportStatus::Verified => "verified",
    }
}

pub(crate) fn units_name(units: &UnitContext) -> String {
    format!("{:?}", units.source)
}

/// Diagnostics flagged as entity-completeness relevant during import.
///
/// A partially- or un-rendered entity is never silently dropped: it contributes
/// a non-empty `completeness.items` or `completeness_issues` entry (audit B30).
pub(crate) fn is_completeness_diagnostic(code: &str) -> bool {
    code.starts_with("import.proxy") || code.starts_with("import.unknown")
}

pub(crate) fn import_completeness_issues(
    report: &cad_import_acadrust::ImportReport,
) -> Vec<serde_json::Value> {
    report
        .diagnostics
        .iter()
        .filter(|d| is_completeness_diagnostic(&d.code))
        .map(|d| {
            serde_json::json!({
                "code": d.code,
                "message": cad_diagnostics::redact_text(&d.message),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------
/// Structured measurement record: numeric values are locale-independent and
/// carry no formatted human text (N01 §5.3 item 7).
pub(crate) fn measurement_json(record: &cad_db::MeasurementRecord) -> serde_json::Value {
    let plane = record.plane.map(|plane| {
        serde_json::json!({
            "origin": [plane.origin.x, plane.origin.y, plane.origin.z],
            "u": [plane.u.x, plane.u.y, plane.u.z],
            "v": [plane.v.x, plane.v.y, plane.v.z],
        })
    });
    serde_json::json!({
        "algorithm": format!("{:?}", record.algorithm),
        "inputs": record.inputs.iter().map(|p| [p.x, p.y, p.z]).collect::<Vec<_>>(),
        "plane": plane,
        "value": record.value,
        "units": units_name(&record.units),
        "source": format!("{:?}", record.source),
        "precision": format!("{:?}", record.precision),
    })
}
