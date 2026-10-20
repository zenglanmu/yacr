//! `YACR_PROBE_POPUP_EVIDENCE`: the modal-popup pipeline evidence driver
//! (ADR-0021 D1/D2).
//!
//! Evidence-only, default OFF, independent of every other probe driver. It drives
//! the **real** host popup pipeline against a real running host and proves, in
//! one bounded timeline, that the popup opens, that the host modal gate really
//! refuses input/commands while it is open, and that the gate is really released
//! after it closes.
//!
//! Unlike the drawing phases in [`super::draw`] this driver is **host-side**: the
//! popup is a native window with its own child webview (it cannot be a DOM
//! overlay over the opaque CAD child), so there is nothing to prove with synthetic
//! DOM pointer events. Every step therefore calls the same host functions the IPC
//! commands call, and reads the same `cad_status` the frontend reads.
//!
//! ## Timeline (seconds after driver start)
//!
//! * `t=3`: `cad_status` before (its `popup` must be `null`), then the open path
//!   `cad_open_popup` uses; log the ack, read `cad_status` again and log the
//!   `popup` block (`kind:label:modal`), then log the popup window's native handle
//!   on the distinctive line `[probe] popup evidence: window xid=0x…` so an
//!   external `apps/app-linux/scripts/x11-window-grab.c <xid-hex> <out.ppm>` can
//!   capture the popup while it is up.
//! * `t=4`: gate proof — the admission functions `probe_input` and `cad_command`
//!   use (`queue_cad_input`, `admit_command`) are exercised on the live shell.
//!   Both must refuse with `popup.modal_open` and must enqueue nothing.
//! * `t=3+hold`: the **same** close path `cad_close_popup` uses; log its ack.
//!   `hold` is `YACR_PROBE_POPUP_HOLD` seconds (default **30**), floored at
//!   [`MIN_HOLD_SECONDS`] so the external grab always has a window in which the
//!   popup is open.
//! * `t=3+hold+1` and `+2`: read `cad_status.popup` (must be `null`; the gate is
//!   cleared by the popup window's `Destroyed` event, so the driver retries once
//!   instead of assuming the event arrived instantly).
//! * `t=3+hold+3`: re-admission — the same two admission functions must now
//!   **accept**, which proves the gate is released rather than merely reported as
//!   released.
//! * `t=3+hold+4`: one summary line.
//! * from `YACR_PROBE_POPUP_TEXTEDIT_AT` (default `220`) onwards: the
//!   **text-edit** phase ([`super::popup_textedit`]), gated by this same variable
//!   but with its own start second and grab hold (the modal gate refuses CAD input
//!   while a popup is open, so stretching this phase's hold to reach the draw
//!   driver's selection would block that driver): precondition on the real
//!   selection, `textEdit` open with the host-derived payload, a replacement
//!   admitted through the real `cad_set_selected_text` queue and drained by the
//!   owner tick, a status verification (stored `\P` form, multiline, entity
//!   count, tool/error), close, and gate re-admission. It has its own
//!   `[probe] popup textedit evidence:` summary line.
//!
//! ## Honest limits
//!
//! The **trigger** is the probe driver, not a real mouse click on a ribbon
//! button, so the summary says `popup_synthetic=1` and
//! `popup_trigger=probe_driver_not_a_real_click`; everything downstream of the
//! trigger (window, gate, status projection, close) is the production path. The
//! XID line exists only on X11/XWayland: on native Wayland the popup is a
//! compositor-owned toplevel, so the driver logs its `wl_surface` pointer instead
//! and reports `popup_xid=-` (the X11 grab helper cannot target it). A screenshot
//! is only the window's own pixels; it is not a visual-correctness, precision or
//! cross-platform claim.
//!
//! ## Failure behaviour
//!
//! A step that fails its expectation logs `[probe] popup evidence: <step> FAILED:
//! …` immediately, records the step, and every later step logs `skipped` instead
//! of running. The summary always prints, with `popup_pass=0` and the failed
//! step named, so a run that stopped early can never read as a pass.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use cad_ui_core::protocol::{InputKind, ProbeInput};
use cad_ui_core::session::{CadPopup, CadStatus};
use cad_ui_core::shell::{admit_command, queue_cad_input, POPUP_MODAL_OPEN};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tauri::Manager;

