//! `YACR_PROBE_POPUP_EVIDENCE` phase 2: the **text-edit popup** (ADR-0021 D4)
//! end-to-end on the live host.
//!
//! Evidence-only, default OFF, gated by the **same** `YACR_PROBE_POPUP_EVIDENCE`
//! as the `about` phase in [`super::popup`] and scheduled **after** it on the same
//! driver thread. It is not a second probe and has no second env switch.
//!
//! The phase proves, against the real host, that the popup payload is derived
//! from the current selection, that a replacement string admitted through the
//! real `cad_set_selected_text` queue is applied by the owner tick, and that
//! `cad_status` is consistent afterwards (stored `\P` form, multiline flag, same
//! entity count, no tool, no error), before the modal gate is released again.
//!
//! ## Timeline (seconds after the shared driver start)
//!
//! The phase starts at `START` (`YACR_PROBE_POPUP_TEXTEDIT_AT`, default **220**,
//! never earlier than the `about` phase's summary) and holds for
//! `hold` (`YACR_PROBE_POPUP_TEXTEDIT_HOLD`, default **20**, floored at 5) — a
//! separate knob from the `about` hold, because the modal gate refuses CAD input
//! while any popup is open, so stretching the `about` hold to reach this point
//! would block the draw driver's own synthetic input.
//!
//! * `START`: precondition — read `cad_status` and log `selection.text` presence,
//!   `selection.text_multiline`, the selection row count/keys and
//!   `entityCount`. No single multi-line editable text ⇒ the phase stops with
//!   `popup_textedit_skipped=no_text_selection` (or `single_line_text`) and
//!   **never** claims a pass.
//! * `START+1`: open `textEdit` through the same host path `cad_open_popup` uses;
//!   log the ack, the **host-derived payload** (length, `multiline`, an 8+8 char
//!   fingerprint — never the whole string), the `cad_status.popup` block and the
//!   popup's native handle (`[probe] popup textedit evidence: window xid=0x…` on
//!   X11/XWayland) so an external grab can capture the dialog.
//! * `START+1+hold`: apply — admit [`PATCH`] through
//!   [`crate::host::admit_selected_text`], the **same admission**
//!   `cad_set_selected_text` runs (real queue, real owner-tick drain).
//! * `START+3+hold`: verify — read `cad_status` after the drain: `selection.text`
//!   must equal the expected stored form [`EXPECTED_STORED`] (`\P`, not a raw
//!   newline), `text_multiline` still `true`, `entityCount` unchanged, `tool`
//!   `null`, `error` `null`; the selection row count/keys are logged so a
//!   display-side rebuild is at least visible in the data.
//! * `START+4+hold`: close through the same host path `cad_close_popup` uses.
//! * `START+5+hold` / `+6+hold`: `cad_status.popup` must be `null` (one retry for
//!   the popup window's `Destroyed` event latency).
//! * `START+7+hold`: re-admission — a harmless input must be **accepted** again,
//!   and is removed from the queue again before the owner tick can drain it.
//! * `START+8+hold`: one greppable summary line
//!   `[probe] popup textedit evidence: …`, plus a failure list.
//!
//! ## Honest limits
//!
//! The selection is whatever the earlier draw phases left selected (the
//! `MLEADER` created by [`super::draw_mleader`]); the trigger is the probe driver,
//! not a real click on the palette's "edit in window" entry. Everything
//! downstream of the trigger (payload derivation, window, gate, queue, tick
//! drain, status projection, close) is the production path. The payload is read
//! through the same host derivation the open path calls, immediately before the
//! open (the window's boot script is not readable back); the summary says so with
//! `te_payload_source=host_derivation_before_open`. A screenshot is only the
//! dialog's own pixels, not a visual-correctness claim.
//!
//! **Schedule dependency**: the draw evidence driver (`YACR_PROBE_DRAW_EVIDENCE`)
//! leaves the MULTILEADER selected only at `t≈212 s`, so this phase defaults to
//! `START=220`. Without that driver (or before its `MLEADER` phase) there is no
//! editable multi-line selection and the phase honestly skips — it never waits,
//! retries or fabricates a target.
//!
//! ## Failure behaviour
//!
//! A step that fails its expectation logs
//! `[probe] popup textedit evidence: <step> FAILED: …` at once and later steps
//! log `skipped` instead of running as if the earlier one had worked. The summary
//! always prints, with `popup_textedit_pass=0` and the failed step named.
//!
//! The recorded reads and the summary formatting live in
//! [`super::popup_textedit_summary`] (split at the 1000-line code-file gate).

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use cad_ui_core::shell::queue_cad_input;
use tauri::Manager;

