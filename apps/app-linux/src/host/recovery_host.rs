//! The Linux host's recovery-snapshot seam.
//!
//! Recovery snapshots are a host file capability (ADR-0037 D12), but the store
//! and the open funnel live on the `!Send` [`CadSession`] owned by the GTK main
//! thread. The shared command layer therefore cannot read them directly; this
//! seam marshals both calls through [`crate::main_thread`] and answers with the
//! session's own list/refusal.
//!
//! Before the native session exists (it is created on the first tick after the
//! page reports its canvas rect) the list is read from the **same durable
//! directory** the session will use, so the start page shows the real snapshots
//! instead of an invented empty or unsupported answer. The open needs the
//! session (it runs the ordinary open funnel), so it refuses by name until then.
//! On the `dom-canvas` carrier there is no native session by design; that path
//! refuses by name too (the chrome routes it to the wasm adapter, which refuses).
//!
//! The UI only ever sends a snapshot **key**; this module resolves the path from
//! the store on the main thread, so no filesystem path crosses the IPC boundary.
//!
//! [`CadSession`]: cad_ui_core::session::CadSession

use std::sync::Arc;

use cad_desktop_shell::state::ShellRecoveryHost;
use cad_domain::CadError;
use cad_platform::Persistence;
use cad_ui_core::session::{
    CadRecoveryFiles, RecoveryEntry, OPEN_RECOVERY_COMMAND, RECOVERY_FILES_COMMAND,
};

use super::CAD;

/// The Linux implementation of [`ShellRecoveryHost`].
pub(super) struct AppRecoveryHost {
    app: tauri::AppHandle,
}

impl AppRecoveryHost {
    pub(super) fn new(app: tauri::AppHandle) -> Arc<Self> {
        Arc::new(Self { app })
    }
}

impl ShellRecoveryHost for AppRecoveryHost {
    fn list(&self) -> Result<CadRecoveryFiles, String> {
        let app = self.app.clone();
        crate::main_thread::on_main_thread(&app, list_on_main_thread)
    }

    fn open_recovery(&self, key: &str) -> Result<(), String> {
        let app = self.app.clone();
        let key = key.to_string();
        crate::main_thread::on_main_thread(&app, move || open_on_main_thread(&key))
    }
}

/// The real list, read on the CAD session's own thread.
fn list_on_main_thread() -> Result<CadRecoveryFiles, String> {
    if crate::backend::surface_mode() == crate::backend::SurfaceMode::DomCanvas {
        // No native session by design on this carrier: the honest answer is the
        // named refusal, never a list the carrier cannot open.
        return Ok(CadRecoveryFiles::unsupported(RECOVERY_FILES_COMMAND));
    }
    if let Some(list) = CAD.with(|cad| {
        cad.borrow()
            .as_ref()
            .map(|session| session.recovery_files())
    }) {
        return list;
    }
    durable_entries()
}

/// The durable snapshots, read from the same directory the session's store uses
/// (so the list is real before the session exists).
fn durable_entries() -> Result<CadRecoveryFiles, String> {
    let Some(dir) = super::options::recovery_dir() else {
        // No durable private directory: this carrier cannot offer snapshots.
        return Ok(CadRecoveryFiles::unsupported(RECOVERY_FILES_COMMAND));
    };
    let store = crate::recovery_store::FsRecoveryStore::new(dir);
    let records = store.list_recovery().map_err(|error| reason(&error))?;
    Ok(CadRecoveryFiles::supported(
        records.into_iter().map(RecoveryEntry::from).collect(),
    ))
}

/// Start one snapshot open on the CAD session's own thread.
///
/// Without a session there is no open funnel to run, so the answer is the named
/// refusal; the session validates the key and resolves the path.
fn open_on_main_thread(key: &str) -> Result<(), String> {
    CAD.with(|cad| {
        cad.borrow()
            .as_ref()
            .map(|session| session.open_recovery(key))
    })
    .unwrap_or_else(|| Err(format!("unsupported-runtime: {OPEN_RECOVERY_COMMAND}")))
}

/// The store's own reason as a technical string (the same rendering the session
/// uses: `CadError`'s `Display` would print the Debug wrapper).
fn reason(error: &CadError) -> String {
    match error {
        CadError::Unsupported(reason) => reason.clone(),
        CadError::NotImplemented(feature) => feature.to_string(),
        other => other.to_string(),
    }
}