use crate::host::{ProbeShared, Shared};

use super::schedule;

/// Default hold (seconds) between open and close. The hold exists so an
/// **external** X11 grab can capture the popup while it is open; it is not an
/// internal sleep and must not be shortened.
const DEFAULT_HOLD_SECONDS: u64 = 30;
/// Floor for `YACR_PROBE_POPUP_HOLD`. The gate proof runs at `t=4`, so a hold
/// below this would close the popup before/while the gate is exercised.
const MIN_HOLD_SECONDS: u64 = 5;
/// Driver-start second at which the popup is opened.
const OPEN_AT: u64 = 3;
/// Driver-start second at which the gate is exercised (inside the hold).
const GATE_AT: u64 = 4;
/// The allowlisted popup kind (the only one the host builds).
const POPUP_KIND: &str = "about";
/// Harmless command name for the command-admission probe.
///
/// It is a real vocabulary entry, so an accepted admission is a genuine
/// admission, while its only effect (a view fit) changes no document entity. The
/// re-admission step removes the admitted name from the queue again **before the
/// owner tick can drain it**, so the probe never executes a command; the summary
/// states that explicitly.
const PROBE_COMMAND: &str = "FIT";
/// Sequence number of the harmless pointer-move payload. It never reaches the
/// input tracker (only `queue_cad_input` is exercised), so it cannot collide with
/// the frontend's own sequence stream.
const PROBE_INPUT_SEQUENCE: u64 = 9_000_001;
/// Nominal CAD surface size the harmless pointer payload is expressed against.
const PROBE_SURFACE_WIDTH: f32 = 926.0;
const PROBE_SURFACE_HEIGHT: f32 = 341.0;

