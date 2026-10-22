//! Native modal-popup window for the Linux host (ADR-0021 D1/D2).
//!
//! The **state machine** — the [`PopupKind`] allowlist, [`PopupAck`], the
//! lifecycle transitions and the modal input gate — lives in
//! [`cad_ui_core::popup`], shared with every host. This module owns only the
//! Linux/Tauri side of the [`PopupOps`] seam and the gate glue:
//!
//! - the single popup window (label [`POPUP_LABEL`], child webview
//!   [`POPUP_WEBVIEW_LABEL`]) whose boot payload is a literal injected with
//!   `initialization_script` (no fetch, so the UI cannot race it);
//! - the window effects (`create`/`focus`/`close`/`exists`) and the
//!   host-derived `textEdit` payload, read from the main-thread CAD session;
//! - the window's own `Destroyed` handler, so the gate is cleared by the window
//!   lifecycle and not by a UI-side pointer release.
//!
//! The CAD partition is an **opaque native child** stacked above the WebView
//! (`gtk_bridge::native`), so a popup can never be a DOM overlay: it is a
//! separate native window with **its own child webview**, built exactly like the
//! main window (`tauri::window::WindowBuilder` + `window.add_child`), not a
//! `WebviewWindowBuilder` (which would produce a different GTK parenting).
//!
//! A caller can never supply a window label, URL or script.

use std::sync::{Arc, Mutex};

use cad_ui_core::popup::{self as machine, PopupGate, PopupKind, PopupOps, TextEditPayload};
use cad_ui_core::session::CadPopup;
use gtk::prelude::*;
use tauri::Manager;

use super::locale::host_locale_script;
use super::options::WINDOW_LABEL;
use super::{lock, Shared};

pub use cad_ui_core::popup::{PopupAck, POPUP_LABEL};

/// Label of the popup's child webview.
pub const POPUP_WEBVIEW_LABEL: &str = "popup-1-webview";

/// The shared host state as the popup state machine's gate.
///
/// The machine never holds this lock across a window effect: each
/// [`PopupGate`] call takes the host lock, reads or writes `shell.popup` and
/// releases it again.
struct SharedGate<'a>(&'a Arc<Mutex<Shared>>);

impl PopupGate for SharedGate<'_> {
    fn current(&self) -> Option<CadPopup> {
        lock(self.0).shell.popup.clone()
    }

    fn replace(&self, next: Option<CadPopup>) {
        lock(self.0).shell.popup = next;
    }

    fn take(&self) -> Option<CadPopup> {
        lock(self.0).shell.popup.take()
    }

    fn clear_if(&self, expected: &CadPopup) {
        let mut state = lock(self.0);
        // Clear only the same instance: a concurrent open must not be erased.
        if state.shell.popup.as_ref() == Some(expected) {
            state.shell.popup = None;
        }
    }
}

/// Open the allowlisted popup (or focus the existing single instance).
///
/// The transitions live in [`cad_ui_core::popup::open_popup`]; this glue only
/// maps the host's shared state onto the gate.
pub(super) fn open_popup(
    shared: &Arc<Mutex<Shared>>,
    requested: &str,
    ops: &dyn PopupOps,
) -> Result<PopupAck, String> {
    machine::open_popup(&SharedGate(shared), requested, ops)
}

/// Close the open popup (idempotent).
///
/// The gate itself is still cleared only by the window's `Destroyed` handler
/// (`TauriPopupOps::create`), which also covers Esc and the window manager.
pub(super) fn close_popup(shared: &Arc<Mutex<Shared>>, ops: &dyn PopupOps) -> Result<(), String> {
    machine::close_popup(&SharedGate(shared), ops)
}

/// Clear the modal input gate after the popup window is destroyed.
pub(super) fn on_popup_destroyed(shared: &Arc<Mutex<Shared>>) {
    machine::on_popup_destroyed(&SharedGate(shared));
}

