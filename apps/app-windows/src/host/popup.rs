//! The Windows host's popup seam (ADR-0021 D1/D2, WP3).
//!
//! Popups are **in-DOM overlays** in the single React tree (the DOM-on-top
//! arrangement), so this host owns no popup window: there is no second
//! `tauri::Window` and no second webview. What remains here is only what needs
//! the host main thread:
//!
//! - the one `!Send` CAD-session read that the derived `textEdit` payload needs
//!   (the assembly rules themselves live in [`cad_ui_core::popup`]);
//! - the WebView2 `ProcessFailed` wiring (platform-specific by nature).
//!
//! Main-thread marshalling is not implemented here: the shared protocol lives in
//! [`cad_desktop_shell::threading`] and the Tauri adapter in
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

/// Trigger ③ wiring: clear the modal gate when the WebView2 renderer dies.
///
/// WebView2 exposes a `ProcessFailed` event on `ICoreWebView2`; it is reached
/// through Tauri's `with_webview` (the platform object), because Tauri exposes no
/// renderer-crash event. The hook clears the gate **and** records a diagnostic
/// through [`cad_desktop_shell::popup::on_renderer_crash`], so the next
/// `cad_status` reports the failure instead of silently releasing CAD input.
///
/// A hook that cannot be installed is logged and the host continues: the other
/// three triggers still clear the gate, and claiming otherwise would be a lie.
pub(super) fn install_renderer_crash_hook(webview: &tauri::Webview, shared: &Arc<Mutex<Shared>>) {
    let shared = shared.clone();
    let result = webview.with_webview(move |platform| {
        let core = match unsafe { platform.controller().CoreWebView2() } {
            Ok(core) => core,
            Err(error) => {
                eprintln!("[probe] webview2 ProcessFailed hook FAILED (no core webview): {error}");
                return;
            }
        };
        // The handler is a COM callback; WebView2 keeps its own reference when
        // `add_ProcessFailed` succeeds, so the local value may be dropped after
        // registration. `token` is only diagnostic (we never remove the hook).
        let handler =
            webview2_com::ProcessFailedEventHandler::create(Box::new(move |_sender, args| {
                let detail = match args {
                    Some(args) => {
                        let mut kind = webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                        match unsafe { args.ProcessFailedKind(&mut kind) } {
                            Ok(()) => format!("kind={kind:?}"),
                            Err(error) => format!("kind unreadable: {error}"),
                        }
                    }
                    None => "kind=unknown".to_string(),
                };
                eprintln!("[probe] webview2 ProcessFailed: {detail}");
                cad_desktop_shell::popup::on_renderer_crash(&shared, &detail);
                Ok(())
            }));
        let mut token = 0i64;
        match unsafe { core.add_ProcessFailed(&handler, &mut token) } {
            Ok(()) => eprintln!("[probe] webview2 ProcessFailed hook installed (token={token})"),
            Err(error) => eprintln!("[probe] webview2 ProcessFailed hook FAILED: {error}"),
        }
    });
    if let Err(error) = result {
        eprintln!("[probe] webview2 with_webview FAILED (ProcessFailed hook): {error}");
    }
}

/// The Windows implementation of [`PopupOps`]: one main-thread CAD-session read.
pub(super) struct HostPopupOps;
impl PopupOps for HostPopupOps {
    fn text_edit_payload(&self) -> Result<TextEditPayload, String> {
        // `open_popup` marshals the whole open onto the host main thread, where
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
