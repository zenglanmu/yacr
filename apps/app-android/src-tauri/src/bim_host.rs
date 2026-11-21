//! The Android host's BIM query seam (`cad_bim_query`, ADR-0042 D11).
//!
//! A BIM query reads the **open document**: the filter, the paging and the
//! counters all come from the `!Send` [`CadSession`] the Tauri/tao loop thread
//! owns, and the filter is caller-supplied, so the answer cannot be mirrored ahead
//! of time the way `cad_layers`/`cad_fonts` are. The shared command layer
//! therefore reaches it through this seam, exactly like the recent-drawings and
//! `.ymodel` seams: IPC thread → loop thread → session.
//!
//! Nothing here decides what a query means: the session's own `bim_query` calls
//! the backend (`cad_query::bim`), and a refusal (an unknown filter value kind,
//! an over-budget filter, a stale document) travels back unchanged. One call
//! answers the page **and** the facet summary of the same filtered set.
//!
//! This carrier is always `native-below`, so the only "no document" state is the
//! window before the native session exists — it is created on the first tick after
//! the page reports its accepted layout rect — and a failed creation. Both answer
//! the stable `unsupported-runtime: bimQuery` marker instead of an empty page that
//! would read as "this document has no BIM data".
//!
//! # Threading
//!
//! Tauri IPC handlers on this carrier usually run on the loop thread, where the
//! session lives; an off-thread caller is marshalled with the same **bounded**
//! wait the popup/host-file seams use
//! (`cad_desktop_shell::threading::run_on_main_thread_timeout`), because the loop
//! also drives rendering and a command must never wedge it. The `Shared` mutex
//! guard is never carried across that hop.
//!
//! The refusal half is target-independent so its contract runs on the development
//! host; only the Tauri marshalling half is Android-specific.
//!
//! [`CadSession`]: cad_ui_core::session::CadSession

use cad_ui_core::bim::BIM_UNSUPPORTED;

/// The answer when this carrier has no native document to read.
///
/// Named, never an empty page: "no BIM lane right now" and "this document has no
/// BIM data" are different facts and the chrome renders them differently.
pub fn no_document_refusal() -> String {
    BIM_UNSUPPORTED.to_string()
}

#[cfg(target_os = "android")]
pub use android_host::AndroidBimQueryHost;

#[cfg(target_os = "android")]
mod android_host {
    use std::sync::Arc;
    use std::time::Duration;

    use cad_desktop_shell::threading::{self, MainThread};
    use cad_ui_core::bim::{BimQueryHost, BimQueryRequest, BimQueryResponse};

    use crate::host::CAD;

    /// How long a marshalled BIM read waits for the loop thread.
    ///
    /// Same bound as the popup/host-file seams: the loop is also driving
    /// rendering, and an IPC reply must not be able to wedge the caller forever.
    /// The task still runs on the loop thread; only the caller stops waiting.
    const MAIN_THREAD_WAIT: Duration = Duration::from_secs(5);

    /// The Android implementation of [`BimQueryHost`].
    pub struct AndroidBimQueryHost {
        app: tauri::AppHandle,
    }

    impl AndroidBimQueryHost {
        /// Bind the seam to the Tauri app handle.
        pub fn new(app: tauri::AppHandle) -> Arc<Self> {
            Arc::new(Self { app })
        }

        fn on_loop_thread<T, F>(&self, task: F) -> Result<T, String>
        where
            T: Send + 'static,
            F: FnOnce() -> Result<T, String> + Send + 'static,
        {
            threading::run_on_main_thread_timeout(
                &AndroidMainThread(&self.app),
                task,
                MAIN_THREAD_WAIT,
            )
        }
    }

    /// The Android/Tauri adapter for the shared [`MainThread`] seam.
    struct AndroidMainThread<'a>(&'a tauri::AppHandle);

    impl MainThread for AndroidMainThread<'_> {
        fn is_main_thread(&self) -> bool {
            // The recorded `ThreadId` of the thread that ran `run()`, i.e. the
            // tao/Tauri loop that owns the `!Send` CAD session. Never a guess.
            crate::host::on_loop_thread()
        }

        fn dispatch(&self, task: Box<dyn FnOnce() + Send>) -> Result<(), String> {
            self.0
                .run_on_main_thread(task)
                .map_err(|error| error.to_string())
        }
    }

    impl BimQueryHost for AndroidBimQueryHost {
        fn bim_query(&self, request: &BimQueryRequest) -> Result<BimQueryResponse, String> {
            let request = request.clone();
            self.on_loop_thread(move || {
                CAD.with(|cad| match cad.borrow().as_ref() {
                    Some(session) => session.bim_query(&request),
                    // Before the session exists (the chrome can ask on the first
                    // mount) or after a failed creation there is no document.
                    None => Err(super::no_document_refusal()),
                })
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::no_document_refusal;

    /// The carrier's "no session yet" answer is the shared marker word for word,
    /// never an empty page. Runs on the development host (the refusal half is not
    /// Android-gated); the marshalling half is pinned by `crate::contract`.
    #[test]
    fn a_carrier_with_no_live_session_refuses_by_name() {
        assert_eq!(no_document_refusal(), cad_ui_core::bim::BIM_UNSUPPORTED);
        assert_eq!(no_document_refusal(), "unsupported-runtime: bimQuery");
    }
}
