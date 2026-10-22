//! The Linux host's popup seam (ADR-0021 D1/D2, WP3).
//!
//! Popups are **in-DOM overlays** in the single React tree (WP2's DOM-on-top
//! arrangement), so this host owns no popup window: there is no second
//! `tauri::Window`, no child webview and no injected boot literal. What remains
//! here is only what needs the GTK main thread:
//!
//! - the one `!Send` CAD-session read that the derived `textEdit` payload needs
//!   (the assembly rules themselves live in [`cad_ui_core::popup`]);
//! - the WebKitGTK renderer-crash signal wiring (platform-specific by nature);
//! - evidence-only accessors that call the **same** shared open/close path.
//!
//! Main-thread marshalling is not implemented here: the shared protocol lives in
//! [`cad_desktop_shell::threading`] and the GTK/Tauri adapter in
//! [`crate::main_thread`], so a second desktop host does not copy either.
//!
//! The lifecycle (the modal gate plus its four clear triggers) lives in
//! [`cad_desktop_shell::popup`], and the allowlist/wire shapes live in
//! [`cad_ui_core::popup`]. A caller can never supply a label, URL or script.

use std::sync::{Arc, Mutex};

use cad_desktop_shell::popup as lifecycle;
use cad_ui_core::popup::{PopupOps, TextEditPayload};

use super::Shared;

pub use cad_ui_core::popup::PopupAck;

/// Open the allowlisted popup (or report the one already open).
///
/// The transitions live in [`cad_ui_core::popup::open_popup`] via the shared
/// host lifecycle; this glue only supplies the payload derivation.
pub(super) fn open_popup(
    shared: &Arc<Mutex<Shared>>,
    requested: &str,
    ops: &dyn PopupOps,
) -> Result<PopupAck, String> {
    lifecycle::open(shared, requested, ops)
}

/// Close the open popup (idempotent) and clear the modal gate.
pub(super) fn close_popup(shared: &Arc<Mutex<Shared>>) -> Result<(), String> {
    lifecycle::close(shared)
}

/// Evidence-only accessor: the **same** open path `cad_open_popup` runs.
///
/// Deliberately not the `#[tauri::command]` wrapper: the driver calls the
/// host-side function with the same [`HostPopupOps`] the command builds, so the
/// state machine and the gate are the production ones.
pub(crate) fn open_popup_for_probe(app: &tauri::AppHandle, kind: &str) -> Result<PopupAck, String> {
    use tauri::Manager;
    let shared = app.state::<super::ProbeShared>().0.clone();
    open_popup(&shared, kind, &HostPopupOps)
}

/// Evidence-only accessor: the **same** close path `cad_close_popup` runs.
pub(crate) fn close_popup_for_probe(app: &tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    let shared = app.state::<super::ProbeShared>().0.clone();
    close_popup(&shared)
}

/// Evidence-only accessor: the **same** `textEdit` payload derivation the open
/// path runs (ADR-0021 D4).
///
/// It is the identical [`PopupOps::text_edit_payload`] call, on the same
/// main-thread session state, and it changes nothing. Used by the
/// `YACR_PROBE_POPUP_EVIDENCE` text-edit phase to report the derived payload
/// length/fingerprint, and to prove the acknowledgement payload matches the
/// derivation. Must be called on the GTK main thread, like every other
/// CAD-session read.
pub(crate) fn text_edit_payload_for_probe() -> Result<TextEditPayload, String> {
    HostPopupOps.text_edit_payload()
}

/// Trigger ③ wiring: clear the modal gate when the webview renderer dies.
///
/// `web-process-crashed` is a WebKitGTK signal on the `WebView` object; it is
/// reached through Tauri's `with_webview` (the platform object), not through a
/// Tauri event, because Tauri exposes no renderer-crash event. The hook clears
/// the gate **and** records a diagnostic through
/// [`cad_desktop_shell::popup::on_renderer_crash`], so the next `cad_status`
/// reports the failure instead of silently releasing CAD input.
///
/// A hook that cannot be installed is logged and the host continues: the other
/// three triggers still clear the gate, and claiming otherwise would be a lie.
pub(super) fn install_renderer_crash_hook(webview: &tauri::Webview, shared: &Arc<Mutex<Shared>>) {
    use gtk::prelude::*;

    let shared = shared.clone();
    let result = webview.with_webview(move |platform| {
        let webview = platform.inner();
        webview.connect_local("web-process-crashed", false, move |values| {
            // `(WebKitWebView*, WebKitWebProcessTerminationReason)`; the
            // reason is an enum value, so read it defensively.
            let detail = values
                .get(1)
                .map(|value| {
                    value
                        .get::<String>()
                        .map(|text| text.to_string())
                        .or_else(|_| value.get::<i32>().map(|reason| format!("reason={reason}")))
                        .unwrap_or_else(|_| format!("{value:?}"))
                })
                .unwrap_or_else(|| "unknown web process termination".to_string());
            eprintln!("[probe] webview web-process-crashed: {detail}");
            cad_desktop_shell::popup::on_renderer_crash(&shared, &detail);
            None
        });
    });
    if let Err(error) = result {
        eprintln!("[probe] webview renderer-crash hook FAILED: {error}");
    }
}

