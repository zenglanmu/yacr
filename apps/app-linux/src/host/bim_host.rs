//! The Linux host's BIM query seam (`cad_bim_query`/`cad_bim_summary`, ADR-0042
//! D11).
//!
//! A BIM query reads the **open document**: the filter, the paging and the
//! counters all come from the `!Send` [`CadSession`] the GTK main thread owns, and
//! the filter is caller-supplied, so the answer cannot be mirrored ahead of time
//! the way `cad_layers`/`cad_fonts` are. The shared command layer therefore
//! reaches it through this seam, exactly like the recent/recovery/ymodel seams:
//! IPC thread → [`crate::main_thread`] → session.
//!
//! Nothing here decides what a query means: the session's own `bim_query`/
//! `bim_summary` calls the backend (`cad_query::bim`), and a refusal (an unknown
//! filter value kind, an over-budget filter, a stale document) travels back
//! unchanged. The `dom-canvas` carrier has no native session by design and
//! refuses with the stable `unsupported-runtime: bimQuery` marker instead of an
//! empty table that would read as "this document has no BIM data".
//!
//! [`CadSession`]: cad_ui_core::session::CadSession

use std::sync::Arc;

use cad_ui_core::bim::{
    BimHost, BimQueryRequest, BimQueryResponse, BimSummaryResponse, BIM_UNSUPPORTED,
};

use super::CAD;

/// The Linux implementation of [`BimHost`].
pub(super) struct AppBimHost {
    app: tauri::AppHandle,
}

impl AppBimHost {
    pub(super) fn new(app: tauri::AppHandle) -> Arc<Self> {
        Arc::new(Self { app })
    }
}

impl BimHost for AppBimHost {
    fn bim_query(&self, request: &BimQueryRequest) -> Result<BimQueryResponse, String> {
        let app = self.app.clone();
        let request = request.clone();
        crate::main_thread::on_main_thread(&app, move || {
            with_session(|session| session.bim_query(&request))
        })
    }

    fn bim_summary(&self, request: &BimQueryRequest) -> Result<BimSummaryResponse, String> {
        let app = self.app.clone();
        let request = request.clone();
        crate::main_thread::on_main_thread(&app, move || {
            with_session(|session| session.bim_summary(&request))
        })
    }
}

/// Run one BIM read on the session's own (main) thread.
///
/// Before the native session exists (it is created on the first tick after the
/// page reports its canvas rect) and on the `dom-canvas` backend there is no
/// native document to query, so the honest answer is the named refusal.
fn with_session<T>(
    read: impl FnOnce(&cad_ui_core::session::CadSession) -> Result<T, String>,
) -> Result<T, String> {
    if crate::backend::surface_mode() == crate::backend::SurfaceMode::DomCanvas {
        // No native session by design on this carrier; the wasm session owns the
        // document there and its adapter answers (or refuses) for itself.
        return Err(BIM_UNSUPPORTED.to_string());
    }
    CAD.with(|cad| match cad.borrow().as_ref() {
        Some(session) => read(session),
        None => Err(BIM_UNSUPPORTED.to_string()),
    })
}
