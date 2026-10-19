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

/// Run the evidence sequence relative to driver start (7 steps):
/// t=10s read the real `cad_status` baseline, t=12s admit `LINE`, t=13s/15s
/// click two **world** points that coincide with real object-snap targets (the
/// endpoints of `fixtures/dwg/synthetic-four-lines.dwg`'s vertical lines),
/// t=18s move-only probe 10 px off the second target (the marker stays on the
/// snapped point while the crosshair follows the pointer), t=21s a second
/// move-only probe on a fixture **midpoint** (a second per-kind glyph), t=25s
/// admit `CONFIRM` (drawing tools never auto-commit), t=27s read the real
/// `cad_status` after the commit can land, t=29s log one single-line report via
/// the eval callback. Each state is held for ~3 s so one integer `--capture-at`
/// lands on it regardless of the app's start-up jitter.
///
/// The click positions are computed from the app's own `cad_status.camera` and
/// the live `.native-input` rect, so the tool cursor lands **on** a snap target
/// and `snapKind` resolves instead of staying `null` (a fixed fraction of the
/// element could not: on the four-line fixture it falls in empty space, ~100 px
/// from any candidate, far outside the ~12-logical-pixel snap aperture).
///
/// Observed on the real fixture: the resolved `endpoint` snap (and therefore the
/// drawn marker) is on screen from just after the **second** click until
/// `CONFIRM`, so an external `--capture-at 17` records the marker with the
/// pointer **on** the snap point (marker and pickbox concentric), `--capture-at 20`
/// records it with the pointer 10 px off it (marker and pickbox side by side), and
/// `--capture-at 23` records the **midpoint** glyph (triangle) instead of the
/// endpoint square. The first click's point did not resolve on this fixture even
/// though it names an endpoint in the fixture's documented summary; an in-process
/// contract test (`cad-host::tests::snap_marker`) pins that a `LINE` endpoint
/// under the cursor does resolve as `endpoint`, so the marker path itself is
/// exercised either way.
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

    // Click A on the world endpoint (0, 0) of the fixture's line at x = 0: a real
    // `SnapKind::Endpoint` target, so the cursor snaps and the marker is drawn.
    let click_a = tool_click_world_js(0.0, 0.0, "A");
    schedule_tool(
        &start,
        13,
        "click A at world (0,0) (a fixture LINE endpoint)",
        || webview.eval(click_a),
    );

    // Click B on the world endpoint (3, 1) of the fixture's line at x = 3: a real
    // `SnapKind::Endpoint` target, so the cursor snaps and the marker is drawn.
    // At this instant the pointer *is* on the snap point, so the marker and the
    // pickbox share a centre — the case where the marker must still dominate.
    let click_b = tool_click_world_js(3.0, 1.0, "B");
    schedule_tool(
        &start,
        15,
        "click B at world (3,1) (a fixture LINE endpoint)",
        || webview.eval(click_b),
    );

    // A **move-only** probe at world (2.95, 1.0): still inside the ~12-logical-px
    // snap aperture of the endpoint (3, 1), so the tool cursor snaps to the
    // endpoint (the marker stays there) while the crosshair follows the pointer
    // 10 px away. This is the "pointer not exactly on the snap point" case, and
    // it makes the marker and the pickbox fully separable in a frame.
    let probe = tool_hover_world_js(2.95, 1.0, "P");
    schedule_tool(
        &start,
        18,
        "hover probe at world (2.95,1.0) (snaps to (3,1), crosshair 10px away)",
        || webview.eval(probe),
    );

    // A second move-only probe on the **midpoint** of the fixture's line at x = 1
    // (world (1, 0.5)): a different `SnapKind`, so a capture here shows a second
    // per-kind glyph (midpoint = triangle) at the same size.
    let mid = tool_hover_world_js(1.0, 0.5, "M");
    schedule_tool(
        &start,
        21,
        "hover probe at world (1,0.5) (a fixture LINE midpoint)",
        || webview.eval(mid),
    );

    schedule_tool(&start, 25, "cad_command CONFIRM", || {
        webview.eval(TOOL_CONFIRM_JS)
    });

    schedule_tool(&start, 27, "after cad_status", || {
        webview.eval(TOOL_AFTER_JS)
    });
    // The report is logged from Rust through the eval callback; the JS also
    // `console.log`s it. A dispatch that merely posts is not a pass — the
    // report carries the real before/after counts.
    schedule_tool(&start, 29, "tool evidence report", || {
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

/// Build one synthetic click at a **world** point that coincides with a real
/// object-snap target. Synthetic, **not a real mouse**.
fn tool_click_world_js(wx: f64, wy: f64, label: &str) -> String {
    TOOL_CLICK_TEMPLATE
        .replace("__WX__", &wx.to_string())
        .replace("__WY__", &wy.to_string())
        .replace("__LABEL__", label)
}

/// Build one **move-only** synthetic probe at a **world** point (no press, so the
/// tool captures no point): it moves the pointer and therefore the crosshair
/// while the tool cursor keeps snapping to the same target. Synthetic, **not a
/// real mouse**.
fn tool_hover_world_js(wx: f64, wy: f64, label: &str) -> String {
    TOOL_HOVER_TEMPLATE
        .replace("__WX__", &wx.to_string())
        .replace("__WY__", &wy.to_string())
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

/// One synthetic click at a **world** point: the script reads the real
/// `cad_status.camera` and the live `.native-input` rect and converts the world
/// point to rect fractions before dispatching `pointermove` (hover),
/// `pointerdown` (button 0 / buttons 1) and `pointerup` (button 0 / buttons 0).
/// Pointer capture is shimmed because these events are not backed by a device.
/// `__WX__`/`__WY__` are world coordinates; `__LABEL__` names the click.
/// Synthetic, **not a real mouse**.
const TOOL_CLICK_TEMPLATE: &str = r#"
(function () {
  var t = window.__yacr_tool = window.__yacr_tool || {};
  var label = '__LABEL__';
  try {
    var el = document.querySelector('.native-input');
    if (!el) { t.clicks_failed = 'no-element'; return 'no-element'; }
    var r = el.getBoundingClientRect();
    if (!(r.width > 20 && r.height > 20)) {
      t.clicks_failed = 'tiny-rect:' + r.width + 'x' + r.height;
      return 'tiny-rect:' + r.width + 'x' + r.height;
    }
    var internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {
      t.clicks_failed = 'no-tauri-internals';
      return 'no-tauri-internals';
    }
    internals.invoke('cad_status').then(function (s) {
      if (!s.camera || !(s.camera.scale > 0)) {
        t.clicks_failed = label + ':no-camera';
        return;
      }
      var cx = s.camera.center[0];
      var cy = s.camera.center[1];
      var sc = s.camera.scale;
      var fx = 0.5 + (__WX__ - cx) / (sc * r.width);
      var fy = 0.5 - (__WY__ - cy) / (sc * r.height);
      t.last_world = [__WX__, __WY__];
      if (!(fx >= 0 && fx <= 1 && fy >= 0 && fy <= 1)) {
        t.clicks_failed = label + ':outside-canvas:' + fx + ',' + fy;
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
      t.clicks = (t.clicks || 0) + 1;
      t.last_click = label + '@world(' + __WX__ + ',' + __WY__ + ') px(' +
        Math.round(x - r.left) + ',' + Math.round(y - r.top) + ')';
    }).catch(function (e) { t.clicks_failed = label + ':' + String(e); });
    return 'click-dispatched ' + label;
  } catch (e) {
    try { t.clicks_failed = label + ':' + e; } catch (_) {}
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

/// One **move-only** synthetic probe at a **world** point: the script reads the
/// real `cad_status.camera` and the live `.native-input` rect, converts the world
/// point to rect fractions and dispatches a single `pointermove` (hover) — no
/// press, so the active tool captures no point. Synthetic, **not a real mouse**.
const TOOL_HOVER_TEMPLATE: &str = r#"
(function () {
  var t = window.__yacr_tool = window.__yacr_tool || {};
  var label = '__LABEL__';
  try {
    var el = document.querySelector('.native-input');
    if (!el) { t.hover_failed = 'no-element'; return 'no-element'; }
    var r = el.getBoundingClientRect();
    if (!(r.width > 20 && r.height > 20)) {
      t.hover_failed = 'tiny-rect:' + r.width + 'x' + r.height;
      return 'tiny-rect:' + r.width + 'x' + r.height;
    }
    var internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {
      t.hover_failed = 'no-tauri-internals';
      return 'no-tauri-internals';
    }
    internals.invoke('cad_status').then(function (s) {
      if (!s.camera || !(s.camera.scale > 0)) { t.hover_failed = label + ':no-camera'; return; }
      var cx = s.camera.center[0];
      var cy = s.camera.center[1];
      var sc = s.camera.scale;
      var fx = 0.5 + (__WX__ - cx) / (sc * r.width);
      var fy = 0.5 - (__WY__ - cy) / (sc * r.height);
      if (!(fx >= 0 && fx <= 1 && fy >= 0 && fy <= 1)) {
        t.hover_failed = label + ':outside-canvas:' + fx + ',' + fy;
        return;
      }
      el.dispatchEvent(new PointerEvent('pointermove', {
        pointerId: 1, pointerType: 'mouse', isPrimary: true,
        button: -1, buttons: 0,
        clientX: r.left + r.width * fx, clientY: r.top + r.height * fy,
        bubbles: true, cancelable: true
      }));
      t.last_hover = label + '@world(' + __WX__ + ',' + __WY__ + ') px(' +
        Math.round(r.width * fx) + ',' + Math.round(r.height * fy) + ')';
    }).catch(function (e) { t.hover_failed = label + ':' + String(e); });
    return 'hover-dispatched ' + label;
  } catch (e) {
    try { t.hover_failed = label + ':' + e; } catch (_) {}
    return 'error:' + e;
  }
})()
"#;
