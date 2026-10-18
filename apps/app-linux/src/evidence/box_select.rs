//! `YACR_PROBE_BOX_EVIDENCE`: the box-selection evidence driver. Split out of
//! `evidence.rs` verbatim (names, env vars, log prefixes and scheduling
//! unchanged).

use std::time::{Duration, Instant};

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
///   as `crossing_count`. During the hold a ±1 px jitter `pointermove` is
///   re-dispatched every 500 ms (t=26.5s..49.5s): the frontend input lane
///   coalesces `pointermove` behind `requestAnimationFrame`, so a single move
///   can be buffered and flushed only together with the release — the periodic
///   moves keep the lane flushing so the live rubber band is really rendered at
///   the external `--capture-at 30`. Every jitter point stays inside the same
///   rectangle (the band looks identical); the commit corners come from the
///   untouched down/up events, so the reported selection counts cannot change;
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
    // Hold the drag open from t=26.5s to t=49.5s: the frontend input lane
    // coalesces `pointermove` behind requestAnimationFrame, so a single silent
    // move can be buffered and reach the host only with the release — the band
    // would never be live at the capture instant. Re-dispatch a ±1 px jitter
    // pointermove every 500 ms instead, which keeps the lane flushing and the
    // live rubber band rendered for an external --capture-at 30. Every jitter
    // point stays inside the same rectangle, so the band is unchanged; the
    // commit corners are the untouched down/up events below.
    let jitter: [(i64, i64); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];
    let mut hold_index = 0u64;
    while 26_500 + hold_index * 500 < 50_000 {
        let (px, py) = jitter[(hold_index as usize) % jitter.len()];
        let hold_move = box_hold_move_js(px, py);
        schedule_box_after(
            &start,
            Duration::from_millis(26_500 + hold_index * 500),
            "phase B hold jitter pointermove (±1px; NOT a real mouse)",
            || webview.eval(hold_move),
        );
        hold_index += 1;
    }
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
    schedule_box_after(start, Duration::from_secs(seconds), label, action);
}

/// Millisecond-precision variant of [`schedule_box`], used by the phase-B hold
/// jitter (~500 ms cadence). Same logging and same "dispatch only is not a pass"
/// contract.
fn schedule_box_after(
    start: &Instant,
    after: Duration,
    label: &str,
    action: impl FnOnce() -> tauri::Result<()>,
) {
    let elapsed = start.elapsed();
    if after > elapsed {
        std::thread::sleep(after - elapsed);
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

/// Build one phase-B hold jitter `pointermove` at a **pixel** offset (`__PX__`,
/// `__PY__`) from the current (25%/25%) corner. A ±1 px offset stays inside the
/// same rectangle, so the live band looks identical. Synthetic, **not a real
/// mouse**.
fn box_hold_move_js(px: i64, py: i64) -> String {
    BOX_HOLD_MOVE_TEMPLATE
        .replace("__PX__", &px.to_string())
        .replace("__PY__", &py.to_string())
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

/// One synthetic jitter `pointermove` for the phase-B hold. It offsets the
/// current (25%/25%) corner by `__PX__`/`__PY__` **pixels** (a ±1 px jitter, so
/// the rectangle is unchanged) and records `b.hold_moves`/`b.last_hold`
/// separately from the tracked-move counter. The point is to re-enter the
/// frontend input lane every ~500 ms: the lane coalesces `pointermove` behind
/// `requestAnimationFrame`, so a buffered move can otherwise reach the host
/// only with the release and the rubber band would not be live at capture time.
/// Synthetic, **not a real mouse**.
const BOX_HOLD_MOVE_TEMPLATE: &str = r#"
(function () {
  var b = window.__yacr_box = window.__yacr_box || {};
  var el = document.querySelector('.native-input');
  try {
    if (!el) { b.hold_failed = 'no-element'; return 'no-element'; }
    var r = el.getBoundingClientRect();
    if (!(r.width > 20 && r.height > 20)) {
      b.hold_failed = 'tiny-rect:' + r.width + 'x' + r.height;
      return 'tiny-rect:' + r.width + 'x' + r.height;
    }
    el.setPointerCapture = function () {};
    el.hasPointerCapture = function () { return false; };
    el.releasePointerCapture = function () {};
    var x = r.left + r.width * 0.25 + __PX__;
    var y = r.top + r.height * 0.25 + __PY__;
    el.dispatchEvent(new PointerEvent('pointermove', {
      pointerId: 1, pointerType: 'mouse', isPrimary: true,
      button: -1, buttons: 1,
      clientX: x, clientY: y, bubbles: true, cancelable: true
    }));
    b.hold_moves = (b.hold_moves || 0) + 1;
    b.last_hold = [Math.round(x - r.left), Math.round(y - r.top)];
    return 'hold x=' + Math.round(x) + ' y=' + Math.round(y);
  } catch (e) {
    try { b.hold_failed = 'fatal:' + e; } catch (_) {}
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
/// `box_overlay_channel=none` instead of inventing a value; whether the band was
/// actually rendered is checked out-of-band on the screenshot by
/// `apps/app-linux/scripts/check-band-pixels.py`. `moves` counts only the
/// tracked drag moves; the phase-B hold jitter is reported as `hold_moves`.
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
    ' hold_moves=' + num(b.hold_moves) +
    ' baseline_failed=' + v(b.baseline_failed) +
    ' wide_window_failed=' + v(b.wide_window_failed) +
    ' window_failed=' + v(b.window_failed) +
    ' crossing_failed=' + v(b.crossing_failed) +
    ' pointer_failed=' + v(b.pointer_failed) +
    ' hold_failed=' + v(b.hold_failed);
  try { console.log('[probe] box evidence report: ' + text); } catch (e) {}
  return text;
})()
"#;
