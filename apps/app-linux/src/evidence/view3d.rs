//! `YACR_PROBE_VIEW3D`: the 3D-view evidence driver.
//!
//! Evidence-only, default OFF, and independent of every other probe driver
//! (setting only this one does not trigger the input/lifecycle/pan/tool/box/
//! hover/edit/draw/popup passes). It proves the ACIS solid is genuinely
//! three-dimensional against a **real running host**:
//!
//! 1. **2D control** — click the same canvas fraction that the 3D phase later
//!    clicks, before the mode switch, and read the real `cad_status` selection.
//!    This separates "picking is broken in general" from "picking is broken in
//!    3D only";
//! 2. read the real `cad_status` (the 2D view reports a `camera` block);
//! 3. dispatch the payload-free `SWITCH2D3D` selector through the real
//!    `cad_command` funnel (the host switches the session view mode to the
//!    perspective 3D camera, so `cad_status.camera` becomes `null`);
//! 4. run a synthetic **primary drag** on `.native-input`, which the host
//!    arbitrates as an orbit in 3D (the same drag is a box selection in 2D);
//! 5. click **empty 3D space** (the deselect control: a real click must clear the
//!    selection left over from the 2D control), then click the **solid** at the
//!    same 50%/45% fraction, reading `cad_status` after each click;
//! 6. print one report line carrying only real reads plus the driver's own
//!    synthetic-event counters (`selection` count/rows/ids, `error`, `tool`,
//!    `hoverPreview`, `snapKind`, click coordinates and rect).
//!
//! ## `YACR_PROBE_VIEW3D_STANDARD=<view>`: the typed-IPC proof
//!
//! A second, separately env-gated phase (`YACR_PROBE_VIEW3D_STANDARD`) proves the
//! **`cad_standard_view`** IPC end to end with **no synthetic input at all**: it
//! reads `cad_status`, invokes the typed command once with the named view,
//! reads `cad_status` again and reports the ack, the real error string and
//! whether the camera block became absent (3D). Nothing here drags, clicks or
//! types, so an isometric frame from this phase can only come from the IPC. The
//! phase is independent: with only `YACR_PROBE_VIEW3D` set the run is exactly the
//! drag-orbit sequence above.
//!
//! The click is a press and a release **at the same point** (the input lane
//! classifies exactly that as a click; any move past the drag threshold makes it
//! a gesture, which never selects). A preceding idle `pointermove` lets the host
//! resolve its hover state first.
//!
//! Nothing here is a product behaviour change, and no number is synthesized:
//! `camera_before` / `camera_after_switch` / `camera_after_orbit` are the real
//! presence of `cad_status.camera`, the `error` fields are the real technical
//! status strings (`null` when there is none), `selection_2d` / `selection_3d`
//! are the real `cad_status.selection` blocks, and the drag/click counters count
//! events this driver really dispatched on the live DOM.
//!
//! Honesty boundary: `cad_status` exposes **no** 3D camera (yaw/pitch/distance),
//! no view revision and **no hovered-entity field**, so an orbit can only be
//! corroborated by its own drag counters plus a screenshot of the running window,
//! and hover by `hoverPreview`/`snapKind` only. The report states that instead of
//! claiming a measured orbit angle or a hovered id.
//!
//! A negative result is reported as it is: if the 3D click selects nothing, the
//! report carries `selection_count_3d=0` with the exact coordinates — the driver
//! never retries other fractions to manufacture a hit and never synthesizes a
//! selection.
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

/// The standard view the typed-IPC phase must request
/// (`YACR_PROBE_VIEW3D_STANDARD=<view>`), or `None` when the phase is off.
///
/// The value is the wire name the `cad_standard_view` IPC expects (`top`,
/// `bottom`, `front`, `back`, `left`, `right`, `isometric`). It is passed through
/// verbatim: an unknown name is the IPC's own refusal and must be reported as
/// one, never silently replaced by a valid view.
pub fn standard_view_request() -> Option<String> {
    let raw = std::env::var("YACR_PROBE_VIEW3D_STANDARD").ok()?;
    let view = raw.trim().to_string();
    if view.is_empty() {
        eprintln!(
            "[probe] view3d-standard: YACR_PROBE_VIEW3D_STANDARD is empty; no standard-view phase (nothing faked)"
        );
        return None;
    }
    Some(view)
}

