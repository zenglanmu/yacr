//! Android-specific panel overlay on top of the shared state funnel.
//!
//! The generic panels (layers/properties/layout/diagnostics/view state), the
//! resources drawer and the view sync are mirrored from `HostUpdates` by the
//! attached `EngineAdapter` (`flush_updates`). This module adds the two channels
//! that are genuinely Android-specific:
//!
//! * the asynchronous-open progress panel (the shared push does not set it; the
//!   poll driver does, and a command must not leave it stale); and
//! * the resources drawer's font summary, which the asset loader reports with
//!   real counts and which must keep `images_modeled: false` (Android does not
//!   model raster images).
//!
//! Doctrine: panels are *derived*, never authored here. A getter that errors or
//! has no data leaves the panel at its explicit empty state rather than showing
//! fabricated rows. All user-facing labels come from the shared catalog, never
//! from Rust literals.

use std::cell::RefCell;

use cad_app::host::HostController;
use cad_engine::protocol::{FontResourceState, ResourceState};
use cad_host::HostCore;
use cad_platform::fonts::FontLoadReport;
use cad_ui_slint::{
    FontResourceSummary, ImportProgressUiState, ImportResourceSummary, ReferencesResourceSummary,
    ResourceCompleteness, ResourceSections, UiHandle,
};

// Last font-loading report, set by the platform font path (the Android asset
// loader). Target-agnostic: the Android loader records it and host tests can set
// it directly, so the resources drawer maps a real `FontLoadReport` without
// pulling the Android-only font module into host builds.
thread_local! {
    static LAST_FONT_REPORT: RefCell<Option<FontLoadReport>> = const { RefCell::new(None) };
    /// A font summary to show after the shared open status, consumed by the poll
    /// driver on the tick that published the document.
    static PENDING_FONT_STATUS: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Record the most recent font-loading report for the resources drawer.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub(crate) fn set_last_font_report(report: FontLoadReport) {
    LAST_FONT_REPORT.with(|slot| *slot.borrow_mut() = Some(report));
}

fn last_font_report() -> Option<FontLoadReport> {
    LAST_FONT_REPORT.with(|slot| slot.borrow().clone())
}

/// Queue the font summary to show after the shared open status.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub(crate) fn set_pending_font_status(text: String) {
    PENDING_FONT_STATUS.with(|slot| *slot.borrow_mut() = Some(text));
}

/// Take the queued font summary, if any.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub(crate) fn take_pending_font_status() -> Option<String> {
    PENDING_FONT_STATUS.with(|slot| slot.borrow_mut().take())
}

/// Overlay the Android asset-loader resource facts onto a host-derived resource
/// state before it reaches the shell.
///
/// The UI-free host cannot read the APK asset loader, so its own
/// `HostState.resources` leaves `fonts: None` (unavailable) and reports
/// `images_modeled: true`. Android resolves fonts through its asset loader and
/// does not model raster images, so the composition root injects the real font
/// report and the honest `images_modeled: false` into every full update. This
/// keeps the drawer correct on command-driven pushes (the generic path) as well
/// as on the poll/overlay path, without changing the shared schema.
pub(crate) fn apply_android_resources(resources: &mut ResourceState) {
    resources.fonts = last_font_report().map(|report| FontResourceState {
        catalog_entries: report.catalog_entries,
        requested: report.requested,
        planned: report.planned,
        registered: report.registered,
        failed: report.failed,
        unresolved: report.unresolved,
        default_face: report.default_face,
    });
    resources.proxy = None;
    resources.images_modeled = false;
}

/// Run one command through the shared core funnel.
///
/// The engine-owned UI callbacks reach the core through `EngineClient::dispatch`
/// and mirror the resulting state via `EngineAdapter::flush_updates`; this
/// helper is retained only for the legacy pre-attach sink, which drives
/// `HostCore::execute` directly.
pub(crate) fn run_command(core: &HostCore, command: cad_app::Command) -> cad_domain::CadResult<()> {
    core.execute(command)
}

/// The two Android-specific channels the generic `HostUpdates` push does not
/// fully cover: the synthesized in-read import snapshot and the asset-loader
/// font summary (which the host cannot read and would otherwise clear).
pub(crate) fn push_android_overlay(handle: &UiHandle, core: &HostCore) {
    push_import_state(handle, core);
    push_resource_sections(handle, core);
}

/// Push the asynchronous-open progress panel from the shared import snapshot.
///
/// Android runs a real background import worker, so the snapshot is usually the
/// controller's own; `core.import_snapshot()` also synthesizes the in-read running
/// state (before the async job exists), so the panel is not cleared during the
/// sample-path read. Idle/opened clear the panel; cancelled/failed stay visible
/// with their explicit terminal text.
pub(crate) fn push_import_state(handle: &UiHandle, core: &HostCore) {
    let messages = crate::platform::messages();
    let snapshot = core.import_snapshot();
    let state = ImportProgressUiState::from_snapshot(snapshot.as_ref(), &messages);
    let _ = handle.set_import_state(&state);
}

/// Push the resources drawer sections from the host's real reports (F10).
///
/// Overrides the shared push's font section with the Android asset loader's real
/// `FontLoadReport`, and keeps `images_modeled: false`: Android does not model
/// raster images this phase.
pub(crate) fn push_resource_sections(handle: &UiHandle, core: &HostCore) {
    let controller = core.controller.borrow();
    let sections = resource_sections(&controller, last_font_report().as_ref());
    let _ = handle.set_resources_sections(&sections);
}

/// Build the resources drawer sections from the host's real reports.
///
/// Only real sources are pushed: the last font-loading report, the last import
/// report, and the open document's external `resource_keys`. Proxy records are
/// not surfaced by the importer in this build, so `proxy` stays `None` rather
/// than being fabricated; the drawer shows its explicit empty state for it.
pub(crate) fn resource_sections(
    controller: &HostController,
    font_report: Option<&FontLoadReport>,
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
                cad_domain::Completeness::Complete => ResourceCompleteness::Complete,
                cad_domain::Completeness::Partial(items) => {
                    ResourceCompleteness::Partial(items.len())
                }
                cad_domain::Completeness::Missing(items) => {
                    ResourceCompleteness::Missing(items.len())
                }
                cad_domain::Completeness::Unverified => ResourceCompleteness::Unverified,
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
