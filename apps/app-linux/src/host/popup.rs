//! Generic modal-popup layer (ADR-0021 D1/D2): the `about` dialog and the text
//! editor (`textEdit`, D4).
//!
//! The CAD partition is an **opaque native child** stacked above the WebView
//! (`gtk_bridge::native`), so a popup can never be a DOM overlay: it is a
//! separate native window with **its own child webview**, built exactly like the
//! main window (`tauri::window::WindowBuilder` + `window.add_child`), not a
//! `WebviewWindowBuilder` (which would produce a different GTK parenting).
//!
//! The host owns two things here, and both are deliberately **not** UI state:
//!
//! - the single popup window (label [`POPUP_LABEL`], child webview
//!   [`POPUP_WEBVIEW_LABEL`]) whose boot payload is a literal injected with
//!   `initialization_script` (no fetch, so the UI cannot race it);
//! - the **modal input gate** ([`ShellState::popup`]). It is set only after the
//!   window was built and cleared from that window's `Destroyed` event — for
//!   any reason (IPC close, Esc in the popup UI, or the window manager). It is
//!   independent of the UI's `suspendPointer`, whose blur path may release at
//!   any time.
//!
//! Only allowlisted kinds are ever built. A caller can never supply a window
//! label, URL or script.

use std::sync::{Arc, Mutex};

use gtk::prelude::*;
use serde::Serialize;
use tauri::Manager;

use super::options::WINDOW_LABEL;
use super::{lock, Shared};

/// Label of the single popup window.
pub const POPUP_LABEL: &str = "popup-1";
/// Label of the popup's child webview.
pub const POPUP_WEBVIEW_LABEL: &str = "popup-1-webview";
/// Popup width in logical pixels (`about`).
pub const POPUP_WIDTH: f64 = 520.0;
/// Popup height in logical pixels (`about`).
pub const POPUP_HEIGHT: f64 = 360.0;
/// Text-edit popup width in logical pixels: wide enough for a full annotation
/// line without wrapping every few words.
pub const TEXT_EDIT_WIDTH: f64 = 640.0;
/// Text-edit popup height in logical pixels: the `about` card is far too short
/// for a multi-line MTEXT / MULTILEADER annotation.
pub const TEXT_EDIT_HEIGHT: f64 = 480.0;

/// The popup kinds the host will build.
///
/// This enum is the allowlist: a string outside it is refused **before** any
/// window or state change, so a caller cannot choose a label, URL or script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupKind {
    /// The trivial "About" dialog that proves the pipeline.
    About,
    /// Edit the current single selection's text (ADR-0021 D4). The **host**
    /// derives the payload from the selection; the caller supplies no content.
    TextEdit,
}

impl PopupKind {
    /// Resolve one wire kind key against the allowlist.
    ///
    /// The refusal names the rejected kind (`popup.unknown_kind: …`) and is
    /// never a silent fallback to a default dialog.
    pub fn from_key(key: &str) -> Result<Self, String> {
        match key {
            "about" => Ok(Self::About),
            "textEdit" => Ok(Self::TextEdit),
            other => Err(format!("popup.unknown_kind: {other:?}")),
        }
    }

    /// The stable machine key of this kind.
    pub fn key(self) -> &'static str {
        match self {
            Self::About => "about",
            Self::TextEdit => "textEdit",
        }
    }

    /// The logical window size of this kind. Every popup is non-resizable; the
    /// text editor simply needs more room than the `about` card.
    pub fn size(self) -> (f64, f64) {
        match self {
            Self::About => (POPUP_WIDTH, POPUP_HEIGHT),
            Self::TextEdit => (TEXT_EDIT_WIDTH, TEXT_EDIT_HEIGHT),
        }
    }
}

/// The host-derived payload of the `textEdit` kind (ADR-0021 D4).
///
/// Both fields are always present in the boot payload. `text` is the raw stored
/// string with only `\P` shown as `\n` (the one mapping the ADR allows, so the
/// dialog displays natural line breaks); `multiline` is `true` for `AcDbMText`
/// and MULTILEADER text, `false` for `AcDbText`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEditPayload {
    pub text: String,
    pub multiline: bool,
}

