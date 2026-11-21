//! The macOS host's request-carrying read seam (`cad_bim_query`, ADR-0042 D11).
//!
//! The request kind, its refusals and the seam shape are shared
//! ([`cad_ui_core::host_read`]); this carrier only injects its hop. The `!Send`
//! [`CadSession`] lives on the Tauri/tao main thread, so the closure marshals
//! there through [`crate::main_thread`] and runs [`CadSession::host_read`].
//!
//! This carrier is always `native-below` (there is no `dom-canvas` arm on macOS),
//! so the only "no document" state is the window before the native session exists
//! — created on the first tick after the page reports its canvas rect — and a
//! failed creation. Both answer the kind's stable `unsupported-runtime: bimQuery`
//! marker instead of an empty page.
//!
//! [`CadSession`]: cad_ui_core::session::CadSession

use std::sync::Arc;

use cad_desktop_shell::state::HostReadFn;
use cad_ui_core::host_read::HostReadRequest;

use super::CAD;

/// One host-read closure bound to the app handle (installed once at startup).
pub(super) fn host_read_host(app: tauri::AppHandle) -> Arc<HostReadFn> {
    HostReadFn::new(move |request| {
        let app = app.clone();
        let request = request.clone();
        crate::main_thread::on_main_thread(&app, move || {
            with_session(&request, |session| session.host_read(&request))
        })
    })
}

/// Run one read on the session's own (main) thread; no session = the named
/// refusal, never an empty page.
fn with_session<T>(
    request: &HostReadRequest,
    read: impl FnOnce(&cad_ui_core::session::CadSession) -> Result<T, String>,
) -> Result<T, String> {
    CAD.with(|cad| match cad.borrow().as_ref() {
        Some(session) => read(session),
        None => Err(cad_ui_core::host_read::no_seam_refusal(request)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_ui_core::bim::{BimFilterWire, BIM_UNSUPPORTED};
    use cad_ui_core::host_read::HostReadRequest;
    use cad_ui_core::param::PARAM_LIST_UNSUPPORTED;

    fn bim_request() -> HostReadRequest {
        HostReadRequest::BimQuery {
            filter: BimFilterWire::default(),
            offset: 0,
            limit: 10,
        }
    }

    /// The carrier's "no session yet" answer is the kind's shared marker, never an
    /// empty page: the chrome keys on it to hide the BIM table.
    ///
    /// Compiled (and run) only for the macOS target: the whole `host` module is
    /// `cfg(target_os = "macos")`, and this crate is only type-checked for macOS in
    /// this environment (no SDK/`objc2` link step), so the run is **NOT VERIFIED**.
    #[test]
    fn a_missing_session_refuses_with_the_stable_marker() {
        let error = with_session(&bim_request(), |_| Ok::<(), String>(()))
            .expect_err("a fresh thread has no CAD session");
        assert_eq!(error, BIM_UNSUPPORTED);
    }

    /// Each request kind keeps its own word: a parameter list is not BIM.
    #[test]
    fn a_missing_session_refuses_a_parameter_list_by_its_own_word() {
        let error = with_session(&HostReadRequest::ParamList, |_| Ok::<(), String>(()))
            .expect_err("a fresh thread has no CAD session");
        assert_eq!(error, PARAM_LIST_UNSUPPORTED);
    }
}
