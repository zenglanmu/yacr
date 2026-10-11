//! Browser-specific panel overlay on top of the shared state funnel.
//!
//! The generic panels (layers/properties/layout/diagnostics/overlay/view state)
//! and the view sync are pushed by `cad_host::HostCore::push`. This module adds
//! the two channels that are genuinely browser-specific:
//!
//! * the asynchronous-open progress panel, whose terminal state on the browser
//!   comes from the synchronous fallback snapshot ([`super::async_open`]); and
//! * the resources drawer's font summary, which the browser fetch path reports
//!   with real counts (the desktop discards its report, so its shared push shows
//!   the explicit empty font state).
//!
//! Doctrine: panels are *derived*, never authored here. A getter that errors or
//! has no data leaves the panel at its explicit empty state rather than showing
//! fabricated rows. All user-facing labels come from the shared catalog
//! (`super::messages`), never from Rust literals.

use cad_app::host::HostController;
use cad_diagnostics::model::{DiagnosticReason, DiagnosticsModel, Severity};
use cad_domain::{Completeness, Diagnostic};
use cad_host::HostCore;
use cad_ui_slint::{
    FontResourceSummary, ImportProgressUiState, ImportResourceSummary, ReferencesResourceSummary,
    ResourceCompleteness, ResourceSections,
};

use super::async_open as async_open_state;
use super::messages::current_messages;

/// Push the asynchronous-open progress panel from the browser's real snapshot.
///
/// A worker-capable host takes the controller snapshot; the browser re-reads the
/// synchronous fallback's terminal outcome. A cancelled/failed open is shown
/// explicitly and a successful open hides the panel.
pub(super) fn push_import_state(core: &HostCore) {
    let Some(handle) = core.handle.borrow().as_ref().cloned() else {
        return;
    };
    let messages = current_messages();
    let snapshot = async_open_state::current_snapshot(&core.controller.borrow());
    let state = ImportProgressUiState::from_snapshot(snapshot.as_ref(), &messages);
    let _ = handle.set_import_state(&state);
}

/// Push the resources drawer sections from the host's real reports (F10).
///
/// Overrides the shared push's font section with the browser's last real
/// `FontLoadReport`, and keeps `images_modeled: false`: the browser resolves
/// raster images through the browser stack in a later phase, so it must not
/// claim they are modeled.
pub(super) fn push_resource_sections(core: &HostCore) {
    let Some(handle) = core.handle.borrow().as_ref().cloned() else {
        return;
    };
    let controller = core.controller.borrow();
    let sections = derive_resource_sections(&controller, super::fonts::last_font_report().as_ref());
    let _ = handle.set_resources_sections(&sections);
}

/// Build the resources drawer sections from the host's real reports.
///
/// Only real sources are pushed: the last font-loading report, the last import
/// report, and the open document's external `resource_keys`. Proxy records are
/// not surfaced by the importer in this build, so `proxy` stays `None` rather
/// than being fabricated; the drawer shows its explicit empty state for it.
pub(super) fn derive_resource_sections(
    controller: &HostController,
    font_report: Option<&cad_platform::fonts::FontLoadReport>,
) -> ResourceSections {
    let references = controller
        .application
        .workspace
        .documents
        .get(&controller.document_id)
        .map(|document| ReferencesResourceSummary {
            keys: document.resource_keys.clone(),
        });
    let import = controller
        .last_import_report
        .as_ref()
        .map(|report| ImportResourceSummary {
            identity: controller.document_name_hint.clone(),
            // The importer reports no DWG version string; left empty, not guessed.
            dwg_version: String::new(),
            completeness: match &report.completeness {
                Completeness::Complete => ResourceCompleteness::Complete,
                Completeness::Partial(items) => ResourceCompleteness::Partial(items.len()),
                Completeness::Missing(items) => ResourceCompleteness::Missing(items.len()),
                Completeness::Unverified => ResourceCompleteness::Unverified,
            },
            diagnostics: Some(report.diagnostics.len()),
            parse_ms: report.parse_ms.map(|ms| ms.max(0.0) as u64),
        });
    ResourceSections {
        fonts: font_report.map(|report| FontResourceSummary {
            catalog_entries: report.catalog_entries,
            requested: report.requested.len(),
            planned: report.planned.len(),
            registered: report.registered.len(),
            failed: report.failed.clone(),
            unresolved: report.unresolved.clone(),
            default_face: report.default_face.clone(),
        }),
        import,
        proxy: None,
        references,
        images_modeled: false,
    }
}

