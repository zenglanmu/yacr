//! The text-edit popup phase's **reopen** steps (split out of
//! [`super::popup_textedit`] at the 1000-line code-file gate).
//!
//! Same evidence-only driver, same env gate, same host paths. The reopen exists
//! so a reviewer can see the **stored** result in the dialog — the field must hold
//! two lines (the stored `\P` mapped back to `\n`) and it is focused, so a
//! caret/multi-line frame becomes possible — and so the dialog is closed again
//! with `cad_status.popup` back to `null`.

use std::sync::{Arc, Mutex};

use tauri::Manager;

use crate::host::{text_edit_payload_for_probe, ProbeShared};

use super::popup::{describe_status_popup, log_popup_overlay_handle};
use super::popup_textedit::{EXPECTED_REOPEN_LINES, PATCH, TEXT_EDIT_KIND};
use super::popup_textedit_summary::{bool_token, fingerprint, lock_evidence, TextEditEvidence};

/// Step 9: reopen `textEdit` on the same (already edited) selection so the
/// **stored** result is visible in the field for an external grab.
///
/// The popup payload is host-derived again, i.e. this is the round trip: the
/// stored `\P` must be mapped back to a newline, so the field must hold exactly
/// one `\n` and render as [`EXPECTED_REOPEN_LINES`] lines (`EVIDENCE` / `EDIT`).
pub(super) fn reopen_open_step(app: &tauri::AppHandle, evidence: &Arc<Mutex<TextEditEvidence>>) {
    if lock_evidence(evidence).is_aborted() {
        lock_evidence(evidence).skip("reopen");
        return;
    }

    let payload = match text_edit_payload_for_probe() {
        Ok(payload) => payload,
        Err(error) => {
            lock_evidence(evidence).fail("reopen_payload", &error);
            return;
        }
    };
    let payload_len = payload.text.chars().count();
    let newlines = payload.text.matches('\n').count();
    let lines = payload.text.split('\n').count();
    let round_trip = payload.text == PATCH;
    eprintln!(
        "[probe] popup textedit evidence: reopen payload len={payload_len} multiline={} \
         newline={newlines} lines={lines} expected_lines={EXPECTED_REOPEN_LINES} \
         fingerprint={} round_trip_matches_patch={round_trip}",
        bool_token(payload.multiline),
        fingerprint(&payload.text),
    );
    {
        let mut state = lock_evidence(evidence);
        state.reopen_payload_len = Some(payload_len);
        state.reopen_has_newline = Some(newlines == 1);
        state.reopen_lines = Some(lines);
    }
    if payload_len == 0 {
        lock_evidence(evidence).fail("reopen_payload", "the reopened payload text is empty");
        return;
    }
    if newlines != 1 {
        lock_evidence(evidence).fail(
            "reopen_newline",
            &format!(
                "the reopened payload has {newlines} newlines; the stored \\P must map back to \
                 exactly one"
            ),
        );
        return;
    }
    if lines != EXPECTED_REOPEN_LINES {
        lock_evidence(evidence).fail(
            "reopen_lines",
            &format!("the reopened payload has {lines} lines, expected {EXPECTED_REOPEN_LINES}"),
        );
        return;
    }
    if !round_trip {
        lock_evidence(evidence).fail(
            "reopen_payload",
            "the reopened payload is not the applied draft (EVIDENCE\\nEDIT); the stored result \
             and the reopened payload disagree",
        );
        return;
    }

    let ack = match crate::host::open_popup_for_probe(app, TEXT_EDIT_KIND) {
        Ok(ack) => ack,
        Err(error) => {
            lock_evidence(evidence).fail("reopen_open", &error);
            return;
        }
    };
    eprintln!(
        "[probe] popup textedit evidence: reopen ack accepted={} kind={} label={} existing={}",
        ack.accepted, ack.kind, ack.popup_label, ack.existing
    );
    lock_evidence(evidence).reopen_ack = Some(format!(
        "accepted={}_existing={}",
        ack.accepted, ack.existing
    ));
    match ack.payload.as_ref() {
        Some(cad_ui_core::popup::PopupPayload::TextEdit { text, multiline })
            if *text == payload.text && *multiline == payload.multiline => {}
        other => {
            lock_evidence(evidence).fail(
                "reopen_payload_ack",
                &format!("the reopen ack payload does not match the host derivation: {other:?}"),
            );
            return;
        }
    }
    eprintln!(
        "[probe] popup textedit evidence: reopen ack payload matches host derivation \
         len={payload_len} newline={newlines}"
    );

    let during = crate::host::cad_status(app.state::<ProbeShared>());
    let during_text = describe_status_popup(&during);
    let expected = format!("{}:{}:true", ack.kind, ack.popup_label);
    eprintln!(
        "[probe] popup textedit evidence: reopen status open popup={during_text} (expected \
         {expected})"
    );
    {
        let mut state = lock_evidence(evidence);
        state.reopen_status_open = Some(during_text.clone());
        if during.popup.is_none() {
            state.fail(
                "reopen_status_open",
                "cad_status.popup is null while the reopened text-edit popup is open",
            );
            return;
        }
        if during_text != expected {
            state.fail(
                "reopen_status_open",
                &format!("expected {expected}, got {during_text}"),
            );
            return;
        }
    }

    let (handle, xid) = log_popup_overlay_handle("textedit reopen");
    {
        let mut state = lock_evidence(evidence);
        state.reopen_handle = Some(handle);
        state.reopen_xid = xid;
    }
    eprintln!(
        "[probe] popup textedit evidence: reopened overlay shows the stored result in the single \
         UI tree; a whole-window capture contains it"
    );
}

