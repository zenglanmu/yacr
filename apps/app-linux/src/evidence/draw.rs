//! `YACR_PROBE_DRAW_EVIDENCE`: the new-drawing-command evidence driver.
//!
//! Evidence-only, default OFF, and independent of every other probe driver
//! (setting only this one does not trigger the input/tool/hover/lifecycle/box/
//! edit passes). It drives the **real** new drawing selectors against a real
//! running host through the normal `cad_command` funnel and synthetic DOM
//! pointer events on `.native-input`, then reads the **real** `cad_status`
//! before/after each commit.
//!
//! What it proves, one phase per claim:
//! 1. **PLINE end to end**: start `PLINE` through the real selector, click three
//!    points, dispatch `CONFIRM` (Enter) and read `cad_status`: `entityCount`
//!    must rise by exactly one and `tool` must return to `null`. This is
//!    multi-point capture plus the Enter-terminated commit on the real host.
//! 2. **POLYGON with a changed side count**: call the real IPC
//!    `cad_set_tool_parameter` with a non-default value (6) for `polygonSides`,
//!    start `POLYGON`, click centre + radius, `CONFIRM`, and read `entityCount` +1
//!    and `tool` `null`. Separately it asserts the side count is visible while the
//!    tool is active (`tool.params.sides == 6`), and that an out-of-range value (2)
//!    is **refused** with the real error text and leaves the visible value
//!    unchanged.
//! 3. **POINT visibility**: start `POINT`, click one point, `CONFIRM`, read
//!    `entityCount` +1, and leave the created POINT on screen so an external
//!    screenshot can be pixel-checked for the marker colour by
//!    `apps/app-linux/scripts/check-point-pixels.py`.
//! 4. **RECTANG end to end** (same funnel): `entityCount` delta +1.
//!
//! Synthetic input, **not a real mouse**: the events are dispatched on
//! `.native-input` and never touch the kernel/evdev/XInput path. Every reported
//! number is a real `cad_status` read. `cad_status` exposes **no** per-entity
//! geometry and **no** vertex count, so the polygon's real 6-sidedness is **not**
//! observable through the status channel; the report says so plainly
//! (`vertex_count_channel=none_...`) instead of claiming a vertex count it
//! cannot read. The one side-count observable that does exist
//! (`tool.params.sides`) is reported verbatim.
//!
//! Fixture note (verified against `fixtures/dwg/synthetic-four-lines.dwg`
//! through the importer contract): the drawing is four mutually parallel
//! vertical LINEs at world x = 0, 1, 2, 3 spanning y = 0..1. Every synthetic
//! click below targets a world point inside that extent, converted to
//! `.native-input` fractions through the app's own `cad_status.camera`.

use std::time::{Duration, Instant};