/// Whether the **popup evidence** driver is enabled
/// (`YACR_PROBE_POPUP_EVIDENCE`).
///
/// Evidence-only, default OFF, independent of every other probe driver.
pub fn popup_evidence_enabled() -> bool {
    matches!(
        std::env::var("YACR_PROBE_POPUP_EVIDENCE").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

/// The configured hold in seconds (`YACR_PROBE_POPUP_HOLD`, default 30),
/// floored at [`MIN_HOLD_SECONDS`]. An unparseable value is logged and replaced
/// by the default rather than silently accepted.
fn hold_seconds() -> u64 {
    let configured = match std::env::var("YACR_PROBE_POPUP_HOLD") {
        Ok(raw) => match raw.trim().parse::<u64>() {
            Ok(seconds) => seconds,
            Err(_) => {
                eprintln!(
                    "[probe] popup evidence: YACR_PROBE_POPUP_HOLD={raw:?} is not an integer \
                     number of seconds; using {DEFAULT_HOLD_SECONDS}"
                );
                DEFAULT_HOLD_SECONDS
            }
        },
        Err(_) => DEFAULT_HOLD_SECONDS,
    };
    if configured < MIN_HOLD_SECONDS {
        eprintln!(
            "[probe] popup evidence: hold {configured}s is below the {MIN_HOLD_SECONDS}s floor \
             (the external grab and the gate proof need the popup open); using {MIN_HOLD_SECONDS}s"
        );
        MIN_HOLD_SECONDS
    } else {
        configured
    }
}

/// Schedule the modal-popup evidence driver if enabled. Never panics on spawn
/// failure (a failed spawn is logged; the run continues unchanged).
///
/// The thread only calls the host's popup open/close accessors, the shell
/// admission helpers the IPC commands use, and the real `cad_status` command. No
/// popup behaviour, session logic or GPU state is changed.
pub fn maybe_spawn_popup_evidence(app: &tauri::AppHandle) {
    if !popup_evidence_enabled() {
        return;
    }
    let hold = hold_seconds();
    let textedit_start = super::popup_textedit::start_seconds(OPEN_AT + hold);
    let textedit_hold = super::popup_textedit::hold_seconds();
    let textedit = super::popup_textedit::PhaseTimes::at(textedit_start, textedit_hold);
    eprintln!(
        "[probe] popup evidence: enabled (YACR_PROBE_POPUP_EVIDENCE); open kind={POPUP_KIND} at \
         t={OPEN_AT}s, gate proof at t={GATE_AT}s, close at t={}s, re-admission at t={}s; \
         popup trigger is the probe driver, NOT a real click",
        OPEN_AT + hold,
        OPEN_AT + hold + 3
    );
    eprintln!(
        "[probe] popup textedit evidence: scheduled (same YACR_PROBE_POPUP_EVIDENCE) after the \
         about phase: precondition t={}s, open t={}s, apply t={}s, verify t={}s, close t={}s, \
         summary t={}s; hold={}s (YACR_PROBE_POPUP_TEXTEDIT_HOLD); it needs the selection left by \
         the earlier draw phases (MLEADER at t≈212s) and skips, never passes, without one",
        textedit.precondition,
        textedit.open,
        textedit.apply,
        textedit.verify,
        textedit.close,
        textedit.summary,
        textedit_hold,
    );
    let app = app.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("probe-popup-evidence".into())
        .spawn(move || run_popup_evidence(app, hold))
    {
        eprintln!("[probe] popup evidence: failed to spawn: {error}");
    }
}

/// Run the bounded popup timeline relative to driver start.
fn run_popup_evidence(app: tauri::AppHandle, hold: u64) {
    let start = Instant::now();
    let evidence = Arc::new(Mutex::new(PopupEvidence::default()));
    let close_at = OPEN_AT + hold;

    // ---- Open, read the status block, log the native handle ----
    {
        let step_app = app.clone();
        let evidence = evidence.clone();
        schedule(
            &start,
            OPEN_AT,
            "popup open + status + native handle",
            move || {
                let main_app = step_app.clone();
                step_app.run_on_main_thread(move || open_step(&main_app, &evidence))
            },
        );
    }

    // ---- Gate proof while the popup is open ----
    {
        let step_app = app.clone();
        let evidence = evidence.clone();
        schedule(
            &start,
            GATE_AT,
            "popup modal gate (input + command refused)",
            move || {
                let main_app = step_app.clone();
                step_app.run_on_main_thread(move || gate_step(&main_app, &evidence))
            },
        );
    }

    // ---- Close (the same host path cad_close_popup uses) ----
    {
        let step_app = app.clone();
        let evidence = evidence.clone();
        schedule(&start, close_at, "popup close", move || {
            let main_app = step_app.clone();
            step_app.run_on_main_thread(move || close_step(&main_app, &evidence))
        });
    }

    // ---- The gate clears on the window's Destroyed event: poll the real status ----
    for attempt in 1..=2u32 {
        let step_app = app.clone();
        let evidence = evidence.clone();
        schedule(
            &start,
            close_at + u64::from(attempt),
            "popup status after close",
            move || {
                let main_app = step_app.clone();
                step_app
                    .run_on_main_thread(move || status_after_step(&main_app, &evidence, attempt))
            },
        );
    }

    // ---- Re-admission: the gate must really be released ----
    {
        let step_app = app.clone();
        let evidence = evidence.clone();
        schedule(
            &start,
            close_at + 3,
            "popup re-admission (input + command accepted)",
            move || {
                let main_app = step_app.clone();
                step_app.run_on_main_thread(move || readmit_step(&main_app, &evidence))
            },
        );
    }

    // ---- One summary line (main-thread queued, so it cannot print before the
    //      re-admission step that was queued ahead of it) ----
    {
        let step_app = app.clone();
        let evidence = evidence.clone();
        schedule(&start, close_at + 4, "popup summary", move || {
            step_app.run_on_main_thread(move || {
                let evidence = lock_evidence(&evidence);
                eprintln!("[probe] popup evidence: {}", summary_line(&evidence, hold));
            })
        });
    }

    // ---- Phase 2: the text-edit popup (`textEdit`), after the `about` phase.
    //      Same env gate, but its own start second and grab hold: the modal gate
    //      refuses CAD input while a popup is open, so stretching the `about`
    //      hold to reach the draw driver's selection would block that driver. ----
    let textedit_start = super::popup_textedit::start_seconds(close_at);
    let textedit_hold = super::popup_textedit::hold_seconds();
    let textedit = super::popup_textedit::PhaseTimes::at(textedit_start, textedit_hold);
    super::popup_textedit::schedule_textedit_phase(&app, &start, textedit, textedit_hold);
}

/// Step 1–3: status before, open through the host path, status while open, and
/// the popup's top-level native handle.
fn open_step(app: &tauri::AppHandle, evidence: &Arc<Mutex<PopupEvidence>>) {
    let before = crate::host::cad_status(app.state::<ProbeShared>());
    let before_text = describe_status_popup(&before);
    eprintln!("[probe] popup evidence: status before popup={before_text} (expected -)");

    if before.popup.is_some() {
        lock_evidence(evidence).fail("status_before", "popup block was not null before open");
        return;
    }
    lock_evidence(evidence).status_before = Some(before_text);

    let ack = match crate::host::open_popup_for_probe(app, POPUP_KIND) {
        Ok(ack) => ack,
        Err(error) => {
            lock_evidence(evidence).fail("open", &error);
            return;
        }
    };
    eprintln!(
        "[probe] popup evidence: open ack accepted={} kind={} label={} existing={}",
        ack.accepted, ack.kind, ack.popup_label, ack.existing
    );
    lock_evidence(evidence).open_ack = Some(format!(
        "accepted={}_existing={}",
        ack.accepted, ack.existing
    ));

    let during = crate::host::cad_status(app.state::<ProbeShared>());
    let during_text = describe_status_popup(&during);
    let expected = format!("{}:{}:true", ack.kind, ack.popup_label);
    eprintln!("[probe] popup evidence: status open popup={during_text} (expected {expected})");
    {
        let mut state = lock_evidence(evidence);
        state.status_open = Some(during_text.clone());
        if during.popup.is_none() {
            state.fail(
                "status_open",
                "cad_status.popup is null while the popup window is open",
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
                let mut state = lock_evidence(evidence);
                state.handle = Some(handle);
                state.xid = xid;
            }
            None => lock_evidence(evidence)
                .fail("window_handle", "no reportable top-level native handle"),
        },
        None => lock_evidence(evidence).fail(
            "window_handle",
            "the popup window is not registered after a successful open",
        ),
    }
}

/// Step 4: both admission functions must refuse with `popup.modal_open`.
fn gate_step(app: &tauri::AppHandle, evidence: &Arc<Mutex<PopupEvidence>>) {
    if lock_evidence(evidence).is_aborted() {
        lock_evidence(evidence).skip("gate");
        return;
    }
    let shared = app.state::<ProbeShared>().0.clone();
    let input = probe_pointer_move(PROBE_INPUT_SEQUENCE);

    let (input_result, inputs_before, inputs_after) = {
        let mut state = lock_shared(&shared);
        let before = state.shell.cad_inputs.len();
        let result = queue_cad_input(&mut state.shell, &input);
        let after = state.shell.cad_inputs.len();
        (result, before, after)
    };
    eprintln!(
        "[probe] popup evidence: gate input probe kind={} sequence={PROBE_INPUT_SEQUENCE} \
         queue={inputs_before}->{inputs_after}",
        input.kind.as_str()
    );
    match input_result {
        Err(reason) if reason == POPUP_MODAL_OPEN => {
            eprintln!(
                "[probe] popup evidence: gate input refused reason={POPUP_MODAL_OPEN} \
                 (nothing queued)"
            );
            lock_evidence(evidence).input_refused = Some(reason);
        }
        Err(other) => lock_evidence(evidence).fail(
            "gate_input",
            &format!("refused with {other:?}, expected {POPUP_MODAL_OPEN}"),
        ),
        Ok(()) => lock_evidence(evidence).fail(
            "gate_input",
            "the input was admitted while the modal popup was open",
        ),
    }
    if inputs_after != inputs_before {
        lock_evidence(evidence).fail(
            "gate_input",
            "the refused input still grew the CAD input queue",
        );
    }

    let (command_result, commands_before, commands_after) = {
        let mut state = lock_shared(&shared);
        let before = state.shell.cad_commands.len();
        let result = admit_command(&mut state.shell, PROBE_COMMAND);
        let after = state.shell.cad_commands.len();
        (result, before, after)
    };
    eprintln!(
        "[probe] popup evidence: gate command probe name={PROBE_COMMAND} \
         queue={commands_before}->{commands_after}"
    );
    match command_result {
        Err(reason) if reason == POPUP_MODAL_OPEN => {
            eprintln!(
                "[probe] popup evidence: gate command refused reason={POPUP_MODAL_OPEN} \
                 (nothing queued)"
            );
            lock_evidence(evidence).command_refused = Some(reason);
        }
        Err(other) => lock_evidence(evidence).fail(
            "gate_command",
            &format!("refused with {other:?}, expected {POPUP_MODAL_OPEN}"),
        ),
        Ok(()) => lock_evidence(evidence).fail(
            "gate_command",
            "the command was admitted while the modal popup was open",
        ),
    }
    if commands_after != commands_before {
        lock_evidence(evidence).fail(
            "gate_command",
            "the refused command still grew the CAD command queue",
        );
    }
}

/// Step 6: close through the host path `cad_close_popup` uses.
fn close_step(app: &tauri::AppHandle, evidence: &Arc<Mutex<PopupEvidence>>) {
    if lock_evidence(evidence).is_aborted() {
        lock_evidence(evidence).skip("close");
        return;
    }
    match crate::host::close_popup_for_probe(app) {
        Ok(()) => {
            eprintln!(
                "[probe] popup evidence: close ack accepted=true (the modal gate clears on the \
                 popup window's Destroyed event, not on this call)"
            );
            lock_evidence(evidence).close_ack = Some("accepted=true".into());
        }
        Err(error) => lock_evidence(evidence).fail("close", &error),
    }
}

/// Steps 6–7: the status block must be `null` again (one retry for the
/// `Destroyed` event latency).
fn status_after_step(app: &tauri::AppHandle, evidence: &Arc<Mutex<PopupEvidence>>, attempt: u32) {
    if lock_evidence(evidence).is_aborted() {
        lock_evidence(evidence).skip("status_after");
        return;
    }
    let status = crate::host::cad_status(app.state::<ProbeShared>());
    let text = describe_status_popup(&status);
    eprintln!(
        "[probe] popup evidence: status after close attempt={attempt} popup={text} (expected -)"
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
                    "[probe] popup evidence: the gate has not cleared yet; retrying after the \
                     settle interval"
                );
            }
        }
    }
}

