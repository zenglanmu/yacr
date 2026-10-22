//! Step bodies for [`super::ui_dialogs`] (`YACR_PROBE_UI_DIALOGS`).
//!
//! Each function is one scheduled second of the driver's timeline: it performs
//! its own real action (a `Webview::eval` of the same JS the UI runs, a
//! main-thread `cad_status` read, or a host popup call) and logs exactly one
//! `[probe] ui-dialogs: t=<n>s …` line describing what it observed. They never
//! invent an outcome: on failure they record the step in
//! [`super::ui_dialogs_report::Evidence`] and return `Err`, which the scheduler
//! turns into a `FAILED` line.
//!
//! Split out of the driver so both files stay under the 1000-line code-file gate
//! (`scripts/check-file-size.py`, no whitelist).

use std::sync::{Arc, Mutex};

use cad_ui_core::session::CadCommandLogKind;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tauri::Manager;

use crate::host::ProbeShared;
use crate::main_thread::on_main_thread;

use super::tool::tool_click_world_js;
use super::ui_dialogs::{
    eval_wait, log, wait_for_chrome, L_CLOSE, L_CLOSE_READ, L_COMMAND_LOG, L_COMMAND_OPTIONS,
    L_MAIN_HANDLE, L_PICK_A, L_PICK_B, L_POPUP_HANDLE, L_POPUP_RETRY, L_PROPERTIES_KEY,
    L_PROPERTIES_READ, L_SELECT_CLICK, L_SELECT_READ,
};
use super::ui_dialogs_js;
use super::ui_dialogs_report::{
    command_log_line, flat, lock_evidence, measurement_value, popup_detail, selection_detail,
    selection_rows_complete, xid_detail, Evidence,
};

/// The world point of the selection click: the midpoint of the LINE at `x = 1`
/// in `fixtures/dwg/synthetic-four-lines.dwg` (four parallel vertical LINEs at
/// `x = 0,1,2,3` spanning `y = 0..1`).
const SELECT_WORLD: (f64, f64) = (1.0, 0.5);
/// The two `DIST` picks: endpoints of the fixture's LINE at `x = 0` and at
/// `x = 3`, so the measured distance is `sqrt(10) ≈ 3.1623`.
const PICK_A_WORLD: (f64, f64) = (0.0, 0.0);
const PICK_B_WORLD: (f64, f64) = (3.0, 1.0);
/// How many of the newest `commandLog` entries are printed (the host keeps up to
/// 200; the log line count is bounded so one run cannot flood the evidence log).
const COMMAND_LOG_SHOWN: usize = 12;

/// The real `cad_status`, read on the main thread (the session is
/// main-thread-local).
pub(super) fn read_status(
    app: &tauri::AppHandle,
) -> Result<cad_ui_core::session::CadStatus, String> {
    let app_main = app.clone();
    on_main_thread(app, move || {
        Ok(crate::host::cad_status(app_main.state::<ProbeShared>()))
    })
}

/// `at`: the main window's native handle, then `Ctrl+1` through the app's own
/// global handler (after confirming the chrome is mounted, so the keydown can
/// never be dispatched into a page that has no listener).
pub(super) fn properties_key(
    app: &tauri::AppHandle,
    webview: &tauri::Webview,
    evidence: &Arc<Mutex<Evidence>>,
    second: u64,
) -> Result<(), String> {
    log_main_handle(app, evidence, second)?;
    wait_for_chrome(webview)?;
    let result = eval_wait(webview, ui_dialogs_js::CTRL_1_JS)?;
    if result != "ctrl+1-dispatched-prevented" {
        lock_evidence(evidence).fail(
            "properties_key",
            &format!("Ctrl+1 was not handled: {result}"),
        );
    }
    log(
        second,
        &format!(
            "{L_PROPERTIES_KEY} dispatched result={result} (window keydown; the app's own Ctrl+1 \
             branch calls preventDefault)"
        ),
    );
    Ok(())
}

