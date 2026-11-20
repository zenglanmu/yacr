//! The Linux host's ymodel-resource seam (`cad_ymodel_resource_preview`).
//!
//! What a `.ymodel` save would embed (ADR-0040 D17) is a read of the **open
//! document** plus a filesystem probe of the fonts/images/xrefs it references, so
//! it must run on the `!Send` [`CadSession`] the GTK main thread owns. The shared
//! command layer therefore reaches it through this seam, exactly like the
//! recovery-snapshot seam: IPC thread → [`crate::main_thread`] → session.
//!
//! Nothing here writes: the preview is `HostCore`'s read-only resolution over the
//! document the session already holds. A carrier/backend with no native session
//! (the X11 `dom-canvas` path) refuses with the stable `unsupported-runtime:
//! ymodel` marker instead of an empty preview that would read as "nothing to
//! embed".
//!
//! [`CadSession`]: cad_ui_core::session::CadSession

use std::sync::Arc;

use cad_desktop_shell::state::ShellYmodelHost;
use cad_ui_core::session::{CadYmodelResources, UNSUPPORTED_YMODEL};

use super::CAD;

/// The Linux implementation of [`ShellYmodelHost`].
pub(super) struct AppYmodelHost {
    app: tauri::AppHandle,
}

impl AppYmodelHost {
    pub(super) fn new(app: tauri::AppHandle) -> Arc<Self> {
        Arc::new(Self { app })
    }
}

impl ShellYmodelHost for AppYmodelHost {
    fn preview(&self) -> Result<CadYmodelResources, String> {
        let app = self.app.clone();
        crate::main_thread::on_main_thread(&app, preview_on_main_thread)
    }
}

/// The real preview, read on the CAD session's own thread.
fn preview_on_main_thread() -> Result<CadYmodelResources, String> {
    if crate::backend::surface_mode() == crate::backend::SurfaceMode::DomCanvas {
        // No native session by design on this carrier: the honest answer is the
        // named refusal, never a preview the carrier cannot save.
        return Err(UNSUPPORTED_YMODEL.to_string());
    }
    match CAD.with(|cad| {
        cad.borrow()
            .as_ref()
            .map(|session| session.ymodel_resource_preview())
    }) {
        Some(answer) => answer,
        // Before the native session exists (it is created on the first tick after
        // the page reports its canvas rect) there is no document to describe.
        None => Err(UNSUPPORTED_YMODEL.to_string()),
    }
}