use crate::host::{admit_selected_text, text_edit_payload_for_probe, ProbeShared, Shared};

use super::popup::{describe_status_popup, log_popup_native_handle, probe_pointer_move};
use super::popup_textedit_summary::{
    bool_token, char_count, fingerprint, flat, lock_evidence, optional_count, row_keys, short_flat,
    summary_line, TextEditEvidence,
};
use super::schedule;

/// The allowlisted popup kind this phase drives.
const TEXT_EDIT_KIND: &str = "textEdit";
/// The replacement string admitted through the real `cad_set_selected_text`
/// queue. It contains a newline so the MTEXT/MULTILEADER `\P` mapping is
/// exercised rather than assumed.
pub(super) const PATCH: &str = "EVIDENCE\nEDIT";
/// `PATCH` as a single summary token (the newline is written as `\n`).
pub(super) const PATCH_LOG: &str = "EVIDENCE\\nEDIT";
/// The stored form `PATCH` must have after the edit: a literal `\P` paragraph
/// break between the two lines, never a raw newline.
pub(super) const EXPECTED_STORED: &str = "EVIDENCE\\PEDIT";
/// The admission ack recorded when the patch was queued (accept = queued).
pub(super) const APPLY_ACK: &str = "accepted=true_queued=1";
/// Seconds after the `about` phase's close before this phase may start. The
/// `about` phase prints its summary at `close + 4`, so this keeps the two
/// timelines from ever overlapping.
pub(super) const MIN_START_AFTER_ABOUT_CLOSE: u64 = 6;
/// Default start second (`YACR_PROBE_POPUP_TEXTEDIT_AT`).
///
/// The draw evidence driver commits its `MLEADER` at `t≈212 s` and prints its
/// report at `t=213 s`; this phase needs that selection, so its default start is
/// after it. It is a **separate** knob from the `about` hold on purpose: the modal
/// gate refuses CAD input while any popup is open, so lengthening the `about` hold
/// to reach this point would block the draw driver's own synthetic input.
const DEFAULT_START_SECONDS: u64 = 220;
/// Default grab hold for this phase (`YACR_PROBE_POPUP_TEXTEDIT_HOLD`).
///
/// Deliberately shorter than the `about` hold: it only has to cover one external
/// capture of the dialog, not the whole about pass.
const DEFAULT_HOLD_SECONDS: u64 = 20;
/// Floor for this phase's grab hold: below this an external grab is unreliable.
const MIN_HOLD_SECONDS: u64 = 5;
/// Sequence of the harmless re-admission pointer payload. Distinct from the
/// `about` phase's `9_000_001` so the two can never be confused.
const READMIT_INPUT_SEQUENCE: u64 = 9_000_002;

/// The absolute (driver-start relative) second of every step, derived once from
/// the phase start and hold so the schedule and its startup log cannot disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PhaseTimes {
    pub precondition: u64,
    pub open: u64,
    pub apply: u64,
    pub verify: u64,
    pub close: u64,
    pub summary: u64,
}

impl PhaseTimes {
    /// `start` is the precondition second; every other step follows from it and
    /// the phase's own grab `hold`.
    pub(super) fn at(start: u64, hold: u64) -> Self {
        Self {
            precondition: start,
            open: start + 1,
            apply: start + 1 + hold,
            verify: start + 3 + hold,
            close: start + 4 + hold,
            summary: start + 8 + hold,
        }
    }
}

