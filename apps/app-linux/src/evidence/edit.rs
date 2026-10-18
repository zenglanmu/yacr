//! `YACR_PROBE_EDIT_EVIDENCE`: the TRIM/EXTEND entity-pick evidence driver.
//!
//! Evidence-only, default OFF, and independent of every other probe driver
//! (setting only this one does not trigger the input/tool/hover/lifecycle/box
//! passes). It drives the **real** interactive edit path against a real running
//! host: start `TRIM` through the normal `cad_command` selector, click an empty
//! point (the capture-time entity pick must refuse it without advancing), click
//! the target and the boundary entities through **synthetic DOM pointer events**
//! on `.native-input`, dispatch `CONFIRM`, and read the **real** `cad_status`
//! before and after.
//!
//! After the TRIM phase it proves a **committed document change** with the one
//! change observable the status channel exposes: a primary click selects one
//! real entity (idle selection through the same synthetic DOM convention), then
//! the real `ERASE` selector removes it. A working path must drop `entityCount`
//! by exactly one and leave the selection empty; if it does not, the report says
//! so plainly.
//!
//! Synthetic input, **not a real mouse**: the events are dispatched on
//! `.native-input` and never touch the kernel/evdev/XInput path. Every reported
//! number is a real `cad_status` read. `cad_status` exposes **no** per-entity
//! geometry and no drawing revision, so the only honest change observables are
//! `entityCount`, the capture state (`tool`), the selection count and the
//! surfaced technical `error`. The report says so instead of inventing a
//! geometry channel.
//!
//! Fixture note (verified against `fixtures/dwg/synthetic-four-lines.dwg`
//! through the importer contract): the drawing is **four mutually parallel
//! vertical LINEs** at world x = 0, 1, 2, 3 spanning y = 0..1. There is **no**
//! horizontal boundary and **no** below-y=0 segment. A TRIM boundary is
//! therefore parallel to every candidate target (explicit no-intersection
//! refusal, nothing changes) and EXTEND has no non-parallel boundary to reach.
//! The driver reports exactly that; it does not tune the run to look successful.

use std::time::{Duration, Instant};