/// Log the main window's top-level native handle under this driver's prefix.
///
/// X11/XWayland yields the XID the external grab helper takes; native Wayland
/// has no XID (the compositor owns the toplevel), so the `wl_surface` pointer is
/// reported and the XID is explicitly `-`.
fn log_main_handle(
    app: &tauri::AppHandle,
    evidence: &Arc<Mutex<Evidence>>,
    second: u64,
) -> Result<(), String> {
    let app_main = app.clone();
    let detail = on_main_thread(app, move || {
        let Some(window) = app_main.get_window(crate::host::WINDOW_LABEL) else {
            return Err("the main window is not registered".to_string());
        };
        let handle = window
            .window_handle()
            .map_err(|error| format!("could not read the main window handle: {error}"))?;
        match handle.as_raw() {
            RawWindowHandle::Xlib(xlib) => Ok(xid_detail("main window", xlib.window)),
            RawWindowHandle::Xcb(xcb) => Ok(xid_detail("main window", xcb.window.get().into())),
            RawWindowHandle::Wayland(wayland) => Ok(format!(
                "main window wayland_surface={:p} xid=-",
                wayland.surface.as_ptr()
            )),
            other => Err(format!("unexpected main window raw handle: {other:?}")),
        }
    })?;
    if let Some(xid) = detail.strip_prefix("main window xid=") {
        lock_evidence(evidence).set_main_xid(xid.to_string());
    }
    log(second, &format!("{L_MAIN_HANDLE} {detail}"));
    Ok(())
}

/// `at+2`: the palette's DOM presence and the live CAD-partition rect.
pub(super) fn properties_read(
    webview: &tauri::Webview,
    evidence: &Arc<Mutex<Evidence>>,
    second: u64,
) -> Result<(), String> {
    let read = eval_wait(webview, ui_dialogs_js::PALETTE_READ_JS)?;
    let present = read.contains("palette=present");
    let canvas = read
        .split_whitespace()
        .find_map(|token| token.strip_prefix("canvas="))
        .map(str::to_string);
    lock_evidence(evidence).set_palette(present, canvas);
    log(second, &format!("{L_PROPERTIES_READ} {read}"));
    Ok(())
}

/// `at+5`: one synthetic canvas click on the fixture's LINE at `x = 1`.
///
/// Gated on the real `cad_status`: a drawing still loading (or one whose camera
/// is absent) is reported as not ready instead of clicking into an unready
/// viewport.
pub(super) fn select_click(
    app: &tauri::AppHandle,
    webview: &tauri::Webview,
    evidence: &Arc<Mutex<Evidence>>,
    second: u64,
) -> Result<(), String> {
    let status = read_status(app)?;
    let document = status.document_label.clone();
    lock_evidence(evidence).set_document(document.clone());
    let ready = !status.loading && status.camera.is_some() && status.entity_count.unwrap_or(0) > 0;
    if !ready {
        let detail = format!(
            "drawing not ready: document={} entity_count={:?} loading={} camera={}",
            document.as_deref().unwrap_or("-"),
            status.entity_count,
            status.loading,
            status.camera.is_some()
        );
        lock_evidence(evidence).fail("select", &detail);
        return Err(detail);
    }
    let script = tool_click_world_js(SELECT_WORLD.0, SELECT_WORLD.1, "SEL");
    let result = eval_wait(webview, &script)?;
    log(
        second,
        &format!(
            "{L_SELECT_CLICK} document={} world({:.3},{:.3}) \
             expect=fixture_line_at_x1_in_synthetic-four-lines.dwg result={result}",
            flat(document.as_deref().unwrap_or("-")),
            SELECT_WORLD.0,
            SELECT_WORLD.1
        ),
    );
    Ok(())
}