/// The `cad_open_popup` acknowledgement (frozen wire shape).
///
/// `accepted` is always `true` on `Ok`; `existing` says whether the host focused
/// an already-open popup instead of building a second one. The wire field is
/// `popup_label` (host window label), never a caller-supplied label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PopupAck {
    pub accepted: bool,
    pub kind: String,
    pub popup_label: String,
    pub existing: bool,
}

/// The window effects an open/close request needs (one seam, so the state
/// machine is testable without a display).
pub(super) trait PopupOps {
    /// Build the single popup window for `kind` (and register its `Destroyed`
    /// handler in the production implementation). `payload` is the host-derived
    /// text payload, present exactly for [`PopupKind::TextEdit`].
    fn create(&self, kind: PopupKind, payload: Option<&TextEditPayload>) -> Result<(), String>;
    /// Focus an existing popup window.
    fn focus(&self, label: &str) -> Result<(), String>;
    /// Ask an existing popup window to close. A missing window is not an error.
    fn close(&self, label: &str) -> Result<(), String>;
    /// Whether the window currently exists (stale-state reconciliation).
    fn exists(&self, label: &str) -> bool;
    /// Derive the `textEdit` payload from the **current** CAD selection.
    ///
    /// A caller can never supply content; a selection that is not exactly one
    /// editable text is refused with `popup.no_text_selection: …` **before** any
    /// window exists.
    fn text_edit_payload(&self) -> Result<TextEditPayload, String>;
}

