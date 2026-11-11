//! The macOS host's recent-drawings seam.
//!
//! The durable list lives on the `!Send` `CadSession` (host core) owned by the
//! Tauri/tao main thread, so the shared command layer reaches it only through
//! this seam. Both calls are marshalled with [`crate::main_thread`] and answer
//! the session's own list/refusal; nothing is fabricated here.

use std::path::PathBuf;
use std::sync::Arc;

use cad_desktop_shell::state::ShellRecentHost;
use cad_host::{recent_files_from_preference, RecentEntry};
use cad_ui_core::session::{CadRecentFiles, OPEN_RECENT_COMMAND};

use super::CAD;

/// The macOS implementation of [`ShellRecentHost`].
pub(crate) struct AppRecentHost {
    app: tauri::AppHandle,
    /// The `preference.json` the host loads (the real durable list before the
    /// session exists).
    preference: Option<PathBuf>,
}

impl AppRecentHost {
    pub(crate) fn new(app: tauri::AppHandle) -> Arc<Self> {
        let (_, preference) = crate::platform::config_paths();
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

fn list_on_main_thread(preference: Option<&PathBuf>) -> CadRecentFiles {
    if let Some(list) = CAD.with(|cad| cad.borrow().as_ref().map(|session| session.recent_files()))
    {
        return list;
    }
    // The session is created on the first tick after the page reports its rect;
    // before that, the durable rows are the real answer (not a refusal, and not
    // an invented row).
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

fn open_on_main_thread(opaque: &str, display_name: &str) -> Result<(), String> {
    CAD.with(|cad| {
        cad.borrow()
            .as_ref()
            .map(|session| session.open_recent(opaque, display_name))
    })
    .unwrap_or_else(|| Err(format!("unsupported-runtime: {OPEN_RECENT_COMMAND}")))
}