/// Schedule the 3D view evidence driver if enabled. Never panics on spawn
/// failure (a failed spawn is logged; the run continues unchanged).
///
/// The thread only dispatches `Webview::eval` scripts that call the public
/// `cad_status`/`cad_command`/`cad_standard_view` commands and dispatch synthetic
/// DOM pointer events. No command/session logic is changed and no GPU state is
/// touched.
pub fn maybe_spawn_view3d(webview: Option<&tauri::Webview>) {
    let orbit = view3d_enabled();
    let standard = standard_view_request();
    if !orbit && standard.is_none() {
        return;
    }
    let Some(webview) = webview else {
        eprintln!("[probe] view3d: no child webview; 3D-view pass skipped (nothing faked)");
        return;
    };
    if orbit {
        eprintln!(
            "[probe] view3d: enabled (YACR_PROBE_VIEW3D); synthetic DOM pointer events, NOT a real mouse; \
             steps: 2D control click + cad_status(after_click_2d) + cad_status(before) + \
             cad_command SWITCH2D3D + cad_status(after_switch) + primary orbit drag + \
             cad_status(after_orbit) + empty 3D click + solid 3D click + cad_status(after_click_3d) + \
             one report line"
        );
        let webview = webview.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("probe-view3d".into())
            .spawn(move || run_view3d(webview))
        {
            eprintln!("[probe] view3d: failed to spawn: {error}");
        }
    }
    if let Some(view) = standard {
        eprintln!(
            "[probe] view3d-standard: enabled (YACR_PROBE_VIEW3D_STANDARD={view}); one typed \
             `cad_standard_view` IPC and NO synthetic pointer input; steps: \
             cad_status(before) + cad_standard_view + cad_status(after) + one report line"
        );
        let webview = webview.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("probe-view3d-standard".into())
            .spawn(move || run_standard_view(webview, view))
        {
            eprintln!("[probe] view3d-standard: failed to spawn: {error}");
        }
    }
}