/// Whether the **new drawing-command evidence** driver is enabled
/// (`YACR_PROBE_DRAW_EVIDENCE`).
///
/// Evidence-only, default OFF, independent of every other probe driver.
///
/// Synthetic input, **not a real mouse**: the events are DOM pointer dispatches
/// on `.native-input`; the kernel/evdev/XInput path is never touched.
pub fn draw_evidence_enabled() -> bool {
    matches!(
        std::env::var("YACR_PROBE_DRAW_EVIDENCE").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

/// Schedule the new drawing-command evidence driver if enabled. Never panics on
/// spawn failure (a failed spawn is logged; the run continues unchanged).
///
/// The thread only dispatches `Webview::eval` scripts that call the public
/// `cad_status`/`cad_command`/`cad_set_tool_parameter` commands and dispatch
/// synthetic DOM pointer events. No command/session logic is changed and no GPU
/// state is touched.
///
/// The host-init entry point lives in [`super::tool`] (the evidence subsystem is
/// self-contained this round, so no file outside `src/evidence/` is touched
/// except the module declaration in `evidence.rs`).
pub fn maybe_spawn_draw_evidence(webview: Option<&tauri::Webview>) {
    if !draw_evidence_enabled() {
        return;
    }
    let Some(webview) = webview else {
        eprintln!("[probe] draw evidence: no child webview; draw pass skipped (nothing faked)");
        return;
    };
    eprintln!(
        "[probe] draw evidence: enabled (YACR_PROBE_DRAW_EVIDENCE); synthetic DOM pointer events, NOT a real mouse; \
         phases: PLINE(3pt+CONFIRM) POLYGON(sides=6, out-of-range=2) POINT(visible) RECTANG"
    );
    let webview = webview.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("probe-draw-evidence".into())
        .spawn(move || run_draw_evidence(webview))
    {
        eprintln!("[probe] draw evidence: failed to spawn: {error}");
    }
}

/// Run the draw evidence sequence relative to driver start:
///
/// * t=10s read the real `cad_status` baseline (`entityCount`, `tool`);
/// * **PLINE**: t=12s start `PLINE`; t=13s read (capture `draw.polyline`);
///   t=14/15/16s click three world points; t=17s read (`points` = 3); t=18s
///   `CONFIRM`; t=19s read (`entityCount` +1, `tool` `null`);
/// * **POLYGON**: t=21s `cad_set_tool_parameter polygonSides=6`; t=22s start
///   `POLYGON`; t=23s read (`tool.params.sides` = 6); t=24s
///   `cad_set_tool_parameter polygonSides=2` (out of range); t=25s read (refusal
///   in `error`, `sides` still 6); t=26/27s click centre + radius; t=28s read
///   (`points` = 2); t=29s `CONFIRM`; t=30s read (`entityCount` +1, `tool`
///   `null`);
/// * **POINT**: t=32s start `POINT`; t=33s click one world point; t=34s read
///   (`points` = 1); t=35s `CONFIRM`; t=36s read (`entityCount` +1, `tool`
///   `null`) — the POINT is now on screen for an external `--capture-at 37`;
/// * **RECTANG**: t=38s start `RECTANG`; t=39/40s click two corners; t=41s read
///   (`points` = 2); t=42s `CONFIRM`; t=43s read (`entityCount` +1);
/// * t=45s log one single-line report via the eval callback.
///
/// Every step is a dispatch only; the **real** outcome is the `cad_status`
/// snapshot read from the host, never a value synthesized here. Pointer
/// positions are computed from the app's own `cad_status.camera` (centre +
/// world-per-logical-pixel) and the live `.native-input` rect.
fn run_draw_evidence(webview: tauri::Webview) {
    let start = Instant::now();

    schedule_draw(&start, 10, "baseline cad_status", || {
        webview.eval(draw_status_js("s_base"))
    });

    // PLINE: three points + CONFIRM.
    schedule_draw(&start, 12, "cad_command PLINE", || {
        webview.eval(draw_start_js("PLINE"))
    });
    schedule_draw(&start, 13, "pline-started cad_status", || {
        webview.eval(draw_status_js("s_pline_started"))
    });
    let pline_a = draw_click_world_js(0.4, 0.3, "pline_a");
    schedule_draw(&start, 14, "pline click A at world (0.4, 0.3)", || {
        webview.eval(pline_a)
    });
    let pline_b = draw_click_world_js(1.5, 0.8, "pline_b");
    schedule_draw(&start, 15, "pline click B at world (1.5, 0.8)", || {
        webview.eval(pline_b)
    });
    let pline_c = draw_click_world_js(2.6, 0.3, "pline_c");
    schedule_draw(&start, 16, "pline click C at world (2.6, 0.3)", || {
        webview.eval(pline_c)
    });
    schedule_draw(&start, 17, "pline-points cad_status", || {
        webview.eval(draw_status_js("s_pline_points"))
    });
    schedule_draw(&start, 18, "cad_command CONFIRM (pline)", || {
        webview.eval(DRAW_CONFIRM_JS)
    });
    schedule_draw(&start, 19, "pline-after cad_status", || {
        webview.eval(draw_status_js("s_pline_after"))
    });

    // POLYGON: set a non-default side count, prove it is visible, prove an
    // out-of-range value is refused without changing it, then commit.
    schedule_draw(&start, 21, "cad_set_tool_parameter polygonSides=6", || {
        webview.eval(draw_set_parameter_number_js("polygonSides", 6.0, "sides_6"))
    });
    schedule_draw(&start, 22, "cad_command POLYGON", || {
        webview.eval(draw_start_js("POLYGON"))
    });
    schedule_draw(
        &start,
        23,
        "polygon-active cad_status (sides must read 6)",
        || webview.eval(draw_status_js("s_polygon_active")),
    );
    schedule_draw(
        &start,
        24,
        "cad_set_tool_parameter polygonSides=2 (out of range)",
        || webview.eval(draw_set_parameter_number_js("polygonSides", 2.0, "sides_2")),
    );
    schedule_draw(
        &start,
        25,
        "polygon-bad-sides cad_status (refusal + unchanged value)",
        || webview.eval(draw_status_js("s_polygon_bad")),
    );
    let poly_c = draw_click_world_js(1.5, 0.4, "polygon_center");
    schedule_draw(
        &start,
        26,
        "polygon centre click at world (1.5, 0.4)",
        || webview.eval(poly_c),
    );
    let poly_r = draw_click_world_js(2.1, 0.4, "polygon_radius");
    schedule_draw(
        &start,
        27,
        "polygon radius click at world (2.1, 0.4)",
        || webview.eval(poly_r),
    );
    schedule_draw(&start, 28, "polygon-points cad_status", || {
        webview.eval(draw_status_js("s_polygon_points"))
    });
    schedule_draw(&start, 29, "cad_command CONFIRM (polygon)", || {
        webview.eval(DRAW_CONFIRM_JS)
    });
    schedule_draw(&start, 30, "polygon-after cad_status", || {
        webview.eval(draw_status_js("s_polygon_after"))
    });

    // POINT: one point + CONFIRM; the created marker must be on screen at the
    // external capture instant (t=37), which is after this commit and before
    // the RECTANG phase below.
    schedule_draw(&start, 32, "cad_command POINT", || {
        webview.eval(draw_start_js("POINT"))
    });
    let point_a = draw_click_world_js(2.4, 0.7, "point");
    schedule_draw(&start, 33, "point click at world (2.4, 0.7)", || {
        webview.eval(point_a)
    });
    schedule_draw(&start, 34, "point-points cad_status", || {
        webview.eval(draw_status_js("s_point_points"))
    });
    schedule_draw(&start, 35, "cad_command CONFIRM (point)", || {
        webview.eval(DRAW_CONFIRM_JS)
    });
    schedule_draw(&start, 36, "point-after cad_status", || {
        webview.eval(draw_status_js("s_point_after"))
    });

    // RECTANG: same funnel, one more `entityCount` delta.
    schedule_draw(&start, 38, "cad_command RECTANG", || {
        webview.eval(draw_start_js("RECTANG"))
    });
    let rect_a = draw_click_world_js(0.3, 0.2, "rectang_a");
    schedule_draw(&start, 39, "rectang click A at world (0.3, 0.2)", || {
        webview.eval(rect_a)
    });
    let rect_b = draw_click_world_js(1.1, 0.7, "rectang_b");
    schedule_draw(&start, 40, "rectang click B at world (1.1, 0.7)", || {
        webview.eval(rect_b)
    });
    schedule_draw(&start, 41, "rectang-points cad_status", || {
        webview.eval(draw_status_js("s_rectang_points"))
    });
    schedule_draw(&start, 42, "cad_command CONFIRM (rectang)", || {
        webview.eval(DRAW_CONFIRM_JS)
    });
    schedule_draw(&start, 43, "rectang-after cad_status", || {
        webview.eval(draw_status_js("s_rectang_after"))
    });

    // The report is logged from Rust through the eval callback; the JS also
    // `console.log`s it. A dispatch that merely posts is not a pass — the report
    // carries the real before/after reads and the real refusals.
    schedule_draw(&start, 45, "draw evidence report", || {
        webview.eval_with_callback(DRAW_REPORT_JS, |payload| {
            eprintln!(
                "[probe] draw evidence report: {}",
                decode_draw_report(&payload)
            );
        })
    });
}

/// Decode the JSON-quoted eval-callback payload into the raw report string.
fn decode_draw_report(payload: &str) -> String {
    let trimmed = payload.trim();
    if trimmed.starts_with('"') {
        if let Ok(decoded) = serde_json::from_str::<String>(trimmed) {
            return decoded;
        }
    }
    trimmed.to_string()
}

/// Sleep until `seconds` after `start`, run `action`, and log the dispatch
/// result under the draw evidence prefix. Only the **dispatch** is reported
/// here; the observable outcome comes from the `cad_status` reads.
fn schedule_draw(
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
        Ok(()) => eprintln!("[probe] draw evidence step: {label} -> dispatched"),
        Err(error) => eprintln!("[probe] draw evidence step: {label} -> dispatch error: {error}"),
    }
}

/// Build the named status-read script (`s_base`, `s_pline_points`, ...).
fn draw_status_js(name: &str) -> String {
    DRAW_STATUS_TEMPLATE.replace("__NAME__", name)
}

/// Build the start script for one draw selector (`PLINE` / `POLYGON` / ...).
fn draw_start_js(tool: &str) -> String {
    DRAW_START_TEMPLATE.replace("__TOOL__", tool)
}

/// Build the `cad_set_tool_parameter` script for one numeric parameter value.
///
/// `__PARAM__` is the parameter's stable machine key (e.g. `polygonSides`),
/// `__NUMBER__` the numeric value and `__LABEL__` the report slot name.
fn draw_set_parameter_number_js(parameter: &str, value: f64, label: &str) -> String {
    DRAW_SET_PARAMETER_TEMPLATE
        .replace("__PARAM__", parameter)
        .replace("__NUMBER__", &value.to_string())
        .replace("__LABEL__", label)
}

/// Build one synthetic click at a **world** point: the script reads the real
/// `cad_status.camera` and the live `.native-input` rect and converts the world
/// point to rect fractions before dispatching the pointer events. Synthetic,
/// **not a real mouse**.
fn draw_click_world_js(wx: f64, wy: f64, label: &str) -> String {
    DRAW_CLICK_TEMPLATE
        .replace("__WX__", &wx.to_string())
        .replace("__WY__", &wy.to_string())
        .replace("__LABEL__", label)
}

/// Read the real `cad_status` and record it under `__NAME__`.
///
/// `points` is the tool's captured-point count, `ready` its confirmability,
/// `sides` the POLYGON `tool.params.sides` (or `null`), and `error` the real
/// technical error string (the surfaced refusal, if any).
const DRAW_STATUS_TEMPLATE: &str = r#"
(function () {
  var d = window.__yacr_draw = window.__yacr_draw || {};
  var name = '__NAME__';
  try {
    var internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {
      d[name + '_failed'] = 'no-tauri-internals';
      return 'no-tauri-internals';
    }
    internals.invoke('cad_status').then(function (s) {
      d[name + '_done'] = true;
      d[name + '_count'] = s.entityCount;
      d[name + '_tool'] = s.tool ? s.tool.key : null;
      d[name + '_points'] = s.tool ? s.tool.points : -1;
      d[name + '_ready'] = s.tool ? !!s.tool.ready : false;
      d[name + '_sides'] = (s.tool && s.tool.params && typeof s.tool.params.sides !== 'undefined')
        ? s.tool.params.sides : null;
      d[name + '_selection'] = s.selection ? s.selection.count : null;
      d[name + '_error'] = s.error;
      if (s.camera && s.camera.center && s.camera.center.length === 2) {
        d.camera = [s.camera.center[0], s.camera.center[1], s.camera.scale];
      }
    }).catch(function (err) { d[name + '_failed'] = String(err); });
    return name + '-dispatched';
  } catch (err) {
    try { d[name + '_failed'] = 'fatal:' + err; } catch (_) {}
    return 'error:' + err;
  }
})()
"#;

/// Start the real interactive draw tool through the public `cad_command`
/// command. `__TOOL__` is the canonical selector (`PLINE` / `POLYGON` / ...).
const DRAW_START_TEMPLATE: &str = r#"
(function () {
  var d = window.__yacr_draw = window.__yacr_draw || {};
  try {
    var internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {
      d.start_failed = 'no-tauri-internals';
      return 'no-tauri-internals';
    }
    internals.invoke('cad_command', { command: '__TOOL__' }).then(function (ack) {
      d.start_ack = (ack && typeof ack.accepted !== 'undefined')
        ? 'accepted=' + ack.accepted
        : String(JSON.stringify(ack));
    }).catch(function (err) { d.start_failed = String(err); });
    return 'start-dispatched';
  } catch (err) {
    try { d.start_failed = 'fatal:' + err; } catch (_) {}
    return 'error:' + err;
  }
})()
"#;

/// Call the real `cad_set_tool_parameter` IPC with one numeric value.
///
/// `__PARAM__` is the parameter's stable machine key (`polygonSides` here),
/// `__NUMBER__` the numeric value, `__LABEL__` the report slot name (e.g.
/// `sides_6`). This deliberately bypasses the frontend range guard and reaches the
/// host: the host admits it into the bounded queue and the owner tick applies
/// `CadSession::set_tool_parameter`, whose authoritative per-parameter check
/// refuses an out-of-range value explicitly (never a silent clamp).
/// `{accepted:true}` is queue admission only; the real outcome surfaces through
/// `cad_status.error`.
const DRAW_SET_PARAMETER_TEMPLATE: &str = r#"
(function () {
  var d = window.__yacr_draw = window.__yacr_draw || {};
  var label = '__LABEL__';
  try {
    var internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {
      d.sides_failed = 'no-tauri-internals';
      return 'no-tauri-internals';
    }
    internals.invoke('cad_set_tool_parameter', { parameter: '__PARAM__', number: __NUMBER__ }).then(function (ack) {
      d[label + '_ack'] = (ack && typeof ack.accepted !== 'undefined')
        ? 'accepted=' + ack.accepted
        : String(JSON.stringify(ack));
    }).catch(function (err) { d[label + '_failed'] = String(err); });
    return 'set-parameter-dispatched';
  } catch (err) {
    try { d.sides_failed = 'fatal:' + err; } catch (_) {}
    return 'error:' + err;
  }
})()
"#;

/// Confirm the started draw tool through the public `cad_command` command.
///
/// Drawing tools never auto-commit, so the captured points alone leave
/// `entityCount` unchanged; `CONFIRM` (Enter) is what commits the entity.
/// Synthetic input, **not a real mouse**.
const DRAW_CONFIRM_JS: &str = r#"
(function () {
  var d = window.__yacr_draw = window.__yacr_draw || {};
  try {
    var internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {
      d.confirm_failed = 'no-tauri-internals';
      return 'no-tauri-internals';
    }
    internals.invoke('cad_command', { command: 'CONFIRM' }).then(function (ack) {
      d.confirm_ack = (ack && typeof ack.accepted !== 'undefined')
        ? 'accepted=' + ack.accepted
        : String(JSON.stringify(ack));
    }).catch(function (err) { d.confirm_failed = String(err); });
    return 'confirm-dispatched';
  } catch (err) {
    try { d.confirm_failed = 'fatal:' + err; } catch (_) {}
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
const DRAW_CLICK_TEMPLATE: &str = r#"
(function () {
  var d = window.__yacr_draw = window.__yacr_draw || {};
  var label = '__LABEL__';
  try {
    var el = document.querySelector('.native-input');
    if (!el) { d[label + '_failed'] = 'no-element'; return 'no-element'; }
    var r = el.getBoundingClientRect();
    if (!(r.width > 20 && r.height > 20)) {
      d[label + '_failed'] = 'tiny-rect:' + r.width + 'x' + r.height;
      return 'tiny-rect:' + r.width + 'x' + r.height;
    }
    var internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {
      d[label + '_failed'] = 'no-tauri-internals';
      return 'no-tauri-internals';
    }
    internals.invoke('cad_status').then(function (s) {
      if (!s.camera || !(s.camera.scale > 0)) {
        d[label + '_failed'] = 'no-camera';
        return;
      }
      var cx = s.camera.center[0];
      var cy = s.camera.center[1];
      var sc = s.camera.scale;
      var fx = 0.5 + (__WX__ - cx) / (sc * r.width);
      var fy = 0.5 - (__WY__ - cy) / (sc * r.height);
      d[label + '_world'] = [__WX__, __WY__];
      d[label + '_frac'] = [fx, fy];
      d.camera = [cx, cy, sc];
      d.rect = [r.left, r.top, r.width, r.height];
      if (!(fx >= 0 && fx <= 1 && fy >= 0 && fy <= 1)) {
        d[label + '_failed'] = 'outside-canvas:' + fx + ',' + fy;
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
      d.clicks = (d.clicks || 0) + 1;
      d[label + '_px'] = [Math.round(x - r.left), Math.round(y - r.top)];
      d[label + '_dispatched'] = true;
    }).catch(function (err) { d[label + '_failed'] = String(err); });
    return label + '-dispatched';
  } catch (err) {
    try { d[label + '_failed'] = 'fatal:' + err; } catch (_) {}
    return 'error:' + err;
  }
})()
"#;

/// Single-line report over `window.__yacr_draw`.
///
/// Fields are whitespace-flattened so the line stays tokenizable; `-` means
/// absent and `-1` a count/value that was never read. Each `*_delta` is only a
/// real number when both counts were read. `polygon_sides_visible` and
/// `polygon_bad_refused` are derived from the real reads (never asserted).
/// `vertex_count_channel=none_...` is the honest statement that `cad_status`
/// exposes no vertex/geometry count, so the polygon's real sidedness cannot be
/// read from the status channel.
const DRAW_REPORT_JS: &str = r#"
(function () {
  var d = window.__yacr_draw || {};
  function v(x) {
    if (x === null || typeof x === 'undefined') { return '-'; }
    try { return String(x).replace(/\s+/g, '_'); } catch (err) { return '?'; }
  }
  function n(x) { return (typeof x === 'number') ? x : -1; }
  function arr(x) {
    if (!x || !x.length) { return '-'; }
    try { return x.map(function (k) { return String(k); }).join(','); } catch (err) { return '?'; }
  }
  function delta(before, after) {
    var b = n(before), a = n(after);
    return (b >= 0 && a >= 0) ? (a - b) : -1;
  }
  var active_sides = d.s_polygon_active_sides;
  var bad_sides = d.s_polygon_bad_sides;
  var bad_error = d.s_polygon_bad_error;
  var sides_visible = (active_sides === 6);
  var bad_refused = (typeof bad_error === 'string' && bad_error.length > 0) &&
    (bad_sides === active_sides);
  var status_failed = d.s_base_failed || d.s_pline_started_failed || d.s_pline_points_failed ||
    d.s_pline_after_failed || d.s_polygon_active_failed || d.s_polygon_bad_failed ||
    d.s_polygon_points_failed || d.s_polygon_after_failed || d.s_point_points_failed ||
    d.s_point_after_failed || d.s_rectang_points_failed || d.s_rectang_after_failed;
  var text = 'draw_synthetic=1_not_a_real_mouse' +
    ' baseline_count=' + v(d.s_base_count) +
    ' pline_before_count=' + v(d.s_base_count) +
    ' pline_started_tool=' + v(d.s_pline_started_tool) +
    ' pline_points=' + v(d.s_pline_points_points) +
    ' pline_after_count=' + v(d.s_pline_after_count) +
    ' pline_delta=' + delta(d.s_base_count, d.s_pline_after_count) +
    ' pline_after_tool=' + v(d.s_pline_after_tool) +
    ' polygon_set6_ack=' + v(d['sides_6_ack']) +
    ' polygon_active_tool=' + v(d.s_polygon_active_tool) +
    ' polygon_active_sides=' + v(active_sides) +
    ' polygon_sides_visible=(' + v(active_sides) + '==6)=' + sides_visible +
    ' polygon_bad_ack=' + v(d['sides_2_ack']) +
    ' polygon_bad_sides=' + v(bad_sides) +
    ' polygon_bad_error=' + v(bad_error) +
    ' polygon_bad_refused=' + bad_refused +
    ' polygon_points=' + v(d.s_polygon_points_points) +
    ' polygon_after_count=' + v(d.s_polygon_after_count) +
    ' polygon_delta=' + delta(d.s_pline_after_count, d.s_polygon_after_count) +
    ' polygon_after_tool=' + v(d.s_polygon_after_tool) +
    ' point_points=' + v(d.s_point_points_points) +
    ' point_after_count=' + v(d.s_point_after_count) +
    ' point_delta=' + delta(d.s_polygon_after_count, d.s_point_after_count) +
    ' point_after_tool=' + v(d.s_point_after_tool) +
    ' rectang_points=' + v(d.s_rectang_points_points) +
    ' rectang_after_count=' + v(d.s_rectang_after_count) +
    ' rectang_delta=' + delta(d.s_point_after_count, d.s_rectang_after_count) +
    ' rectang_after_tool=' + v(d.s_rectang_after_tool) +
    ' vertex_count_channel=none_cad_status_has_no_vertex_or_entity_geometry' +
    ' point_world=' + arr(d.point_world) +
    ' point_frac=' + arr(d.point_frac) +
    ' camera=' + arr(d.camera) +
    ' rect=' + arr(d.rect) +
    ' clicks=' + n(d.clicks) +
    ' start_failed=' + v(d.start_failed) +
    ' sides_failed=' + v(d.sides_failed) +
    ' confirm_failed=' + v(d.confirm_failed) +
    ' status_failed=' + v(status_failed) +
    ' pline_a_failed=' + v(d.pline_a_failed) +
    ' pline_b_failed=' + v(d.pline_b_failed) +
    ' pline_c_failed=' + v(d.pline_c_failed) +
    ' polygon_center_failed=' + v(d.polygon_center_failed) +
    ' polygon_radius_failed=' + v(d.polygon_radius_failed) +
    ' point_failed=' + v(d.point_failed) +
    ' rectang_a_failed=' + v(d.rectang_a_failed) +
    ' rectang_b_failed=' + v(d.rectang_b_failed);
  try { console.log('[probe] draw evidence report: ' + text); } catch (err) {}
  return text;
})()
"#;