/// The literal boot payload injected with `initialization_script`.
///
/// It must be a JSON literal so the popup UI can read `window.__YACR_POPUP__` at
/// mount with no fetch and no race. The values are static host facts; serde_json
/// performs the escaping (a caller never supplies any of them).
fn boot_payload_script(kind: PopupKind, text: Option<&TextEditPayload>) -> Result<String, String> {
    let payload = match kind {
        PopupKind::About => serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "build": if cfg!(debug_assertions) { "debug" } else { "release" },
        }),
        PopupKind::TextEdit => {
            let text = text.ok_or_else(|| {
                "popup.payload_missing: textEdit needs a host-derived text payload".to_string()
            })?;
            serde_json::json!({
                "text": text.text,
                "multiline": text.multiline,
            })
        }
        // No payload: the dialog reads `cad_settings` after mount.
        PopupKind::Options => serde_json::json!({}),
    };
    let boot = serde_json::json!({
        "kind": kind.key(),
        "modal": true,
        "label": POPUP_LABEL,
        "payload": payload,
    });
    Ok(format!("window.__YACR_POPUP__ = {boot};"))
}

/// Show the ADR's one allowed opening mapping: an **unescaped** `\P` becomes a
/// newline, so the dialog edits natural line breaks.
///
/// Only `\P` is touched. `\\P` (an escaped backslash followed by `P`), `{`, `}`,
/// `\f…;`, `\W…;` and every other code pass through byte-identically — this is
/// not a rich-text parser.
fn paragraph_breaks_to_newlines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('P') => out.push('\n'),
            // Keep the escape pair verbatim: `\\` is a literal backslash, and
            // `\X` is some other code that this dialog must not interpret.
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Run one popup task on the GTK **main thread**.
///
/// The CAD session is a main-thread-local `!Send` value and the popup is a
/// native GTK window, so the payload derivation and the window build must both
/// happen there. On this host Tauri delivers commands on the GTK main thread, so
/// the task runs inline; the guard keeps the path correct if that ever changes
/// (an off-main-thread caller is marshalled instead of silently reading an empty
/// thread-local and refusing with `popup.no_text_selection`).
pub(super) fn on_main_thread<T, F>(app: &tauri::AppHandle, task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    if gtk::is_initialized_main_thread() {
        return task();
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = sender.send(task());
    })
    .map_err(|error| format!("popup.main_thread_failed: {error}"))?;
    receiver
        .recv()
        .map_err(|_| "popup.main_thread_failed: the main thread dropped the request".to_string())?
}

/// Evidence-only accessor: the live popup window, if the host created one.
///
/// This changes no state and never synthesises a window: it reads the same
/// registry the command path uses. Used by the `YACR_PROBE_POPUP_EVIDENCE`
/// driver to read the popup's native handle for an external X11 grab.
pub(crate) fn popup_window_for_probe(app: &tauri::AppHandle) -> Option<tauri::Window> {
    app.get_window(POPUP_LABEL)
}

/// Evidence-only accessor: the **same** open path `cad_open_popup` runs.
///
/// Deliberately not the `#[tauri::command]` wrapper: the driver calls the
/// host-side function with the same [`TauriPopupOps`] the command builds, so the
/// window, the state machine and the gate are the production ones.
pub(crate) fn open_popup_for_probe(app: &tauri::AppHandle, kind: &str) -> Result<PopupAck, String> {
    let shared = app.state::<super::ProbeShared>().0.clone();
    let ops = TauriPopupOps::new(app.clone(), shared.clone());
    open_popup(&shared, kind, &ops)
}

/// Evidence-only accessor: the **same** close path `cad_close_popup` runs.
///
/// As with [`open_popup_for_probe`] this is the host-side function, not the
/// command wrapper. The gate is still cleared only by the window's `Destroyed`
/// event, so the driver polls `cad_status` for that clear instead of assuming it.
pub(crate) fn close_popup_for_probe(app: &tauri::AppHandle) -> Result<(), String> {
    let shared = app.state::<super::ProbeShared>().0.clone();
    let ops = TauriPopupOps::new(app.clone(), shared.clone());
    close_popup(&shared, &ops)
}

