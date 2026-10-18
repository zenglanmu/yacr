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

/// Whether the **hover cost evidence** driver is enabled (`YACR_PROBE_HOVER`).
///
/// Evidence-only, default OFF, independent of every other probe driver.
///
/// Synthetic input, **not a real mouse**: the events are DOM
/// `pointermove` dispatches on `.native-input` with `buttons: 0` (no press, no
/// drag, no click); the kernel/evdev/XInput path is never touched. The driver
/// measures what the running host actually does for plain hover, and reports
/// only measured values. Per-hover pick cost comes from the host's own
/// `YACR_PROFILE_PICK` stderr lines (`crates/cad-host/src/input/hover.rs`);
/// it is deliberately **not** recomputed in JS.
pub fn hover_evidence_enabled() -> bool {
    matches!(
        std::env::var("YACR_PROBE_HOVER").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

/// Schedule the hover cost evidence driver if enabled. Never panics on spawn
/// failure (a failed spawn is logged; the run continues unchanged).
///
/// The thread only dispatches `Webview::eval` scripts that read the public
/// `cad_status` command and dispatch synthetic DOM pointer events. No
/// command/session logic is changed and no GPU state is touched.
pub fn maybe_spawn_hover_evidence(webview: Option<&tauri::Webview>) {
    if !hover_evidence_enabled() {
        return;
    }
    let Some(webview) = webview else {
        eprintln!(
            "[probe] hover evidence: no child webview; synthetic hover skipped (nothing faked)"
        );
        return;
    };
    eprintln!(
        "[probe] hover evidence: enabled (YACR_PROBE_HOVER); synthetic DOM pointermove, NOT a real mouse"
    );
    let webview = webview.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("probe-hover-evidence".into())
        .spawn(move || run_hover_evidence(webview))
    {
        eprintln!("[probe] hover evidence: failed to spawn: {error}");
    }
}

/// Run the hover cost evidence sequence relative to driver start (2 steps):
/// t=8s read the real `cad_status` baseline and sweep 180 synthetic
/// `pointermove` events (~60 Hz, ~3 s) along a path that crosses drawing
/// geometry and empty space, then settle ~1 s and read `cad_status` again;
/// t=14s log one single-line report via the eval callback.
///
/// The values reported are the real ones: the JS dispatch count, the
/// host-admitted `pointer_move` count (from the real `probe_input` acks),
/// `performance.now()` sweep time (a synthetic-input-side number, not a host
/// cost), the `cad_status` `frameCount`/`entityCount` deltas, and every
/// recorded failure. Pick cost stays in the `YACR_PROFILE_PICK` log lines.
fn run_hover_evidence(webview: tauri::Webview) {
    let start = Instant::now();
    schedule_hover(
        &start,
        8,
        "hover baseline + 180 synthetic pointermove sweep (NOT a real mouse)",
        || webview.eval(HOVER_BASELINE_JS),
    );
    schedule_hover(&start, 14, "hover evidence report", || {
        webview.eval_with_callback(HOVER_REPORT_JS, |payload| {
            eprintln!(
                "[probe] hover evidence report: {}",
                decode_hover_report(&payload)
            );
        })
    });
}

/// Decode the JSON-quoted eval-callback payload into the raw report string.
fn decode_hover_report(payload: &str) -> String {
    let trimmed = payload.trim();
    if trimmed.starts_with('"') {
        if let Ok(decoded) = serde_json::from_str::<String>(trimmed) {
            return decoded;
        }
    }
    trimmed.to_string()
}

/// Sleep until `seconds` after `start`, run `action`, and log the dispatch
/// result under the hover evidence prefix. Only the **dispatch** is reported
/// here; the observable numbers come from the report payload.
fn schedule_hover(
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
        Ok(()) => eprintln!("[probe] hover evidence step: {label} -> dispatched"),
        Err(error) => eprintln!("[probe] hover evidence step: {label} -> dispatch error: {error}"),
    }
}

/// Baseline + sweep. Reads the real `cad_status` (`frameCount`/`entityCount`)
/// **before** the sweep, then emits 180 synthetic `pointermove` events, one per
/// animation frame, with `buttons: 0` / `button: -1` (plain hover: no press, no
/// drag, no click). A `probe_input` ack wrapper counts the `pointer_move`
/// inputs the host actually accepted. Pointer capture is shimmed because these
/// events are not backed by a device. Synthetic, **not a real mouse**.
const HOVER_BASELINE_JS: &str = r#"
(function () {
  var h = window.__yacr_hover = window.__yacr_hover || {};
  h.dispatched = 0;
  h.accepted_moves = 0;
  h.refused_moves = 0;
  h.invoke_moves = 0;
  h.failed = [];
  h.synthetic = true;
  function fail(where, e) {
    try { h.failed.push(where + ':' + String(e)); } catch (_) {}
  }
  try {
    var el = document.querySelector('.native-input');
    if (!el) { fail('setup', 'no-element'); h.sweep_done = true; h.after_done = true; return 'no-element'; }
    var r = el.getBoundingClientRect();
    if (!(r.width > 20 && r.height > 20)) {
      fail('setup', 'tiny-rect:' + r.width + 'x' + r.height);
      h.sweep_done = true; h.after_done = true;
      return 'tiny-rect:' + r.width + 'x' + r.height;
    }
    el.setPointerCapture = function () {};
    el.hasPointerCapture = function () { return false; };
    el.releasePointerCapture = function () {};
    h.rect = [r.left, r.top, r.width, r.height];

    // Count the pointer_move inputs the host actually admitted: every
    // `probe_input` ack carries the real `accepted` flag.
    var internals = window.__TAURI_INTERNALS__;
    var original_invoke = null;
    if (internals && typeof internals.invoke === 'function') {
      original_invoke = internals.invoke;
      try {
        internals.invoke = function (command, args) {
          var result = original_invoke.apply(this, arguments);
          try {
            if (command === 'probe_input' && args && args.input && args.input.kind === 'pointer_move') {
              h.invoke_moves++;
              if (result && typeof result.then === 'function') {
                result.then(function (ack) {
                  if (ack && ack.accepted) { h.accepted_moves++; } else { h.refused_moves++; }
                }).catch(function (e) { fail('ack', e); });
              }
            }
          } catch (e) { fail('invoke-wrap', e); }
          return result;
        };
        h.invoke_wrapped = true;
      } catch (e) { fail('invoke-wrap', e); }
    } else {
      fail('invoke-wrap', 'no-tauri-internals');
    }

    function fire(x, y) {
      el.dispatchEvent(new PointerEvent('pointermove', {
        pointerId: 1, pointerType: 'mouse', isPrimary: true,
        button: -1, buttons: 0,
        clientX: x, clientY: y, bubbles: true, cancelable: true
      }));
      h.dispatched++;
    }

    // left -> right, top -> bottom, then a small jitter circle in the middle:
    // the hovered set both changes and stays during the sweep.
    var TOTAL = 180;
    var PER_SEGMENT = 60;
    var cx = r.left + r.width / 2;
    var cy = r.top + r.height / 2;
    var radius = Math.min(r.width, r.height) * 0.08;
    function point(i) {
      if (i < PER_SEGMENT) {
        var tx = i / (PER_SEGMENT - 1);
        return [r.left + r.width * (0.05 + 0.90 * tx), cy];
      }
      if (i < 2 * PER_SEGMENT) {
        var ty = (i - PER_SEGMENT) / (PER_SEGMENT - 1);
        return [cx, r.top + r.height * (0.05 + 0.90 * ty)];
      }
      var k = i - 2 * PER_SEGMENT;
      var a = (k / PER_SEGMENT) * Math.PI * 4;
      return [cx + Math.cos(a) * radius, cy + Math.sin(a) * radius];
    }

    var i = 0;
    var lastStep = 0;
    var timer = null;
    var started = false;
    function readAfter() {
      try {
        if (internals && typeof internals.invoke === 'function') {
          internals.invoke('cad_status').then(function (s) {
            h.after_frame = s.frameCount;
            h.after_entities = s.entityCount;
            h.after_done = true;
          }).catch(function (e) { fail('after', e); h.after_done = true; });
        } else {
          fail('after', 'no-tauri-internals');
          h.after_done = true;
        }
      } catch (e) { fail('after', e); h.after_done = true; }
      if (original_invoke && h.invoke_wrapped) {
        try { internals.invoke = original_invoke; h.invoke_wrapped = false; }
        catch (e) { fail('unwrap', e); }
      }
    }
    function finish() {
      if (h.sweep_end) { return; }
      h.sweep_end = performance.now();
      h.sweep_ms = Math.round(h.sweep_end - h.sweep_start);
      h.sweep_done = true;
      try { if (timer !== null) { clearInterval(timer); timer = null; } } catch (e) {}
      // Let the last hover settle before the after snapshot.
      setTimeout(readAfter, 1000);
    }
    function step() {
      if (!started || i >= TOTAL) { return; }
      var p = point(i);
      try { fire(p[0], p[1]); } catch (e) { fail('move', e); }
      i++;
      lastStep = performance.now();
      if (i >= TOTAL) { finish(); }
    }
    function raf() {
      if (started && i < TOTAL) { step(); requestAnimationFrame(raf); }
    }
    function start() {
      if (started) { return; }
      started = true;
      h.started = Date.now();
      h.sweep_start = performance.now();
      requestAnimationFrame(raf);
      // Stall guard: if rAF is throttled/absent, keep the sweep advancing.
      timer = setInterval(function () {
        if (!started || i >= TOTAL) { clearInterval(timer); return; }
        if (performance.now() - lastStep > 100) { step(); }
      }, 50);
    }

    // Baseline first, so frame_delta/entity_delta are genuinely before -> after.
    try {
      if (internals && typeof internals.invoke === 'function') {
        internals.invoke('cad_status').then(function (s) {
          h.before_frame = s.frameCount;
          h.before_entities = s.entityCount;
          start();
        }).catch(function (e) { fail('before', e); start(); });
      } else {
        fail('before', 'no-tauri-internals');
        start();
      }
    } catch (e) { fail('before', e); start(); }
    return 'started rect=' + h.rect.join(',');
  } catch (e) {
    fail('fatal', e);
    h.sweep_done = true;
    h.after_done = true;
    return 'error:' + e;
  }
})()
"#;

/// Single-line report over `window.__yacr_hover` in the exact
/// `moves= frame_delta= entity_delta= js_sweep_ms= notes=` shape. `-1` means the
/// value was not measured; absent fields are `-`. Every failure recorded by the
/// baseline/sweep is surfaced in `notes` (never swallowed).
const HOVER_REPORT_JS: &str = r#"
(function () {
  var h = window.__yacr_hover || {};
  function v(x) {
    if (x === null || typeof x === 'undefined') { return '-'; }
    try { return String(x).replace(/\s+/g, '_'); } catch (e) { return '?'; }
  }
  function num(x) { return (typeof x === 'number') ? x : -1; }
  var bf = num(h.before_frame), af = num(h.after_frame);
  var be = num(h.before_entities), ae = num(h.after_entities);
  var frame_delta = (bf >= 0 && af >= 0) ? (af - bf) : -1;
  var entity_delta = (be >= 0 && ae >= 0) ? (ae - be) : -1;
  var failed = (h.failed && h.failed.length) ? h.failed.slice(0, 8).join('|') : '';
  var notes = 'accepted_moves=' + (h.accepted_moves || 0) +
    ' refused_moves=' + (h.refused_moves || 0) +
    ' invoke_moves=' + (h.invoke_moves || 0) +
    ' before_frame=' + v(h.before_frame) +
    ' after_frame=' + v(h.after_frame) +
    ' before_entities=' + v(h.before_entities) +
    ' after_entities=' + v(h.after_entities) +
    ' sweep_done=' + (!!h.sweep_done) +
    ' after_done=' + (!!h.after_done) +
    ' synthetic=1 pick_cost=YACR_PROFILE_PICK_stderr_lines' +
    ' failed=' + (failed || '-');
  var text = 'moves=' + (h.dispatched || 0) +
    ' frame_delta=' + frame_delta +
    ' entity_delta=' + entity_delta +
    ' js_sweep_ms=' + num(h.sweep_ms) +
    ' notes=' + notes;
  try { console.log('[probe] hover evidence report: ' + text); } catch (e) {}
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

/// Whether the **box-selection evidence** driver is enabled
/// (`YACR_PROBE_BOX_EVIDENCE`).
///
/// Evidence-only, default OFF, and independent of every other probe driver
/// (setting only this one does not trigger the input/tool/hover/lifecycle
/// passes). It proves the 2D primary-drag path end-to-end against a **real
/// running host**: a primary `pointerdown` then a drag past the drag threshold
/// starts a box gesture, further moves track the live rubber band, and the
/// release dispatches the real `SelectInBox` command. Every reported number is
/// a real `cad_status` read (`selection.count`), never synthesized here.
/// Left-to-right must read as WINDOW and right-to-left as CROSSING.
///
/// Synthetic input, **not a real mouse**: the events are dispatched on
/// `.native-input` and never touch the kernel/evdev/XInput path. There is **no
/// probe-visible channel for the rubber-band overlay** (`selection_box` lives
/// only in the render overlays, not in `cad_status`); this driver does not
/// invent one.
pub fn box_evidence_enabled() -> bool {
    matches!(
        std::env::var("YACR_PROBE_BOX_EVIDENCE").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

/// Schedule the box-selection evidence driver if enabled. Never panics on spawn
/// failure (a failed spawn is logged; the run continues unchanged).
///
/// The thread only dispatches `Webview::eval` scripts that read the public
/// `cad_status` command and dispatch synthetic DOM pointer events. No
/// command/session logic is changed and no GPU state is touched.
pub fn maybe_spawn_box_evidence(webview: Option<&tauri::Webview>) {
    if !box_evidence_enabled() {
        return;
    }
    let Some(webview) = webview else {
        eprintln!("[probe] box evidence: no child webview; box drag pass skipped (nothing faked)");
        return;
    };
    eprintln!(
        "[probe] box evidence: enabled (YACR_PROBE_BOX_EVIDENCE); synthetic DOM pointer events, NOT a real mouse; rubber-band overlay selection_box has no probe-visible channel (not read from cad_status)"
    );
    let webview = webview.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("probe-box-evidence".into())
        .spawn(move || run_box_evidence(webview))
    {
        eprintln!("[probe] box evidence: failed to spawn: {error}");
    }
}

/// Run the box-selection evidence sequence relative to driver start:
///
/// * t=8s read the real `cad_status` baseline (`selection.count`, `entityCount`,
///   `snapEnabled`) as one report line;
/// * Phase W-wide **WINDOW**: t=10s primary `pointerdown` at ~2%/2%, t=11s
///   `pointermove` to ~98%/98%, t=12s `pointerup`; t=14s read `cad_status` as
///   `wide_window_count`. The rectangle spans nearly the whole canvas, so a
///   working WINDOW path must fully contain the visible geometry and select
///   something (a positive control against the 25%->60% WINDOW, which is
///   expected to contain nothing);
/// * Phase A **WINDOW** (25% -> 60%): t=17s `pointerdown`, t=18s `pointermove`,
///   t=19s `pointerup`; t=21s read `cad_status` as `window_count`;
/// * Phase B **CROSSING** (60% -> 25%): t=24s `pointerdown`, t=26s
///   `pointermove`, then **hold the drag open until t=50s** so an external
///   screenshot can land mid-gesture, then `pointerup`; t=53s read `cad_status`
///   as `crossing_count`;
/// * t=55s one single-line report with the real numbers, `wide_window_count` and
///   `discriminating=(crossing_count > window_count)`.
///
/// Every step is a dispatch only; the observed counts come from the real
/// `cad_status` reads on the running host, never from a value synthesized here.
/// The whole driver finishes by ~56s so it composes with a bounded run timeout;
/// the held CROSSING drag (t=26..50) covers an external `--capture-at 30`.
fn run_box_evidence(webview: tauri::Webview) {
    let start = Instant::now();

    // t=8s: real baseline cad_status (selection.count / entityCount / snapEnabled).
    schedule_box(&start, 8, "baseline cad_status", || {
        webview.eval(BOX_BASELINE_JS)
    });

    // Phase W-wide — WINDOW (~2% -> ~98%: nearly the whole canvas). A working
    // WINDOW path must fully contain the visible geometry here and select
    // something; this is the positive control for the narrow phase A below.
    let w_down = box_pointer_js("pointerdown", 0, 1, 0.02, 0.02);
    schedule_box(
        &start,
        10,
        "phase W-wide pointerdown (primary) at 2%/2%",
        || webview.eval(w_down),
    );
    let w_move = box_pointer_js("pointermove", -1, 1, 0.98, 0.98);
    schedule_box(
        &start,
        11,
        "phase W-wide pointermove to 98%/98% (WINDOW)",
        || webview.eval(w_move),
    );
    let w_up = box_pointer_js("pointerup", 0, 0, 0.98, 0.98);
    schedule_box(&start, 12, "phase W-wide pointerup (WINDOW commit)", || {
        webview.eval(w_up)
    });
    schedule_box(&start, 14, "wide-window cad_status (phase W-wide)", || {
        webview.eval(BOX_WIDE_WINDOW_JS)
    });

    // Phase A — WINDOW (left -> right primary drag, 25% -> 60%).
    let a_down = box_pointer_js("pointerdown", 0, 1, 0.25, 0.25);
    schedule_box(
        &start,
        17,
        "phase A pointerdown (primary) at 25%/25%",
        || webview.eval(a_down),
    );
    let a_move = box_pointer_js("pointermove", -1, 1, 0.60, 0.60);
    schedule_box(
        &start,
        18,
        "phase A pointermove to 60%/60% (WINDOW)",
        || webview.eval(a_move),
    );
    let a_up = box_pointer_js("pointerup", 0, 0, 0.60, 0.60);
    schedule_box(&start, 19, "phase A pointerup (WINDOW commit)", || {
        webview.eval(a_up)
    });
    schedule_box(&start, 21, "window cad_status (phase A)", || {
        webview.eval(BOX_WINDOW_JS)
    });

    // Phase B — CROSSING (right -> left primary drag, 60% -> 25%, held open).
    let b_down = box_pointer_js("pointerdown", 0, 1, 0.60, 0.60);
    schedule_box(
        &start,
        24,
        "phase B pointerdown (primary) at 60%/60%",
        || webview.eval(b_down),
    );
    let b_move = box_pointer_js("pointermove", -1, 1, 0.25, 0.25);
    schedule_box(
        &start,
        26,
        "phase B pointermove to 25%/25% (CROSSING; held)",
        || webview.eval(b_move),
    );
    // Hold the drag open from t=26s to t=50s: no events in between, so the box
    // gesture (and its rubber band) stays live for an external --capture-at 30.
    let b_up = box_pointer_js("pointerup", 0, 0, 0.25, 0.25);
    schedule_box(&start, 50, "phase B pointerup (CROSSING commit)", || {
        webview.eval(b_up)
    });
    schedule_box(&start, 53, "crossing cad_status (phase B)", || {
        webview.eval(BOX_CROSSING_JS)
    });

    // t=55s: final single-line report; logged from Rust through the eval
    // callback so the line carries the real numbers (a dispatch is not a pass).
    schedule_box(&start, 55, "box evidence report", || {
        webview.eval_with_callback(BOX_REPORT_JS, |payload| {
            eprintln!(
                "[probe] box evidence report: {}",
                decode_box_report(&payload)
            );
        })
    });
}

/// Decode the JSON-quoted eval-callback payload into the raw report string.
fn decode_box_report(payload: &str) -> String {
    let trimmed = payload.trim();
    if trimmed.starts_with('"') {
        if let Ok(decoded) = serde_json::from_str::<String>(trimmed) {
            return decoded;
        }
    }
    trimmed.to_string()
}

/// Sleep until `seconds` after `start`, run `action`, and log the dispatch
/// result under the box evidence prefix. Only the **dispatch** is reported here;
/// the observable counts come from the `cad_status` reads.
fn schedule_box(
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
        Ok(()) => eprintln!("[probe] box evidence step: {label} -> dispatched"),
        Err(error) => eprintln!("[probe] box evidence step: {label} -> dispatch error: {error}"),
    }
}

/// Build one synthetic pointer script at a fraction of the `.native-input`
/// rect. `kind` is `pointerdown`/`pointermove`/`pointerup`; `button`/`buttons`
/// follow the DOM `MouseEvent` convention (press: `0`/`1`, move: `-1`/`1`,
/// release: `0`/`0`). Pointer capture is shimmed because these events are not
/// backed by a device. Synthetic, **not a real mouse**.
fn box_pointer_js(kind: &str, button: i64, buttons: i64, fx: f64, fy: f64) -> String {
    BOX_POINTER_TEMPLATE
        .replace("__KIND__", kind)
        .replace("__BUTTON__", &button.to_string())
        .replace("__BUTTONS__", &buttons.to_string())
        .replace("__FX__", &fx.to_string())
        .replace("__FY__", &fy.to_string())
}

/// Baseline: record the real `cad_status` `selection.count`/`entityCount`/
/// `snapEnabled` and log one report line. Synthetic, **not a real mouse**.
const BOX_BASELINE_JS: &str = r#"
(function () {
  var b = window.__yacr_box = window.__yacr_box || {};
  try {
    var internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {
      b.baseline_failed = 'no-tauri-internals';
      return 'no-tauri-internals';
    }
    internals.invoke('cad_status').then(function (s) {
      b.baseline_count = s.selection ? s.selection.count : undefined;
      b.baseline_entities = s.entityCount;
      b.baseline_snap = s.snapEnabled;
      try {
        console.log('[probe] box evidence baseline: selection_count=' + b.baseline_count +
          ' entity_count=' + b.baseline_entities + ' snap_enabled=' + b.baseline_snap +
          ' synthetic=1 not_a_real_mouse');
      } catch (e) {}
    }).catch(function (e) { b.baseline_failed = String(e); });
    return 'baseline-dispatched';
  } catch (e) {
    try { b.baseline_failed = 'fatal:' + e; } catch (_) {}
    return 'error:' + e;
  }
})()
"#;

/// Phase W-wide read: record the real `cad_status` `selection.count` after the
/// near-full-canvas WINDOW drag commits, and log one report line.
const BOX_WIDE_WINDOW_JS: &str = r#"
(function () {
  var b = window.__yacr_box = window.__yacr_box || {};
  try {
    var internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {
      b.wide_window_failed = 'no-tauri-internals';
      return 'no-tauri-internals';
    }
    internals.invoke('cad_status').then(function (s) {
      b.wide_window_count = s.selection ? s.selection.count : undefined;
      try {
        console.log('[probe] box evidence wide-window: selection_count=' + b.wide_window_count);
      } catch (e) {}
    }).catch(function (e) { b.wide_window_failed = String(e); });
    return 'wide-window-dispatched';
  } catch (e) {
    try { b.wide_window_failed = 'fatal:' + e; } catch (_) {}
    return 'error:' + e;
  }
})()
"#;

/// Phase A read: record the real `cad_status` `selection.count` after the WINDOW
/// drag commits, and log one report line.
const BOX_WINDOW_JS: &str = r#"
(function () {
  var b = window.__yacr_box = window.__yacr_box || {};
  try {
    var internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {
      b.window_failed = 'no-tauri-internals';
      return 'no-tauri-internals';
    }
    internals.invoke('cad_status').then(function (s) {
      b.window_count = s.selection ? s.selection.count : undefined;
      try {
        console.log('[probe] box evidence window: selection_count=' + b.window_count);
      } catch (e) {}
    }).catch(function (e) { b.window_failed = String(e); });
    return 'window-dispatched';
  } catch (e) {
    try { b.window_failed = 'fatal:' + e; } catch (_) {}
    return 'error:' + e;
  }
})()
"#;

/// Phase B read: record the real `cad_status` `selection.count` after the
/// CROSSING drag commits, and log one report line.
const BOX_CROSSING_JS: &str = r#"
(function () {
  var b = window.__yacr_box = window.__yacr_box || {};
  try {
    var internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {
      b.crossing_failed = 'no-tauri-internals';
      return 'no-tauri-internals';
    }
    internals.invoke('cad_status').then(function (s) {
      b.crossing_count = s.selection ? s.selection.count : undefined;
      try {
        console.log('[probe] box evidence crossing: selection_count=' + b.crossing_count);
      } catch (e) {}
    }).catch(function (e) { b.crossing_failed = String(e); });
    return 'crossing-dispatched';
  } catch (e) {
    try { b.crossing_failed = 'fatal:' + e; } catch (_) {}
    return 'error:' + e;
  }
})()
"#;

/// One synthetic pointer event inside `.native-input`. `__KIND__` is the DOM
/// event type; `__BUTTON__`/`__BUTTONS__` are the DOM `MouseEvent` button fields
/// (press: 0/1, move: -1/1, release: 0/0); `__FX__`/`__FY__` are fractions of the
/// element rect. Synthetic, **not a real mouse**.
const BOX_POINTER_TEMPLATE: &str = r#"
(function () {
  var b = window.__yacr_box = window.__yacr_box || {};
  var el = document.querySelector('.native-input');
  try {
    if (!el) { b.pointer_failed = 'no-element'; return 'no-element'; }
    var r = el.getBoundingClientRect();
    if (!(r.width > 20 && r.height > 20)) {
      b.pointer_failed = 'tiny-rect:' + r.width + 'x' + r.height;
      return 'tiny-rect:' + r.width + 'x' + r.height;
    }
    el.setPointerCapture = function () {};
    el.hasPointerCapture = function () { return false; };
    el.releasePointerCapture = function () {};
    b.rect = [r.left, r.top, r.width, r.height];
    var x = r.left + r.width * __FX__;
    var y = r.top + r.height * __FY__;
    el.dispatchEvent(new PointerEvent('__KIND__', {
      pointerId: 1, pointerType: 'mouse', isPrimary: true,
      button: __BUTTON__, buttons: __BUTTONS__,
      clientX: x, clientY: y, bubbles: true, cancelable: true
    }));
    if ('__KIND__' === 'pointerdown') { b.downs = (b.downs || 0) + 1; b.last_down = [Math.round(x - r.left), Math.round(y - r.top)]; }
    if ('__KIND__' === 'pointermove') { b.moves = (b.moves || 0) + 1; b.last_move = [Math.round(x - r.left), Math.round(y - r.top)]; }
    if ('__KIND__' === 'pointerup') { b.ups = (b.ups || 0) + 1; b.last_up = [Math.round(x - r.left), Math.round(y - r.top)]; }
    return '__KIND__ x=' + Math.round(x) + ' y=' + Math.round(y);
  } catch (e) {
    try { b.pointer_failed = '__KIND__:' + e; } catch (_) {}
    return 'error:' + e;
  }
})()
"#;

/// Single-line report over `window.__yacr_box`. Fields are whitespace-flattened
/// so the line stays tokenizable; `-` means absent and `-1` means a count that
/// was never read. `discriminating` is only a real comparison when **both**
/// counts were actually read, otherwise it is reported as `false` (never
/// asserted). `wide_window_count` is the positive WINDOW control (near-full
/// canvas); it is reported verbatim, even when `0`. The rubber-band overlay
/// (`selection_box`) is not projected into `cad_status`, so the report says
/// `box_overlay_channel=none` instead of inventing a value.
const BOX_REPORT_JS: &str = r#"
(function () {
  var b = window.__yacr_box || {};
  function v(x) {
    if (x === null || typeof x === 'undefined') { return '-'; }
    try { return String(x).replace(/\s+/g, '_'); } catch (e) { return '?'; }
  }
  function num(x) { return (typeof x === 'number') ? x : -1; }
  var wc = num(b.window_count), cc = num(b.crossing_count);
  var discriminating = (wc >= 0 && cc >= 0) ? (cc > wc) : false;
  var text = 'baseline_count=' + v(b.baseline_count) +
    ' wide_window_count=' + v(b.wide_window_count) +
    ' window_count=' + v(b.window_count) +
    ' crossing_count=' + v(b.crossing_count) +
    ' discriminating=(' + cc + '>' + wc + ')=' + discriminating +
    ' entity_count=' + v(b.baseline_entities) +
    ' snap_enabled=' + v(b.baseline_snap) +
    ' box_overlay_channel=none' +
    ' synthetic=1 not_a_real_mouse' +
    ' downs=' + num(b.downs) +
    ' moves=' + num(b.moves) +
    ' ups=' + num(b.ups) +
    ' baseline_failed=' + v(b.baseline_failed) +
    ' wide_window_failed=' + v(b.wide_window_failed) +
    ' window_failed=' + v(b.window_failed) +
    ' crossing_failed=' + v(b.crossing_failed) +
    ' pointer_failed=' + v(b.pointer_failed);
  try { console.log('[probe] box evidence report: ' + text); } catch (e) {}
  return text;
})()
"#;
