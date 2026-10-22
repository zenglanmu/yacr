//! The Linux GTK/Tauri adapter for [`cad_desktop_shell::threading`].
//!
//! The shared crate owns the "run there and wait" protocol; this module owns the
//! only platform-specific answers: GTK decides whether the caller is already on
//! the UI thread, and Tauri's `AppHandle::run_on_main_thread` performs the
//! dispatch. Both the production host and the evidence drivers use this same
//! adapter, so there is no second copy of the channel code.

use cad_desktop_shell::threading::{self, MainThread};

/// The GTK/Tauri implementation of the shared [`MainThread`] seam.
pub(crate) struct LinuxMainThread<'a>(pub(crate) &'a tauri::AppHandle);

impl MainThread for LinuxMainThread<'_> {
    fn is_main_thread(&self) -> bool {
        gtk::is_initialized_main_thread()
    }

    fn dispatch(&self, task: Box<dyn FnOnce() + Send>) -> Result<(), String> {
        self.0
            .run_on_main_thread(task)
            .map_err(|error| error.to_string())
    }
}

/// Run one task on the GTK main thread and wait for its result.
///
/// Runs inline when the caller already is on the GTK main thread (the normal
/// Tauri command case) and is marshalled otherwise. The CAD session is a
/// main-thread-local `!Send` value, so every CAD-session read must go through
/// this seam.
pub(crate) fn on_main_thread<T, F>(app: &tauri::AppHandle, task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    threading::run_on_main_thread(&LinuxMainThread(app), task)
}