/// Evidence-only accessor: the **same** `textEdit` payload derivation the open
/// path runs (ADR-0021 D4).
///
/// It is the identical [`PopupOps::text_edit_payload`] call, on the same
/// main-thread session state, and it changes nothing. Used by the
/// `YACR_PROBE_POPUP_EVIDENCE` text-edit phase to report the derived payload
/// length/fingerprint: the popup's boot script is injected into the child webview
/// and is not readable back, so the derivation is read immediately before the
/// open and the summary states that source explicitly. Must be called on the GTK
/// main thread, like every other CAD-session read.
pub(crate) fn text_edit_payload_for_probe(
    app: &tauri::AppHandle,
) -> Result<TextEditPayload, String> {
    let shared = app.state::<super::ProbeShared>().0.clone();
    TauriPopupOps::new(app.clone(), shared).text_edit_payload()
}

/// The Tauri/GTK implementation of [`PopupOps`].
pub(super) struct TauriPopupOps {
    app: tauri::AppHandle,
    shared: Arc<Mutex<Shared>>,
}

impl TauriPopupOps {
    pub(super) fn new(app: tauri::AppHandle, shared: Arc<Mutex<Shared>>) -> Self {
        Self { app, shared }
    }
}

impl PopupOps for TauriPopupOps {
    fn create(&self, kind: PopupKind, payload: Option<&TextEditPayload>) -> Result<(), String> {
        let window = create_popup_window(&self.app, kind, payload)?;
        // The **only** authoritative gate clear: any destroy path lands here.
        let events_shared = self.shared.clone();
        window.on_window_event(move |event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                on_popup_destroyed(&events_shared);
            }
        });
        Ok(())
    }

    fn focus(&self, label: &str) -> Result<(), String> {
        let window = self
            .app
            .get_window(label)
            .ok_or_else(|| format!("popup.window_missing: {label}"))?;
        window
            .set_focus()
            .map_err(|error| format!("popup.focus_failed: {error}"))
    }

    fn close(&self, label: &str) -> Result<(), String> {
        if let Some(window) = self.app.get_window(label) {
            window
                .destroy()
                .map_err(|error| format!("popup.close_failed: {error}"))?;
        }
        Ok(())
    }

    fn exists(&self, label: &str) -> bool {
        self.app.get_window(label).is_some()
    }

    fn text_edit_payload(&self) -> Result<TextEditPayload, String> {
        // `open_popup` marshals the whole open onto the GTK main thread, where
        // the CAD session (main-thread-local) holds the authoritative selection.
        let selected = super::CAD.with(|cad| {
            let cad = cad.borrow();
            cad.as_ref()
                .and_then(|session| session.selected_text_edit().ok().flatten())
        });
        let Some((text, multiline)) = selected else {
            return Err(
                "popup.no_text_selection: select exactly one TEXT, MTEXT or MULTILEADER text to \
                 edit"
                    .into(),
            );
        };
        Ok(TextEditPayload {
            // Opening shows `\P` as a newline (the one ADR-permitted mapping);
            // single-line TEXT has no such code and is passed through.
            text: if multiline {
                paragraph_breaks_to_newlines(&text)
            } else {
                text
            },
            multiline,
        })
    }
}