/// Step 10: close the reopened dialog through the host path `cad_close_popup`
/// uses.
pub(super) fn reopen_close_step(app: &tauri::AppHandle, evidence: &Arc<Mutex<TextEditEvidence>>) {
    if lock_evidence(evidence).is_aborted() {
        lock_evidence(evidence).skip("reopen_close");
        return;
    }
    match crate::host::close_popup_for_probe(app) {
        Ok(()) => {
            eprintln!(
                "[probe] popup textedit evidence: reopen close ack accepted=true (an explicit \
                 close clears the modal gate directly; no window Destroyed event is involved)"
            );
            lock_evidence(evidence).reopen_close_ack = Some("accepted=true".into());
        }
        Err(error) => lock_evidence(evidence).fail("reopen_close", &error),
    }
}

/// Step 11: `cad_status.popup` must be `null` after the reopen close (one retry).
#[allow(deprecated)] // frozen `cad_status.popup.label` wire field
pub(super) fn reopen_status_after_step(
    app: &tauri::AppHandle,
    evidence: &Arc<Mutex<TextEditEvidence>>,
    attempt: u32,
) {
    if lock_evidence(evidence).is_aborted() {
        lock_evidence(evidence).skip("reopen_status_after");
        return;
    }
    if lock_evidence(evidence).reopen_close_ack.is_none() {
        lock_evidence(evidence).fail(
            "reopen_status_after",
            "the reopen close was never acknowledged",
        );
        return;
    }
    let status = crate::host::cad_status(app.state::<ProbeShared>());
    let text = describe_status_popup(&status);
    eprintln!(
        "[probe] popup textedit evidence: reopen status after close attempt={attempt} popup={text} \
         (expected -)"
    );
    let mut state = lock_evidence(evidence);
    match status.popup {
        None => {
            state.reopen_status_after = Some(text);
            state.reopen_closed_confirmed = true;
        }
        Some(popup) => {
            if attempt >= 2 {
                state.fail(
                    "reopen_status_after",
                    &format!(
                        "cad_status.popup is still {}:{}:{} after the reopen close settle interval",
                        popup.kind, popup.label, popup.modal
                    ),
                );
            } else {
                eprintln!(
                    "[probe] popup textedit evidence: the reopen gate has not cleared yet; \
                     retrying after the settle interval"
                );
            }
        }
    }
}