/// The configured phase start (`YACR_PROBE_POPUP_TEXTEDIT_AT`, default
/// [`DEFAULT_START_SECONDS`]), never earlier than the `about` phase's summary.
///
/// An unparseable value is logged and replaced by the default rather than
/// silently accepted; a value that would overlap the `about` phase is raised to
/// [`MIN_START_AFTER_ABOUT_CLOSE`] and that correction is logged too.
pub(super) fn start_seconds(about_close: u64) -> u64 {
    let floor = about_close + MIN_START_AFTER_ABOUT_CLOSE;
    let configured = match std::env::var("YACR_PROBE_POPUP_TEXTEDIT_AT") {
        Ok(raw) => match raw.trim().parse::<u64>() {
            Ok(seconds) => seconds,
            Err(_) => {
                eprintln!(
                    "[probe] popup textedit evidence: YACR_PROBE_POPUP_TEXTEDIT_AT={raw:?} is not \
                     an integer number of seconds; using {DEFAULT_START_SECONDS}"
                );
                DEFAULT_START_SECONDS
            }
        },
        Err(_) => DEFAULT_START_SECONDS,
    };
    if configured < floor {
        eprintln!(
            "[probe] popup textedit evidence: start {configured}s is below the {floor}s floor \
             (the about phase must finish first); using {floor}s"
        );
        floor
    } else {
        configured
    }
}

/// The configured grab hold for this phase (`YACR_PROBE_POPUP_TEXTEDIT_HOLD`,
/// default [`DEFAULT_HOLD_SECONDS`]), floored at [`MIN_HOLD_SECONDS`]. An
/// unparseable value is logged and replaced by the default.
pub(super) fn hold_seconds() -> u64 {
    let configured = match std::env::var("YACR_PROBE_POPUP_TEXTEDIT_HOLD") {
        Ok(raw) => match raw.trim().parse::<u64>() {
            Ok(seconds) => seconds,
            Err(_) => {
                eprintln!(
                    "[probe] popup textedit evidence: YACR_PROBE_POPUP_TEXTEDIT_HOLD={raw:?} is not \
                     an integer number of seconds; using {DEFAULT_HOLD_SECONDS}"
                );
                DEFAULT_HOLD_SECONDS
            }
        },
        Err(_) => DEFAULT_HOLD_SECONDS,
    };
    if configured < MIN_HOLD_SECONDS {
        eprintln!(
            "[probe] popup textedit evidence: hold {configured}s is below the {MIN_HOLD_SECONDS}s \
             floor (the external grab needs the dialog open); using {MIN_HOLD_SECONDS}s"
        );
        MIN_HOLD_SECONDS
    } else {
        configured
    }
}

/// Schedule the text-edit phase on the shared driver thread, after the `about`
/// phase. Every step is marshalled onto the GTK main thread (the CAD session is
/// main-thread-local), exactly like the `about` phase's steps.
pub(super) fn schedule_textedit_phase(
    app: &tauri::AppHandle,
    start: &Instant,
    times: PhaseTimes,
    hold: u64,
) {
    let evidence = Arc::new(Mutex::new(TextEditEvidence::default()));

    // ---- Precondition: the selection must be one editable multi-line text ----
    {
        let step_app = app.clone();
        let evidence = evidence.clone();
        schedule(
            start,
            times.precondition,
            "textedit popup precondition",
            move || {
                let main_app = step_app.clone();
                step_app.run_on_main_thread(move || precondition_step(&main_app, &evidence))
            },
        );
    }

    // ---- Open through the host path, log payload + status + native handle ----
    {
        let step_app = app.clone();
        let evidence = evidence.clone();
        schedule(start, times.open, "textedit popup open", move || {
            let main_app = step_app.clone();
            step_app.run_on_main_thread(move || open_step(&main_app, &evidence))
        });
    }

    // ---- Apply the replacement through the real admission queue ----
    {
        let step_app = app.clone();
        let evidence = evidence.clone();
        schedule(start, times.apply, "textedit popup apply", move || {
            let main_app = step_app.clone();
            step_app.run_on_main_thread(move || apply_step(&main_app, &evidence))
        });
    }

    // ---- Verify the drained edit against the real status ----
    {
        let step_app = app.clone();
        let evidence = evidence.clone();
        schedule(start, times.verify, "textedit popup verify", move || {
            let main_app = step_app.clone();
            step_app.run_on_main_thread(move || verify_step(&main_app, &evidence))
        });
    }

    // ---- Close through the host path ----
    {
        let step_app = app.clone();
        let evidence = evidence.clone();
        schedule(start, times.close, "textedit popup close", move || {
            let main_app = step_app.clone();
            step_app.run_on_main_thread(move || close_step(&main_app, &evidence))
        });
    }

    // ---- The gate clears on the popup window's Destroyed event ----
    for attempt in 1..=2u32 {
        let step_app = app.clone();
        let evidence = evidence.clone();
        schedule(
            start,
            times.close + u64::from(attempt),
            "textedit popup status after close",
            move || {
                let main_app = step_app.clone();
                step_app
                    .run_on_main_thread(move || status_after_step(&main_app, &evidence, attempt))
            },
        );
    }

    // ---- Re-admission: the modal gate must really be released ----
    {
        let step_app = app.clone();
        let evidence = evidence.clone();
        schedule(
            start,
            times.close + 3,
            "textedit popup re-admission",
            move || {
                let main_app = step_app.clone();
                step_app.run_on_main_thread(move || readmit_step(&main_app, &evidence))
            },
        );
    }

    // ---- One summary line, queued after the re-admission step ----
    {
        let step_app = app.clone();
        let evidence = evidence.clone();
        schedule(start, times.summary, "textedit popup summary", move || {
            step_app.run_on_main_thread(move || {
                let evidence = lock_evidence(&evidence);
                eprintln!(
                    "[probe] popup textedit evidence: {}",
                    summary_line(&evidence, hold)
                );
            })
        });
    }
}