/// Step 8: the same two admission functions must now accept, and the command
/// probe must not reach the owner tick.
fn readmit_step(app: &tauri::AppHandle, evidence: &Arc<Mutex<PopupEvidence>>) {
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
    let input = probe_pointer_move(PROBE_INPUT_SEQUENCE);

    let input_result = {
        let mut state = lock_shared(&shared);
        queue_cad_input(&mut state.shell, &input)
    };
    match input_result {
        Ok(()) => {
            eprintln!("[probe] popup evidence: re-admission input accepted=true reason=none");
            lock_evidence(evidence).input_admitted = Some(true);
        }
        Err(reason) => {
            eprintln!("[probe] popup evidence: re-admission input refused reason={reason}");
            let mut state = lock_evidence(evidence);
            state.input_admitted = Some(false);
            state.fail(
                "readmit_input",
                &format!("refused with {reason:?} after the popup closed"),
            );
        }
    }

    let (command_result, dequeued) = {
        let mut state = lock_shared(&shared);
        let result = admit_command(&mut state.shell, PROBE_COMMAND);
        // The probe proves admission; it must not execute a command. This runs on
        // the main thread while the lock is held, so the owner tick cannot drain
        // the queue in between.
        let dequeued = if result.is_ok() {
            state
                .shell
                .cad_commands
                .pop()
                .is_some_and(|name| name == PROBE_COMMAND)
        } else {
            false
        };
        (result, dequeued)
    };
    match command_result {
        Ok(()) if dequeued => {
            eprintln!(
                "[probe] popup evidence: re-admission command accepted=true name={PROBE_COMMAND} \
                 (removed from the queue before the owner tick)"
            );
            let mut state = lock_evidence(evidence);
            state.command_admitted = Some(true);
            state.command_dequeued = Some(true);
        }
        Ok(()) => {
            let mut state = lock_evidence(evidence);
            state.command_admitted = Some(true);
            state.command_dequeued = Some(false);
            state.fail(
                "readmit_command",
                "admitted command was not the queue tail; not removed",
            );
        }
        Err(reason) => {
            eprintln!("[probe] popup evidence: re-admission command refused reason={reason}");
            let mut state = lock_evidence(evidence);
            state.command_admitted = Some(false);
            state.fail(
                "readmit_command",
                &format!("refused with {reason:?} after the popup closed"),
            );
        }
    }
}

