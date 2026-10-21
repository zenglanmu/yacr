//! `YACR_PROBE_VIEW3D`: the 3D-view evidence driver.
//!
//! Evidence-only, default OFF, and independent of every other probe driver
//! (setting only this one does not trigger the input/lifecycle/pan/tool/box/
//! hover/edit/draw/popup passes). It proves the ACIS solid is genuinely
//! three-dimensional against a **real running host**:
//!
//! 1. read the real `cad_status` (the 2D view reports a `camera` block);
//! 2. dispatch the payload-free `SWITCH2D3D` selector through the real
//!    `cad_command` funnel (the host switches the session view mode to the
//!    perspective 3D camera, so `cad_status.camera` becomes `null`);
//! 3. run a synthetic **primary drag** on `.native-input`, which the host
//!    arbitrates as an orbit in 3D (the same drag is a box selection in 2D);
//! 4. read the real `cad_status` again and print one report line carrying only
//!    real reads plus the driver's own synthetic-event counters.
//!
//! Nothing here is a product behaviour change, and no number is synthesized:
//! `camera_before` / `camera_after_switch` / `camera_after_orbit` are the real
//! presence of `cad_status.camera`, the two `error` fields are the real
//! technical status strings (`null` when there is none), and the drag counters
//! count events this driver really dispatched on the live DOM.
//!
//! Honesty boundary: `cad_status` exposes **no** 3D camera (yaw/pitch/distance)
//! and no view revision, so an orbit can only be corroborated by its own drag
//! counters plus a screenshot of the running window — never by a status field
//! that does not exist. The report is explicit about that instead of claiming a
//! measured orbit angle.
//!
//! Synthetic input, **not a real mouse**: the events are dispatched on
//! `.native-input` and never touch the kernel/evdev/XInput path.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Whether the **3D view evidence** driver is enabled (`YACR_PROBE_VIEW3D`).
///
/// Evidence-only, default OFF, independent of every other probe driver.
pub fn view3d_enabled() -> bool {
    matches!(
        std::env::var("YACR_PROBE_VIEW3D").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

/// Schedule the 3D view evidence driver if enabled. Never panics on spawn
/// failure (a failed spawn is logged; the run continues unchanged).
///
/// The thread only dispatches `Webview::eval` scripts that call the public
/// `cad_status`/`cad_command` commands and dispatch synthetic DOM pointer
/// events. No command/session logic is changed and no GPU state is touched.
pub fn maybe_spawn_view3d(webview: Option<&tauri::Webview>) {
    if !view3d_enabled() {
        return;
    }
    let Some(webview) = webview else {
        eprintln!("[probe] view3d: no child webview; 3D-view pass skipped (nothing faked)");
        return;
    };
    eprintln!(
        "[probe] view3d: enabled (YACR_PROBE_VIEW3D); synthetic DOM pointer events, NOT a real mouse; \
         steps: cad_status(before) + cad_command SWITCH2D3D + cad_status(after_switch) + \
         primary orbit drag + cad_status(after_orbit) + one report line"
    );
    let webview = webview.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("probe-view3d".into())
        .spawn(move || run_view3d(webview))
    {
        eprintln!("[probe] view3d: failed to spawn: {error}");
    }
}

/// Run the 3D view evidence sequence relative to driver start:
///
/// * t=2.5 s: read the real `cad_status` as `before` (a loaded drawing in the
///   default 2D view reports a `camera` block);
/// * t=2.7 s: dispatch `cad_command { command: "SWITCH2D3D" }`. The read above
///   is issued first so the host serves it before the switch is queued; the
///   gap keeps `before` an honest **pre-switch** read;
/// * t=3.5 s: read `cad_status` as `after_switch` (`camera` must be `absent`,
///   `error` must stay `null`);
/// * t=4.0 s: dispatch the synthetic primary drag script ([`VIEW3D_ORBIT_JS`]);
///   in the 3D view mode the host's primary-drag branch orbits instead of
///   starting a box selection;
/// * t=6.5 s: read `cad_status` as `after_orbit`;
/// * t=7.0 s (retried once at t=7.8 s if the asynchronous read has not landed):
///   print **one** report line from [`VIEW3D_REPORT_JS`] through the
///   `eval_with_callback` idiom the pan driver uses.
///
/// Every step is a dispatch only; the outcome is the **real** `cad_status`
/// snapshot read from the host, never a value synthesized here.
fn run_view3d(webview: tauri::Webview) {
    let start = Instant::now();

    schedule_view3d(&start, 2_500, "before cad_status", || {
        webview.eval(view3d_status_js("before"))
    });
    // Payload-free selector through the same `cad_command` funnel the other
    // evidence drivers use; the ack lands in the shared draw-evidence store.
    schedule_view3d(&start, 2_700, "cad_command SWITCH2D3D", || {
        webview.eval(super::draw::draw_command_js("SWITCH2D3D", "switch_2d3d"))
    });
    schedule_view3d(&start, 3_500, "after_switch cad_status", || {
        webview.eval(view3d_status_js("after_switch"))
    });
    schedule_view3d(
        &start,
        4_000,
        "synthetic primary orbit drag on .native-input (NOT a real mouse)",
        || webview.eval(VIEW3D_ORBIT_JS),
    );
    schedule_view3d(&start, 6_500, "after_orbit cad_status", || {
        webview.eval(view3d_status_js("after_orbit"))
    });

    // The status reads are asynchronous (`invoke(...).then(...)`); the report is
    // assembled from what those promises really stored. One retry covers a slow
    // read without ever printing two lines.
    let reported = Arc::new(AtomicBool::new(false));
    for millis in [7_000u64, 7_800] {
        let webview = webview.clone();
        let reported = reported.clone();
        schedule_view3d(&start, millis, "view3d report", move || {
            webview.eval_with_callback(VIEW3D_REPORT_JS, move |payload| {
                if reported.load(Ordering::SeqCst) {
                    return;
                }
                let text = decode_view3d_report(&payload);
                if text.is_empty() || text == "not-ready" {
                    eprintln!(
                        "[probe] view3d-report deferred ({text}); the after_orbit cad_status read has not landed yet"
                    );
                    return;
                }
                reported.store(true, Ordering::SeqCst);
                eprintln!("[probe] view3d-report {text}");
            })
        });
    }
    // Let the last eval callback land before deciding the report never arrived.
    std::thread::sleep(Duration::from_millis(600));
    if !reported.load(Ordering::SeqCst) {
        eprintln!(
            "[probe] view3d-report FAILED (no after_orbit cad_status read landed before the bound; nothing faked)"
        );
    }
}

/// Decode the JSON-quoted eval-callback payload into the raw report string (the
/// `eval_with_callback` idiom, identical to the pan/box reports).
fn decode_view3d_report(payload: &str) -> String {
    let trimmed = payload.trim();
    if trimmed.starts_with('"') {
        if let Ok(decoded) = serde_json::from_str::<String>(trimmed) {
            return decoded;
        }
    }
    trimmed.to_string()
}

/// Sleep until `millis` after `start`, run `action`, and log the dispatch result
/// under the view3d prefix. Only the **dispatch** is reported here; the
/// observable outcome comes from the `cad_status` reads.
fn schedule_view3d(
    start: &Instant,
    millis: u64,
    label: &str,
    action: impl FnOnce() -> tauri::Result<()>,
) {
    let target = Duration::from_millis(millis);
    let elapsed = start.elapsed();
    if target > elapsed {
        std::thread::sleep(target - elapsed);
    }
    match action() {
        Ok(()) => eprintln!("[probe] view3d step: {label} -> dispatched"),
        Err(error) => eprintln!("[probe] view3d step: {label} -> dispatch error: {error}"),
    }
}

/// Build the named status-read script (`before`, `after_switch`, `after_orbit`).
fn view3d_status_js(name: &str) -> String {
    VIEW3D_STATUS_TEMPLATE.replace("__NAME__", name)
}

/// Read the real `cad_status` and record it under `__NAME__` on
/// `window.__yacr_view3d`.
///
/// `camera` is the **real presence** of the 2D camera block (`center` +
/// `scale`), `entity_count`/`document` the real drawing state, `error` the real
/// technical error string (the surfaced refusal, if any). A 3D view reports
/// `camera: null` because the host only projects a 2D orthographic camera.
const VIEW3D_STATUS_TEMPLATE: &str = r#"
(function () {
  var d = window.__yacr_view3d = window.__yacr_view3d || {};
  var name = '__NAME__';
  try {
    var internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {
      d[name + '_failed'] = 'no-tauri-internals';
      return 'no-tauri-internals';
    }
    internals.invoke('cad_status').then(function (s) {
      var cam = !!(s.camera && s.camera.center && s.camera.center.length === 2 && s.camera.scale > 0);
      d[name] = {
        camera: cam,
        center: cam ? [s.camera.center[0], s.camera.center[1]] : null,
        scale: cam ? s.camera.scale : null,
        entity_count: (typeof s.entityCount === 'number') ? s.entityCount : null,
        document: s.documentLabel || null,
        crosshair: !!s.crosshair,
        error: s.error || null
      };
    }).catch(function (err) { d[name + '_failed'] = String(err); });
    return name + '-dispatched';
  } catch (err) {
    try { d[name + '_failed'] = 'fatal:' + err; } catch (_) {}
    return 'error:' + err;
  }
})()
"#;

/// The synthetic **primary orbit drag** (rAF-paced DOM pointer events, **not** a
/// real mouse).
///
/// Press at 35% width / 50% height, then move right/down over `DURATION` ms and
/// **always** release with a `pointerup`. The documented target is ~60% width /
/// 65% height; on a wide partition that would be a multi-radian yaw, so the
/// pixel delta is capped at ~90 px right and ~70 px down (0.008 rad/px -> ~0.72
/// rad yaw / ~0.56 rad pitch), which shows a clearly oblique cube. The real
/// pixel start/end/deltas are recorded, never a computed angle.
///
/// A timer stall-guard keeps the gesture advancing when rAF is throttled or
/// absent, and a hard `setTimeout` guarantees the final `pointerup`. Counts are
/// recorded on `window.__yacr_view3d` (`downs`/`moves`/`ups`/`failed`).
const VIEW3D_ORBIT_JS: &str = r#"
(function () {
  try {
    var d = window.__yacr_view3d = window.__yacr_view3d || {};
    var el = document.querySelector('.native-input');
    if (!el) { d.failed = 'no-element'; return 'no-element'; }
    var r = el.getBoundingClientRect();
    if (!(r.width > 20 && r.height > 20)) {
      d.failed = 'tiny-rect:' + r.width + 'x' + r.height;
      return 'tiny-rect:' + r.width + 'x' + r.height;
    }
    el.setPointerCapture = function () {};
    el.hasPointerCapture = function () { return false; };
    el.releasePointerCapture = function () {};
    var x0 = r.left + r.width * 0.35;
    var y0 = r.top + r.height * 0.50;
    var dx = Math.min(r.width * 0.25, 90);
    var dy = Math.min(r.height * 0.15, 70);
    var x1 = x0 + dx;
    var y1 = y0 + dy;
    var DURATION = 600;
    var now0 = performance.now();
    d.downs = 0;
    d.moves = 0;
    d.ups = 0;
    d.failed = '';
    d.rect = [r.left, r.top, r.width, r.height];
    d.start_px = [Math.round(x0 - r.left), Math.round(y0 - r.top)];
    d.end_px = [Math.round(x1 - r.left), Math.round(y1 - r.top)];
    d.dx_px = Math.round(dx);
    d.dy_px = Math.round(dy);
    d.drag_done = false;
    d.drag_start = now0;
    d.drag_elapsed_ms = -1;
    d.last_step = now0;
    function fire(type, x, y, button, buttons) {
      el.dispatchEvent(new PointerEvent(type, {
        pointerId: 1, pointerType: 'mouse', isPrimary: true,
        button: button, buttons: buttons,
        clientX: x, clientY: y, bubbles: true, cancelable: true
      }));
    }
    function end(x, y) {
      if (d.drag_done) { return; }
      d.drag_done = true;
      d.drag_elapsed_ms = Math.round(performance.now() - d.drag_start);
      try { fire('pointerup', x, y, 0, 0); d.ups++; }
      catch (e) { d.failed = 'up:' + e; }
      try { clearInterval(timer); } catch (e) {}
    }
    function step() {
      if (d.drag_done) { return; }
      var t = (performance.now() - d.drag_start) / DURATION;
      if (t >= 1) { end(x1, y1); return; }
      var p = t < 0 ? 0 : t;
      try {
        fire('pointermove', x0 + (x1 - x0) * p, y0 + (y1 - y0) * p, -1, 1);
        d.moves++;
        d.last_step = performance.now();
      } catch (e) { d.failed = 'move:' + e; }
    }
    try { fire('pointerdown', x0, y0, 0, 1); d.downs++; }
    catch (e) { d.failed = 'down:' + e; }
    function raf() { if (!d.drag_done) { step(); requestAnimationFrame(raf); } }
    requestAnimationFrame(raf);
    // Stall guard: if rAF is throttled/absent, keep the drag advancing.
    var timer = setInterval(function () {
      if (d.drag_done) { clearInterval(timer); return; }
      if (performance.now() - d.last_step > 100) { step(); }
    }, 50);
    // Hard guarantee: end with a pointerup even if every loop stalled.
    setTimeout(function () { end(x1, y1); }, DURATION + 500);
    return 'started start_px=' + d.start_px.join(',') + ' end_px=' + d.end_px.join(',') +
           ' dur=' + DURATION;
  } catch (e) {
    try {
      var d2 = window.__yacr_view3d = window.__yacr_view3d || {};
      d2.failed = 'fatal:' + e;
    } catch (_) {}
    return 'error:' + e;
  }
})()
"#;

/// Assemble the single report line from the **real** stored reads and the
/// driver's own synthetic-event counters.
///
/// Camera fields are `present`/`absent` only when a real read landed; a missing
/// read is `unread:<reason>` (never `absent`, which would be a claim). Errors
/// are the raw technical status strings (`null` when the read reported none).
/// The 3D camera itself is not part of `cad_status`, so no yaw/pitch is claimed
/// here — only the measured pixel drag.
const VIEW3D_REPORT_JS: &str = r#"
(function () {
  try {
    var d = window.__yacr_view3d;
    if (!d || !d.after_orbit) { return 'not-ready'; }
    function cam(name) {
      var v = d[name];
      if (!v) { return 'unread:' + (d[name + '_failed'] || 'no-read'); }
      return v.camera ? 'present' : 'absent';
    }
    function err(name) {
      var v = d[name];
      if (!v) { return 'unread:' + (d[name + '_failed'] || 'no-read'); }
      return v.error;
    }
    function field(name, key) {
      var v = d[name];
      if (!v) { return null; }
      return (typeof v[key] === 'undefined') ? null : v[key];
    }
    var raw = window.__yacr_draw || {};
    var result = {
      camera_before: cam('before'),
      camera_after_switch: cam('after_switch'),
      camera_after_orbit: cam('after_orbit'),
      error_after_switch: err('after_switch'),
      error_after_orbit: err('after_orbit'),
      switch_ack: raw['switch_2d3d_ack'] || null,
      switch_failed: raw['switch_2d3d_failed'] || null,
      document: field('after_orbit', 'document') || field('before', 'document'),
      entity_before: field('before', 'entity_count'),
      entity_after_orbit: field('after_orbit', 'entity_count'),
      downs: d.downs || 0,
      moves: d.moves || 0,
      ups: d.ups || 0,
      failed: d.failed ? String(d.failed) : null,
      drag_done: !!d.drag_done,
      drag_elapsed_ms: (typeof d.drag_elapsed_ms === 'number') ? d.drag_elapsed_ms : -1,
      rect: d.rect || null,
      start_px: d.start_px || null,
      end_px: d.end_px || null,
      dx_px: (typeof d.dx_px === 'number') ? d.dx_px : null,
      dy_px: (typeof d.dy_px === 'number') ? d.dy_px : null,
      orbit_angle_channel: 'none (cad_status exposes no 3D camera; drag pixels are the real read)'
    };
    return JSON.stringify(result);
  } catch (e) { return 'error:' + e; }
})()
"#;

#[cfg(test)]
mod tests {
    use super::decode_view3d_report;

    #[test]
    fn decodes_a_json_quoted_eval_payload() {
        let payload = r#""{\"camera_before\":\"present\"}""#;
        assert_eq!(
            decode_view3d_report(payload),
            r#"{"camera_before":"present"}"#
        );
    }

    #[test]
    fn passes_a_raw_marker_through_unchanged() {
        assert_eq!(decode_view3d_report("  not-ready\n"), "not-ready");
    }
}