/// Step 1: read the real status and decide whether this phase can run at all.
fn precondition_step(app: &tauri::AppHandle, evidence: &Arc<Mutex<TextEditEvidence>>) {
    let status = crate::host::cad_status(app.state::<ProbeShared>());
    let selection = &status.selection;
    eprintln!(
        "[probe] popup textedit evidence: precondition selection_count={} text_present={} \
         text_multiline={} text_len={} rows={} entity_count={}",
        selection.count,
        bool_token(selection.text.is_some()),
        bool_token(selection.text_multiline),
        selection.text.as_deref().map(char_count).unwrap_or(0),
        selection.rows.len(),
        optional_count(status.entity_count),
    );

    let Some(text) = selection.text.as_deref() else {
        report_skip(
            evidence,
            "no_text_selection",
            "cad_status.selection.text is null: no single editable text is selected, so the \
             text-edit popup cannot be opened (nothing is claimed)",
        );
        return;
    };
    if !selection.text_multiline {
        report_skip(
            evidence,
            "single_line_text",
            "the selected text is single-line (AcDbText): the newline patch would be refused by \
             the textedit policy, so this phase has nothing to prove (nothing is claimed)",
        );
        return;
    }

    eprintln!(
        "[probe] popup textedit evidence: selected text fingerprint={} expected_multiline=true",
        fingerprint(text)
    );
    let mut state = lock_evidence(evidence);
    state.selection_text = Some(fingerprint(text));
    state.selection_multiline = Some(true);
    state.rows_before = Some(selection.rows.len());
    state.row_keys_before = Some(row_keys(&status));
    state.entities_before = status.entity_count;
}