/// Map an import report's plain [`Diagnostic`]s into the structured drawer model.
///
/// The import report carries a stable machine `code` for every reason but no
/// severity or typed parameters, so this mapping is deliberately conservative:
/// the code is preserved verbatim (the drawer localizes it from the catalog),
/// parameters are left empty rather than invented, and the severity word is a
/// display-only heuristic from the code family (`unavailable`/`missing`/
/// `not_implemented`/`undecoded` are errors, everything else a warning). It
/// never adds a row the report did not contain.
pub(super) fn diagnostics_model_from_import(diagnostics: &[Diagnostic]) -> DiagnosticsModel {
    let mut model = DiagnosticsModel::new();
    for diagnostic in diagnostics {
        let severity = severity_for_code(&diagnostic.code);
        let reason = DiagnosticReason::new(
            diagnostic.code.clone(),
            severity,
            completeness_for_severity(severity),
            Vec::new(),
        );
        match diagnostic.object {
            Some(object) => model.add(object, reason),
            None => model.add_document(reason),
        }
    }
    model
}

/// Display-only severity word for a stable diagnostic code.
fn severity_for_code(code: &str) -> Severity {
    const ERROR_SUFFIXES: [&str; 4] =
        [".unavailable", ".missing", ".not_implemented", ".undecoded"];
    if ERROR_SUFFIXES.iter().any(|suffix| code.ends_with(suffix)) {
        Severity::Error
    } else {
        Severity::Warning
    }
}

/// Completeness implied by the heuristic severity, so the drawer summary is not
/// "complete" while a reason is shown.
fn completeness_for_severity(severity: Severity) -> Completeness {
    match severity {
        Severity::Error => Completeness::Missing(Vec::new()),
        Severity::Warning => Completeness::Partial(Vec::new()),
        Severity::Info => Completeness::Unverified,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_diagnostics::model::codes;
    use cad_domain::ObjectId;

    #[test]
    fn diagnostics_model_preserves_every_reason_and_object() {
        let diagnostics = vec![
            Diagnostic {
                object: Some(ObjectId(3)),
                code: codes::REPRESENTATION_UNAVAILABLE.to_string(),
                message: "unavailable".to_string(),
            },
            Diagnostic {
                object: Some(ObjectId(3)),
                code: codes::REPRESENTATION_APPROXIMATE.to_string(),
                message: "approx".to_string(),
            },
            Diagnostic {
                object: None,
                code: codes::FONT_UNRESOLVED.to_string(),
                message: "font".to_string(),
            },
        ];
        let model = diagnostics_model_from_import(&diagnostics);
        // Document reason + both object reasons are kept.
        assert_eq!(model.document.len(), 1);
        assert_eq!(model.objects.len(), 1);
        assert_eq!(model.objects[0].reasons.len(), 2);
        assert!(!model.summary().complete);
    }

    #[test]
    fn no_import_diagnostics_means_an_explicit_empty_drawer() {
        let model = diagnostics_model_from_import(&[]);
        assert!(model.document.is_empty());
        assert!(model.objects.is_empty());
    }

    #[test]
    fn severity_mapping_is_conservative_and_documented() {
        assert_eq!(
            severity_for_code(codes::REPRESENTATION_UNAVAILABLE),
            Severity::Error
        );
        assert_eq!(
            severity_for_code(codes::REPRESENTATION_APPROXIMATE),
            Severity::Warning
        );
        // An unknown code is never invented as an error.
        assert_eq!(severity_for_code("something.else"), Severity::Warning);
    }
}