/// `at+7`: the real `cad_status.selection` (count, groups, row key/value pairs)
/// plus the synthetic-click read-back the template stores on `window`.
pub(super) fn select_read(
    app: &tauri::AppHandle,
    webview: &tauri::Webview,
    evidence: &Arc<Mutex<Evidence>>,
    second: u64,
) -> Result<(), String> {
    let readback = eval_wait(webview, ui_dialogs_js::CLICK_READBACK_JS)?;
    log(
        second,
        &format!("{L_SELECT_READ} click-readback {readback}"),
    );
    let status = read_status(app)?;
    let detail = selection_detail(&status.selection);
    let complete = selection_rows_complete(&status.selection);
    lock_evidence(evidence).set_selection(status.selection.count, complete);
    log(second, &format!("{L_SELECT_READ} {detail}"));
    // The palette only exists with a drawing open; report a read that carries no
    // rows at all instead of a silent pass.
    if status.selection.rows.is_empty() {
        lock_evidence(evidence).fail("selection", "cad_status.selection.rows is empty");
    }
    Ok(())
}

/// `at+12` (`DIST`) and `at+24` (`OPTIONS`): type the command into the real
/// field and press Enter through the field's own handler.
pub(super) fn type_command(
    webview: &tauri::Webview,
    evidence: &Arc<Mutex<Evidence>>,
    second: u64,
    command: &'static str,
    label: &str,
) -> Result<(), String> {
    let result = eval_wait(webview, &ui_dialogs_js::type_command_js(command))?;
    if !result.starts_with("typed+") {
        lock_evidence(evidence).fail(label, &result);
    }
    log(second, &format!("{label} {result}"));
    Ok(())
}

/// The `OPTIONS` step notes that the chrome intercepts the text before the
/// router, which is worth stating on the line itself.
pub(super) fn type_options(
    webview: &tauri::Webview,
    evidence: &Arc<Mutex<Evidence>>,
    second: u64,
) -> Result<(), String> {
    type_command(
        webview,
        evidence,
        second,
        "OPTIONS",
        &format!("{L_COMMAND_OPTIONS} (the chrome intercepts OPTIONS before the router)"),
    )
}

/// One `DIST` pick as a synthetic canvas click at a world point.
pub(super) fn pick(
    webview: &tauri::Webview,
    second: u64,
    world: (f64, f64),
    pointer_label: &'static str,
    step: &str,
) -> Result<(), String> {
    let script = tool_click_world_js(world.0, world.1, pointer_label);
    let result = eval_wait(webview, &script)?;
    log(
        second,
        &format!(
            "{step} world({:.3},{:.3}) result={result}",
            world.0, world.1
        ),
    );
    Ok(())
}

/// The `DIST` pick seconds and their world points.
pub(super) const PICKS: [(f64, f64, &str, &str); 2] = [
    (PICK_A_WORLD.0, PICK_A_WORLD.1, "P1", L_PICK_A),
    (PICK_B_WORLD.0, PICK_B_WORLD.1, "P2", L_PICK_B),
];

/// `at+20`: the real `cad_status.commandLog`, newest entries first-class: the
/// total, then one line per entry (sequence/kind/command/code/algorithm/value/
/// unit/precision/points).
pub(super) fn command_log(
    app: &tauri::AppHandle,
    evidence: &Arc<Mutex<Evidence>>,
    second: u64,
) -> Result<(), String> {
    let status = read_status(app)?;
    let entries = &status.command_log;
    let shown = entries.len().min(COMMAND_LOG_SHOWN);
    let first = entries.len() - shown;
    let dist_command = entries.iter().any(|entry| {
        entry.kind == CadCommandLogKind::Command && entry.command.as_deref() == Some("DIST")
    });
    let dist_value = entries.iter().rev().find_map(|entry| {
        let measurement = entry.measurement.as_ref()?;
        (entry.kind == CadCommandLogKind::Result && measurement.algorithm == "distance")
            .then(|| measurement_value(measurement.value))
    });
    lock_evidence(evidence).set_command_log(entries.len(), dist_command, dist_value);
    log(
        second,
        &format!(
            "{L_COMMAND_LOG} entries={} shown={} first_seq={}",
            entries.len(),
            shown,
            entries
                .get(first)
                .map(|entry| entry.sequence.to_string())
                .unwrap_or_else(|| "-".to_string())
        ),
    );
    for entry in entries.iter().skip(first) {
        log(second, &command_log_line(entry));
    }
    Ok(())
}

