//! Modal-popup state-machine tests (moved from the Linux host with the code).

use std::cell::Cell;
use std::sync::{Mutex, MutexGuard};

use super::*;
use crate::protocol::{InputKind, ProbeInput};
use crate::shell::{admit_command, modal_gate_open, projected_popup, queue_cad_input, ShellState};
use crate::unsupported_status;

fn shell_gate(cad_mode: bool) -> Mutex<ShellState> {
    Mutex::new(ShellState::new(cad_mode))
}

fn state(gate: &Mutex<ShellState>) -> MutexGuard<'_, ShellState> {
    gate.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

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

fn pointer(sequence: u64) -> ProbeInput {
    ProbeInput {
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
        let gate = shell_gate(true);
        let ops = FakeOps::default();
        let error = open_popup(&gate, rejected, &ops).unwrap_err();
        assert!(
            error.starts_with("popup.unknown_kind"),
            "{rejected:?} must be refused by name: {error}"
        );
        assert_eq!(ops.creates.get(), 0, "no window for {rejected:?}");
        assert!(state(&gate).popup.is_none(), "no gate for {rejected:?}");
    }

    let gate = shell_gate(true);
    let ops = FakeOps::default();
    let ack = open_popup(&gate, "about", &ops).expect("about opens");
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
    assert!(modal_gate_open(&state(&gate)));
}

#[test]
fn second_open_focuses_the_single_instance_and_sets_the_gate_once() {
    let gate = shell_gate(true);
    let ops = FakeOps::default();
    open_popup(&gate, "about", &ops).expect("first open");
    let first_gate = state(&gate).popup.clone();
    assert!(first_gate.is_some());

    let ack = open_popup(&gate, "about", &ops).expect("second open");
    assert!(
        ack.existing,
        "the second open reports the existing instance"
    );
    assert_eq!(ack.popup_label, POPUP_LABEL);
    assert_eq!(ops.creates.get(), 1, "no second window is built");
    assert_eq!(ops.focuses.get(), 1, "the existing window is focused");
    assert_eq!(
        state(&gate).popup,
        first_gate,
        "the gate is set exactly once (same single state)"
    );
}

#[test]
fn modal_gate_refuses_cad_input_until_the_popup_is_destroyed() {
    let gate = shell_gate(true);
    let ops = FakeOps::default();
    state(&gate).cad_ready = true;
    open_popup(&gate, "about", &ops).expect("open");

    {
        let mut state = state(&gate);
        assert_eq!(
            admit_command(&mut state, "OPEN").unwrap_err(),
            crate::shell::POPUP_MODAL_OPEN
        );
        assert_eq!(
            queue_cad_input(&mut state, &pointer(1)).unwrap_err(),
            crate::shell::POPUP_MODAL_OPEN
        );
        assert!(state.cad_inputs.is_empty());
    }

    // Simulated destroy (the window manager's path): the gate clears and
    // both funnels are admitted again.
    on_popup_destroyed(&gate);
    assert!(state(&gate).popup.is_none());
    {
        let mut state = state(&gate);
        assert!(admit_command(&mut state, "OPEN").is_ok());
        assert!(queue_cad_input(&mut state, &pointer(2)).is_ok());
    }
}

#[test]
fn status_popup_is_null_before_populated_while_open_and_null_after() {
    let gate = shell_gate(true);
    let ops = FakeOps::default();
    let before = projected_popup(unsupported_status(None), state(&gate).popup.as_ref());
    assert_eq!(before.popup, None);

    open_popup(&gate, "about", &ops).expect("open");
    let during = projected_popup(unsupported_status(None), state(&gate).popup.as_ref());
    let popup = during.popup.as_ref().expect("populated while open").clone();
    assert_eq!(popup.kind, "about");
    assert_eq!(popup.label, POPUP_LABEL);
    assert!(popup.modal);
    // The serialized block is pinned by the host's `cad_status` wire test (this
    // crate deliberately carries no `serde_json`), so the projection itself is
    // asserted on the value above and on the cleared value below.

    on_popup_destroyed(&gate);
    let after = projected_popup(unsupported_status(None), state(&gate).popup.as_ref());
    assert_eq!(after.popup, None);
}

#[test]
fn close_is_idempotent_and_waits_for_the_destroyed_gate_clear() {
    let gate = shell_gate(true);
    let ops = FakeOps::default();
    // Nothing open: closing is a success, not an error.
    close_popup(&gate, &ops).expect("idempotent close");
    assert_eq!(ops.closes.get(), 0);

    open_popup(&gate, "about", &ops).expect("open");
    close_popup(&gate, &ops).expect("close");
    assert_eq!(ops.closes.get(), 1);
    // The gate is owned by the Destroyed event, not by the close call.
    assert!(modal_gate_open(&state(&gate)));

    // A second close (window already gone) reconciles the stale state and
    // still succeeds without another destroy.
    close_popup(&gate, &ops).expect("second close");
    assert_eq!(ops.closes.get(), 1);
    assert!(!modal_gate_open(&state(&gate)));
}