/// The harmless pointer payload used for both input admissions.
///
/// `pub(super)`: the text-edit phase in [`super::popup_textedit`] reuses the same
/// payload with its own sequence, so the two admissions can never be confused.
pub(super) fn probe_pointer_move(sequence: u64) -> ProbeInput {
    ProbeInput {
        sequence,
        kind: InputKind::PointerMove,
        x: Some(PROBE_SURFACE_WIDTH / 2.0),
        y: Some(PROBE_SURFACE_HEIGHT / 2.0),
        dx: None,
        dy: None,
        width: Some(PROBE_SURFACE_WIDTH),
        height: Some(PROBE_SURFACE_HEIGHT),
        dpr: None,
        button: None,
        shift: None,
    }
}

/// Log the popup window's top-level native handle and return
/// `(kind label, xid)`.
///
/// This is the popup-specific equivalent of the main window's
/// `host::telemetry::log_top_level_native_handle` (same
/// `window_handle()`/`RawWindowHandle` logic), but the line is distinctive so the
/// capture script can extract exactly the popup's XID:
/// `[probe] popup evidence: window xid=0x…`. On native Wayland there is no XID
/// (the compositor owns the toplevel), so the `wl_surface` pointer is logged and
/// no X11 grab is claimed.
///
/// `pub(super)`: the text-edit phase in [`super::popup_textedit`] logs the same
/// line for its own window.
pub(super) fn log_popup_native_handle(
    window: &tauri::Window,
) -> Option<(&'static str, Option<String>)> {
    let handle = match window.window_handle() {
        Ok(handle) => handle,
        Err(error) => {
            eprintln!("[probe] popup evidence: could not read the popup window handle: {error}");
            return None;
        }
    };
    match handle.as_raw() {
        RawWindowHandle::Xlib(xlib) => {
            let xid = format!("0x{:x}", xlib.window);
            eprintln!("[probe] popup evidence: window xid={xid}");
            Some(("x11_xid", Some(xid)))
        }
        RawWindowHandle::Xcb(xcb) => {
            let xid = format!("0x{:x}", xcb.window.get());
            eprintln!("[probe] popup evidence: window xid={xid}");
            Some(("x11_xid", Some(xid)))
        }
        RawWindowHandle::Wayland(wayland) => {
            eprintln!(
                "[probe] popup evidence: window wayland_surface={:p} xid=-",
                wayland.surface.as_ptr()
            );
            Some(("wayland_surface", None))
        }
        other => {
            eprintln!("[probe] popup evidence: unexpected popup raw handle: {other:?}");
            None
        }
    }
}

