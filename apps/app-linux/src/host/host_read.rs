//! The Linux host's request-carrying read seam (`cad_bim_query`, ADR-0042 D11).
//!
//! The request kind, its refusals and the seam shape are shared
//! ([`cad_ui_core::host_read`]); this carrier only injects its hop. The `!Send`
//! [`CadSession`] lives on the GTK main thread, so the closure marshals there
//! through [`crate::main_thread`] and runs [`CadSession::host_read`].
//!
//! Nothing here decides what a read means: the session's own `host_read` calls
//! the backend (`cad_query::bim` for the one BIM kind), and a refusal (an unknown
//! filter value kind, an over-budget filter, a stale document) travels back
//! unchanged. `dom-canvas` (X11/XWayland) has no native session by design — the
//! wasm session inside the page owns the document — so this side refuses with the
//! kind's own marker instead of an empty page that would read as "this document
//! has no BIM data".
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

/// Run one read on the session's own (main) thread.
///
/// Before the native session exists (created on the first tick after the page
/// reports its canvas rect), on `dom-canvas`, and after a failed creation there is
/// no document to read: the honest answer is the request kind's own named refusal.
fn with_session<T>(
    request: &HostReadRequest,
    read: impl FnOnce(&cad_ui_core::session::CadSession) -> Result<T, String>,
) -> Result<T, String> {
    if crate::backend::surface_mode() == crate::backend::SurfaceMode::DomCanvas {
        return Err(cad_ui_core::host_read::no_seam_refusal(request));
    }
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

    /// The carrier's "no live session" answer is the kind's shared marker, never
    /// an empty page: the chrome keys on it to hide the BIM table.
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