/// Build the popup window and its child webview, mirroring `host::init`.
///
/// Returns the native window handle; the caller registers the lifecycle hook.
fn create_popup_window(
    app: &tauri::AppHandle,
    kind: PopupKind,
    payload: Option<&TextEditPayload>,
) -> Result<tauri::Window, String> {
    let parent = app
        .get_window(WINDOW_LABEL)
        .ok_or_else(|| "popup.main_window_missing".to_string())?;
    let (width, height) = kind.size();
    let script = boot_payload_script(kind, payload)?;

    // Transparency is only safe on a composited screen; a non-compositing X11
    // session would otherwise get black corners. This is audited against the
    // **real** GDK screen, not assumed from the backend name.
    let composited = parent
        .gtk_window()
        .ok()
        .and_then(|window| gtk::prelude::WidgetExt::screen(&window))
        .map(|screen| screen.is_composited())
        .unwrap_or(false);

    let window = tauri::window::WindowBuilder::new(app, POPUP_LABEL)
        .title("Yacr")
        .inner_size(width, height)
        .resizable(false)
        .decorations(false)
        .focused(true)
        .transparent(composited)
        // Linux: `gtk_window_set_transient_for`, so the WM keeps the popup above
        // its parent. Tauri 2.11.6 has no `modal()` builder option.
        .parent(&parent)
        .map_err(|error| format!("popup.build_failed: {error}"))?
        .build()
        .map_err(|error| format!("popup.build_failed: {error}"))?;

    // Modality is a GTK-level property: it constrains GTK-routed input, while
    // the host's own gate (D2) covers the CAD funnel, which never receives
    // real OS input.
    match window.gtk_window() {
        Ok(gtk_window) => {
            gtk_window.set_modal(true);
            // Wayland ignores this by protocol; there the compositor decides.
            gtk_window.set_position(gtk::WindowPosition::CenterOnParent);
        }
        Err(error) => {
            eprintln!("[probe] popup: GTK window unavailable, modal flag not set: {error}")
        }
    }

    let size = window
        .inner_size()
        .unwrap_or_else(|_| tauri::PhysicalSize::new(width as u32, height as u32));
    let webview = tauri::webview::WebviewBuilder::new(
        POPUP_WEBVIEW_LABEL,
        tauri::WebviewUrl::App("index.html".into()),
    )
    .transparent(composited)
    .auto_resize()
    // Same host-resolved locale as the main window, injected before the document
    // is parsed, so the dialog's chrome matches its native labels.
    .initialization_script(host_locale_script())
    // Literal boot payload, read at mount with no race.
    .initialization_script(script);
    window
        .add_child(
            webview,
            tauri::PhysicalPosition::new(0, 0),
            tauri::PhysicalSize::new(size.width.max(1), size.height.max(1)),
        )
        .map_err(|error| format!("popup.webview_failed: {error}"))?;

    eprintln!(
        "[probe] popup window built label={POPUP_LABEL} kind={} composited={composited} \
         transparent={composited}",
        kind.key()
    );
    Ok(window)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::new_shared;
    use crate::host::options::LaunchOptions;

    #[test]
    fn the_gate_glue_reads_and_writes_the_real_shell_state() {
        let shared = new_shared(LaunchOptions::default());
        let gate = SharedGate(&shared);
        assert!(gate.current().is_none());
        let popup = CadPopup {
            kind: "about".into(),
            label: POPUP_LABEL.into(),
            modal: true,
        };
        gate.replace(Some(popup.clone()));
        assert_eq!(lock(&shared).shell.popup, Some(popup.clone()));
        assert_eq!(gate.take(), Some(popup.clone()));
        assert!(lock(&shared).shell.popup.is_none());

        // A stale expectation never clears a different instance.
        gate.replace(Some(popup.clone()));
        let other = CadPopup {
            kind: "options".into(),
            label: POPUP_LABEL.into(),
            modal: true,
        };
        gate.clear_if(&other);
        assert_eq!(gate.current(), Some(popup));
    }

    #[test]
    fn the_status_projection_serializes_the_popup_block() {
        use cad_ui_core::shell::projected_popup;
        use cad_ui_core::unsupported_status;

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
    fn boot_payload_is_a_literal_with_the_host_facts() {
        let script = boot_payload_script(PopupKind::About, None).expect("about has no payload");
        let prefix = "window.__YACR_POPUP__ = ";
        assert!(script.starts_with(prefix), "{script}");
        let json = script[prefix.len()..]
            .strip_suffix(';')
            .expect("the assignment is terminated");
        let value: serde_json::Value = serde_json::from_str(json).expect("the payload is JSON");
        assert_eq!(value["kind"], "about");
        assert_eq!(value["modal"], true);
        assert_eq!(value["label"], POPUP_LABEL);
        assert_eq!(value["payload"]["version"], env!("CARGO_PKG_VERSION"));
        let build = value["payload"]["build"]
            .as_str()
            .expect("build is a string");
        assert!(
            build == "debug" || build == "release",
            "unexpected build label: {build}"
        );
        // `about` never carries the text-edit fields.
        assert!(value["payload"].get("text").is_none());
        assert!(value["payload"].get("multiline").is_none());
    }

    #[test]
    fn text_edit_payload_has_the_selection_text_and_multiline_flag() {
        let payload = TextEditPayload {
            text: "line one\nline two".into(),
            multiline: true,
        };
        let script =
            boot_payload_script(PopupKind::TextEdit, Some(&payload)).expect("the text payload");
        let prefix = "window.__YACR_POPUP__ = ";
        let json = script[prefix.len()..]
            .strip_suffix(';')
            .expect("the assignment is terminated");
        let value: serde_json::Value = serde_json::from_str(json).expect("the payload is JSON");
        assert_eq!(value["kind"], "textEdit");
        assert_eq!(value["modal"], true);
        assert_eq!(value["label"], POPUP_LABEL);
        assert_eq!(value["payload"]["text"], "line one\nline two");
        assert_eq!(value["payload"]["multiline"], true);
        // A single-line payload is present too, never omitted.
        let single = TextEditPayload {
            text: "one line".into(),
            multiline: false,
        };
        let script = boot_payload_script(PopupKind::TextEdit, Some(&single)).expect("single line");
        let json = script[prefix.len()..]
            .strip_suffix(';')
            .expect("the assignment is terminated");
        let value: serde_json::Value = serde_json::from_str(json).expect("the payload is JSON");
        assert_eq!(value["payload"]["text"], "one line");
        assert_eq!(value["payload"]["multiline"], false);

        // A textEdit boot without a payload is refused, never an empty editor.
        let error = boot_payload_script(PopupKind::TextEdit, None).unwrap_err();
        assert!(error.starts_with("popup.payload_missing"), "{error}");
    }

    #[test]
    fn options_boot_payload_is_the_bare_envelope() {
        let script =
            boot_payload_script(PopupKind::Options, None).expect("options needs no payload");
        let prefix = "window.__YACR_POPUP__ = ";
        assert!(script.starts_with(prefix), "{script}");
        let json = script[prefix.len()..]
            .strip_suffix(';')
            .expect("the assignment is terminated");
        let value: serde_json::Value = serde_json::from_str(json).expect("the payload is JSON");
        assert_eq!(value["kind"], "options");
        assert_eq!(value["modal"], true);
        assert_eq!(value["label"], POPUP_LABEL);
        // An empty object, not `null` and not an invented settings snapshot.
        assert_eq!(value["payload"], serde_json::json!({}));
        assert_eq!(
            value["payload"].as_object().map(serde_json::Map::len),
            Some(0)
        );
    }

    #[test]
    fn only_unescaped_paragraph_breaks_become_newlines() {
        assert_eq!(paragraph_breaks_to_newlines("a\\Pb"), "a\nb");
        assert_eq!(paragraph_breaks_to_newlines("a\\P\\Pb"), "a\n\nb");
        // An escaped backslash is a literal character, not a code.
        assert_eq!(paragraph_breaks_to_newlines("a\\\\Pb"), "a\\\\Pb");
        // Every other code passes through byte-identically.
        for unchanged in [
            "{\\fArial;text}",
            "\\W0.8;wide",
            "a\\~b",
            "a\\Hb2;b",
            "100%",
        ] {
            assert_eq!(paragraph_breaks_to_newlines(unchanged), unchanged);
        }
        assert_eq!(paragraph_breaks_to_newlines("trailing\\"), "trailing\\");
    }
}
