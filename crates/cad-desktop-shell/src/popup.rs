//! The platform-neutral half of the popup lifecycle (ADR-0021 D1/D2, WP3).
//!
//! Popups are in-DOM overlays in the single UI tree, so this module owns no
//! window: it maps the host's [`Shared`] state onto the UI-framework-free state
//! machine in [`cad_ui_core::popup`] and gives every desktop host the **same**
//! four-trigger clear lifecycle:
//!
//! 1. [`close`] — an explicit close (the overlay's close button or Esc);
//! 2. [`on_reload`] — the UI document reloaded or navigated, so the overlay is
//!    gone;
//! 3. [`on_renderer_crash`] — the webview renderer failed; the gate is cleared
//!    **and** the failure is recorded as a [`HostErrorKind::Renderer`]
//!    diagnostic (visible through `cad_status.error`);
//! 4. [`on_process_exit`] — the host process is exiting.
//!
//! The gate itself stays [`cad_ui_core::shell::ShellState::popup`]; setting it
//! is the state machine's job ([`open`]). The concrete host owns the main-thread
//! payload derivation ([`PopupOps::text_edit_payload`]) and the toolkit hooks
//! that call triggers ②–④.

use std::sync::{Arc, Mutex, MutexGuard};

use cad_ui_core::popup::{self as machine, PopupAck, PopupGate, PopupOps, PopupReset};
use cad_ui_core::session::wire::POPUP_HOST_UNAVAILABLE;
use cad_ui_core::session::{CadPopup, PopupTarget};
use cad_ui_core::shell::{note_host_error, HostErrorKind};

use crate::state::{Shared, ShellPopupHost};

/// The desktop [`cad_ui_core::session::PopupTarget`]: the platform half of the
/// popup commands.
///
/// [`Shared::popup_host`] is the host-installed seam; the shared wire layer only
/// knows the [`PopupTarget`] trait. Constructing the target clones the seam out
/// **before** any other lock is taken (the command releases `Shared` first), so
/// the platform implementation can marshal to its own main thread without the
/// shell mutex held — the lock order documented in [`crate::state`].
pub struct DesktopPopupTarget<'a> {
    host: Arc<dyn ShellPopupHost>,
    shared: &'a Arc<Mutex<Shared>>,
}

impl<'a> DesktopPopupTarget<'a> {
    /// Bind the registered popup seam, or refuse with the shared reason.
    ///
    /// `None` means the host did not install a seam (probe mode, or a host
    /// without popups): the explicit `popup.host_unavailable` refusal, never a
    /// pretend acknowledgement.
    pub fn new(shared: &'a Arc<Mutex<Shared>>) -> Result<Self, String> {
        let host = lock(shared)
            .popup_host
            .clone()
            .ok_or_else(|| POPUP_HOST_UNAVAILABLE.to_string())?;
        Ok(Self { host, shared })
    }
}

impl PopupTarget for DesktopPopupTarget<'_> {
    fn open(&mut self, kind: &str) -> Result<PopupAck, String> {
        self.host.open(self.shared, kind)
    }

    fn close(&mut self) -> Result<(), String> {
        self.host.close(self.shared)
    }
}

/// Stable diagnostic prefix for trigger ③ (`cad_status.error`).
///
/// Technical English only: it is a host/transport caption, not UI prose.
pub const RENDERER_CRASH_ERROR: &str = "webview renderer crashed";

/// The shared host state as the popup state machine's gate.
///
/// The machine never holds this lock across an effect: each [`PopupGate`] call
/// takes the host lock, reads or writes `shell.popup` and releases it again.
pub struct SharedGate<'a>(&'a Arc<Mutex<Shared>>);

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
}

/// Open the allowlisted popup (or report the one already open).
///
/// The transitions live in [`cad_ui_core::popup::open_popup`]; this wrapper only
/// maps the host's shared state onto the gate.
pub fn open(
    shared: &Arc<Mutex<Shared>>,
    requested: &str,
    ops: &dyn PopupOps,
) -> Result<PopupAck, String> {
    machine::open_popup(&SharedGate(shared), requested, ops)
}

/// Trigger ①: close the open popup and clear the modal gate (idempotent).
pub fn close(shared: &Arc<Mutex<Shared>>) -> Result<(), String> {
    machine::close_popup(&SharedGate(shared))
}

