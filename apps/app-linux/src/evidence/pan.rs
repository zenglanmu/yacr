//! `YACR_PROBE_POSTLOAD_PAN`: the post-load pan gesture scripts consumed by
//! `host::postload`. Split out of `evidence.rs` verbatim (names, env vars, log
//! prefixes and scheduling unchanged).

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