/// Whether the **TRIM/EXTEND edit evidence** driver is enabled
/// (`YACR_PROBE_EDIT_EVIDENCE`).
///
/// Evidence-only, default OFF, independent of every other probe driver.
///
/// Synthetic input, **not a real mouse**: the events are DOM pointer dispatches
/// on `.native-input`; the kernel/evdev/XInput path is never touched.
pub fn edit_evidence_enabled() -> bool {
    matches!(
        std::env::var("YACR_PROBE_EDIT_EVIDENCE").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

/// Schedule the TRIM/EXTEND edit evidence driver if enabled. Never panics on
/// spawn failure (a failed spawn is logged; the run continues unchanged).
///
/// The thread only dispatches `Webview::eval` scripts that call the public
/// `cad_status`/`cad_command` commands and dispatch synthetic DOM pointer
/// events. No command/session logic is changed and no GPU state is touched.
///
/// The host-init entry point lives in [`super::tool`] (the evidence subsystem is
/// self-contained this round, so no file outside `src/evidence/` is touched
/// except the module declaration in `evidence.rs`).
pub fn maybe_spawn_edit_evidence(webview: Option<&tauri::Webview>) {
    if !edit_evidence_enabled() {
        return;
    }
    let Some(webview) = webview else {
        eprintln!(
            "[probe] edit evidence: no child webview; TRIM/extend pass skipped (nothing faked)"
        );
        return;
    };
    eprintln!(
        "[probe] edit evidence: enabled (YACR_PROBE_EDIT_EVIDENCE); synthetic DOM pointer events, NOT a real mouse; \
         fixture note: four parallel vertical LINEs x=0,1,2,3 y=0..1 (no horizontal boundary, no below-y=0 segment)"
    );
    let webview = webview.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("probe-edit-evidence".into())
        .spawn(move || run_edit_evidence(webview))
    {
        eprintln!("[probe] edit evidence: failed to spawn: {error}");
    }
}

/// Run the edit evidence sequence relative to driver start:
///
/// * t=10s read the real `cad_status` baseline (`entityCount`, `tool`,
///   `selection.count`, `camera`);
/// * t=12s start the real `TRIM` selector through `cad_command`;
/// * t=13s read `cad_status` (capture must be `draw.trim` with 0 points);
/// * t=14s click **empty space** at world (0.5, 0.5): the capture-time entity
///   pick must refuse it immediately and **not** advance the capture;
/// * t=15s read `cad_status` (points unchanged, refusal in `error`);
/// * t=16s click the target vertical at world (0, 0.5);
/// * t=17s read `cad_status` (points = 1);
/// * t=18s click the boundary vertical at world (1, 0.5);
/// * t=19s read `cad_status` (points = 2, ready = true);
/// * t=20s dispatch `CONFIRM` (drawing tools never auto-commit);
/// * t=22s read `cad_status` after the command layer's real outcome;
/// * t=23s `CANCEL` so a refused capture is cleared;
/// * t=24s read `cad_status` (idle);
/// * t=25s primary-click select the LINE at x = 3 (world (3, 0.5)); no tool is
///   active, so the click is a real replace-selection;
/// * t=26s read `cad_status` (`erase_before`: `entityCount`,
///   `selection.count`);
/// * t=27s dispatch the real `ERASE` selector through `cad_command`;
/// * t=28s read `cad_status` (`erase_after`: `entityCount`, `selection.count`);
/// * t=29s log one single-line report via the eval callback.
///
/// Everything is bounded so an external `--capture-at 30` still lands on an
/// idle host (now after the ERASE), exactly as it did before the phase existed.
///
/// Every step is a dispatch only; the **real** outcome is the `cad_status`
/// snapshot read from the host, never a value synthesized here. Pointer
/// positions are computed from the **app's own** `cad_status.camera` (centre +
/// world-per-logical-pixel) and the live `.native-input` rect, not from
/// hardcoded screen pixels.
fn run_edit_evidence(webview: tauri::Webview) {
    let start = Instant::now();

    schedule_edit(&start, 10, "baseline cad_status", || {
        webview.eval(edit_status_js("s_base"))
    });

    schedule_edit(&start, 12, "cad_command TRIM", || {
        webview.eval(edit_start_js("TRIM"))
    });
    schedule_edit(&start, 13, "trim-started cad_status", || {
        webview.eval(edit_status_js("s_trim_started"))
    });

    let miss = edit_click_world_js(0.5, 0.5, "miss");
    schedule_edit(
        &start,
        14,
        "miss click at world (0.5, 0.5) during the target step",
        || webview.eval(miss),
    );
    schedule_edit(
        &start,
        15,
        "miss cad_status (capture must not advance)",
        || webview.eval(edit_status_js("s_miss")),
    );

    let target = edit_click_world_js(0.0, 0.5, "target");
    schedule_edit(
        &start,
        16,
        "target click at world (0, 0.5) (LINE x=0)",
        || webview.eval(target),
    );
    schedule_edit(&start, 17, "target cad_status", || {
        webview.eval(edit_status_js("s_target"))
    });

    let boundary = edit_click_world_js(1.0, 0.5, "boundary");
    schedule_edit(
        &start,
        18,
        "boundary click at world (1, 0.5) (LINE x=1)",
        || webview.eval(boundary),
    );
    schedule_edit(&start, 19, "boundary cad_status", || {
        webview.eval(edit_status_js("s_boundary"))
    });

    schedule_edit(&start, 20, "cad_command CONFIRM", || {
        webview.eval(EDIT_CONFIRM_JS)
    });
    schedule_edit(&start, 22, "confirmed cad_status", || {
        webview.eval(edit_status_js("s_confirmed"))
    });

    schedule_edit(&start, 23, "cad_command CANCEL", || {
        webview.eval(EDIT_CANCEL_JS)
    });
    schedule_edit(&start, 24, "cancelled cad_status", || {
        webview.eval(edit_status_js("s_cancelled"))
    });

    // ERASE phase: prove a real committed document change. The TRIM phase left
    // the tool idle; an idle primary click is a replace-selection, so this uses
    // the same synthetic pointer convention as the TRIM picks.
    let erase_select = edit_click_world_js(3.0, 0.5, "erase_select");
    schedule_edit(
        &start,
        25,
        "erase select click at world (3, 0.5) (LINE x=3)",
        || webview.eval(erase_select),
    );
    schedule_edit(&start, 26, "erase-before cad_status", || {
        webview.eval(edit_status_js("s_erase_before"))
    });
    schedule_edit(&start, 27, "cad_command ERASE", || {
        webview.eval(EDIT_ERASE_JS)
    });
    schedule_edit(&start, 28, "erase-after cad_status", || {
        webview.eval(edit_status_js("s_erase_after"))
    });

    // The report is logged from Rust through the eval callback; the JS also
    // `console.log`s it. A dispatch that merely posts is not a pass — the report
    // carries the real before/after reads and the real refusals.
    schedule_edit(&start, 29, "edit evidence report", || {
        webview.eval_with_callback(EDIT_REPORT_JS, |payload| {
            eprintln!(
                "[probe] edit evidence report: {}",
                decode_edit_report(&payload)
            );
        })
    });
}

/// Decode the JSON-quoted eval-callback payload into the raw report string.
fn decode_edit_report(payload: &str) -> String {
    let trimmed = payload.trim();
    if trimmed.starts_with('"') {
        if let Ok(decoded) = serde_json::from_str::<String>(trimmed) {
            return decoded;
        }
    }
    trimmed.to_string()
}

/// Sleep until `seconds` after `start`, run `action`, and log the dispatch
/// result under the edit evidence prefix. Only the **dispatch** is reported
/// here; the observable outcome comes from the `cad_status` reads.
fn schedule_edit(
    start: &Instant,
    seconds: u64,
    label: &str,
    action: impl FnOnce() -> tauri::Result<()>,
) {
    let target = Duration::from_secs(seconds);
    let elapsed = start.elapsed();
    if target > elapsed {
        std::thread::sleep(target - elapsed);
    }
    match action() {
        Ok(()) => eprintln!("[probe] edit evidence step: {label} -> dispatched"),
        Err(error) => eprintln!("[probe] edit evidence step: {label} -> dispatch error: {error}"),
    }
}

/// Build the start script for one tool selector (`TRIM` / `EXTEND`).
fn edit_start_js(tool: &str) -> String {
    EDIT_START_TEMPLATE.replace("__TOOL__", tool)
}

/// Build the named status-read script (`base`, `miss`, `target`, …).
fn edit_status_js(name: &str) -> String {
    EDIT_STATUS_TEMPLATE.replace("__NAME__", name)
}

/// Build one synthetic click at a **world** point: the script reads the real
/// `cad_status.camera` and the live `.native-input` rect and converts the world
/// point to rect fractions before dispatching the pointer events. Synthetic,
/// **not a real mouse**.
fn edit_click_world_js(wx: f64, wy: f64, label: &str) -> String {
    EDIT_CLICK_TEMPLATE
        .replace("__WX__", &wx.to_string())
        .replace("__WY__", &wy.to_string())
        .replace("__LABEL__", label)
}

/// Read the real `cad_status` and record it under `__NAME__`.
///
/// `points` is the tool's captured-point count, `ready` its confirmability and
/// `error` the real technical error string (the surfaced refusal, if any).
const EDIT_STATUS_TEMPLATE: &str = r#"
(function () {
  var e = window.__yacr_edit = window.__yacr_edit || {};
  var name = '__NAME__';
  try {
    var internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {
      e[name + '_failed'] = 'no-tauri-internals';
      return 'no-tauri-internals';
    }
    internals.invoke('cad_status').then(function (s) {
      e[name + '_done'] = true;
      e[name + '_count'] = s.entityCount;
      e[name + '_tool'] = s.tool ? s.tool.key : null;
      e[name + '_points'] = s.tool ? s.tool.points : -1;
      e[name + '_ready'] = s.tool ? !!s.tool.ready : false;
      e[name + '_selection'] = s.selection ? s.selection.count : null;
      e[name + '_error'] = s.error;
      if (s.camera && s.camera.center && s.camera.center.length === 2) {
        e.camera = [s.camera.center[0], s.camera.center[1], s.camera.scale];
      }
    }).catch(function (err) { e[name + '_failed'] = String(err); });
    return name + '-dispatched';
  } catch (err) {
    try { e[name + '_failed'] = 'fatal:' + err; } catch (_) {}
    return 'error:' + err;
  }
})()
"#;

/// Start the real interactive edit tool through the public `cad_command`
/// command. `__TOOL__` is the canonical selector (`TRIM` / `EXTEND`).
const EDIT_START_TEMPLATE: &str = r#"
(function () {
  var e = window.__yacr_edit = window.__yacr_edit || {};
  try {
    var internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {
      e.start_failed = 'no-tauri-internals';
      return 'no-tauri-internals';
    }
    internals.invoke('cad_command', { command: '__TOOL__' }).then(function (ack) {
      e.start_ack = (ack && typeof ack.accepted !== 'undefined')
        ? 'accepted=' + ack.accepted
        : String(JSON.stringify(ack));
    }).catch(function (err) { e.start_failed = String(err); });
    return 'start-dispatched';
  } catch (err) {
    try { e.start_failed = 'fatal:' + err; } catch (_) {}
    return 'error:' + err;
  }
})()
"#;

/// Confirm the started edit tool through the public `cad_command` command.
///
/// Only `CONFIRM` asks the command layer to commit the captured picks; the two
/// clicks alone leave the database untouched. Synthetic input, **not a real
/// mouse**.
const EDIT_CONFIRM_JS: &str = r#"
(function () {
  var e = window.__yacr_edit = window.__yacr_edit || {};
  try {
    var internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {
      e.confirm_failed = 'no-tauri-internals';
      return 'no-tauri-internals';
    }
    internals.invoke('cad_command', { command: 'CONFIRM' }).then(function (ack) {
      e.confirm_ack = (ack && typeof ack.accepted !== 'undefined')
        ? 'accepted=' + ack.accepted
        : String(JSON.stringify(ack));
    }).catch(function (err) { e.confirm_failed = String(err); });
    return 'confirm-dispatched';
  } catch (err) {
    try { e.confirm_failed = 'fatal:' + err; } catch (_) {}
    return 'error:' + err;
  }
})()
"#;

/// Cancel the edit tool through the public `cad_command` command so a refused
/// capture does not leak into the rest of the run.
const EDIT_CANCEL_JS: &str = r#"
(function () {
  var e = window.__yacr_edit = window.__yacr_edit || {};
  try {
    var internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {
      e.cancel_failed = 'no-tauri-internals';
      return 'no-tauri-internals';
    }
    internals.invoke('cad_command', { command: 'CANCEL' }).then(function (ack) {
      e.cancel_ack = (ack && typeof ack.accepted !== 'undefined')
        ? 'accepted=' + ack.accepted
        : String(JSON.stringify(ack));
    }).catch(function (err) { e.cancel_failed = String(err); });
    return 'cancel-dispatched';
  } catch (err) {
    try { e.cancel_failed = 'fatal:' + err; } catch (_) {}
    return 'error:' + err;
  }
})()
"#;

/// Erase the current selection through the public `cad_command` command.
///
/// `ERASE` is a non-interactive command: it consumes the session selection that
/// the preceding idle click established. A real removal is observable as an
/// `entityCount` drop of one.
const EDIT_ERASE_JS: &str = r#"
(function () {
  var e = window.__yacr_edit = window.__yacr_edit || {};
  try {
    var internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {
      e.erase_failed = 'no-tauri-internals';
      return 'no-tauri-internals';
    }
    internals.invoke('cad_command', { command: 'ERASE' }).then(function (ack) {
      e.erase_ack = (ack && typeof ack.accepted !== 'undefined')
        ? 'accepted=' + ack.accepted
        : String(JSON.stringify(ack));
    }).catch(function (err) { e.erase_failed = String(err); });
    return 'erase-dispatched';
  } catch (err) {
    try { e.erase_failed = 'fatal:' + err; } catch (_) {}
    return 'error:' + err;
  }
})()
"#;

/// One synthetic click at a **world** point.
///
/// Reads the real `cad_status.camera` (`center`, world-per-logical-pixel) and
/// the live `.native-input` rect, converts the world point to rect fractions,
/// then dispatches `pointermove` (hover), `pointerdown` (button 0 / buttons 1)
/// and `pointerup` (button 0 / buttons 0). Pointer capture is shimmed because
/// these events are not backed by a device. `__WX__`/`__WY__` are world
/// coordinates; `__LABEL__` names the click. Synthetic, **not a real mouse**.
const EDIT_CLICK_TEMPLATE: &str = r#"
(function () {
  var e = window.__yacr_edit = window.__yacr_edit || {};
  var label = '__LABEL__';
  try {
    var el = document.querySelector('.native-input');
    if (!el) { e[label + '_failed'] = 'no-element'; return 'no-element'; }
    var r = el.getBoundingClientRect();
    if (!(r.width > 20 && r.height > 20)) {
      e[label + '_failed'] = 'tiny-rect:' + r.width + 'x' + r.height;
      return 'tiny-rect:' + r.width + 'x' + r.height;
    }
    var internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {
      e[label + '_failed'] = 'no-tauri-internals';
      return 'no-tauri-internals';
    }
    internals.invoke('cad_status').then(function (s) {
      if (!s.camera || !(s.camera.scale > 0)) {
        e[label + '_failed'] = 'no-camera';
        return;
      }
      var cx = s.camera.center[0];
      var cy = s.camera.center[1];
      var sc = s.camera.scale;
      var fx = 0.5 + (__WX__ - cx) / (sc * r.width);
      var fy = 0.5 - (__WY__ - cy) / (sc * r.height);
      e[label + '_world'] = [__WX__, __WY__];
      e[label + '_frac'] = [fx, fy];
      e.camera = [cx, cy, sc];
      e.rect = [r.left, r.top, r.width, r.height];
      if (!(fx >= 0 && fx <= 1 && fy >= 0 && fy <= 1)) {
        e[label + '_failed'] = 'outside-canvas:' + fx + ',' + fy;
        return;
      }
      el.setPointerCapture = function () {};
      el.hasPointerCapture = function () { return false; };
      el.releasePointerCapture = function () {};
      function fire(type, x, y, button, buttons) {
        el.dispatchEvent(new PointerEvent(type, {
          pointerId: 1, pointerType: 'mouse', isPrimary: true,
          button: button, buttons: buttons,
          clientX: x, clientY: y, bubbles: true, cancelable: true
        }));
      }
      var x = r.left + r.width * fx;
      var y = r.top + r.height * fy;
      fire('pointermove', x, y, -1, 0);
      fire('pointerdown', x, y, 0, 1);
      fire('pointerup', x, y, 0, 0);
      e.clicks = (e.clicks || 0) + 1;
      e[label + '_px'] = [Math.round(x - r.left), Math.round(y - r.top)];
      e[label + '_dispatched'] = true;
    }).catch(function (err) { e[label + '_failed'] = String(err); });
    return label + '-dispatched';
  } catch (err) {
    try { e[label + '_failed'] = 'fatal:' + err; } catch (_) {}
    return 'error:' + err;
  }
})()
"#;

/// Single-line report over `window.__yacr_edit`.
///
/// Fields are whitespace-flattened so the line stays tokenizable; `-` means
/// absent and `-1` a value that was never read. `entity_delta` is only a real
/// number when both counts were read. `miss_advanced` is derived from the real
/// capture-point counts. `extend_demonstrable=false` is the honest fixture
/// conclusion (no non-parallel boundary entity exists), not a failed attempt.
const EDIT_REPORT_JS: &str = r#"
(function () {
  var e = window.__yacr_edit || {};
  function v(x) {
    if (x === null || typeof x === 'undefined') { return '-'; }
    try { return String(x).replace(/\s+/g, '_'); } catch (err) { return '?'; }
  }
  function n(x) { return (typeof x === 'number') ? x : -1; }
  function arr(x) {
    if (!x || !x.length) { return '-'; }
    try { return x.map(function (k) { return String(k); }).join(','); } catch (err) { return '?'; }
  }
  var before = n(e.s_base_count);
  var after = n(e.s_confirmed_count);
  var delta = (before >= 0 && after >= 0) ? (after - before) : -1;
  var miss_advanced = n(e.s_miss_points) > n(e.s_trim_started_points);
  var erase_before = n(e.s_erase_before_count);
  var erase_after = n(e.s_erase_after_count);
  var erase_delta = (erase_before >= 0 && erase_after >= 0) ? (erase_after - erase_before) : -1;
  var erase_selection_ok = (n(e.s_erase_before_selection) === 1 &&
    n(e.s_erase_after_selection) === 0);
  var status_failed = e.s_base_failed || e.s_trim_started_failed || e.s_miss_failed ||
    e.s_target_failed || e.s_boundary_failed || e.s_confirmed_failed ||
    e.s_cancelled_failed || e.s_erase_before_failed || e.s_erase_after_failed;
  var text = 'trim_synthetic=1_not_a_real_mouse' +
    ' before_count=' + v(e.s_base_count) +
    ' after_count=' + v(e.s_confirmed_count) +
    ' entity_delta=' + delta +
    ' before_selection=' + v(e.s_base_selection) +
    ' after_selection=' + v(e.s_confirmed_selection) +
    ' trim_start_ack=' + v(e.start_ack) +
    ' trim_started_tool=' + v(e.s_trim_started_tool) +
    ' trim_started_points=' + v(e.s_trim_started_points) +
    ' miss_points_before=' + v(e.s_trim_started_points) +
    ' miss_points_after=' + v(e.s_miss_points) +
    ' miss_advanced=' + miss_advanced +
    ' miss_error=' + v(e.s_miss_error) +
    ' target_points=' + v(e.s_target_points) +
    ' target_tool=' + v(e.s_target_tool) +
    ' target_error=' + v(e.s_target_error) +
    ' boundary_points=' + v(e.s_boundary_points) +
    ' boundary_ready=' + v(e.s_boundary_ready) +
    ' boundary_error=' + v(e.s_boundary_error) +
    ' confirm_ack=' + v(e.confirm_ack) +
    ' confirmed_tool=' + v(e.s_confirmed_tool) +
    ' confirmed_points=' + v(e.s_confirmed_points) +
    ' confirmed_error=' + v(e.s_confirmed_error) +
    ' cancel_ack=' + v(e.cancel_ack) +
    ' cancelled_tool=' + v(e.s_cancelled_tool) +
    ' erase_before_count=' + v(e.s_erase_before_count) +
    ' erase_before_selection=' + v(e.s_erase_before_selection) +
    ' erase_ack=' + v(e.erase_ack) +
    ' erase_after_count=' + v(e.s_erase_after_count) +
    ' erase_after_selection=' + v(e.s_erase_after_selection) +
    ' erase_count_delta=' + erase_delta +
    ' erase_selection_ok=' + erase_selection_ok +
    ' erase_after_error=' + v(e.s_erase_after_error) +
    ' erase_world=' + arr(e.erase_select_world) +
    ' erase_frac=' + arr(e.erase_select_frac) +
    ' camera=' + arr(e.camera) +
    ' rect=' + arr(e.rect) +
    ' miss_world=' + arr(e.miss_world) +
    ' miss_frac=' + arr(e.miss_frac) +
    ' target_world=' + arr(e.target_world) +
    ' target_frac=' + arr(e.target_frac) +
    ' boundary_world=' + arr(e.boundary_world) +
    ' boundary_frac=' + arr(e.boundary_frac) +
    ' extend_demonstrable=false' +
    ' extend_reason=fixture_has_no_non_parallel_boundary' +
    ' geometry_channel=none_cad_status_has_no_entity_geometry_or_revision' +
    ' clicks=' + n(e.clicks) +
    ' start_failed=' + v(e.start_failed) +
    ' confirm_failed=' + v(e.confirm_failed) +
    ' cancel_failed=' + v(e.cancel_failed) +
    ' miss_failed=' + v(e.miss_failed) +
    ' target_failed=' + v(e.target_failed) +
    ' boundary_failed=' + v(e.boundary_failed) +
    ' erase_failed=' + v(e.erase_failed) +
    ' erase_select_failed=' + v(e.erase_select_failed) +
    ' status_failed=' + v(status_failed);
  try { console.log('[probe] edit evidence report: ' + text); } catch (err) {}
  return text;
})()
"#;