/// The literal boot payload injected with `initialization_script`.
///
/// It must be a JSON literal so the popup UI can read `window.__YACR_POPUP__` at
/// mount with no fetch and no race. The values are static host facts; serde_json
/// performs the escaping (a caller never supplies any of them).
pub(super) fn boot_payload_script(
    kind: PopupKind,
    text: Option<&TextEditPayload>,
) -> Result<String, String> {
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
pub(super) fn paragraph_breaks_to_newlines(text: &str) -> String {
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

/// Open the allowlisted popup (or focus the existing single instance).
///
/// Order matters:
/// 1. validate the kind (an unknown kind touches nothing);
/// 2. reconcile stale state (the window may have gone away before its
///    `Destroyed` event was observed);
/// 3. focus an existing popup, or build exactly one and only then set the gate.
pub(super) fn open_popup(
    shared: &Arc<Mutex<Shared>>,
    requested: &str,
    ops: &dyn PopupOps,
) -> Result<PopupAck, String> {
    let kind = PopupKind::from_key(requested)?;
    reconcile_popup_state(shared, ops);

    let open = lock(shared).shell.popup.clone();
    match open {
        Some(open) => {
            // Single instance: never build a second window and never re-set the
            // gate; only raise the one that is already there.
            ops.focus(&open.label)?;
            Ok(PopupAck {
                accepted: true,
                kind: open.kind,
                popup_label: open.label,
                existing: true,
            })
        }
        None => {
            // The payload is derived **before** the window exists: a refusal
            // (`popup.no_text_selection`) leaves nothing to undo and creates no
            // window/gate.
            let payload = match kind {
                PopupKind::About => None,
                PopupKind::TextEdit => Some(ops.text_edit_payload()?),
            };
            ops.create(kind, payload.as_ref())?;
            let open = cad_ui_core::session::CadPopup {
                kind: kind.key().to_string(),
                label: POPUP_LABEL.to_string(),
                modal: true,
            };
            // Set the gate **once**, and only after the window was built: a
            // failed build must never leave CAD input refused forever.
            lock(shared).shell.popup = Some(open.clone());
            Ok(PopupAck {
                accepted: true,
                kind: open.kind,
                popup_label: open.label,
                existing: false,
            })
        }
    }
}

/// Close the open popup (idempotent).
///
/// The gate is intentionally **not** cleared here: the authoritative clear is
/// the window's `Destroyed` event, which also covers Esc and the window manager.
/// A window that is already gone is reconciled instead, so the call stays an
/// idempotent success and nothing is left dangling.
pub(super) fn close_popup(shared: &Arc<Mutex<Shared>>, ops: &dyn PopupOps) -> Result<(), String> {
    reconcile_popup_state(shared, ops);
    let open = lock(shared).shell.popup.clone();
    match open {
        None => Ok(()),
        Some(open) => ops.close(&open.label),
    }
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

/// Clear the modal input gate after the popup window is destroyed.
///
/// Called from the popup window's `Destroyed` event, so IPC close, Esc from the
/// popup UI and a window-manager close all release input by the same path.
pub(super) fn on_popup_destroyed(shared: &Arc<Mutex<Shared>>) {
    let cleared = lock(shared).shell.popup.take();
    if let Some(open) = cleared {
        eprintln!(
            "[probe] popup destroyed label={} kind={}; modal input gate cleared",
            open.label, open.kind
        );
    }
}

/// Re-validate the recorded popup against a window that truly exists.
///
/// A `Destroyed` event is the normal clear path, but a state left over from an
/// already-gone window must never wedge CAD input: `open`/`close` reconcile
/// first, so the next call rebuilds a fresh window and the gate stays honest.
fn reconcile_popup_state(shared: &Arc<Mutex<Shared>>, ops: &dyn PopupOps) {
    let open = lock(shared).shell.popup.clone();
    let Some(open) = open else {
        return;
    };
    if ops.exists(&open.label) {
        return;
    }
    eprintln!(
        "[probe] popup state was stale (window {} is gone); clearing the modal input gate",
        open.label
    );
    let mut state = lock(shared);
    // Clear only the same instance: a concurrent open must not be erased.
    if state.shell.popup.as_ref() == Some(&open) {
        state.shell.popup = None;
    }
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
    use std::cell::Cell;

    use super::*;
    use crate::host::new_shared;
    use crate::host::options::LaunchOptions;
    use cad_ui_core::protocol::InputKind;
    use cad_ui_core::shell::{admit_command, modal_gate_open, projected_popup, queue_cad_input};
    use cad_ui_core::unsupported_status;

    /// Recording stand-in for the real window layer.
    #[derive(Default)]
    struct FakeOps {
        creates: Cell<u32>,
        focuses: Cell<u32>,
        closes: Cell<u32>,
        live: Cell<bool>,
        fail_create: Cell<bool>,
        /// The payload `text_edit_payload` returns (`None` = no live selection).
        payload: std::cell::RefCell<Option<TextEditPayload>>,
        /// Records the payload `create` actually received.
        created: std::cell::RefCell<Option<Option<TextEditPayload>>>,
        /// Records the kind `create` actually received.
        created_kind: Cell<Option<PopupKind>>,
    }

    impl FakeOps {
        fn with_payload(text: &str, multiline: bool) -> Self {
            let ops = Self::default();
            *ops.payload.borrow_mut() = Some(TextEditPayload {
                text: text.into(),
                multiline,
            });
            ops
        }
    }

    impl PopupOps for FakeOps {
        fn create(&self, kind: PopupKind, payload: Option<&TextEditPayload>) -> Result<(), String> {
            self.creates.set(self.creates.get() + 1);
            self.created_kind.set(Some(kind));
            *self.created.borrow_mut() = Some(payload.cloned());
            if self.fail_create.get() {
                return Err("popup.build_failed: test".into());
            }
            self.live.set(true);
            Ok(())
        }

        fn focus(&self, _label: &str) -> Result<(), String> {
            self.focuses.set(self.focuses.get() + 1);
            Ok(())
        }

        fn close(&self, _label: &str) -> Result<(), String> {
            self.closes.set(self.closes.get() + 1);
            self.live.set(false);
            Ok(())
        }

        fn exists(&self, _label: &str) -> bool {
            self.live.get()
        }

        fn text_edit_payload(&self) -> Result<TextEditPayload, String> {
            self.payload.borrow().clone().ok_or_else(|| {
                "popup.no_text_selection: no single editable text is selected".to_string()
            })
        }
    }

    fn pointer(sequence: u64) -> cad_ui_core::protocol::ProbeInput {
        cad_ui_core::protocol::ProbeInput {
            sequence,
            kind: InputKind::PointerDown,
            x: Some(1.0),
            y: Some(2.0),
            dx: None,
            dy: None,
            width: None,
            height: None,
            dpr: None,
            button: None,
            shift: None,
        }
    }

    #[test]
    fn only_the_allowlisted_kind_is_built() {
        for rejected in ["settings", "", "<script>alert(1)</script>", "About"] {
            let shared = new_shared(LaunchOptions::default());
            let ops = FakeOps::default();
            let error = open_popup(&shared, rejected, &ops).unwrap_err();
            assert!(
                error.starts_with("popup.unknown_kind"),
                "{rejected:?} must be refused by name: {error}"
            );
            assert_eq!(ops.creates.get(), 0, "no window for {rejected:?}");
            assert!(
                lock(&shared).shell.popup.is_none(),
                "no gate for {rejected:?}"
            );
        }

        let shared = new_shared(LaunchOptions::default());
        let ops = FakeOps::default();
        let ack = open_popup(&shared, "about", &ops).expect("about opens");
        assert_eq!(
            ack,
            PopupAck {
                accepted: true,
                kind: "about".into(),
                popup_label: POPUP_LABEL.into(),
                existing: false,
            }
        );
        assert_eq!(ops.creates.get(), 1);
        assert!(modal_gate_open(&lock(&shared).shell));
    }

    #[test]
    fn second_open_focuses_the_single_instance_and_sets_the_gate_once() {
        let shared = new_shared(LaunchOptions::default());
        let ops = FakeOps::default();
        open_popup(&shared, "about", &ops).expect("first open");
        let first_gate = lock(&shared).shell.popup.clone();
        assert!(first_gate.is_some());

        let ack = open_popup(&shared, "about", &ops).expect("second open");
        assert!(
            ack.existing,
            "the second open reports the existing instance"
        );
        assert_eq!(ack.popup_label, POPUP_LABEL);
        assert_eq!(ops.creates.get(), 1, "no second window is built");
        assert_eq!(ops.focuses.get(), 1, "the existing window is focused");
        assert_eq!(
            lock(&shared).shell.popup,
            first_gate,
            "the gate is set exactly once (same single state)"
        );
    }

    #[test]
    fn modal_gate_refuses_cad_input_until_the_popup_is_destroyed() {
        let shared = new_shared(LaunchOptions::default());
        let ops = FakeOps::default();
        {
            let mut state = lock(&shared);
            state.shell.cad_ready = true;
        }
        open_popup(&shared, "about", &ops).expect("open");

        {
            let mut state = lock(&shared);
            assert_eq!(
                admit_command(&mut state.shell, "OPEN").unwrap_err(),
                cad_ui_core::shell::POPUP_MODAL_OPEN
            );
            assert_eq!(
                queue_cad_input(&mut state.shell, &pointer(1)).unwrap_err(),
                cad_ui_core::shell::POPUP_MODAL_OPEN
            );
            assert!(state.shell.cad_inputs.is_empty());
        }

        // Simulated destroy (the window manager's path): the gate clears and
        // both funnels are admitted again.
        on_popup_destroyed(&shared);
        assert!(lock(&shared).shell.popup.is_none());
        {
            let mut state = lock(&shared);
            assert!(admit_command(&mut state.shell, "OPEN").is_ok());
            assert!(queue_cad_input(&mut state.shell, &pointer(2)).is_ok());
        }
    }

    #[test]
    fn status_popup_is_null_before_populated_while_open_and_null_after() {
        let shared = new_shared(LaunchOptions::default());
        let ops = FakeOps::default();
        let before = projected_popup(unsupported_status(None), lock(&shared).shell.popup.as_ref());
        assert_eq!(before.popup, None);

        open_popup(&shared, "about", &ops).expect("open");
        let during = projected_popup(unsupported_status(None), lock(&shared).shell.popup.as_ref());
        let popup = during.popup.as_ref().expect("populated while open").clone();
        assert_eq!(popup.kind, "about");
        assert_eq!(popup.label, POPUP_LABEL);
        assert!(popup.modal);
        let wire = serde_json::to_value(&during).expect("status serializes");
        assert_eq!(
            wire.get("popup")
                .and_then(|block| block.get("label"))
                .and_then(|label| label.as_str()),
            Some(POPUP_LABEL)
        );

        on_popup_destroyed(&shared);
        let after = projected_popup(unsupported_status(None), lock(&shared).shell.popup.as_ref());
        assert_eq!(after.popup, None);
        let wire = serde_json::to_value(&after).expect("status serializes");
        assert_eq!(wire.get("popup"), Some(&serde_json::Value::Null));
    }

    #[test]
    fn close_is_idempotent_and_waits_for_the_destroyed_gate_clear() {
        let shared = new_shared(LaunchOptions::default());
        let ops = FakeOps::default();
        // Nothing open: closing is a success, not an error.
        close_popup(&shared, &ops).expect("idempotent close");
        assert_eq!(ops.closes.get(), 0);

        open_popup(&shared, "about", &ops).expect("open");
        close_popup(&shared, &ops).expect("close");
        assert_eq!(ops.closes.get(), 1);
        // The gate is owned by the Destroyed event, not by the close call.
        assert!(modal_gate_open(&lock(&shared).shell));

        // A second close (window already gone) reconciles the stale state and
        // still succeeds without another destroy.
        close_popup(&shared, &ops).expect("second close");
        assert_eq!(ops.closes.get(), 1);
        assert!(!modal_gate_open(&lock(&shared).shell));
    }

    #[test]
    fn a_stale_popup_state_is_reconciled_before_a_fresh_open() {
        let shared = new_shared(LaunchOptions::default());
        let ops = FakeOps::default();
        open_popup(&shared, "about", &ops).expect("first open");
        // Simulate a missed Destroyed event: the window is gone, the state is
        // not. The next open must not focus/return the phantom instance.
        ops.live.set(false);
        let ack = open_popup(&shared, "about", &ops).expect("re-open");
        assert!(!ack.existing, "a fresh window is built for the stale state");
        assert_eq!(ops.creates.get(), 2);
        assert!(modal_gate_open(&lock(&shared).shell));
    }

    #[test]
    fn a_failed_build_never_sets_the_gate() {
        let shared = new_shared(LaunchOptions::default());
        let ops = FakeOps::default();
        ops.fail_create.set(true);
        let error = open_popup(&shared, "about", &ops).unwrap_err();
        assert!(error.starts_with("popup.build_failed"), "{error}");
        assert!(lock(&shared).shell.popup.is_none());
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
    fn popup_sizes_are_per_kind_and_the_text_editor_is_larger() {
        assert_eq!(PopupKind::About.size(), (520.0, 360.0));
        assert_eq!(PopupKind::TextEdit.size(), (640.0, 480.0));
        assert_eq!(PopupKind::from_key("textEdit"), Ok(PopupKind::TextEdit));
        assert_eq!(PopupKind::TextEdit.key(), "textEdit");
    }

    #[test]
    fn text_edit_opens_with_the_host_payload_and_about_stays_unchanged() {
        let shared = new_shared(LaunchOptions::default());
        let ops = FakeOps::with_payload("hello", false);
        let ack = open_popup(&shared, "textEdit", &ops).expect("textEdit opens");
        assert_eq!(
            ack,
            PopupAck {
                accepted: true,
                kind: "textEdit".into(),
                popup_label: POPUP_LABEL.into(),
                existing: false,
            }
        );
        assert_eq!(ops.created_kind.get(), Some(PopupKind::TextEdit));
        assert_eq!(
            ops.created.borrow().clone().flatten(),
            Some(TextEditPayload {
                text: "hello".into(),
                multiline: false,
            })
        );
        assert!(modal_gate_open(&lock(&shared).shell));

        // `about` still carries no text payload and the same ack shape.
        let about = new_shared(LaunchOptions::default());
        let ops = FakeOps::with_payload("ignored", true);
        let ack = open_popup(&about, "about", &ops).expect("about opens");
        assert_eq!(ack.kind, "about");
        assert_eq!(ops.created.borrow().clone(), Some(None));
    }

    #[test]
    fn text_edit_without_a_text_selection_creates_no_window_and_no_gate() {
        let shared = new_shared(LaunchOptions::default());
        let ops = FakeOps::default();
        let error = open_popup(&shared, "textEdit", &ops).unwrap_err();
        assert!(
            error.starts_with("popup.no_text_selection"),
            "the refusal must be named: {error}"
        );
        assert_eq!(ops.creates.get(), 0, "no window is built");
        assert!(lock(&shared).shell.popup.is_none(), "no modal gate is set");
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