/// `at+27` (and one retry at `at+30`): the real `cad_status.popup` plus the
/// popup's in-DOM overlay identity.
///
/// The line is [`super::popup::log_popup_overlay_handle`]'s own
/// `[probe] popup evidence: … handle=dom_overlay xid=-`, because the overlay has
/// no window of its own; a whole-window capture contains it.
///
/// A popup that is still closed at the **first** read is not a failure (the
/// retry is scheduled for exactly that case); a popup that is still closed at
/// the **retry** is, and is recorded in the summary.
pub(super) fn popup_handle(
    app: &tauri::AppHandle,
    evidence: &Arc<Mutex<Evidence>>,
    second: u64,
    retry: bool,
) -> Result<(), String> {
    if retry && lock_evidence(evidence).popup_opened() {
        log(second, &format!("{L_POPUP_RETRY} skipped=already-read"));
        return Ok(());
    }
    let app_main = app.clone();
    let popup = on_main_thread(app, move || {
        let status = crate::host::cad_status(app_main.state::<ProbeShared>());
        Ok(popup_detail(&status))
    })?;
    let (handle, xid) = super::popup::log_popup_overlay_handle("dialogs popup");
    let (kind, xid) = (Some(handle), xid);
    lock_evidence(evidence).set_popup(popup.clone(), kind, xid.clone());
    let opened = lock_evidence(evidence).popup_opened();
    let label = if retry { L_POPUP_RETRY } else { L_POPUP_HANDLE };
    log(
        second,
        &format!(
            "{label} popup={popup} native={} xid={} opened={} retry_scheduled={}",
            kind.unwrap_or("-"),
            xid.as_deref().unwrap_or("-"),
            if opened { "1" } else { "0" },
            if opened || retry { "0" } else { "1" }
        ),
    );
    if opened {
        return Ok(());
    }
    if !retry {
        return Ok(());
    }
    lock_evidence(evidence).fail_popup_never_opened(&popup);
    Err(format!(
        "cad_status.popup is still null at the retry read ({popup})"
    ))
}

/// `at+24+hold`: the same host close path `cad_close_popup` runs.
pub(super) fn close(
    app: &tauri::AppHandle,
    evidence: &Arc<Mutex<Evidence>>,
    second: u64,
) -> Result<(), String> {
    let app_main = app.clone();
    if let Err(error) = on_main_thread(app, move || crate::host::close_popup_for_probe(&app_main)) {
        lock_evidence(evidence).fail("close", &error);
        return Err(format!("close failed: {error}"));
    }
    lock_evidence(evidence).set_close_ack();
    log(
        second,
        &format!(
            "{L_CLOSE} ack accepted=true (an explicit close clears the modal gate directly; no \
             window Destroyed event is involved)"
        ),
    );
    Ok(())
}

/// `close+2`: the real `cad_status.popup` must be back to `null`.
pub(super) fn popup_after_close(
    app: &tauri::AppHandle,
    evidence: &Arc<Mutex<Evidence>>,
    second: u64,
) -> Result<(), String> {
    let status = read_status(app)?;
    let detail = popup_detail(&status);
    lock_evidence(evidence).set_popup_after_close(detail.clone());
    log(
        second,
        &format!("{L_CLOSE_READ} popup={detail} (expected -)"),
    );
    Ok(())
}

/// `close+4`: the one summary line.
pub(super) fn summary(
    evidence: &Arc<Mutex<Evidence>>,
    second: u64,
    hold: u64,
) -> Result<(), String> {
    let state = lock_evidence(evidence);
    log(
        second,
        &format!("{} {}", super::ui_dialogs::L_SUMMARY, state.summary(hold)),
    );
    Ok(())
}
