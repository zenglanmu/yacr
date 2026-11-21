//! The Android host's request-carrying read seam (`cad_bim_query`, ADR-0042 D11).
//!
//! The request kind, its refusals and the seam shape are shared
//! ([`cad_ui_core::host_read`]); this carrier only injects its hop. The `!Send`
//! [`CadSession`] lives on the Tauri/tao loop thread, and an off-thread caller is
//! marshalled with the same **bounded** wait the popup/host-file seams use
//! (`cad_desktop_shell::threading::run_on_main_thread_timeout`): the loop also
//! drives rendering, so a command must never wedge it. The `Shared` mutex guard is
//! never carried across that hop.
//!
//! The refusal half is target-independent so its contract runs on the development
//! host; only the Tauri marshalling half is Android-specific.
//!
//! [`CadSession`]: cad_ui_core::session::CadSession

use cad_ui_core::bim::BIM_UNSUPPORTED;

/// The answer when this carrier has no native document to read.
///
/// Named, never an empty page: "no read lane right now" and "this document has no
/// BIM data" are different facts and the chrome renders them differently.
pub fn no_document_refusal() -> String {
    BIM_UNSUPPORTED.to_string()
}

#[cfg(target_os = "android")]
pub use android_host::host_read_host;

#[cfg(target_os = "android")]
mod android_host {
    use std::{sync::Arc, time::Duration};

    use cad_desktop_shell::state::HostReadFn;
    use cad_desktop_shell::threading::{self, MainThread};

    use crate::host::CAD;

    /// Same bound as the popup/host-file seams: the task still runs on the loop
    /// thread; only the caller stops waiting.
    const MAIN_THREAD_WAIT: Duration = Duration::from_secs(5);

    /// The Android/Tauri adapter for the shared [`MainThread`] seam.
    struct AndroidMainThread<'a>(&'a tauri::AppHandle);

    impl MainThread for AndroidMainThread<'_> {
        fn is_main_thread(&self) -> bool {
            // The recorded thread that ran `run()`, i.e. the tao/Tauri loop that
            // owns the `!Send` CAD session. Never a guess.
            crate::host::on_loop_thread()
        }

        fn dispatch(&self, task: Box<dyn FnOnce() + Send>) -> Result<(), String> {
            self.0
                .run_on_main_thread(task)
                .map_err(|error| error.to_string())
        }
    }

    /// One host-read closure bound to the app handle (installed once at startup).
    pub fn host_read_host(app: tauri::AppHandle) -> Arc<HostReadFn> {
        HostReadFn::new(move |request| {
            let request = request.clone();
            threading::run_on_main_thread_timeout(
                &AndroidMainThread(&app),
                move || {
                    CAD.with(|cad| match cad.borrow().as_ref() {
                        Some(session) => session.host_read(&request),
                        // Before the session exists (the chrome can ask on the
                        // first mount) or after a failed creation: no document.
                        None => Err(super::no_document_refusal()),
                    })
                },
                MAIN_THREAD_WAIT,
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::no_document_refusal;

    /// The carrier's "no session yet" answer is the shared marker word for word,
    /// never an empty page. Runs on the development host (the refusal half is not
    /// Android-gated); the marshalling half is pinned by `crate::contract`.
    #[test]
    fn a_carrier_with_no_live_session_refuses_bim_by_name() {
        assert_eq!(no_document_refusal(), cad_ui_core::bim::BIM_UNSUPPORTED);
        assert_eq!(no_document_refusal(), "unsupported-runtime: bimQuery");
    }
}