/// Step 2: open `textEdit` through the same host path `cad_open_popup` uses.
fn open_step(app: &tauri::AppHandle, evidence: &Arc<Mutex<TextEditEvidence>>) {
    if lock_evidence(evidence).is_aborted() {
        lock_evidence(evidence).skip("open");
        return;
    }

    // The host-derived payload, read through the same derivation the open path
    // calls (ADR-0021 D4: the caller never supplies content).
    let payload = match text_edit_payload_for_probe(app) {
        Ok(payload) => payload,
        Err(error) => {
            lock_evidence(evidence).fail("payload", &error);
            return;
        }
    };
    let payload_len = payload.text.chars().count();
    let payload_fingerprint = fingerprint(&payload.text);
    eprintln!(
        "[probe] popup textedit evidence: payload len={payload_len} multiline={} \
         fingerprint={payload_fingerprint} source=host_derivation_before_open",
        payload.multiline
    );
    {
        let mut state = lock_evidence(evidence);
        state.payload_len = Some(payload_len);
        state.payload_multiline = Some(payload.multiline);
        state.payload_fingerprint = Some(payload_fingerprint);
    }
    if payload_len == 0 {
        lock_evidence(evidence).fail("payload", "the host-derived payload text is empty");
        return;
    }
    if !payload.multiline {
        // The precondition already required multiline, so a disagreement here is
        // a real inconsistency worth naming rather than silently continuing.
        lock_evidence(evidence).fail(
            "payload",
            "the host-derived payload reports multiline=false after a multiline selection",
        );
        return;
    }

    let ack = match crate::host::open_popup_for_probe(app, TEXT_EDIT_KIND) {
        Ok(ack) => ack,
        Err(error) => {
            lock_evidence(evidence).fail("open", &error);
            return;
        }
    };
    eprintln!(
        "[probe] popup textedit evidence: open ack accepted={} kind={} label={} existing={}",
        ack.accepted, ack.kind, ack.popup_label, ack.existing
    );
    lock_evidence(evidence).open_ack = Some(format!(
        "accepted={}_existing={}",
        ack.accepted, ack.existing
    ));

    let during = crate::host::cad_status(app.state::<ProbeShared>());
    let during_text = describe_status_popup(&during);
    let expected = format!("{}:{}:true", ack.kind, ack.popup_label);
    eprintln!(
        "[probe] popup textedit evidence: status open popup={during_text} (expected {expected})"
    );
    {
        let mut state = lock_evidence(evidence);
        state.status_open = Some(during_text.clone());
        if during.popup.is_none() {
            state.fail(
                "status_open",
                "cad_status.popup is null while the text-edit popup window is open",
            );
            return;
        }
        if during_text != expected {
            state.fail(
                "status_open",
                &format!("expected {expected}, got {during_text}"),
            );
            return;
        }
    }

    match crate::host::popup_window_for_probe(app) {
        Some(window) => match log_popup_native_handle(&window) {
            Some((handle, xid)) => {
                match xid.as_deref() {
                    Some(xid) => eprintln!(
                        "[probe] popup textedit evidence: window xid={xid} handle={handle} \
                         (external grab target)"
                    ),
                    None => eprintln!(
                        "[probe] popup textedit evidence: window xid=- handle={handle} \
                         (no X11 XID: the compositor owns the toplevel; no X11 grab is claimed)"
                    ),
                }
                let mut state = lock_evidence(evidence);
                state.handle = Some(handle);
                state.xid = xid;
            }
            None => lock_evidence(evidence).fail(
                "window_handle",
                "no reportable top-level native handle for the popup window",
            ),
        },
        None => lock_evidence(evidence).fail(
            "window_handle",
            "the popup window is not registered after a successful open",
        ),
    }
}

/// Step 3: admit the replacement through the **same** admission
/// `cad_set_selected_text` uses (a real queue entry; the owner tick drains it).
fn apply_step(app: &tauri::AppHandle, evidence: &Arc<Mutex<TextEditEvidence>>) {
    if lock_evidence(evidence).is_aborted() {
        lock_evidence(evidence).skip("apply");
        return;
    }
    let shared = app.state::<ProbeShared>().0.clone();
    let (result, queued_before, queued_after) = {
        let mut state = lock_shared(&shared);
        let cad_mode = state.shell.cad_mode;
        let before = state.cad_selected_texts.len();
        let result =
            admit_selected_text(&mut state.cad_selected_texts, cad_mode, PATCH.to_string());
        (result, before, state.cad_selected_texts.len())
    };
    eprintln!(
        "[probe] popup textedit evidence: apply patch={PATCH_LOG} \
         queue={queued_before}->{queued_after} expected_stored={EXPECTED_STORED}"
    );
    match result {
        Ok(()) if queued_after == queued_before + 1 => {
            eprintln!(
                "[probe] popup textedit evidence: apply admitted (queued; the owner tick drains it \
                 into CadSession::set_selected_text)"
            );
            lock_evidence(evidence).apply_ack = Some(APPLY_ACK.to_string());
        }
        Ok(()) => lock_evidence(evidence).fail(
            "apply",
            "the admission reported success but the selected-text queue did not grow",
        ),
        Err(reason) => lock_evidence(evidence).fail("apply", &reason),
    }
}

