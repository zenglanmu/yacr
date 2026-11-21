//! The Windows host's request-carrying read seam (`cad_bim_query`, ADR-0042 D11).
//!
//! The request kind, its refusals and the seam shape are shared
//! ([`cad_ui_core::host_read`]); this carrier only injects its hop. The `!Send`
//! [`CadSession`] lives on the Tauri/tao main thread, so the closure marshals
//! there through [`crate::main_thread`] and runs [`CadSession::host_read`].
//!
//! This carrier is always `native-below` (there is no `dom-canvas` arm on
//! Windows), so the only "no document" state is the window before the native
//! session exists — created on the first tick after the page reports its canvas
//! rect — and a failed creation. Both answer the kind's stable
//! `unsupported-runtime: bimQuery` marker instead of an empty page.
//!
//! [`CadSession`]: cad_ui_core::session::CadSession

use std::sync::Arc;

use cad_desktop_shell::state::HostReadFn;
use cad_ui_core::bim::BIM_UNSUPPORTED;

use super::CAD;

/// One host-read closure bound to the app handle (installed once at startup).
pub(super) fn host_read_host(app: tauri::AppHandle) -> Arc<HostReadFn> {
    HostReadFn::new(move |request| {
        let app = app.clone();
        let request = request.clone();
        crate::main_thread::on_main_thread(&app, move || {
            with_session(|session| session.host_read(&request))
        })
    })
}

/// Run one read on the session's own (main) thread; no session = the named
/// refusal, never an empty page.
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
    /// page: the chrome keys on it to hide the BIM table.
    ///
    /// Compiled (and run) only for the Windows target: the whole `host` module is
    /// `cfg(target_os = "windows")`.
    #[test]
    fn a_missing_session_refuses_with_the_stable_marker() {
        let error =
            with_session(|_| Ok::<(), String>(())).expect_err("a fresh thread has no CAD session");
        assert_eq!(error, BIM_UNSUPPORTED);
    }
}
