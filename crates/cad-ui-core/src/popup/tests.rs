//! Modal-popup state-machine tests (moved from the Linux host with the code).

use std::cell::{Cell, RefCell};
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

/// Recording stand-in for the real host payload derivation.
#[derive(Default)]
struct FakeOps {
    /// The payload `text_edit_payload` returns (`None` = no live selection).
    payload: RefCell<Option<TextEditPayload>>,
    /// How many times the derivation ran (the refusal path must run it once).
    calls: Cell<u32>,
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
    fn text_edit_payload(&self) -> Result<TextEditPayload, String> {
        self.calls.set(self.calls.get() + 1);
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

#[test]
fn the_text_edit_payload_is_assembled_from_the_selection() {
    // Multiline: the one ADR-permitted `\P` -> newline mapping is applied here.
    let payload = TextEditPayload::from_selection(Some(("a\\Pb".into(), true)))
        .expect("one multiline selection assembles");
    assert_eq!(
        payload,
        TextEditPayload {
            text: "a\nb".into(),
            multiline: true,
        }
    );

    // Single-line TEXT has no paragraph code: pass it through byte-identically.
    let payload = TextEditPayload::from_selection(Some(("a\\Pb".into(), false)))
        .expect("one single-line selection assembles");
    assert_eq!(
        payload,
        TextEditPayload {
            text: "a\\Pb".into(),
            multiline: false,
        }
    );

    // No single editable text: one shared refusal, before any gate is set.
    let error = TextEditPayload::from_selection(None).expect_err("no selection is refused");
    assert_eq!(error, NO_TEXT_SELECTION);
    assert!(
        error.starts_with("popup.no_text_selection"),
        "the refusal keeps its machine prefix: {error}"
    );
}

#[test]
fn only_the_allowlisted_kind_is_opened() {
    for rejected in ["settings", "", "<script>alert(1)</script>", "About"] {
        let gate = shell_gate(true);
        let ops = FakeOps::default();
        let error = open_popup(&gate, rejected, &ops).unwrap_err();
        assert!(
            error.starts_with("popup.unknown_kind"),
            "{rejected:?} must be refused by name: {error}"
        );
        assert_eq!(ops.calls.get(), 0, "no payload derivation for {rejected:?}");
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
            payload: Some(PopupPayload::About {
                version: env!("CARGO_PKG_VERSION").into(),
                build: if cfg!(debug_assertions) {
                    "debug".into()
                } else {
                    "release".into()
                },
            }),
        }
    );
    assert!(modal_gate_open(&state(&gate)));
}

#[test]
fn second_open_reports_the_single_instance_and_sets_the_gate_once() {
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
    assert_eq!(
        ack.payload, None,
        "an existing popup never gets a replacement payload"
    );
    assert_eq!(
        state(&gate).popup,
        first_gate,
        "the gate is set exactly once (same single state)"
    );
}

#[test]
fn modal_gate_refuses_cad_input_until_the_popup_is_closed() {
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

    // Explicit close (trigger ①): the gate clears and both funnels are admitted.
    close_popup(&gate).expect("explicit close");
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
    assert!(popup.modal);
    // The serialized block is pinned by the host's `cad_status` wire test (this
    // crate deliberately carries no `serde_json`), so the projection itself is
    // asserted on the value above and on the cleared value below.

    close_popup(&gate).expect("close");
    let after = projected_popup(unsupported_status(None), state(&gate).popup.as_ref());
    assert_eq!(after.popup, None);
}

#[test]
fn close_is_idempotent_and_clears_the_gate_directly() {
    let gate = shell_gate(true);
    let ops = FakeOps::default();
    // Nothing open: closing is a success, not an error.
    close_popup(&gate).expect("idempotent close");
    assert!(!modal_gate_open(&state(&gate)));

    open_popup(&gate, "about", &ops).expect("open");
    close_popup(&gate).expect("close");
    // Trigger ① owns the clear: the overlay and the gate are the same document,
    // so there is no asynchronous teardown to wait for.
    assert!(!modal_gate_open(&state(&gate)));

    close_popup(&gate).expect("second close remains a success");
    assert!(!modal_gate_open(&state(&gate)));
}

#[test]
fn a_failed_payload_derivation_never_sets_the_gate() {
    let gate = shell_gate(true);
    let ops = FakeOps::default();
    let error = open_popup(&gate, "textEdit", &ops).unwrap_err();
    assert!(error.starts_with("popup.no_text_selection"), "{error}");
    assert_eq!(ops.calls.get(), 1, "the derivation was attempted once");
    assert!(state(&gate).popup.is_none());
}

#[test]
fn every_reset_trigger_clears_the_gate() {
    // Triggers ②–④: reload/navigation, renderer crash, process exit. Each one
    // releases CAD input; none of them is allowed to leave the gate set.
    for reason in [
        PopupReset::Reload,
        PopupReset::RendererCrash,
        PopupReset::ProcessExit,
    ] {
        let gate = shell_gate(true);
        let ops = FakeOps::default();
        state(&gate).cad_ready = true;
        open_popup(&gate, "about", &ops).expect("open");
        assert!(modal_gate_open(&state(&gate)), "{reason:?}");

        on_popup_reset(&gate, reason);
        assert!(!modal_gate_open(&state(&gate)), "{reason:?} must clear");
        let mut state = state(&gate);
        assert!(
            admit_command(&mut state, "OPEN").is_ok(),
            "{reason:?} must release the command funnel"
        );
        assert!(
            queue_cad_input(&mut state, &pointer(3)).is_ok(),
            "{reason:?} must release the input funnel"
        );
    }
}

#[test]
fn a_reset_with_no_popup_open_is_a_no_op() {
    let gate = shell_gate(true);
    for reason in [
        PopupReset::Reload,
        PopupReset::RendererCrash,
        PopupReset::ProcessExit,
    ] {
        on_popup_reset(&gate, reason);
        assert!(state(&gate).popup.is_none());
    }
}

#[test]
fn popup_kind_keys_round_trip_through_the_allowlist() {
    assert_eq!(PopupKind::from_key("textEdit"), Ok(PopupKind::TextEdit));
    assert_eq!(PopupKind::TextEdit.key(), "textEdit");
    assert_eq!(PopupKind::from_key("options"), Ok(PopupKind::Options));
    assert_eq!(PopupKind::Options.key(), "options");
    assert_eq!(PopupKind::from_key("about"), Ok(PopupKind::About));
    assert_eq!(PopupKind::About.key(), "about");
    assert_eq!(PopupReset::Reload.key(), "reload");
    assert_eq!(PopupReset::RendererCrash.key(), "renderer_crash");
    assert_eq!(PopupReset::ProcessExit.key(), "process_exit");
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
            payload: None,
        }
    );
    assert_eq!(ops.calls.get(), 0, "options derives no payload");
    assert!(modal_gate_open(&state(&gate)));

    // The same single instance is reported on a second request.
    let again = open_popup(&gate, "options", &ops).expect("second open");
    assert!(again.existing);
}