/// `cad_status.popup` as `kind:label:modal`, or `-` when it is `null`.
pub(super) fn describe_status_popup(status: &CadStatus) -> String {
    status
        .popup
        .as_ref()
        .map(describe_popup)
        .unwrap_or_else(|| "-".to_string())
}

fn describe_popup(popup: &CadPopup) -> String {
    format!("{}:{}:{}", popup.kind, popup.label, popup.modal)
}

/// The driver's accumulated reads; all values are real host/`cad_status` reads.
#[derive(Default)]
struct PopupEvidence {
    status_before: Option<String>,
    open_ack: Option<String>,
    status_open: Option<String>,
    input_refused: Option<String>,
    command_refused: Option<String>,
    close_ack: Option<String>,
    status_after: Option<String>,
    input_admitted: Option<bool>,
    command_admitted: Option<bool>,
    command_dequeued: Option<bool>,
    xid: Option<String>,
    handle: Option<&'static str>,
    closed_confirmed: bool,
    failed: Vec<String>,
    aborted: Option<String>,
}

impl PopupEvidence {
    fn is_aborted(&self) -> bool {
        self.aborted.is_some()
    }

    /// Log a failed step at once and remember it; the first failure is the abort
    /// point every later step checks.
    fn fail(&mut self, step: &str, detail: &str) {
        eprintln!("[probe] popup evidence: {step} FAILED: {detail}");
        self.failed.push(format!("{step}={}", flat(detail)));
        if self.aborted.is_none() {
            self.aborted = Some(step.to_string());
        }
    }