/// The Linux implementation of [`PopupOps`]: one main-thread CAD-session read.
pub(super) struct HostPopupOps;
impl PopupOps for HostPopupOps {
    fn text_edit_payload(&self) -> Result<TextEditPayload, String> {
        // `open_popup` marshals the whole open onto the GTK main thread, where
        // the CAD session (main-thread-local) holds the authoritative selection.
        // This is the only platform part; the refusal and the one ADR-permitted
        // `\P` mapping are shared in `cad_ui_core::popup`.
        let selected = super::CAD.with(|cad| {
            let cad = cad.borrow();
            cad.as_ref()
                .and_then(|session| session.selected_text_edit().ok().flatten())
        });
        TextEditPayload::from_selection(selected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_desktop_shell::popup::RENDERER_CRASH_ERROR;
    use cad_ui_core::popup::{PopupReset, POPUP_LABEL};
    use cad_ui_core::session::CadPopup;
    use cad_ui_core::shell::{modal_gate_open, projected_popup};
    use cad_ui_core::unsupported_status;

    #[test]
    fn the_status_projection_keeps_the_frozen_popup_block() {
        #[allow(deprecated)] // the frozen `cad_status` wire label
        let popup = CadPopup {
            kind: "about".into(),
            label: POPUP_LABEL.into(),
            modal: true,
        };
        let status = projected_popup(unsupported_status(None), Some(&popup));
        let wire = serde_json::to_value(&status).expect("status serializes");
        assert_eq!(
            wire.get("popup")
                .and_then(|block| block.get("label"))
                .and_then(|label| label.as_str()),
            Some(POPUP_LABEL)
        );
        // After the popup is gone the block is `null`, never a stale popup.
        let cleared = projected_popup(status, None);
        let wire = serde_json::to_value(&cleared).expect("status serializes");
        assert_eq!(wire.get("popup"), Some(&serde_json::Value::Null));
    }

    #[test]
    fn the_shared_lifecycle_owns_the_four_triggers() {
        // The app module no longer implements a gate of its own: it maps onto the
        // shared lifecycle, whose triggers are contract-tested in
        // `cad_desktop_shell::popup`. This test pins that plumbing.
        use cad_ui_core::popup::TextEditPayload;

        struct Ops;
        impl PopupOps for Ops {
            fn text_edit_payload(&self) -> Result<TextEditPayload, String> {
                Err("popup.no_text_selection: test".into())
            }
        }

        let shared = super::super::new_shared(cad_ui_core::launch::LaunchOptions::default());
        assert!(close_popup(&shared).is_ok(), "close is idempotent");
        assert!(open_popup(&shared, "unknown", &Ops).is_err());
        assert!(open_popup(&shared, "textEdit", &Ops).is_err());
        assert!(open_popup(&shared, "about", &Ops).is_ok());
        assert!(modal_gate_open(
            &cad_desktop_shell::state::lock(&shared).shell
        ));

        let open = cad_desktop_shell::state::lock(&shared).shell.popup.clone();
        assert_eq!(open.map(|popup| popup.kind), Some("about".to_string()));
        lifecycle::on_reload(&shared);
        assert!(!modal_gate_open(
            &cad_desktop_shell::state::lock(&shared).shell
        ));
        lifecycle::on_renderer_crash(&shared, "test");
        lifecycle::on_process_exit(&shared);
        assert_eq!(PopupReset::RendererCrash.key(), "renderer_crash");
        assert!(RENDERER_CRASH_ERROR.contains("crashed"));
    }
}
