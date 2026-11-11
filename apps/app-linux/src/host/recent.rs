//! The Linux host's recent-drawings seam.
//!
//! Recent drawings are a host file capability, but the durable list lives on the
//! `!Send` [`CadSession`] (the host core) owned by the GTK main thread. The
//! shared command layer therefore cannot read it directly; this seam marshals
//! the two calls through [`crate::main_thread`] and answers with the session's
//! own list/refusal (never a fabricated row).
//!
//! Before the native session exists (it is created on the first tick after the
//! page reports its canvas rect) the answer is the **durable** list read from the
//! same `preference.json` the host will load: the carrier does track recents, so
//! an `unsupported` answer would be wrong, and the durable rows are real. On the
//! `dom-canvas` carrier there is no native session by design; that path refuses
//! by name (the chrome routes it to the wasm adapter, which refuses too).
//!
//! [`CadSession`]: cad_ui_core::session::CadSession

use std::path::PathBuf;
use std::sync::Arc;

use cad_desktop_shell::state::ShellRecentHost;
use cad_host::{recent_files_from_preference, RecentEntry};
use cad_ui_core::session::{CadRecentFiles, OPEN_RECENT_COMMAND, RECENT_FILES_COMMAND};

use super::CAD;

/// The Linux implementation of [`ShellRecentHost`].
pub(super) struct AppRecentHost {
    app: tauri::AppHandle,
    /// The `preference.json` the host loads (so a pre-session read is the real
    /// durable list, not an invented one).
    preference: Option<PathBuf>,
}

impl AppRecentHost {
    pub(super) fn new(app: tauri::AppHandle) -> Arc<Self> {
        let (_, preference) = super::options::config_paths();
        Arc::new(Self { app, preference })
    }
}

impl ShellRecentHost for AppRecentHost {
    fn list(&self) -> Result<CadRecentFiles, String> {
        let app = self.app.clone();
        let preference = self.preference.clone();
        crate::main_thread::on_main_thread(&app, move || {
            Ok(list_on_main_thread(preference.as_ref()))
        })
    }

    fn open_recent(&self, opaque: &str, display_name: &str) -> Result<(), String> {
        let app = self.app.clone();
        let opaque = opaque.to_string();
        let display_name = display_name.to_string();
        crate::main_thread::on_main_thread(&app, move || {
            open_on_main_thread(&opaque, &display_name)
        })
    }
}

/// The real list, read on the CAD session's own thread.
fn list_on_main_thread(preference: Option<&PathBuf>) -> CadRecentFiles {
    if crate::backend::surface_mode() == crate::backend::SurfaceMode::DomCanvas {
        // No native session by design on this carrier: the honest answer is the
        // named refusal, never a supported list the carrier cannot reopen.
        return CadRecentFiles::unsupported(RECENT_FILES_COMMAND);
    }
    if let Some(list) = CAD.with(|cad| cad.borrow().as_ref().map(|session| session.recent_files()))
    {
        return list;
    }
    CadRecentFiles::supported(durable_entries(preference))
}

/// The durable rows, exactly as the host will load them (empty on error).
fn durable_entries(preference: Option<&PathBuf>) -> Vec<RecentEntry> {
    let Some(path) = preference else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    recent_files_from_preference(&text)
}

/// Start one reopen on the CAD session's own thread.
fn open_on_main_thread(opaque: &str, display_name: &str) -> Result<(), String> {
    CAD.with(|cad| {
        cad.borrow()
            .as_ref()
            .map(|session| session.open_recent(opaque, display_name))
    })
    .unwrap_or_else(|| Err(format!("unsupported-runtime: {OPEN_RECENT_COMMAND}")))
}