    /// A later step that did not run because an earlier one failed.
    fn skip(&mut self, step: &str) {
        let at = self.aborted.clone().unwrap_or_else(|| "-".to_string());
        eprintln!("[probe] popup evidence: {step} skipped (aborted at {at})");
    }
}

/// Build the single summary line from the recorded reads.
///
/// `-` means "absent / never read"; `popup_pass` is derived from the reads and is
/// `0` whenever any step is missing, wrong, or an earlier step failed.
fn summary_line(evidence: &PopupEvidence, hold: u64) -> String {
    let text = |value: &Option<String>| {
        value
            .as_deref()
            .map(flat)
            .unwrap_or_else(|| "-".to_string())
    };
    let flag = |value: Option<bool>| match value {
        Some(true) => "1",
        Some(false) => "0",
        None => "-",
    };

    let opened = u8::from(
        evidence.open_ack.is_some()
            && evidence
                .status_open
                .as_deref()
                .is_some_and(|value| value != "-"),
    );
    let closed =
        u8::from(evidence.close_ack.is_some() && evidence.status_after.as_deref() == Some("-"));
    let pass = u8::from(
        evidence.aborted.is_none()
            && opened == 1
            && closed == 1
            && evidence.status_before.as_deref() == Some("-")
            && evidence.input_refused.as_deref() == Some(POPUP_MODAL_OPEN)
            && evidence.command_refused.as_deref() == Some(POPUP_MODAL_OPEN)
            && evidence.input_admitted == Some(true)
            && evidence.command_admitted == Some(true)
            && evidence.handle.is_some(),
    );
    let failed = if evidence.failed.is_empty() {
        "-".to_string()
    } else {
        evidence.failed.join(",")
    };
    let aborted = evidence
        .aborted
        .as_deref()
        .map(flat)
        .unwrap_or_else(|| "-".to_string());

    format!(
        "popup_synthetic=1 popup_opened={opened} popup_status_before={} \
         popup_status_open={} popup_input_refused={} popup_command_refused={} \
         popup_closed={closed} popup_status_after={} popup_input_admitted={} \
         popup_command_admitted={} popup_xid={} popup_pass={pass} popup_hold_s={hold} \
         popup_open_ack={} popup_close_ack={} popup_native_handle={} \
         popup_command_dequeued={} popup_trigger=probe_driver_not_a_real_click \
         popup_gate_channel=queue_cad_input_and_admit_command_the_ipc_admission_functions \
         popup_command_probe=admit_command_then_dequeue_before_the_owner_tick \
         popup_step_failed={failed} popup_aborted_at={aborted}",
        text(&evidence.status_before),
        text(&evidence.status_open),
        text(&evidence.input_refused),
        text(&evidence.command_refused),
        text(&evidence.status_after),
        flag(evidence.input_admitted),
        flag(evidence.command_admitted),
        text(&evidence.xid),
        text(&evidence.open_ack),
        text(&evidence.close_ack),
        evidence.handle.unwrap_or("-"),
        flag(evidence.command_dequeued),
    )
}