#[test]
fn text_edit_carries_the_host_payload_and_about_carries_its_own() {
    let gate = shell_gate(true);
    let ops = FakeOps::with_payload("hello", false);
    let ack = open_popup(&gate, "textEdit", &ops).expect("textEdit opens");
    assert_eq!(
        ack.payload,
        Some(PopupPayload::TextEdit {
            text: "hello".into(),
            multiline: false,
        })
    );
    assert_eq!(ack.kind, "textEdit");
    assert!(modal_gate_open(&state(&gate)));

    // `about` never touches the selection derivation and carries host facts.
    let about = shell_gate(true);
    let ops = FakeOps::with_payload("ignored", true);
    let ack = open_popup(&about, "about", &ops).expect("about opens");
    assert_eq!(ops.calls.get(), 0, "about derives no text payload");
    match ack.payload {
        Some(PopupPayload::About { version, build }) => {
            assert_eq!(version, env!("CARGO_PKG_VERSION"));
            assert!(build == "debug" || build == "release", "{build}");
        }
        other => panic!("about must carry its own payload, got {other:?}"),
    }
}

#[test]
fn text_edit_without_a_text_selection_creates_no_gate() {
    let gate = shell_gate(true);
    let ops = FakeOps::default();
    let error = open_popup(&gate, "textEdit", &ops).unwrap_err();
    assert!(
        error.starts_with("popup.no_text_selection"),
        "the refusal must be named: {error}"
    );
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
    close_popup(&slot).expect("close");
    assert!(slot.lock().expect("gate").is_none());

    open_popup(&slot, "about", &ops).expect("reopen");
    on_popup_reset(&slot, PopupReset::Reload);
    assert!(slot.lock().expect("gate").is_none());
}
