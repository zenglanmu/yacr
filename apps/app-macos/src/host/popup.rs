//! The macOS host's popup seam (ADR-0021 D1/D2, WP3).
//!
//! Popups are **in-DOM overlays** in the single React tree (the DOM-on-top
//! arrangement), so this host owns no popup window: there is no second
//! `tauri::Window` and no second webview. What remains here is only what needs
//! the host main thread:
//!
//! - the one `!Send` CAD-session read that the derived `textEdit` payload needs
//!   (the assembly rules themselves live in [`cad_ui_core::popup`]);
//! - the renderer-crash lifecycle trigger ④, which is **explicitly NOT
//!   IMPLEMENTED** on macOS (see [`install_renderer_crash_hook`]).
//!
//! Main-thread marshalling is not implemented here: the shared protocol lives in
//! [`cad_desktop_shell::threading`] and the Tauri adapter in
//! [`crate::main_thread`], so a second desktop host does not copy either.
//!
//! The lifecycle (the modal gate plus its four clear triggers) lives in
//! [`cad_desktop_shell::popup`], and the allowlist/wire shapes live in
//! [`cad_ui_core::popup`]. A caller can never supply a label, URL or script.
//!
//! # Deviation from `apps/app-windows/src/host/popup.rs`
//!
//! Windows reaches WebView2's `ProcessFailed` event through
//! `tauri::Webview::with_webview` and reports the crash through
//! `cad_desktop_shell::popup::on_renderer_crash`. **tauri 2.12.3 exposes no
//! equivalent macOS hook** for the WKWebView content process terminating (no
//! `webContentProcessDidTerminate` surface in `tauri`/`wry`: both crates were
//! checked in the vendored registry). The trigger is therefore modelled as an
//! honest one-line report — never a fabricated crash notification and never a
//! silent no-op that would look wired. The remaining three clear triggers
//! (`close` / `reload` / `exit`) are unaffected.

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

/// Trigger ④ (renderer crash): **NOT IMPLEMENTED** on macOS.
///
/// WebView2 (`ICoreWebView2::add_ProcessFailed`) has no macOS counterpart that
/// Tauri surfaces: `tauri::Webview` gives no web-content-terminate event and
/// `wry 0.57` ships no `webContentProcessDidTerminate` delegate on the
/// `WKWebView`, so there is nothing to subscribe to. Reaching into
/// objc2/WKWebView behind Tauri's back would be a second, unowned hook on a
/// webview this host does not own.
///
/// The signature is kept identical to the Windows/Linux hook so `host::init`
/// ports without an integration branch. This function **only** logs the gap: it
/// does not call
/// [`cad_desktop_shell::popup::on_renderer_crash`], because that would report a
/// crash that was never observed. Consensus of the modal gate after a real
/// renderer crash therefore falls to the other three triggers
/// (`close` / `reload` / `exit`) — a documented limitation of this host, not an
/// implemented trigger.
pub(super) fn install_renderer_crash_hook(_webview: &tauri::Webview, _shared: &Arc<Mutex<Shared>>) {
    eprintln!(
        "[probe] popup lifecycle trigger 4 (renderer crash): NOT IMPLEMENTED on macOS (tauri 2.12.3 exposes no web-content-terminate hook)"
    );
}

/// The macOS implementation of [`PopupOps`]: one main-thread CAD-session read.
pub(super) struct HostPopupOps;
impl PopupOps for HostPopupOps {
    fn text_edit_payload(&self) -> Result<TextEditPayload, String> {
        // `open_popup` marshals the whole open onto the host main thread, where
        // the CAD session (main-thread-local) holds the authoritative selection.
        // This is the only platform part; the refusal and the one ADR-permitted
        // `\P` mapping are shared in `cad_ui_core::popup`.
        //
        let selected = super::CAD.with(|cad| {
            let cad = cad.borrow();
            cad.as_ref()
                .and_then(|session| session.selected_text_edit().ok().flatten())
        });
        TextEditPayload::from_selection(selected)
    }
}