/// Step 4: read the real status after the owner-tick drain and check it.
fn verify_step(app: &tauri::AppHandle, evidence: &Arc<Mutex<TextEditEvidence>>) {
    if lock_evidence(evidence).is_aborted() {
        lock_evidence(evidence).skip("verify");
        return;
    }
    let status = crate::host::cad_status(app.state::<ProbeShared>());
    let selection = &status.selection;
    let text = selection.text.clone();
    let tool = status.tool.as_ref().map(|tool| tool.key.clone());
    let tool_token = tool.clone().unwrap_or_else(|| "-".to_string());
    let error_token = status
        .error
        .as_deref()
        .map(flat)
        .unwrap_or_else(|| "-".to_string());
    eprintln!(
        "[probe] popup textedit evidence: after drain text={} expected={EXPECTED_STORED} \
         text_multiline={} entity_count={} rows={} row_keys={} tool={tool_token} error={error_token}",
        text.as_deref().map(short_flat).unwrap_or_else(|| "-".to_string()),
        bool_token(selection.text_multiline),
        optional_count(status.entity_count),
        selection.rows.len(),
        row_keys(&status),
    );

    let mut state = lock_evidence(evidence);
    state.text_after = Some(
        text.as_deref()
            .map(short_flat)
            .unwrap_or_else(|| "-".to_string()),
    );
    state.multiline_after = Some(selection.text_multiline);
    state.entities_after = status.entity_count;
    state.rows_after = Some(selection.rows.len());
    state.row_keys_after = Some(row_keys(&status));
    state.tool_after = Some(tool_token);
    state.error_after = Some(error_token);

    let text_ok = text.as_deref() == Some(EXPECTED_STORED);
    state.text_ok = Some(text_ok);
    if !text_ok {
        state.fail(
            "verify_text",
            &format!(
                "selection.text is {:?}, expected {EXPECTED_STORED:?} (the stored \\P form, not a \
                 raw newline)",
                text.as_deref()
            ),
        );
    }
    if !selection.text_multiline {
        state.fail(
            "verify_multiline",
            "selection.text_multiline is false after a multiline edit",
        );
    }
    match (state.entities_before, status.entity_count) {
        (Some(before), Some(after)) if before == after => {}
        (Some(before), Some(after)) => state.fail(
            "verify_entities",
            &format!("entityCount changed {before}->{after}; a text edit must not add/remove"),
        ),
        _ => state.fail(
            "verify_entities",
            "entityCount was not readable before and after the edit",
        ),
    }
    if tool.is_some() {
        state.fail(
            "verify_tool",
            "a tool is still active after the text edit (expected none)",
        );
    }
    if status.error.is_some() {
        state.fail(
            "verify_error",
            "cad_status.error is not null after the text edit",
        );
    }
}

/// Step 5: close through the host path `cad_close_popup` uses.
fn close_step(app: &tauri::AppHandle, evidence: &Arc<Mutex<TextEditEvidence>>) {
    if lock_evidence(evidence).is_aborted() {
        lock_evidence(evidence).skip("close");
        return;
    }
    match crate::host::close_popup_for_probe(app) {
        Ok(()) => {
            eprintln!(
                "[probe] popup textedit evidence: close ack accepted=true (the modal gate clears \
                 on the popup window's Destroyed event, not on this call)"
            );
            lock_evidence(evidence).close_ack = Some("accepted=true".into());
        }
        Err(error) => lock_evidence(evidence).fail("close", &error),
    }
}

/// Steps 6–7: `cad_status.popup` must be `null` again (one retry).
fn status_after_step(
    app: &tauri::AppHandle,
    evidence: &Arc<Mutex<TextEditEvidence>>,
    attempt: u32,
) {
    if lock_evidence(evidence).is_aborted() {
        lock_evidence(evidence).skip("status_after");
        return;
    }
    let status = crate::host::cad_status(app.state::<ProbeShared>());
    let text = describe_status_popup(&status);
    eprintln!(
        "[probe] popup textedit evidence: status after close attempt={attempt} popup={text} \
         (expected -)"
    );
    let mut state = lock_evidence(evidence);
    match status.popup {
        None => {
            state.status_after = Some(text);
            state.closed_confirmed = true;
        }
        Some(popup) => {
            if attempt >= 2 {
                state.fail(
                    "status_after",
                    &format!(
                        "cad_status.popup is still {}:{}:{} after the settle interval",
                        popup.kind, popup.label, popup.modal
                    ),
                );
            } else {
                eprintln!(
                    "[probe] popup textedit evidence: the gate has not cleared yet; retrying \
                     after the settle interval"
                );
            }
        }
    }
}

