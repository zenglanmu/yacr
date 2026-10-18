//! `YACR_PROBE_HOVER`: the hover-cost evidence driver. Split out of
//! `evidence.rs` verbatim (names, env vars, log prefixes and scheduling
//! unchanged).

use std::time::{Duration, Instant};

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