/// Collapse whitespace so the summary stays one whitespace-free field list.
fn flat(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join("_")
}

fn lock_evidence(evidence: &Arc<Mutex<PopupEvidence>>) -> MutexGuard<'_, PopupEvidence> {
    evidence
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn lock_shared(shared: &Arc<Mutex<Shared>>) -> MutexGuard<'_, Shared> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every read the pass criterion needs, as it looks after a clean run.
    fn passing() -> PopupEvidence {
        PopupEvidence {
            status_before: Some("-".into()),
            open_ack: Some("accepted=true_existing=false".into()),
            status_open: Some("about:popup-1:true".into()),
            input_refused: Some(POPUP_MODAL_OPEN.into()),
            command_refused: Some(POPUP_MODAL_OPEN.into()),
            close_ack: Some("accepted=true".into()),
            status_after: Some("-".into()),
            input_admitted: Some(true),
            command_admitted: Some(true),
            command_dequeued: Some(true),
            xid: Some("0x1".into()),
            handle: Some("x11_xid"),
            closed_confirmed: true,
            failed: Vec::new(),
            aborted: None,
        }
    }

    #[test]
    fn summary_is_one_line_with_the_required_fields_on_a_clean_run() {
        let line = summary_line(&passing(), 30);
        assert!(!line.contains('\n'), "one summary line: {line}");
        for expected in [
            "popup_synthetic=1",
            "popup_opened=1",
            "popup_status_before=-",
            "popup_status_open=about:popup-1:true",
            "popup_input_refused=popup.modal_open",
            "popup_command_refused=popup.modal_open",
            "popup_closed=1",
            "popup_status_after=-",
            "popup_input_admitted=1",
            "popup_command_admitted=1",
            "popup_xid=0x1",
            "popup_pass=1",
        ] {
            assert!(line.contains(expected), "missing {expected}: {line}");
        }
    }

    #[test]
    fn any_missing_or_failed_read_clears_the_pass_flag() {
        let mut wrong_before = passing();
        wrong_before.status_before = Some("about:popup-1:true".into());
        assert!(summary_line(&wrong_before, 30).contains("popup_pass=0"));

        let mut wrong_reason = passing();
        wrong_reason.input_refused = Some("cad_waiting".into());
        assert!(summary_line(&wrong_reason, 30).contains("popup_pass=0"));

        let mut still_open = passing();
        still_open.status_after = Some("about:popup-1:true".into());
        assert!(summary_line(&still_open, 30).contains("popup_pass=0"));

        let mut aborted = passing();
        aborted.aborted = Some("close".into());
        assert!(summary_line(&aborted, 30).contains("popup_pass=0"));
    }
}