/// Step 8: a harmless input must be admitted again, and is removed again before
/// the owner tick can drain it (the probe proves admission, not execution).
fn readmit_step(app: &tauri::AppHandle, evidence: &Arc<Mutex<TextEditEvidence>>) {
    if lock_evidence(evidence).is_aborted() {
        lock_evidence(evidence).skip("readmission");
        return;
    }
    if !lock_evidence(evidence).closed_confirmed {
        lock_evidence(evidence).fail(
            "readmission",
            "re-admission was attempted before cad_status.popup was confirmed null",
        );
        return;
    }
    let shared = app.state::<ProbeShared>().0.clone();
    let input = probe_pointer_move(READMIT_INPUT_SEQUENCE);
    let (result, dequeued) = {
        let mut state = lock_shared(&shared);
        let result = queue_cad_input(&mut state.shell, &input);
        // Runs on the main thread while the lock is held, so the owner tick
        // cannot drain it in between.
        let dequeued = if result.is_ok() {
            state
                .shell
                .cad_inputs
                .pop()
                .is_some_and(|queued| queued.sequence == READMIT_INPUT_SEQUENCE)
        } else {
            false
        };
        (result, dequeued)
    };
    match result {
        Ok(()) if dequeued => {
            eprintln!(
                "[probe] popup textedit evidence: re-admission input accepted=true kind={} \
                 sequence={READMIT_INPUT_SEQUENCE} (removed from the queue before the owner tick)",
                input.kind.as_str()
            );
            lock_evidence(evidence).gate_released = Some(true);
        }
        Ok(()) => {
            eprintln!(
                "[probe] popup textedit evidence: re-admission input accepted=true \
                 dequeue=false (the queue tail was not the probe payload)"
            );
            lock_evidence(evidence).gate_released = Some(true);
        }
        Err(reason) => {
            eprintln!(
                "[probe] popup textedit evidence: re-admission input refused reason={reason}"
            );
            lock_evidence(evidence).gate_released = Some(false);
            lock_evidence(evidence).fail(
                "readmit_input",
                &format!("refused with {reason:?} after the popup closed"),
            );
        }
    }
}

/// Record and log the precondition skip. The summary then reports
/// `popup_textedit_skipped=<reason>` and `popup_textedit_pass=0`; no later step
/// runs and nothing is claimed.
fn report_skip(evidence: &Arc<Mutex<TextEditEvidence>>, reason: &str, detail: &str) {
    eprintln!(
        "[probe] popup textedit evidence: popup_textedit_skipped={reason} at precondition: {detail}"
    );
    lock_evidence(evidence).skipped = Some(reason.to_string());
}

fn lock_shared(shared: &Arc<Mutex<Shared>>) -> MutexGuard<'_, Shared> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_times_follow_the_explicit_start_and_hold() {
        let times = PhaseTimes::at(220, 20);
        assert_eq!(times.precondition, 220);
        assert_eq!(times.open, 221);
        assert_eq!(times.apply, 241);
        assert_eq!(times.verify, 243);
        assert_eq!(times.close, 244);
        assert_eq!(times.summary, 248);
        // The phase start floor is after the about phase's summary (close + 4);
        // the default start is after the draw driver's report; and the default
        // hold is never below its floor. Checked at compile time.
        const {
            assert!(MIN_START_AFTER_ABOUT_CLOSE >= 5);
        }
        const {
            assert!(
                DEFAULT_START_SECONDS > 213,
                "after the draw driver's report"
            );
        }
        const {
            assert!(DEFAULT_HOLD_SECONDS >= MIN_HOLD_SECONDS);
        }
    }
}