/// Trigger ②: the UI document reloaded/navigated; the overlay no longer exists.
pub fn on_reload(shared: &Arc<Mutex<Shared>>) {
    machine::on_popup_reset(&SharedGate(shared), PopupReset::Reload);
}

/// Trigger ③: the webview renderer failed.
///
/// Clears the gate **and** records a technical diagnostic, so the next
/// `cad_status` reports the crash through `error` instead of silently releasing
/// CAD input.
pub fn on_renderer_crash(shared: &Arc<Mutex<Shared>>, detail: &str) {
    {
        let mut state = lock(shared);
        note_host_error(
            &mut state.shell,
            HostErrorKind::Renderer,
            format!("{RENDERER_CRASH_ERROR}: {detail}"),
        );
    }
    machine::on_popup_reset(&SharedGate(shared), PopupReset::RendererCrash);
}

/// Trigger ④: the host process is exiting.
pub fn on_process_exit(shared: &Arc<Mutex<Shared>>) {
    machine::on_popup_reset(&SharedGate(shared), PopupReset::ProcessExit);
}

fn lock(shared: &Arc<Mutex<Shared>>) -> MutexGuard<'_, Shared> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use cad_ui_core::launch::LaunchOptions;
    use cad_ui_core::popup::{PopupPayload, TextEditPayload};
    use cad_ui_core::protocol::{InputKind, ProbeInput};
    use cad_ui_core::session::CadStatus;
    use cad_ui_core::shell::{admit_command, modal_gate_open, POPUP_MODAL_OPEN};

    use super::*;
    use crate::commands::cad_status_impl;
    use crate::state::new_shared;

    #[derive(Default)]
    struct FakeOps {
        payload: RefCell<Option<TextEditPayload>>,
    }

    impl PopupOps for FakeOps {
        fn text_edit_payload(&self) -> Result<TextEditPayload, String> {
            self.payload
                .borrow()
                .clone()
                .ok_or_else(|| "popup.no_text_selection: none".to_string())
        }
    }

    fn shared() -> Arc<Mutex<Shared>> {
        let shared = new_shared(LaunchOptions::default());
        {
            let mut state = lock(&shared);
            state.shell.cad_ready = true;
            // A real session snapshot exists once the owner tick has run; seeding
            // the explicit unsupported one makes `cad_status` project the host
            // error exactly as it does in a live host with no document yet.
            state.shell.cad_status = Some(cad_ui_core::unsupported_status(None));
        }
        shared
    }

    fn pointer(sequence: u64) -> ProbeInput {
        ProbeInput {
            sequence,
            kind: InputKind::PointerMove,
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

    /// Both funnels the IPC commands use, on the live shared shell state.
    fn admission(shared: &Arc<Mutex<Shared>>) -> (Result<(), String>, Result<(), String>) {
        let mut state = lock(shared);
        let command = admit_command(&mut state.shell, "FIT");
        let input = cad_ui_core::shell::queue_cad_input(&mut state.shell, &pointer(7));
        (command, input)
    }

    fn status(shared: &Arc<Mutex<Shared>>) -> CadStatus {
        cad_status_impl(shared)
    }

    #[test]
    fn the_gate_refuses_both_funnels_while_open_and_releases_on_explicit_close() {
        let shared = shared();
        let ops = FakeOps::default();
        assert!(!modal_gate_open(&lock(&shared).shell));

        open(&shared, "about", &ops).expect("open");
        assert!(modal_gate_open(&lock(&shared).shell));
        let (command, input) = admission(&shared);
        assert_eq!(command.unwrap_err(), POPUP_MODAL_OPEN);
        assert_eq!(input.unwrap_err(), POPUP_MODAL_OPEN);
        // Both refusals happened before the queues were touched.
        let state = lock(&shared);
        assert!(state.shell.cad_commands.is_empty());
        assert!(state.shell.cad_inputs.is_empty());
        drop(state);
        assert_eq!(
            status(&shared).popup.as_ref().map(|p| p.kind.as_str()),
            Some("about")
        );

        // Trigger ①: explicit close.
        close(&shared).expect("explicit close is idempotent");
        assert!(!modal_gate_open(&lock(&shared).shell));
        assert_eq!(status(&shared).popup, None);
        let (command, input) = admission(&shared);
        assert!(command.is_ok());
        assert!(input.is_ok());

        // Idempotent: a second close is still a success and clears nothing else.
        close(&shared).expect("second close");
    }

    #[test]
    fn trigger_reload_clears_the_gate_and_modal_open_regresses_to_zero() {
        let shared = shared();
        open(&shared, "about", &FakeOps::default()).expect("open");
        assert!(modal_gate_open(&lock(&shared).shell));

        on_reload(&shared);
        assert!(
            !modal_gate_open(&lock(&shared).shell),
            "a reload must release the modal gate (regression: modal_open stayed set)"
        );
        assert_eq!(status(&shared).popup, None);
        let (command, input) = admission(&shared);
        assert!(command.is_ok(), "{command:?}");
        assert!(input.is_ok(), "{input:?}");

        // A reload with nothing open is a no-op, not an error.
        on_reload(&shared);
        assert!(!modal_gate_open(&lock(&shared).shell));
    }

    #[test]
    fn trigger_renderer_crash_clears_the_gate_and_records_a_diagnostic() {
        let shared = shared();
        open(&shared, "options", &FakeOps::default()).expect("options does not need a payload");
        assert!(modal_gate_open(&lock(&shared).shell));

        on_renderer_crash(&shared, "test process failed");
        assert!(!modal_gate_open(&lock(&shared).shell));
        assert_eq!(status(&shared).popup, None);
        let reported = status(&shared).error.expect("the crash must be visible");
        assert!(
            reported.contains(RENDERER_CRASH_ERROR),
            "the diagnostic must name the crash: {reported}"
        );
        assert!(reported.contains("test process failed"), "{reported}");
        let (command, input) = admission(&shared);
        assert!(command.is_ok(), "{command:?}");
        assert!(input.is_ok(), "{input:?}");
    }

    #[test]
    fn trigger_process_exit_clears_the_gate() {
        let shared = shared();
        open(&shared, "about", &FakeOps::default()).expect("open");
        on_process_exit(&shared);
        assert!(!modal_gate_open(&lock(&shared).shell));
        assert_eq!(status(&shared).popup, None);
    }

    #[test]
    fn a_refused_text_payload_leaves_no_gate() {
        let shared = shared();
        let error = open(&shared, "textEdit", &FakeOps::default()).unwrap_err();
        assert!(error.starts_with("popup.no_text_selection"), "{error}");
        assert!(!modal_gate_open(&lock(&shared).shell));
    }

    #[test]
    fn the_ack_payload_is_additive_and_omitted_when_absent() {
        let about = open(&shared(), "about", &FakeOps::default()).expect("about");
        let wire = serde_json::to_value(&about).expect("ack serializes");
        assert_eq!(wire["kind"], "about");
        assert_eq!(wire["existing"], false);
        assert_eq!(wire["payload"]["kind"], "about");
        assert!(wire["payload"]["version"].is_string());
        assert!(wire["payload"].get("text").is_none());

        let text_ops = FakeOps {
            payload: RefCell::new(Some(TextEditPayload {
                text: "hello".into(),
                multiline: true,
            })),
        };
        let text = open(&shared(), "textEdit", &text_ops).expect("textEdit");
        assert_eq!(
            text.payload,
            Some(PopupPayload::TextEdit {
                text: "hello".into(),
                multiline: true,
            })
        );
        let wire = serde_json::to_value(&text).expect("ack serializes");
        assert_eq!(wire["payload"]["kind"], "textEdit");
        assert_eq!(wire["payload"]["text"], "hello");
        assert_eq!(wire["payload"]["multiline"], true);

        // `options` has no payload at all; the old four-field shape is intact.
        let options = open(&shared(), "options", &FakeOps::default()).expect("options");
        let wire = serde_json::to_value(&options).expect("ack serializes");
        assert!(
            wire.get("payload").is_none(),
            "an absent payload must be skipped, not null: {wire}"
        );
        assert_eq!(
            wire.as_object().map(|object| object.len()),
            Some(4),
            "the frozen ack fields are unchanged: {wire}"
        );

        // `existing: true` carries no replacement payload either.
        let gate = shared();
        let _ = open(&gate, "about", &FakeOps::default()).expect("first");
        let again = open(&gate, "about", &FakeOps::default()).expect("second");
        assert!(again.existing);
        assert_eq!(again.payload, None);
    }
}
