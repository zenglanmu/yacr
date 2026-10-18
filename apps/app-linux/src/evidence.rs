//! Env-gated, evidence-only lifecycle driver for the real-window probe run.
//!
//! Enabled only when `YACR_PROBE_EVIDENCE` is truthy; a normal run is
//! unaffected. On a background thread it schedules (relative to driver start):
//! - synthetic input injected through the real webview (`Webview::eval`),
//!   explicitly **synthetic, not a real mouse**;
//! - a window resize;
//! - a minimize, then an unminimize (restore);
//! - a final resize.
//!
//! The thread only calls public `Window`/`Webview` dispatch methods. The GPU
//! presenter still runs on the main-thread tick, and all GPU work stays inside
//! `SurfaceProbePresenter`; this module touches no GPU state.

use std::time::{Duration, Instant};

/// Whether the evidence driver is enabled.
pub fn enabled() -> bool {
    matches!(
        std::env::var("YACR_PROBE_EVIDENCE").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

/// Schedule the evidence lifecycle if enabled. Never panics on spawn failure.
pub fn maybe_spawn(window: &tauri::Window, webview: Option<&tauri::Webview>) {
    if !enabled() {
        return;
    }
    let Some(webview) = webview else {
        eprintln!("[probe] evidence driver: no child webview; synthetic input skipped");
        return;
    };
    eprintln!("[probe] evidence driver enabled (YACR_PROBE_EVIDENCE)");
    let window = window.clone();
    let webview = webview.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("probe-evidence".into())
        .spawn(move || run(window, webview))
    {
        eprintln!("[probe] evidence driver failed to spawn: {error}");
    }
}

fn run(window: tauri::Window, webview: tauri::Webview) {
    let start = Instant::now();
    schedule(
        &start,
        8,
        "synthetic input (eval; NOT a real mouse)",
        || webview.eval(SYNTHETIC_INPUT_JS),
    );
    schedule(&start, 18, "resize grow to 1200x800", || {
        let before = window.inner_size().map(|size| (size.width, size.height));
        let result = window.set_size(tauri::PhysicalSize::new(1200, 800));
        eprintln!("[probe] resize grow before={before:?} result={result:?}");
        result
    });
    schedule(&start, 28, "minimize", || window.minimize());
    schedule(&start, 34, "unminimize (restore)", || window.unminimize());
    schedule(&start, 40, "resize shrink to 800x500", || {
        let before = window.inner_size().map(|size| (size.width, size.height));
        let result = window.set_size(tauri::PhysicalSize::new(800, 500));
        eprintln!("[probe] resize shrink before={before:?} result={result:?}");
        result
    });
}

/// Sleep until `seconds` after `start`, then run `action` and log the result.
fn schedule(
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
        Ok(()) => eprintln!("[probe] evidence step: {label} -> ok"),
        Err(error) => eprintln!("[probe] evidence step: {label} -> error: {error}"),
    }
}

/// Whether the lifecycle driver is enabled and its mode.
fn lifecycle_mode() -> Option<String> {
    match std::env::var("YACR_PROBE_LIFECYCLE") {
        Ok(value) => {
            let value = value.trim().to_lowercase();
            if value.is_empty() || value == "0" || value == "false" {
                None
            } else {
                Some(value)
            }
        }
        Err(_) => None,
    }
}

/// Schedule the evidence-only **lifecycle** driver if enabled. Unlike the input
/// driver this drives real window/resize/minimize/close/exit plus a forced
/// overlay unrealize→realize; it is env-gated and never runs on a normal path.
pub fn maybe_spawn_lifecycle(window: &tauri::Window, app: &tauri::AppHandle) {
    let Some(mode) = lifecycle_mode() else {
        return;
    };
    eprintln!("[probe] lifecycle driver enabled mode={mode}");
    let window = window.clone();
    let app = app.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("probe-lifecycle".into())
        .spawn(move || run_lifecycle(window, app, mode))
    {
        eprintln!("[probe] lifecycle driver failed to spawn: {error}");
    }
}

fn run_lifecycle(window: tauri::Window, app: tauri::AppHandle, mode: String) {
    let start = Instant::now();

    // `hold`: minimize once and do NOTHING else (no Tauri unminimize/set_focus/
    // show). The external harness then restores the window via a real WM/OS path
    // (_NET_ACTIVE_WINDOW / task switch), so the restore is not an app API call.
    if mode == "hold" {
        let w_min = window.clone();
        schedule(
            &start,
            8,
            "minimize (hold; no app-side restore)",
            move || w_min.minimize(),
        );
        let w_close = window.clone();
        schedule(&start, 45, "window.close()", move || w_close.close());
        return;
    }

    let do_resize = matches!(mode.as_str(), "full" | "resize" | "close" | "exit");
    let do_minimize = matches!(mode.as_str(), "full" | "minimize");
    let do_unrealize = matches!(mode.as_str(), "full" | "unrealize");

    if do_resize {
        let window = window.clone();
        schedule(&start, 8, "resize grow to 1200x800", move || {
            let before = window.inner_size().map(|size| (size.width, size.height));
            let result = window.set_size(tauri::PhysicalSize::new(1200, 800));
            eprintln!("[probe] resize grow before={before:?} result={result:?}");
            result
        });
    }
    if do_minimize {
        let w_min = window.clone();
        schedule(&start, 12, "minimize", move || w_min.minimize());
        let w_unmin = window.clone();
        schedule(&start, 15, "unminimize (restore)", move || {
            w_unmin.unminimize()
        });
        if mode == "minimize" {
            // Diagnostic: try the usual extra restore remedies.
            let w_focus = window.clone();
            schedule(&start, 17, "set_focus", move || w_focus.set_focus());
            let w_show = window.clone();
            schedule(&start, 19, "show", move || w_show.show());
        }
    }
    if do_resize {
        let window = window.clone();
        schedule(&start, 19, "resize shrink to 900x560", move || {
            let before = window.inner_size().map(|size| (size.width, size.height));
            let result = window.set_size(tauri::PhysicalSize::new(900, 560));
            eprintln!("[probe] resize shrink before={before:?} result={result:?}");
            result
        });
    }
    if do_unrealize {
        schedule_main(&start, 24, &app, "force overlay unrealize", || {
            crate::host::force_unrealize()
        });
        schedule_main(&start, 26, &app, "force overlay realize/map", || {
            crate::host::force_realize()
        });
    }
    if mode == "exit" {
        let app = app.clone();
        schedule(&start, 32, "app.exit(0)", move || {
            app.exit(0);
            Ok(())
        });
    } else {
        let window = window.clone();
        schedule(&start, 32, "window.close()", move || window.close());
    }
}

/// Sleep until `seconds` after `start`, then run a main-thread `action` and log
/// its boolean result (a dispatch that merely posts is not a pass).
fn schedule_main(
    start: &Instant,
    seconds: u64,
    app: &tauri::AppHandle,
    label: &str,
    action: impl FnOnce() -> bool + Send + 'static,
) {
    let target = Duration::from_secs(seconds);
    let elapsed = start.elapsed();
    if target > elapsed {
        std::thread::sleep(target - elapsed);
    }
    let label = label.to_string();
    let label_in = label.clone();
    match app.run_on_main_thread(move || {
        let ok = action();
        eprintln!("[probe] lifecycle step: {label_in} -> ok={ok}");
    }) {
        Ok(()) => {}
        Err(error) => eprintln!("[probe] lifecycle step: {label} -> dispatch error: {error}"),
    }
}

/// Whether the **post-load pan** diagnostic driver is enabled
/// (`YACR_PROBE_POSTLOAD_PAN`). Default OFF; independent of the input/lifecycle
/// drivers (setting only this does not trigger them).
pub fn postload_pan_enabled() -> bool {
    matches!(
        std::env::var("YACR_PROBE_POSTLOAD_PAN").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

/// Post-load pan gesture script (synthetic DOM primary gesture, **not** a real
/// mouse). Returns immediately with the start diagnostics; the gesture itself is
/// rAF-paced for `DURATION` ms (with a timer stall-guard so a throttled/absent
/// rAF cannot stall it) and **always** ends with a `pointerup`. All counts are
/// recorded on `window.__yacr_pan` for the host's RUNID-tracked report poll.
pub const POSTLOAD_PAN_JS: &str = r#"
(function () {
  try {
    var RUN_ID = (window.__yacr_pan_run || 0) + 1;
    window.__yacr_pan_run = RUN_ID;
    var el = document.querySelector('.native-input');
    if (!el) { window.__yacr_pan = { run_id: RUN_ID, failed: 'no-element' }; return 'no-element'; }
    var r = el.getBoundingClientRect();
    if (!(r.width > 20 && r.height > 20)) {
      window.__yacr_pan = { run_id: RUN_ID, failed: 'tiny-rect' };
      return 'tiny-rect:' + r.width + 'x' + r.height;
    }
    el.setPointerCapture = function () {};
    el.hasPointerCapture = function () { return false; };
    el.releasePointerCapture = function () {};
    var y = r.top + r.height / 2;
    var x0 = r.left + r.width * 0.1;
    var x1 = r.left + r.width * 0.9;
    var DURATION = 8000;
    var pan = { run_id: RUN_ID, start: performance.now(), end: -1, moves: 0, downs: 0, ups: 0,
                done: false, failed: '', rect: [r.left, r.top, r.width, r.height],
                duration_ms: DURATION, last_step: performance.now() };
    window.__yacr_pan = pan;
    function fire(type, x, buttons) {
      el.dispatchEvent(new PointerEvent(type, {
        pointerId: 1, pointerType: 'mouse', isPrimary: true, buttons: buttons,
        clientX: x, clientY: y, bubbles: true, cancelable: true
      }));
    }
    function end(x) {
      if (pan.done) { return; }
      pan.done = true; pan.end = performance.now();
      try { fire('pointerup', x, 0); pan.ups++; }
      catch (e) { pan.failed = 'up:' + e; }
      try { clearInterval(timer); } catch (e) {}
    }
    function step() {
      if (pan.done) { return; }
      var now = performance.now();
      var t = (now - pan.start) / DURATION;
      if (t >= 1) { end(x1); return; }
      var p = t < 0.5 ? t * 2 : (1 - t) * 2;
      try { fire('pointermove', x0 + (x1 - x0) * p, 1); pan.moves++; pan.last_step = now; }
      catch (e) { pan.failed = 'move:' + e; }
    }
    try { fire('pointerdown', x0, 1); pan.downs++; } catch (e) { pan.failed = 'down:' + e; }
    function raf() { if (!pan.done) { step(); requestAnimationFrame(raf); } }
    requestAnimationFrame(raf);
    // Stall guard: if rAF is throttled/absent, keep the gesture advancing.
    var timer = setInterval(function () {
      if (pan.done) { clearInterval(timer); return; }
      if (performance.now() - pan.last_step > 100) { step(); }
    }, 50);
    // Hard guarantee: end with a pointerup even if every loop stalled.
    setTimeout(function () { end(x1); }, DURATION + 500);
    return 'started run=' + RUN_ID + ' rect=' + pan.rect.join(',') + ' dur=' + DURATION;
  } catch (e) { window.__yacr_pan = { failed: 'fatal:' + e }; return 'error:' + e; }
})()
"#;

/// Structured report poll: returns the RUNID + gesture counts/elapsed/failed flag.
/// The host parses this (a tracked marker via the eval response) and never treats
/// a view change alone as gesture completion.
pub const POSTLOAD_PAN_REPORT_JS: &str = r#"
(function () {
  try {
    var p = window.__yacr_pan;
    if (!p) { return 'no-gesture'; }
    var elapsed = (p.end >= 0 && p.start >= 0) ? Math.round(p.end - p.start) : -1;
    return 'run=' + (p.run_id || 0) + ' done=' + (!!p.done) + ' moves=' + (p.moves || 0) +
           ' downs=' + (p.downs || 0) + ' ups=' + (p.ups || 0) +
           ' elapsed_ms=' + elapsed + ' failed=' + (p.failed || '');
  } catch (e) { return 'error:' + e; }
})()
"#;

/// Whether the **interactive tool evidence** driver is enabled
/// (`YACR_PROBE_TOOL_EVIDENCE`).
///
/// Evidence-only, default OFF, and independent of the `YACR_PROBE_EVIDENCE`
/// driver (setting only this one does not trigger the input/resize/lifecycle
/// pass). The driver proves the newly wired interactive tool path end-to-end
/// against a **real running host**: start `LINE`, place two points through
/// **synthetic DOM pointer events**, `CONFIRM` the commit (drawing tools never
/// auto-commit), and read the **real** `cad_status` `entityCount`/`tool`
/// before and after.
///
/// Synthetic input, **not a real mouse**: the events are dispatched on
/// `.native-input` and never touch the kernel/evdev/XInput path. The pass
/// criterion is a real `entityCount` delta plus `tool` returning to `null`;
/// nothing here fakes a result.
pub fn tool_evidence_enabled() -> bool {
    matches!(
        std::env::var("YACR_PROBE_TOOL_EVIDENCE").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

/// Schedule the interactive tool evidence driver if enabled. Never panics on
/// spawn failure (a failed spawn is logged; the run continues unchanged).
///
/// The thread only dispatches `Webview::eval` scripts that call the public
/// `cad_status`/`cad_command` commands and dispatch synthetic DOM pointer
/// events. No command/session logic is changed and no GPU state is touched.
pub fn maybe_spawn_tool_evidence(webview: Option<&tauri::Webview>) {
    if !tool_evidence_enabled() {
        return;
    }
    let Some(webview) = webview else {
        eprintln!(
            "[probe] tool evidence: no child webview; LINE click pass skipped (nothing faked)"
        );
        return;
    };
    eprintln!(
        "[probe] tool evidence: enabled (YACR_PROBE_TOOL_EVIDENCE); synthetic DOM input, NOT a real mouse"
    );
    let webview = webview.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("probe-tool-evidence".into())
        .spawn(move || run_tool_evidence(webview))
    {
        eprintln!("[probe] tool evidence: failed to spawn: {error}");
    }
}

/// Run the evidence sequence relative to driver start (6 steps):
/// t=10s read the real `cad_status` baseline, t=12s admit `LINE`, t=14s/15s
/// click two points inside `.native-input`, t=16s admit `CONFIRM` (drawing
/// tools never auto-commit), t=18s read the real `cad_status` after the commit
/// can land, t=20s log one single-line report via the eval callback.
///
/// Every step is a dispatch only; the **real** outcome is the `cad_status`
/// snapshot read from the host, never a value synthesized here.
fn run_tool_evidence(webview: tauri::Webview) {
    let start = Instant::now();

    schedule_tool(&start, 10, "before cad_status", || {
        webview.eval(TOOL_BEFORE_JS)
    });
    schedule_tool(&start, 12, "cad_command LINE", || {
        webview.eval(TOOL_START_JS)
    });

    let click_a = tool_click_js(0.30, 0.40, "A");
    schedule_tool(&start, 14, "click A at 30%/40% of .native-input", || {
        webview.eval(click_a)
    });

    let click_b = tool_click_js(0.65, 0.60, "B");
    schedule_tool(&start, 15, "click B at 65%/60% of .native-input", || {
        webview.eval(click_b)
    });

    schedule_tool(&start, 16, "cad_command CONFIRM", || {
        webview.eval(TOOL_CONFIRM_JS)
    });

    schedule_tool(&start, 18, "after cad_status", || {
        webview.eval(TOOL_AFTER_JS)
    });
    // The report is logged from Rust through the eval callback; the JS also
    // `console.log`s it. A dispatch that merely posts is not a pass — the
    // report carries the real before/after counts.
    schedule_tool(&start, 20, "tool evidence report", || {
        webview.eval_with_callback(TOOL_REPORT_JS, |payload| {
            eprintln!(
                "[probe] tool evidence report: {}",
                decode_tool_report(&payload)
            );
        })
    });
}

/// Decode the JSON-quoted eval-callback payload into the raw report string.
fn decode_tool_report(payload: &str) -> String {
    let trimmed = payload.trim();
    if trimmed.starts_with('"') {
        if let Ok(decoded) = serde_json::from_str::<String>(trimmed) {
            return decoded;
        }
    }
    trimmed.to_string()
}

/// Sleep until `seconds` after `start`, run `action`, and log the dispatch
/// result under the tool evidence prefix. Only the **dispatch** is reported
/// here; the observable outcome comes from the `cad_status` reads.
fn schedule_tool(
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
        Ok(()) => eprintln!("[probe] tool evidence step: {label} -> dispatched"),
        Err(error) => eprintln!("[probe] tool evidence step: {label} -> dispatch error: {error}"),
    }
}

/// Build one synthetic click script (hover, press, release) at a fraction of the
/// `.native-input` rect. Synthetic, **not a real mouse**.
fn tool_click_js(fx: f64, fy: f64, label: &str) -> String {
    TOOL_CLICK_TEMPLATE
        .replace("__FX__", &fx.to_string())
        .replace("__FY__", &fy.to_string())
        .replace("__LABEL__", label)
}

/// Baseline: record the real `cad_status` `entityCount`/`tool` before `LINE`.
const TOOL_BEFORE_JS: &str = r#"
(function () {
  var t = window.__yacr_tool = window.__yacr_tool || {};
  try {
    t.started = Date.now();
    var internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {
      t.before_failed = 'no-tauri-internals';
      return 'no-tauri-internals';
    }
    internals.invoke('cad_status').then(function (s) {
      t.before_count = s.entityCount;
      t.before_tool = s.tool ? s.tool.key : null;
    }).catch(function (e) { t.before_failed = String(e); });
    return 'before-dispatched';
  } catch (e) {
    try { t.before_failed = 'fatal:' + e; } catch (_) {}
    return 'error:' + e;
  }
})()
"#;

/// Start the real interactive tool through the public `cad_command` command.
const TOOL_START_JS: &str = r#"
(function () {
  var t = window.__yacr_tool = window.__yacr_tool || {};
  try {
    var internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {
      t.start_failed = 'no-tauri-internals';
      return 'no-tauri-internals';
    }
    internals.invoke('cad_command', { command: 'LINE' }).then(function (ack) {
      t.start_ack = (ack && typeof ack.accepted !== 'undefined')
        ? 'accepted=' + ack.accepted
        : String(JSON.stringify(ack));
    }).catch(function (e) { t.start_failed = String(e); });
    return 'line-dispatched';
  } catch (e) {
    try { t.start_failed = 'fatal:' + e; } catch (_) {}
    return 'error:' + e;
  }
})()
"#;

/// Confirm the started drawing tool through the public `cad_command` command.
///
/// Drawing tools never auto-commit (`SessionState::capture_point` returns
/// `Ok(false)` for `ToolState::Drawing`; only measurements auto-ready), so the
/// two captured points alone leave `entityCount` unchanged. `CONFIRM` is what
/// actually commits the line. Synthetic input, **not a real mouse**.
const TOOL_CONFIRM_JS: &str = r#"
(function () {
  var t = window.__yacr_tool = window.__yacr_tool || {};
  try {
    var internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {
      t.confirm_failed = 'no-tauri-internals';
      return 'no-tauri-internals';
    }
    internals.invoke('cad_command', { command: 'CONFIRM' }).then(function (ack) {
      t.confirm_ack = (ack && typeof ack.accepted !== 'undefined')
        ? 'accepted=' + ack.accepted
        : String(JSON.stringify(ack));
    }).catch(function (e) { t.confirm_failed = String(e); });
    return 'confirm-dispatched';
  } catch (e) {
    try { t.confirm_failed = 'fatal:' + e; } catch (_) {}
    return 'error:' + e;
  }
})()
"#;

/// After: record the real `cad_status` `entityCount`/`tool` once both points
/// have been placed and the line has been confirmed.
const TOOL_AFTER_JS: &str = r#"
(function () {
  var t = window.__yacr_tool = window.__yacr_tool || {};
  try {
    var internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {
      t.after_failed = 'no-tauri-internals';
      return 'no-tauri-internals';
    }
    internals.invoke('cad_status').then(function (s) {
      t.after_count = s.entityCount;
      t.after_tool = s.tool ? s.tool.key : null;
      t.error = s.error;
    }).catch(function (e) { t.after_failed = String(e); });
    return 'after-dispatched';
  } catch (e) {
    try { t.after_failed = 'fatal:' + e; } catch (_) {}
    return 'error:' + e;
  }
})()
"#;

/// One synthetic click inside `.native-input`: `pointermove` (hover),
/// `pointerdown` (button 0 / buttons 1), `pointerup` (button 0 / buttons 0).
/// Pointer capture is shimmed because these events are not backed by a device.
/// __FX__/__FY__ are fractions of the element rect; __LABEL__ names the click.
const TOOL_CLICK_TEMPLATE: &str = r#"
(function () {
  var t = window.__yacr_tool = window.__yacr_tool || {};
  var el = document.querySelector('.native-input');
  try {
    if (!el) { t.clicks_failed = 'no-element'; return 'no-element'; }
    var r = el.getBoundingClientRect();
    if (!(r.width > 20 && r.height > 20)) {
      t.clicks_failed = 'tiny-rect:' + r.width + 'x' + r.height;
      return 'tiny-rect:' + r.width + 'x' + r.height;
    }
    el.setPointerCapture = function () {};
    el.hasPointerCapture = function () { return false; };
    el.releasePointerCapture = function () {};
    t.rect = [r.left, r.top, r.width, r.height];
    function fire(type, x, y, button, buttons) {
      el.dispatchEvent(new PointerEvent(type, {
        pointerId: 1, pointerType: 'mouse', isPrimary: true,
        button: button, buttons: buttons,
        clientX: x, clientY: y, bubbles: true, cancelable: true
      }));
    }
    var x = r.left + r.width * __FX__;
    var y = r.top + r.height * __FY__;
    fire('pointermove', x, y, -1, 0);
    fire('pointerdown', x, y, 0, 1);
    fire('pointerup', x, y, 0, 0);
    t.clicks = (t.clicks || 0) + 1;
    t.last_click = '__LABEL__@' + Math.round(x - r.left) + ',' + Math.round(y - r.top);
    return 'clicked __LABEL__ x=' + Math.round(x) + ' y=' + Math.round(y) + ' clicks=' + t.clicks;
  } catch (e) {
    t.clicks_failed = '__LABEL__:' + e;
    return 'error:' + e;
  }
})()
"#;

/// Single-line report over `window.__yacr_tool`. Fields are whitespace-flattened
/// so the line stays tokenizable; `-` means absent. `delta` is only a real number
/// when both counts were actually read (`-1` otherwise).
const TOOL_REPORT_JS: &str = r#"
(function () {
  var t = window.__yacr_tool || {};
  function v(x) {
    if (x === null || typeof x === 'undefined') { return '-'; }
    try { return String(x).replace(/\s+/g, '_'); } catch (e) { return '?'; }
  }
  var before = (typeof t.before_count === 'number') ? t.before_count : -1;
  var after = (typeof t.after_count === 'number') ? t.after_count : -1;
  var delta = (before >= 0 && after >= 0) ? (after - before) : -1;
  var text = 'clicks=' + (t.clicks || 0) +
    ' before_count=' + v(t.before_count) +
    ' after_count=' + v(t.after_count) +
    ' delta=' + delta +
    ' before_tool=' + v(t.before_tool) +
    ' after_tool=' + v(t.after_tool) +
    ' start_ack=' + v(t.start_ack) +
    ' start_failed=' + v(t.start_failed) +
    ' confirm_ack=' + v(t.confirm_ack) +
    ' confirm_failed=' + v(t.confirm_failed) +
    ' before_failed=' + v(t.before_failed) +
    ' after_failed=' + v(t.after_failed) +
    ' clicks_failed=' + v(t.clicks_failed) +
    ' error=' + v(t.error);
  try { console.log('[probe] tool evidence report: ' + text); } catch (e) {}
  return text;
})()
"#;

/// Synthetic input script. It dispatches DOM pointer/wheel events that the
/// frontend forwards over the real IPC bridge. Pointer capture is shimmed
/// because these events are not backed by a real device.
const SYNTHETIC_INPUT_JS: &str = r#"
(function () {
  try {
    var el = document.querySelector('.native-input');
    if (!el) { return 'no-element'; }
    el.setPointerCapture = function () {};
    el.hasPointerCapture = function () { return false; };
    el.releasePointerCapture = function () {};
    function fire(type, x) {
      el.dispatchEvent(new PointerEvent(type, {
        pointerId: 1, pointerType: 'mouse', isPrimary: true,
        clientX: x, clientY: 140, bubbles: true, cancelable: true
      }));
    }
    fire('pointerdown', 60);
    fire('pointermove', 200);
    fire('pointermove', 340);
    fire('pointerup', 380);
    el.dispatchEvent(new WheelEvent('wheel', {
      deltaX: 0, deltaY: 240, deltaMode: 0,
      clientX: 220, clientY: 140, bubbles: true, cancelable: true
    }));
    return 'injected';
  } catch (e) { return 'error:' + e; }
})()
"#;
