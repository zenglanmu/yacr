//! The Android popup seam (ADR-0021 D1/D2): in-DOM overlays, no native window.
//!
//! Popups are overlays in the single React tree, so this host owns no window and
//! no second webview. The only platform work is the `textEdit` payload: it is
//! derived from the live native `CadSession`, which is `!Send` and lives in a
//! thread-local of the Tauri loop thread (the thread the tick drives). The
//! lifecycle itself (the modal gate plus its four clear triggers) is shared in
//! [`cad_desktop_shell::popup`] and the allowlist/wire shapes live in
//! [`cad_ui_core::popup`]; a caller can never supply a label, URL or script.
//!
//! # Threading
//!
//! Tauri IPC handlers on this carrier run on the loop thread, so the normal path
//! runs inline. If a caller is on another thread, [`AndroidPopupHost`] marshals
//! the whole open/close onto the loop thread through
//! [`cad_desktop_shell::threading`] with a **bounded** wait (the loop is also
//! driving rendering, so a popup command must never hang forever). The `Shared`
//! mutex guard is never held across that hop — only an `Arc` clone is moved.
//!
//! [`AndroidPopupOps::text_edit_payload`] refuses explicitly when it is invoked
//! off the loop thread instead of reading a thread-local that belongs to another
//! thread and returning a fabricated payload.

use cad_ui_core::popup::{PopupOps, TextEditPayload};

/// Stable refusal when the `textEdit` payload is requested off the owner thread.
///
/// Technical English only: a host/transport caption the chrome maps to catalog
/// copy. It is never a payload with empty text.
pub const OFF_OWNER_THREAD: &str =
    "popup.main_thread_required: the CAD selection is only readable on the Tauri loop thread";

/// The Android implementation of [`PopupOps`]: one owner-thread CAD-session read.
///
/// Compiled on every non-wasm target so the refusal contract runs on the
/// development host; only the selection source is Android-specific.
pub struct AndroidPopupOps;

impl PopupOps for AndroidPopupOps {
    fn text_edit_payload(&self) -> Result<TextEditPayload, String> {
        TextEditPayload::from_selection(selected_text_edit()?)
    }
}

/// The live selection, read on the session's owner thread.
///
/// The refusal and the one ADR-permitted `\P` mapping stay shared in
/// `cad_ui_core::popup`; this is only the platform read.
#[cfg(target_os = "android")]
fn selected_text_edit() -> Result<Option<(String, bool)>, String> {
    if !crate::host::on_loop_thread() {
        // Another thread's thread-local has no CAD session; reading it would
        // report "nothing selected" for the wrong reason.
        return Err(OFF_OWNER_THREAD.to_string());
    }
    Ok(crate::host::CAD.with(|cad| {
        let cad = cad.borrow();
        cad.as_ref()
            .and_then(|session| session.selected_text_edit().ok().flatten())
    }))
}

/// No `CadSession` exists when this crate is built for a non-Android target (it
/// is a modelled shell there), so the honest answer is "nothing selected".
///
/// The shared assembler turns that into [`cad_ui_core::popup::NO_TEXT_SELECTION`]
/// — the same refusal a live session with no editable selection produces. This is
/// never a fabricated payload and it is what the host-target test asserts.
#[cfg(not(target_os = "android"))]
fn selected_text_edit() -> Result<Option<(String, bool)>, String> {
    Ok(None)
}

#[cfg(target_os = "android")]
pub use android_host::AndroidPopupHost;

#[cfg(target_os = "android")]
mod android_host {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use cad_desktop_shell::state::{Shared, ShellPopupHost};
    use cad_desktop_shell::threading::{self, MainThread};
    use cad_ui_core::popup::PopupAck;

    use super::AndroidPopupOps;

    /// How long a marshalled popup command waits for the loop thread.
    ///
    /// Bounded on purpose: the loop thread is also driving the renderer, and an
    /// IPC reply must not be able to wedge the caller indefinitely. The task
    /// still runs on the loop thread; only the caller stops waiting.
    pub const MAIN_THREAD_WAIT: Duration = Duration::from_secs(5);

    /// The platform popup seam for the shared `cad_open_popup`/`cad_close_popup`
    /// commands.
    ///
    /// It holds only the app handle (no `Shared` reference, so no cycle) and
    /// never locks the shell state itself: the shared command already cloned the
    /// seam out of `Shared` and released the lock before calling.
    pub struct AndroidPopupHost {
        app: tauri::AppHandle,
    }

    impl AndroidPopupHost {
        /// Bind the seam to the Tauri app handle.
        pub fn new(app: tauri::AppHandle) -> Self {
            Self { app }
        }

        /// Run `task` on the Tauri loop thread and wait at most
        /// [`MAIN_THREAD_WAIT`].
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
            // tao/Tauri loop that owns the `!Send` CAD session. Never a guess: a
            // mistaken `true` would read another thread's thread-local.
            crate::host::on_loop_thread()
        }

        fn dispatch(&self, task: Box<dyn FnOnce() + Send>) -> Result<(), String> {
            self.0
                .run_on_main_thread(task)
                .map_err(|error| error.to_string())
        }
    }

    impl ShellPopupHost for AndroidPopupHost {
        fn open(&self, shared: &Arc<Mutex<Shared>>, kind: &str) -> Result<PopupAck, String> {
            // Move an `Arc` clone, never a locked guard: the marshalling hop must
            // not carry the shell mutex.
            let shared = shared.clone();
            let kind = kind.to_string();
            self.on_loop_thread(move || {
                cad_desktop_shell::popup::open(&shared, &kind, &AndroidPopupOps)
            })
        }

        fn close(&self, shared: &Arc<Mutex<Shared>>) -> Result<(), String> {
            let shared = shared.clone();
            self.on_loop_thread(move || cad_desktop_shell::popup::close(&shared))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_session_is_the_shared_no_text_selection_refusal() {
        // On the development host there is no CAD session, which is the same
        // answer a live session with nothing editable selected gives: the shared
        // `NO_TEXT_SELECTION` string, never a payload with empty text.
        let error = AndroidPopupOps
            .text_edit_payload()
            .expect_err("no selection must be refused");
        assert_eq!(error, cad_ui_core::popup::NO_TEXT_SELECTION);
        assert!(error.starts_with("popup.no_text_selection"), "{error}");
    }

    #[test]
    fn the_off_thread_refusal_is_a_named_contract() {
        assert!(OFF_OWNER_THREAD.starts_with("popup.main_thread_required:"));
        assert!(OFF_OWNER_THREAD.contains("loop thread"));
    }
}
