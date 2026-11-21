//! The macOS host's BIM query seam (`cad_bim_query`, ADR-0042 D11).
//!
//! A BIM query reads the **open document**: the filter, the paging and the
//! counters all come from the `!Send` [`CadSession`] the Tauri/tao main thread
//! owns, and the filter is caller-supplied, so the answer cannot be mirrored ahead
//! of time the way `cad_layers`/`cad_fonts` are. The shared command layer
//! therefore reaches it through this seam, exactly like the recent-drawings seam:
//! IPC thread → [`crate::main_thread`] → session.
//!
//! Nothing here decides what a query means: the session's own `bim_query` calls
//! the backend (`cad_query::bim`), and a refusal (an unknown filter value kind,
//! an over-budget filter, a stale document) travels back unchanged. One call
//! answers the page **and** the facet summary of the same filtered set.
//!
//! This carrier is always `native-below` (there is no `dom-canvas` arm on macOS),
//! so the only "no document" state is the window before the native session exists
//! — it is created on the first tick after the page reports its canvas rect — and
//! a failed creation. Both answer the stable `unsupported-runtime: bimQuery`
//! marker instead of an empty page that would read as "this document has no BIM
//! data".
//!
//! [`CadSession`]: cad_ui_core::session::CadSession

use std::sync::Arc;

use cad_ui_core::bim::{BimQueryHost, BimQueryRequest, BimQueryResponse, BIM_UNSUPPORTED};

use super::CAD;

/// The macOS implementation of [`BimQueryHost`].
pub(super) struct AppBimQueryHost {
    app: tauri::AppHandle,
}

impl AppBimQueryHost {
    pub(super) fn new(app: tauri::AppHandle) -> Arc<Self> {
        Arc::new(Self { app })
    }
}

impl BimQueryHost for AppBimQueryHost {
    fn bim_query(&self, request: &BimQueryRequest) -> Result<BimQueryResponse, String> {
        let app = self.app.clone();
        let request = request.clone();
        crate::main_thread::on_main_thread(&app, move || {
            with_session(|session| session.bim_query(&request))
        })
    }
}

/// Run one BIM read on the session's own (main) thread.
///
/// Before the native session exists there is no document to query, so the honest
/// answer is the named refusal — never an empty page.
fn with_session<T>(
    read: impl FnOnce(&cad_ui_core::session::CadSession) -> Result<T, String>,
) -> Result<T, String> {
    CAD.with(|cad| match cad.borrow().as_ref() {
        Some(session) => read(session),
        None => Err(BIM_UNSUPPORTED.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The carrier's "no session yet" answer is the shared marker, never an empty
    /// page: the chrome keys on it to hide the BIM table instead of showing it as
    /// "this document has no BIM data".
    ///
    /// Compiled (and run) only for the macOS target: the whole `host` module is
    /// `cfg(target_os = "macos")`, and this crate is only type-checked for macOS in
    /// this environment (no SDK/`objc2` link step), so the run is **NOT VERIFIED**.
    #[test]
    fn a_missing_session_refuses_with_the_stable_marker() {
        let error =
            with_session(|_| Ok::<(), String>(())).expect_err("a fresh thread has no CAD session");
        assert_eq!(error, BIM_UNSUPPORTED);
    }
}