/// Run the 3D view evidence sequence relative to driver start:
///
/// * t=2.4 s: **2D control click** at 50% width / 45% height of the
///   `.native-input` rect (the fraction the 3D phase reuses);
/// * t=2.8 s: read `cad_status` as `after_click_2d` (real selection count/rows);
/// * t=3.1 s: read `cad_status` as `before` (a loaded drawing in the default 2D
///   view reports a `camera` block);
/// * t=3.3 s: dispatch `cad_command { command: "SWITCH2D3D" }`. The reads above
///   are issued first so the host serves them before the switch is queued;
/// * t=4.0 s: read `cad_status` as `after_switch` (`camera` must be `absent`,
///   `error` must stay `null`);
/// * t=4.5 s: dispatch the synthetic primary drag script ([`VIEW3D_ORBIT_JS`]);
///   in the 3D view mode the host's primary-drag branch orbits instead of
///   starting a box selection;
/// * t=7.0 s: read `cad_status` as `after_orbit`;
/// * t=7.2 s: click **empty** 3D space at 95%/95% (the deselect control);
/// * t=7.6 s: read `cad_status` as `after_click_3d_empty`;
/// * t=7.8 s: click the **solid** at 50%/45% (the same fraction as the 2D
///   control);
/// * t=8.3 s: read `cad_status` as `after_click_3d`;
/// * t=8.7 s (retried once at t=9.5 s if the asynchronous read has not landed):
///   print **one** report line from [`VIEW3D_REPORT_JS`] through the
///   `eval_with_callback` idiom the pan driver uses.
///
/// Every step is a dispatch only; the outcome is the **real** `cad_status`
/// snapshot read from the host, never a value synthesized here.
fn run_view3d(webview: tauri::Webview) {
    let start = Instant::now();

    // 2D control: the same canvas fraction the 3D phase clicks later, so a
    // selection failure can be attributed to the 3D view rather than to picking
    // in general.
    let click_2d = view3d_click_js("click_2d", 0.50, 0.45);
    schedule_view3d(
        &start,
        2_400,
        "2D control click at 50%/45% of .native-input (synthetic, NOT a real mouse)",
        || webview.eval(click_2d),
    );
    schedule_view3d(&start, 2_800, "after_click_2d cad_status", || {
        webview.eval(view3d_status_js("after_click_2d"))
    });

    schedule_view3d(&start, 3_100, "before cad_status", || {
        webview.eval(view3d_status_js("before"))
    });
    // Payload-free selector through the same `cad_command` funnel the other
    // evidence drivers use; the ack lands in the shared draw-evidence store.
    schedule_view3d(&start, 3_300, "cad_command SWITCH2D3D", || {
        webview.eval(super::draw::draw_command_js("SWITCH2D3D", "switch_2d3d"))
    });
    schedule_view3d(&start, 4_000, "after_switch cad_status", || {
        webview.eval(view3d_status_js("after_switch"))
    });
    schedule_view3d(
        &start,
        4_500,
        "synthetic primary orbit drag on .native-input (NOT a real mouse)",
        || webview.eval(VIEW3D_ORBIT_JS),
    );
    schedule_view3d(&start, 7_000, "after_orbit cad_status", || {
        webview.eval(view3d_status_js("after_orbit"))
    });

    // Deselect control: a real click on empty 3D space must clear the selection
    // the 2D control left behind. Without it, a `selection_count_3d == 1` could
    // be the stale 2D selection rather than evidence that the 3D click hit.
    let click_3d_empty = view3d_click_js("click_3d_empty", 0.95, 0.95);
    schedule_view3d(
        &start,
        7_200,
        "3D empty-space click at 95%/95% (deselect control)",
        || webview.eval(click_3d_empty),
    );
    schedule_view3d(&start, 7_600, "after_click_3d_empty cad_status", || {
        webview.eval(view3d_status_js("after_click_3d_empty"))
    });

    // The measurement: the same fraction the 2D control hit, clicked once in 3D.
    let click_3d = view3d_click_js("click_3d", 0.50, 0.45);
    schedule_view3d(
        &start,
        7_800,
        "3D click on the solid at 50%/45% (synthetic, NOT a real mouse)",
        || webview.eval(click_3d),
    );
    schedule_view3d(&start, 8_300, "after_click_3d cad_status", || {
        webview.eval(view3d_status_js("after_click_3d"))
    });

    // The status reads are asynchronous (`invoke(...).then(...)`); the report is
    // assembled from what those promises really stored. One retry covers a slow
    // read without ever printing two lines.
    let reported = Arc::new(AtomicBool::new(false));
    for millis in [8_700u64, 9_500] {
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
                        "[probe] view3d-report deferred ({text}); the after_click_3d cad_status read has not landed yet"
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
            "[probe] view3d-report FAILED (no after_click_3d cad_status read landed before the bound; nothing faked)"
        );
    }
}

/// Run the **typed-IPC** standard-view phase relative to driver start:
///
/// * t=2.4 s: read `cad_status` as `standard_before` (a loaded drawing in the
///   default 2D view reports a `camera` block);
/// * t=3.0 s: invoke the real `cad_standard_view` IPC once with `{view:<name>}`.
///   The ack (or the real refusal) is stored on `window.__yacr_view3d`;
/// * t=3.9 s: read `cad_status` as `standard_after` (`camera` must be absent for a
///   non-plan view, and `error` must stay `null`);
/// * t=4.6 s (retried once at t=5.4 s): print **one** report line through the
///   `eval_with_callback` idiom the other drivers use.
///
/// There is deliberately **no** click, drag or command-line input in this phase:
/// a 3D isometric frame after it can only come from the typed IPC.
fn run_standard_view(webview: tauri::Webview, view: String) {
    let start = Instant::now();
    schedule_view3d(&start, 2_400, "standard_before cad_status", || {
        webview.eval(view3d_status_js("standard_before"))
    });
    let invoke = standard_view_invoke_js(&view);
    schedule_view3d(
        &start,
        3_000,
        "cad_standard_view { view } (typed IPC, no synthetic input)",
        || webview.eval(invoke),
    );
    schedule_view3d(&start, 3_900, "standard_after cad_status", || {
        webview.eval(view3d_status_js("standard_after"))
    });

    let reported = Arc::new(AtomicBool::new(false));
    for millis in [4_600u64, 5_400] {
        let webview = webview.clone();
        let reported = reported.clone();
        schedule_view3d(&start, millis, "view3d standard report", move || {
            webview.eval_with_callback(STANDARD_VIEW_REPORT_JS, move |payload| {
                if reported.load(Ordering::SeqCst) {
                    return;
                }
                let text = decode_view3d_report(&payload);
                if text.is_empty() || text == "not-ready" {
                    eprintln!(
                        "[probe] view3d-standard-report deferred ({text}); the standard_after cad_status read has not landed yet"
                    );
                    return;
                }
                reported.store(true, Ordering::SeqCst);
                eprintln!("[probe] view3d-standard-report {text}");
            })
        });
    }
    // Let the last eval callback land before deciding the report never arrived.
    std::thread::sleep(Duration::from_millis(600));
    if !reported.load(Ordering::SeqCst) {
        eprintln!(
            "[probe] view3d-standard-report FAILED (no standard_after cad_status read landed before the bound; nothing faked)"
        );
    }
}

