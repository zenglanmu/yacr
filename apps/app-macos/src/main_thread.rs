//! The macOS/Tauri adapter for [`cad_desktop_shell::threading`].
//!
//! The shared crate owns the "run there and wait" protocol; this module owns the
//! only platform-specific answers: which thread is the host's UI thread, and how
//! to dispatch onto it. The main thread is the one that ran Tauri `setup`
//! (recorded once); Tauri's `AppHandle::run_on_main_thread` performs the
//! dispatch. Both the production host and the evidence path use this adapter, so
//! there is no second copy of the channel code.
//!
//! The probe is a real recorded `ThreadId`, never a guess: a mistaken `true`
//! would run a `!Send` CAD-session read on a worker thread.

use std::sync::OnceLock;
use std::thread::ThreadId;

use cad_desktop_shell::threading::{self, MainThread};

/// The recorded UI thread, set once during Tauri `setup`.
static MAIN_THREAD: OnceLock<ThreadId> = OnceLock::new();

/// Record the calling thread as the host UI thread (idempotent).
///
/// Called from `host::init`, which Tauri runs on the main thread before any IPC
/// command can arrive.
pub(crate) fn record_main_thread() {
    let _ = MAIN_THREAD.set(std::thread::current().id());
}

/// The Tauri implementation of the shared [`MainThread`] seam.
pub(crate) struct MacosMainThread<'a>(pub(crate) &'a tauri::AppHandle);

impl MainThread for MacosMainThread<'_> {
    fn is_main_thread(&self) -> bool {
        MAIN_THREAD
            .get()
            .is_some_and(|main| *main == std::thread::current().id())
    }

    fn dispatch(&self, task: Box<dyn FnOnce() + Send>) -> Result<(), String> {
        self.0
            .run_on_main_thread(task)
            .map_err(|error| error.to_string())
    }
}

/// Run one task on the host UI thread and wait for its result.
///
/// Runs inline when the caller already is on the UI thread (the normal Tauri
/// command case) and is marshalled otherwise. The CAD session is a
/// main-thread-local `!Send` value, so every CAD-session read must go through
/// this seam.
pub(crate) fn on_main_thread<T, F>(app: &tauri::AppHandle, task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    threading::run_on_main_thread(&MacosMainThread(app), task)
}
