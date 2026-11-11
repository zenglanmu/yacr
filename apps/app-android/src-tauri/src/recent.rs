//! The Android host's recent-drawings seam.
//!
//! The durable list lives on the `!Send` `CadSession` (host core) owned by the
//! Tauri/tao loop thread, so the shared command layer reaches it only through
//! this seam. IPC handlers normally already run on that thread; an off-thread
//! caller is marshalled with a **bounded** wait (the loop also drives rendering,
//! so a command must never hang forever).
//!
//! The list itself is the same host-owned array every carrier uses; Android's
//! rows have `location: null` (a SAF URI has no human directory hint) and their
//! `opaque` handle is the persisted `content://` URI, re-granted by
//! `begin_open_target`.

use std::sync::Arc;
use std::time::Duration;

use cad_desktop_shell::state::ShellRecentHost;
use cad_desktop_shell::threading::{self, MainThread};
use cad_ui_core::session::{CadRecentFiles, OPEN_RECENT_COMMAND};

use crate::host::CAD;

/// How long a marshalled recent command waits for the loop thread.
const MAIN_THREAD_WAIT: Duration = Duration::from_secs(5);

/// The Android implementation of [`ShellRecentHost`].
pub struct AndroidRecentHost {
    app: tauri::AppHandle,
}

impl AndroidRecentHost {
    pub fn new(app: tauri::AppHandle) -> Arc<Self> {
        Arc::new(Self { app })
    }

    fn on_loop_thread<T, F>(&self, task: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, String> + Send + 'static,
    {
        threading::run_on_main_thread_timeout(&AndroidMainThread(&self.app), task, MAIN_THREAD_WAIT)
    }
}

struct AndroidMainThread<'a>(&'a tauri::AppHandle);

impl MainThread for AndroidMainThread<'_> {
    fn is_main_thread(&self) -> bool {
        // The recorded thread id of the tao/Tauri loop that owns the session;
        // never a guess (a wrong `true` would read another thread's thread-local).
        crate::host::on_loop_thread()
    }

    fn dispatch(&self, task: Box<dyn FnOnce() + Send>) -> Result<(), String> {
        self.0
            .run_on_main_thread(task)
            .map_err(|error| error.to_string())
    }
}

impl ShellRecentHost for AndroidRecentHost {
    fn list(&self) -> Result<CadRecentFiles, String> {
        self.on_loop_thread(list_on_owner_thread)
    }

    fn open_recent(&self, opaque: &str, display_name: &str) -> Result<(), String> {
        let opaque = opaque.to_string();
        let display_name = display_name.to_string();
        self.on_loop_thread(move || open_on_owner_thread(&opaque, &display_name))
    }
}

fn list_on_owner_thread() -> Result<CadRecentFiles, String> {
    // The native session is created on the first tick after the page reports its
    // rect. Before that there is no list to read; the carrier still supports
    // recents, so an empty supported list is the honest answer (never an
    // `unsupported` one, which would hide the feature for the whole mount).
    Ok(CAD.with(|cad| match cad.borrow().as_ref() {
        Some(session) => session.recent_files(),
        None => CadRecentFiles::supported(Vec::new()),
    }))
}

fn open_on_owner_thread(opaque: &str, display_name: &str) -> Result<(), String> {
    CAD.with(|cad| match cad.borrow().as_ref() {
        Some(session) => Some(session.open_recent(opaque, display_name)),
        None => None,
    })
    .unwrap_or_else(|| Err(format!("unsupported-runtime: {OPEN_RECENT_COMMAND}")))
}