#[test]
fn a_stale_popup_state_is_reconciled_before_a_fresh_open() {
    let gate = shell_gate(true);
    let ops = FakeOps::default();
    open_popup(&gate, "about", &ops).expect("first open");
    // Simulate a missed Destroyed event: the window is gone, the state is
    // not. The next open must not focus/return the phantom instance.
    ops.live.set(false);
    let ack = open_popup(&gate, "about", &ops).expect("re-open");
    assert!(!ack.existing, "a fresh window is built for the stale state");
    assert_eq!(ops.creates.get(), 2);
    assert!(modal_gate_open(&state(&gate)));
}

#[test]
fn a_failed_build_never_sets_the_gate() {
    let gate = shell_gate(true);
    let ops = FakeOps::default();
    ops.fail_create.set(true);
    let error = open_popup(&gate, "about", &ops).unwrap_err();
    assert!(error.starts_with("popup.build_failed"), "{error}");
    assert!(state(&gate).popup.is_none());
}

#[test]
fn popup_sizes_are_per_kind_and_the_text_editor_is_larger() {
    assert_eq!(PopupKind::About.size(), (520.0, 360.0));
    assert_eq!(PopupKind::TextEdit.size(), (640.0, 480.0));
    assert_eq!(PopupKind::Options.size(), (720.0, 560.0));
    assert_eq!(PopupKind::from_key("textEdit"), Ok(PopupKind::TextEdit));
    assert_eq!(PopupKind::TextEdit.key(), "textEdit");
    assert_eq!(PopupKind::from_key("options"), Ok(PopupKind::Options));
    assert_eq!(PopupKind::Options.key(), "options");
}

#[test]
fn options_opens_without_a_payload_and_without_a_selection() {
    let gate = shell_gate(true);
    // No text payload is available: Options must not depend on a selection.
    let ops = FakeOps::default();
    let ack = open_popup(&gate, "options", &ops).expect("options opens");
    assert_eq!(
        ack,
        PopupAck {
            accepted: true,
            kind: "options".into(),
            popup_label: POPUP_LABEL.into(),
            existing: false,
        }
    );
    assert_eq!(ops.created_kind.get(), Some(PopupKind::Options));
    assert_eq!(
        ops.created.borrow().clone(),
        Some(None),
        "options carries no host-derived payload"
    );
    assert!(modal_gate_open(&state(&gate)));

    // The same single instance is focused on a second request.
    let again = open_popup(&gate, "options", &ops).expect("second open");
    assert!(again.existing);
    assert_eq!(ops.creates.get(), 1);
    assert_eq!(ops.focuses.get(), 1);
}

#[test]
fn text_edit_opens_with_the_host_payload_and_about_stays_unchanged() {
    let gate = shell_gate(true);
    let ops = FakeOps::with_payload("hello", false);
    let ack = open_popup(&gate, "textEdit", &ops).expect("textEdit opens");
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
    assert!(modal_gate_open(&state(&gate)));

    // `about` still carries no text payload and the same ack shape.
    let about = shell_gate(true);
    let ops = FakeOps::with_payload("ignored", true);
    let ack = open_popup(&about, "about", &ops).expect("about opens");
    assert_eq!(ack.kind, "about");
    assert_eq!(ops.created.borrow().clone(), Some(None));
}

#[test]
fn text_edit_without_a_text_selection_creates_no_window_and_no_gate() {
    let gate = shell_gate(true);
    let ops = FakeOps::default();
    let error = open_popup(&gate, "textEdit", &ops).unwrap_err();
    assert!(
        error.starts_with("popup.no_text_selection"),
        "the refusal must be named: {error}"
    );
    assert_eq!(ops.creates.get(), 0, "no window is built");
    assert!(state(&gate).popup.is_none(), "no modal gate is set");
}

#[test]
fn the_bare_popup_slot_gate_is_equivalent_to_the_shell_gate() {
    // The same transitions through a gate over a bare slot: the state machine
    // does not depend on the host's `ShellState` wrapping.
    let slot: Mutex<Option<crate::session::CadPopup>> = Mutex::new(None);
    let ops = FakeOps::default();
    let ack = open_popup(&slot, "about", &ops).expect("open");
    assert!(!ack.existing);
    assert_eq!(
        slot.lock().expect("gate").as_ref().map(|p| p.kind.as_str()),
        Some("about")
    );
    close_popup(&slot, &ops).expect("close");
    on_popup_destroyed(&slot);
    assert!(slot.lock().expect("gate").is_none());
}
