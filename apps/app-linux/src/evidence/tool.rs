//! `YACR_PROBE_TOOL_EVIDENCE`: the interactive-tool evidence driver. Split out
//! of `evidence.rs` verbatim (names, env vars, log prefixes and scheduling
//! unchanged).

use std::time::{Duration, Instant};

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
    // The TRIM/EXTEND edit driver (`YACR_PROBE_EDIT_EVIDENCE`) is independent of
    // this LINE driver: it has its own env var and runs even when
    // `YACR_PROBE_TOOL_EVIDENCE` is off. It is spawned from this host-init entry
    // point so the evidence subsystem stays self-contained (no file outside
    // `src/evidence/` is touched except the module declaration).
    crate::evidence::maybe_spawn_edit_evidence(webview);
    // The new drawing-command driver (`YACR_PROBE_DRAW_EVIDENCE`) is likewise
    // independent: its own env var, default OFF, spawned from this same entry
    // point so the evidence subsystem stays self-contained.
    crate::evidence::maybe_spawn_draw_evidence(webview);
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