/// Build the script that invokes the real `cad_standard_view` IPC and stores the
/// ack (or the real refusal) on `window.__yacr_view3d`.
fn standard_view_invoke_js(view: &str) -> String {
    STANDARD_VIEW_INVOKE_TEMPLATE.replace("__VIEW__", view)
}

/// Invoke `cad_standard_view` with `{ view: "__VIEW__" }` (synthetic JS, real IPC).
const STANDARD_VIEW_INVOKE_TEMPLATE: &str = r#"
(function () {
  var d = window.__yacr_view3d = window.__yacr_view3d || {};
  var view = '__VIEW__';
  d.standard_requested = view;
  d.standard_ack = null;
  d.standard_failed = null;
  try {
    var internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {
      d.standard_failed = 'no-tauri-internals';
      return 'no-tauri-internals';
    }
    internals.invoke('cad_standard_view', { view: view }).then(function (ack) {
      d.standard_ack = ack;
    }).catch(function (err) {
      d.standard_failed = String(err);
    });
    return 'cad_standard_view dispatched';
  } catch (err) {
    d.standard_failed = 'fatal:' + err;
    return 'error:' + err;
  }
})()
"#;

/// Assemble the single typed-IPC report line from the **real** stored reads.
///
/// `ack` is the raw `CadCommandAck` (`{accepted:true}` means *queued*, never
/// completion), `error_*` are the real technical `cad_status.error` strings, and
/// `camera_before`/`camera_after` are the real presence of the 2D camera block.
/// A missing read is `unread:<reason>` (never `absent`, which would be a claim).
const STANDARD_VIEW_REPORT_JS: &str = r#"
(function () {
  try {
    var d = window.__yacr_view3d;
    if (!d || !d.standard_after) { return 'not-ready'; }
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
    var after = d.standard_after || {};
    var result = {
      requested: d.standard_requested || null,
      ack: d.standard_ack || null,
      ack_failed: d.standard_failed || null,
      camera_before: cam('standard_before'),
      camera_after: cam('standard_after'),
      error_before: err('standard_before'),
      error_after: err('standard_after'),
      center_after: after.center || null,
      scale_after: (typeof after.scale === 'number') ? after.scale : null,
      document: after.document || null,
      entity_count: (typeof after.entity_count === 'number') ? after.entity_count : null,
      tool: after.tool || null,
      synthetic_input: 'none (this phase sends no pointer input; the frame comes from the typed IPC)'
    };
    return JSON.stringify(result);
  } catch (e) { return 'error:' + e; }
})()
"#;

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

/// Build the named status-read script (`before`, `after_switch`, `after_orbit`,
/// `after_click_2d`, `after_click_3d_empty`, `after_click_3d`).
fn view3d_status_js(name: &str) -> String {
    VIEW3D_STATUS_TEMPLATE.replace("__NAME__", name)
}

/// Build the labeled synthetic click at canvas fractions `(fx, fy)`.
fn view3d_click_js(label: &str, fx: f64, fy: f64) -> String {
    VIEW3D_CLICK_TEMPLATE
        .replace("__LABEL__", label)
        .replace("__FX__", &fx.to_string())
        .replace("__FY__", &fy.to_string())
}

/// Read the real `cad_status` and record it under `__NAME__` on
/// `window.__yacr_view3d`.
///
/// `camera` is the **real presence** of the 2D camera block (`center` +
/// `scale`), `entity_count`/`document` the real drawing state, `selection_*` the
/// real `cad_status.selection` block (count, display rows such as
/// `id`/`handle`/`type`/`layer`, mixed keys, text present), `tool`/`hover_*`/
/// `snap_kind` the cheap real tool/hover fields, and `error` the real technical
/// error string (the surfaced refusal, if any). A 3D view reports `camera: null`
/// because the host only projects a 2D orthographic camera.
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
      var sel = s.selection || null;
      d[name] = {
        camera: cam,
        center: cam ? [s.camera.center[0], s.camera.center[1]] : null,
        scale: cam ? s.camera.scale : null,
        entity_count: (typeof s.entityCount === 'number') ? s.entityCount : null,
        document: s.documentLabel || null,
        crosshair: !!s.crosshair,
        selection_count: (sel && typeof sel.count === 'number') ? sel.count : null,
        selection_rows: (sel && sel.rows) ? sel.rows.map(function (row) {
          return row.key + '=' + row.value;
        }) : [],
        selection_mixed_keys: (sel && sel.mixedKeys) ? sel.mixedKeys : [],
        selection_text_present: !!(sel && sel.text),
        tool: s.tool ? s.tool.key : null,
        tool_points: (s.tool && typeof s.tool.points === 'number') ? s.tool.points : null,
        hover_preview: !!s.hoverPreview,
        snap_kind: s.snapKind || null,
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

/// Build one synthetic **click** at a fraction of the `.native-input` rect
/// (`__LABEL__` names the report slot, `__FX__`/`__FY__` are canvas fractions).
///
/// The sequence is: one idle `pointermove` (lets the host resolve its hover
/// state) then `pointerdown` + `pointerup` at the **same point**. That exact
/// press+release pair is what the input lane classifies as a click; any move
/// past the drag threshold turns it into a gesture, which never selects.
/// Pointer capture is shimmed because these events are not backed by a device.
/// Synthetic, **not a real mouse**.
const VIEW3D_CLICK_TEMPLATE: &str = r#"
(function () {
  var d = window.__yacr_view3d = window.__yacr_view3d || {};
  var label = '__LABEL__';
  try {
    var el = document.querySelector('.native-input');
    if (!el) { d[label + '_failed'] = 'no-element'; return 'no-element'; }
    var r = el.getBoundingClientRect();
    if (!(r.width > 20 && r.height > 20)) {
      d[label + '_failed'] = 'tiny-rect:' + r.width + 'x' + r.height;
      return 'tiny-rect:' + r.width + 'x' + r.height;
    }
    el.setPointerCapture = function () {};
    el.hasPointerCapture = function () { return false; };
    el.releasePointerCapture = function () {};
    var x = r.left + r.width * __FX__;
    var y = r.top + r.height * __FY__;
    var info = {
      client: [Math.round(x * 100) / 100, Math.round(y * 100) / 100],
      canvas: [Math.round((x - r.left) * 100) / 100, Math.round((y - r.top) * 100) / 100],
      rect: [r.left, r.top, r.width, r.height],
      fractions: [__FX__, __FY__],
      hover_moves: 0,
      downs: 0,
      ups: 0,
      failed: ''
    };
    d[label] = info;
    function fire(type, button, buttons) {
      el.dispatchEvent(new PointerEvent(type, {
        pointerId: 1, pointerType: 'mouse', isPrimary: true,
        button: button, buttons: buttons,
        clientX: x, clientY: y, bubbles: true, cancelable: true
      }));
    }
    try { fire('pointermove', -1, 0); info.hover_moves++; }
    catch (e) { info.failed = 'move:' + e; }
    try { fire('pointerdown', 0, 1); info.downs++; }
    catch (e) { info.failed = 'down:' + e; }
    try { fire('pointerup', 0, 0); info.ups++; }
    catch (e) { info.failed = 'up:' + e; }
    return label + '-dispatched client=' + info.client.join(',');
  } catch (e) {
    try {
      var d2 = window.__yacr_view3d = window.__yacr_view3d || {};
      d2[label + '_failed'] = 'fatal:' + e;
    } catch (_) {}
    return 'error:' + e;
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
/// `selection_2d` / `selection_3d` are the real `cad_status.selection` blocks
/// (count, display rows with `id`/`handle`/`type`/`layer`, mixed keys, whether a
/// text target is present); `selection_3d_empty` is the deselect control. The
/// 3D camera itself is not part of `cad_status` and there is no hovered-entity
/// field, so neither a yaw/pitch nor a hovered id is claimed here — only the
/// measured pixel drag and the real status fields.
const VIEW3D_REPORT_JS: &str = r#"
(function () {
  try {
    var d = window.__yacr_view3d;
    if (!d || !d.after_click_3d) { return 'not-ready'; }
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
    function sel(name) {
      var v = d[name];
      if (!v) { return null; }
      return {
        count: v.selection_count,
        rows: v.selection_rows,
        mixed_keys: v.selection_mixed_keys,
        text_present: v.selection_text_present,
        document: v.document,
        entity_count: v.entity_count,
        error: v.error,
        tool: v.tool,
        tool_points: v.tool_points,
        hover_preview: v.hover_preview,
        snap_kind: v.snap_kind,
        crosshair: v.crosshair
      };
    }
    function click(name) {
      var c = d[name];
      if (!c) { return null; }
      return {
        client: c.client,
        canvas: c.canvas,
        rect: c.rect,
        fractions: c.fractions,
        hover_moves: c.hover_moves,
        downs: c.downs,
        ups: c.ups,
        failed: c.failed ? String(c.failed) : null
      };
    }
    var raw = window.__yacr_draw || {};
    var result = {
      camera_before: cam('before'),
      camera_after_switch: cam('after_switch'),
      camera_after_orbit: cam('after_orbit'),
      error_after_switch: err('after_switch'),
      error_after_orbit: err('after_orbit'),
      error_after_click_2d: err('after_click_2d'),
      error_after_click_3d_empty: err('after_click_3d_empty'),
      error_after_click_3d: err('after_click_3d'),
      switch_ack: raw['switch_2d3d_ack'] || null,
      switch_failed: raw['switch_2d3d_failed'] || null,
      document: field('after_click_3d', 'document') || field('before', 'document'),
      entity_before: field('before', 'entity_count'),
      entity_after_orbit: field('after_orbit', 'entity_count'),
      entity_after_click_3d: field('after_click_3d', 'entity_count'),
      selection_count_2d: field('after_click_2d', 'selection_count'),
      selection_count_3d_empty: field('after_click_3d_empty', 'selection_count'),
      selection_count_3d: field('after_click_3d', 'selection_count'),
      selection_2d: sel('after_click_2d'),
      selection_3d_empty: sel('after_click_3d_empty'),
      selection_3d: sel('after_click_3d'),
      click_2d: click('click_2d'),
      click_3d_empty: click('click_3d_empty'),
      click_3d: click('click_3d'),
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
      orbit_angle_channel: 'none (cad_status exposes no 3D camera; drag pixels are the real read)',
      hover_channel: 'none (cad_status has no hovered-entity field; hoverPreview/snapKind only)'
    };
    return JSON.stringify(result);
  } catch (e) { return 'error:' + e; }
})()
"#;

#[cfg(test)]
mod tests {
    use super::{decode_view3d_report, standard_view_invoke_js};

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

    #[test]
    fn standard_view_invoke_carries_the_requested_key_verbatim() {
        let script = standard_view_invoke_js("isometric");
        assert!(script.contains("var view = 'isometric';"), "{script}");
        assert!(
            script.contains("invoke('cad_standard_view', { view: view })"),
            "{script}"
        );
        assert!(
            !script.contains("__VIEW__"),
            "the placeholder is substituted"
        );
    }
}
