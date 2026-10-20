//! `YACR_PROBE_DRAW_EVIDENCE` P7c `MLEADER` creation phase.
//!
//! Split out as a sibling module for the same reason as
//! [`super::draw_hatch`]: every evidence file stays well under the 1000-line
//! code-file gate (`scripts/check-file-size.py`, no whitelist). This is the same
//! evidence-only, default-OFF, env-gated driver: it starts the **real**
//! `MLEADER` selector through the normal `cad_command` funnel, feeds the arrow
//! and landing picks as synthetic DOM pointer events on `.native-input`, submits
//! the annotation text through the **real** `cad_set_tool_text` IPC (the
//! `admit_tool_text` admission onto the shared `cad_tool_texts` queue the host
//! tick drains), and reads the **real** `cad_status` before/after each step.
//!
//! This is the **pixel criterion** ADR 0019 decision J owes for P7c: a
//! real-display run must show that the leader the command creates actually
//! renders, in a frame where a wrong leader (a missing arrowhead, a missing
//! dogleg, a detached or invisible text) would be obvious.
//!
//! ## What the phase drives
//!
//! 1. Start the real `MLEADER` selector (two points, then the text; ADR 0019
//!    decision C: `PointArity::Exact(2)` plus the text step).
//! 2. Click the **arrowhead** point, then the **landing** point. Both are given
//!    in nominal logical pixels and scaled onto the live `.native-input` rect
//!    (the P6b hatch precedent), so the layout is frame-relative instead of
//!    derived from `cad_status.camera`. The summary logs the real camera, because
//!    which furniture sits under these pixels depends on it; the layout below is
//!    authored against the measured frame and is not presented as
//!    camera-independent.
//! 3. Submit a short ASCII string (`LEAD`) through `cad_set_tool_text` —
//!    the dedicated IPC that calls `admit_tool_text` on the shared
//!    `cad_tool_texts` queue. The driver never writes into tool state directly.
//!    Text is **terminal** for `MLEADER`: submitting it completes the command, so
//!    there is no second `CONFIRM` (and `CONFIRM` would be refused while
//!    `awaiting_text` is set).
//! 4. Read `cad_status` and compare: the tool cleared (`tool: null`, hence
//!    `awaiting_text: false`), `entityCount` rose by **exactly one**, and `error`
//!    is `null`. Those three are derived into `mleader_pass` in the summary; none
//!    of them is asserted by omission.
//! 5. Click-select the leader on its own ink and log the resulting
//!    `selection.rows` — the real `style`/`roots`/`lines`/`display` rows the
//!    single-selection projection returns. That is the only geometric channel
//!    this status shape has (`selection.text` is populated only for a `Text`
//!    entity, so the submitted string is **not** observable here).
//!
//! ## Why the leader sits in the ring↔hatch corridor, slanted
//!
//! The first layout put the leader flat in the bottom band at y = 292 px and the
//! bounded real-display run falsified that choice: the frame carries a
//! full-width horizontal stroke on y ≈ 291.5, so the leader was drawn on top of
//! it, both picks shared one screen y (the elbow degenerated into a collinear
//! continuation of the leader), the annotation text landed over the hatched
//! rectangle and the select click resolved **two** entities. Every number of
//! that run is in its report; the layout below is authored against the frame
//! that run captured (926 × 341, camera `(1.5, 0.5)`, scale `0.005032`
//! world/px), whose measured furniture is: full-width strokes on y ≈ 269.5 and
//! y ≈ 291.5, a full-height stroke on x ≈ 559.5, the hatch rectangle
//! x 339..600 / y 39..250, the ring centred (140, 180) r ≈ 81 and the disc
//! centred (790, 180) r ≈ 80.
//!
//! The clear corridor between the ring (ends x ≈ 221) and the hatch (starts
//! x ≈ 339) is where everything goes, and the two picks get **different y** so
//! the dogleg is a real elbow:
//!
//! * arrowhead at px (270, 240) — 49 px right of the ring, 69 px left of the
//!   hatch, 29 px above the y ≈ 269.5 stroke; even its ≈ 36 px arrowhead box
//!   (±18 px ⇒ y ≈ 258) keeps 11 px clear of that stroke;
//! * landing at px (255, 90) — 9 px above the ring's top (y ≈ 99) and 304 px left
//!   of the full-height stroke;
//! * the domain derives the dogleg start as `landing − dogleg_length` toward the
//!   text side (`landing.x < arrow.x` ⇒ side `−1`), so with the pinned
//!   `Standard` style (`dogleg_length = 0.36` world ≈ 72 px) the elbow is at
//!   px ≈ (326.5, 90) and the dogleg is drawn leftward to
//!   `landing + landing_gap` ≈ px (273, 90); the leader line is therefore a
//!   ≈ 160 px **slanted** segment from (270, 240) to (326.5, 90), not a
//!   horizontal continuation, and the elbow is visible as a turn;
//! * the text is anchored at the landing point and, with attachment point `2`,
//!   is horizontally centred there — the glyphs are ≈ 82 px wide for the 4-letter
//!   string, i.e. x ≈ 214..296, 43 px clear of the hatch; vertically the created
//!   leader uses the style's `left_attachment = 1` (`TextAlignV::Top`), which
//!   `cad-representation`'s `finalize_text` implements as a one-text-height
//!   baseline shift, so the glyphs sit ≈ 37..62 px **above** the landing:
//!   y ≈ 28..53, clear of the ring (top y ≈ 99) and of the viewport top.
//!
//! The whole leader therefore lives inside x 214..327 / y 28..258, touching no
//! stroke, no ring, no disc and no hatch. The select click is the midpoint of
//! the **displayed** arrow→dogleg segment, px (298, 165): it is 41 px from the
//! nearest hatch border, 78 px from the ring and >100 px from every full-width
//! stroke, so the pick cannot resolve to anything but the new leader — which is
//! what the strict `selection.count == 1` gate then proves. The summary states
//! that isolation in `mleader_select_isolation` so a reviewer does not have to
//! re-derive it from the frame.
//!
//! ## What attachment code `2` actually means
//!
//! `CREATED_TEXT_ATTACHMENT_POINT` (`cad-domain`, DXF group `1`) is the
//! **horizontal** attachment only: `2` ⇒ `TextAlignH::Center`, i.e. the text is
//! centred on the landing point in x. The vertical attachment is *not* part of
//! that code: a created leader stores the resolved style's `left_attachment`
//! (`1` for the pinned `Standard` row), which maps to `TextAlignV::Top`
//! (`MLeaderText::v_align`). `cad-representation`'s `finalize_text` implements
//! `Top` as a **one-text-height baseline shift**, so the glyphs are drawn one
//! line height *above* the landing point — exactly the ≈ 37..62 px the captured
//! frame shows. The created annotation is therefore **top-centre**, not
//! middle-centre; the earlier description of it as "centre" was wrong. This is a
//! description fix only: the behaviour is the committed creation path and is
//! pinned by its tests, so nothing here changes it.
//!
//! ## Honest limits
//!
//! Synthetic input, **not a real mouse**: the events are DOM `PointerEvent`
//! dispatches from Rust through `webview.eval`, never the kernel/evdev/XInput
//! path — the CAD surface is input-transparent by design. `cad_status` has **no
//! per-entity geometry channel**: it exposes no leader vertices, no arrowhead and
//! no annotation string. This driver therefore proves "two points captured, a
//! real text admission completed the command, one entity added, tool cleared, no
//! error, the new entity selects", and the pixel read of the captured frame
//! proves "leader ink and text are on screen where the picks say". The exact
//! geometry (vertices, dogleg, attachment, DXF round trip) is pinned by the
//! `cad-domain`/`cad-app` contract tests, not by this channel. The screenshot is
//! the application window's own drawable only, from one bounded local run;
//! nothing here is a cross-platform, vendor or precision claim.
//!
//! ## Failure behaviour
//!
//! A step that fails its expectation sets `window.__yacr_draw.mleader_abort` and
//! logs `[probe] draw evidence mleader abort: …` immediately. Every later
//! dispatch (click, text, select) checks that flag and **skips** instead of
//! pretending the step ran; only the read-only `cad_status` reads continue. The
//! summary reports `mleader_aborted_at=` and `mleader_pass=0`, so a run that
//! stopped early cannot read as a pass.

use std::time::Instant;

use super::draw::{draw_command_js, schedule_draw_ms};

// Nominal CAD surface size the pixel layout is authored against: 926 x 341
// logical pixels (the live `.native-input` rect). The click template scales the
// nominal pixel onto the live rect, so a rect a pixel or two different keeps the
// same proportional layout; the summary carries both the logical pixel and the
// rect fraction each event actually hit.

/// Arrowhead pick, in nominal logical px: the bottom of the clear corridor
/// between the ring (ends x ≈ 221) and the hatch (starts x ≈ 339), above both
/// full-width horizontal strokes (y ≈ 269.5 and y ≈ 291.5).
const ML_ARROW: (f64, f64) = (270.0, 240.0);
/// Landing (content connection) pick, in nominal logical px: 150 px above the
/// arrow and 15 px to its left, so the leader is slanted and the derived dogleg
/// forms a visible elbow instead of continuing the leader line. The dogleg start
/// is derived from it by the pinned style, so it is not a third pick.
const ML_LANDING: (f64, f64) = (255.0, 90.0);
/// Select click at the midpoint of the **displayed** arrow→dogleg segment
/// (`arrow → landing − dogleg_length`), the one point on the leader with no
/// coincident stroke: 41 px from the hatch border, 78 px from the ring.
const ML_SELECT: (f64, f64) = (298.0, 165.0);
/// Click-free pointer park so the viewport-spanning crosshair leaves the leader.
const ML_PARK: (f64, f64) = (10.0, 337.0);
/// The terminal annotation string: short, ASCII, four glyphs (≈ 82 px wide) so
/// the centred text stays 43 px clear of the hatch border. It renders one text
/// height **above** the landing point (attachment `2` + `TextAlignV::Top`).
const ML_TEXT: &str = "LEAD";

/// Schedule the P7c `MLEADER` phase relative to the driver start.
///
/// It starts at driver t=206 s (after the P6b hatch phases, which end at
/// t=204.5 s) and steps every 400–600 ms; the combined report is scheduled after
/// it at t=213 s (see [`super::draw_edit::schedule_draw_report`]). Every step
/// reads the real `cad_status`; a dispatch that merely posts is not a pass.
///
/// The timeline:
/// * t=206 s: `cad_command MLEADER`, then a status read (tool active,
///   `points = 0`, `awaiting_text = false`) and a gate that aborts the phase if
///   the real selector did not start;
/// * t=207.2 s..208 s: arrowhead pick at px (270, 240), read `points = 1`, gate;
/// * t=208.4 s..209.2 s: landing pick at px (255, 90), read `points = 2` and
///   `awaiting_text = true` (`ready` false), gate;
/// * t=209.6 s..210.6 s: submit `LEAD` through `cad_set_tool_text`, read
///   `entityCount` (**+1**), `tool` `null`, `awaiting_text` `false`, `error`
///   `null`, gate;
/// * t=211 s..211.8 s: click-select at px (298, 165), read `selection.count` and
///   the real property rows, gate;
/// * t=212 s: click-free pointer move to (10, 337) so the viewport-spanning
///   crosshair is parked outside the leader;
/// * t=212.4 s: build `window.__yacr_mleader_summary`, which the combined report
///   appends as the `mleader_*`/`ml_*`/`s_ml_*` fields.
pub(super) fn schedule_mleader_phases(webview: &tauri::Webview, start: &Instant) {
    // ---- Start the real selector and prove it is active ----
    schedule_draw_ms(start, 206_000, "cad_command MLEADER", || {
        webview.eval(draw_command_js("MLEADER", "ml_start"))
    });
    schedule_draw_ms(
        start,
        206_400,
        "mleader-start cad_status (tool draw.mleader, points 0, not awaiting text)",
        || webview.eval(ml_status_js("s_ml_start")),
    );
    // The gate checks the authoritative state, not the ack: `cad_command`'s
    // `{accepted:true}` means "queued", and a promise that resolves a few
    // milliseconds late must not abort a run that actually started. The ack is
    // still reported in the summary.
    let gate_start = ml_expect_js(
        "ml_gate_start",
        &[
            ("s_ml_start_done", "true"),
            ("s_ml_start_tool", "'draw.mleader'"),
            ("s_ml_start_points", "0"),
            ("s_ml_start_awaiting", "false"),
        ],
    );
    schedule_draw_ms(
        start,
        206_800,
        "mleader-start gate (aborts the phase if the selector did not start)",
        || webview.eval(gate_start),
    );

    // ---- Arrowhead pick ----
    let arrow = ml_click_px_js(ML_ARROW.0, ML_ARROW.1, "ml_arrow");
    schedule_draw_ms(
        start,
        207_200,
        "mleader arrowhead click at px (270, 240)",
        || webview.eval(arrow),
    );
    schedule_draw_ms(
        start,
        207_600,
        "mleader-arrow cad_status (points 1, still not awaiting text)",
        || webview.eval(ml_status_js("s_ml_arrow")),
    );
    let gate_arrow = ml_expect_js(
        "ml_gate_arrow",
        &[
            ("ml_arrow_dispatched", "true"),
            ("s_ml_arrow_done", "true"),
            ("s_ml_arrow_points", "1"),
        ],
    );
    schedule_draw_ms(
        start,
        208_000,
        "mleader-arrow gate (aborts if the arrowhead point was not captured)",
        || webview.eval(gate_arrow),
    );

    // ---- Landing pick: the point complement is complete, the text prompt opens ----
    let landing = ml_click_px_js(ML_LANDING.0, ML_LANDING.1, "ml_landing");
    schedule_draw_ms(
        start,
        208_400,
        "mleader landing click at px (255, 90)",
        || webview.eval(landing),
    );
    schedule_draw_ms(
        start,
        208_800,
        "mleader-landing cad_status (points 2, awaiting_text true, ready false)",
        || webview.eval(ml_status_js("s_ml_landing")),
    );
    let gate_landing = ml_expect_js(
        "ml_gate_landing",
        &[
            ("ml_landing_dispatched", "true"),
            ("s_ml_landing_done", "true"),
            ("s_ml_landing_points", "2"),
            ("s_ml_landing_awaiting", "true"),
        ],
    );
    schedule_draw_ms(
        start,
        209_200,
        "mleader-landing gate (aborts unless the text prompt is open)",
        || webview.eval(gate_landing),
    );

    // ---- Terminal text through the real IPC admission ----
    let text = ml_text_js(ML_TEXT, "ml_text");
    schedule_draw_ms(
        start,
        209_600,
        "cad_set_tool_text 'LEAD' (real IPC; admit_tool_text into cad_tool_texts)",
        || webview.eval(text),
    );
    schedule_draw_ms(
        start,
        210_200,
        "mleader-after cad_status (entityCount +1, tool null, awaiting_text false)",
        || webview.eval(ml_status_js("s_ml_text")),
    );
    // Same discipline as the start gate: the admission ack is reported, but the
    // pass criterion is the authoritative outcome (completion, delta, error), so
    // a late-resolving ack cannot abort a command that really completed.
    let gate_text = ml_expect_js(
        "ml_gate_text",
        &[
            ("s_ml_text_done", "true"),
            ("s_ml_text_tool", "null"),
            ("s_ml_text_awaiting", "false"),
            ("s_ml_text_error", "null"),
            ("s_ml_text_delta_from_start", "1"),
        ],
    );
    schedule_draw_ms(
        start,
        210_600,
        "mleader-after gate (delta 1, tool cleared, awaiting_text cleared, no error)",
        || webview.eval(gate_text),
    );

    // ---- Select the new entity and read its real property rows ----
    let select = ml_click_px_js(ML_SELECT.0, ML_SELECT.1, "ml_select");
    schedule_draw_ms(
        start,
        211_000,
        "mleader select click on its own ink at px (298, 165)",
        || webview.eval(select),
    );
    schedule_draw_ms(
        start,
        211_400,
        "mleader-selected cad_status (selection.count 1, rows are the real projection)",
        || webview.eval(ml_status_js("s_ml_select")),
    );
    let gate_select = ml_expect_js(
        "ml_gate_select",
        &[
            ("ml_select_dispatched", "true"),
            ("s_ml_select_done", "true"),
            ("s_ml_select_selection", "1"),
        ],
    );
    schedule_draw_ms(
        start,
        211_800,
        "mleader-select gate (aborts unless exactly the new leader is selected)",
        || webview.eval(gate_select),
    );

    // ---- Park the crosshair outside the leader, then build the report fragment ----
    let park = ml_move_px_js(ML_PARK.0, ML_PARK.1, "ml_park");
    schedule_draw_ms(
        start,
        212_000,
        "mleader click-free pointer move to px (10, 337)",
        || webview.eval(park),
    );
    schedule_draw_ms(start, 212_400, "mleader summary fragment", || {
        webview.eval(ml_summary_js())
    });
}

/// Build the named status read for this phase (`s_ml_start`, ...).
///
/// Unlike [`super::draw::draw_status_js`] this read also records
/// `awaiting_text` and the single-selection property rows, and derives the real
/// `entityCount` delta against the phase's own baseline — the three fields this
/// phase's pass criterion needs.
fn ml_status_js(name: &str) -> String {
    ML_STATUS_TEMPLATE.replace("__NAME__", name)
}

/// Build one synthetic click at a **logical-pixel** position on the CAD surface.
///
/// Same diagnostics as the hatch click (`*_frac`, `*_px`, `*_dispatched`,
/// `*_failed`, `d.clicks`); the position is scaled from the nominal surface onto
/// the live `.native-input` rect. Synthetic, **not a real mouse**.
fn ml_click_px_js(x: f64, y: f64, label: &str) -> String {
    ML_CLICK_TEMPLATE
        .replace("__X__", &x.to_string())
        .replace("__Y__", &y.to_string())
        .replace("__LABEL__", label)
}

/// Build a click-free pointer move at a **logical-pixel** position.
///
/// Only `pointermove` is dispatched: no press, no release, no selection change.
/// Used to park the renderer's viewport-spanning crosshair away from the leader.
fn ml_move_px_js(x: f64, y: f64, label: &str) -> String {
    ML_MOVE_TEMPLATE
        .replace("__X__", &x.to_string())
        .replace("__Y__", &y.to_string())
        .replace("__LABEL__", label)
}

/// Build the terminal-text submission through the real `cad_set_tool_text` IPC.
///
/// The host command calls `admit_tool_text` on the shared `cad_tool_texts`
/// queue; the owner tick drains it into `CadSession::set_tool_text`, which is the
/// only authority on "is a tool awaiting text". `{accepted:true}` is admission
/// only; the outcome is read back from `cad_status`.
fn ml_text_js(text: &str, label: &str) -> String {
    ML_TEXT_TEMPLATE
        .replace("__TEXT__", text)
        .replace("__LABEL__", label)
}

/// Build the phase summary script (the combined report appends its fragment).
fn ml_summary_js() -> String {
    ML_SUMMARY_TEMPLATE.to_string()
}

/// Build one expectation gate over the recorded reads.
///
/// `checks` are `(recorded key, JS literal expected value)` pairs compared with
/// `!==` against `window.__yacr_draw[key]`. The first mismatch sets
/// `mleader_abort` and logs the reason immediately; every later dispatch step
/// checks that flag and skips. This is how the driver stops instead of
/// continuing as if a failed step had worked.
fn ml_expect_js(label: &str, checks: &[(&str, &str)]) -> String {
    let list = checks
        .iter()
        .map(|(key, value)| format!("['{key}', {value}]"))
        .collect::<Vec<_>>()
        .join(",");
    ML_EXPECT_TEMPLATE
        .replace("__LABEL__", label)
        .replace("__CHECKS__", &list)
}

/// Read the real `cad_status` and record the fields this phase compares.
///
/// `*_delta_from_start` is only a number when both `entityCount` reads exist; a
/// missing side leaves it `null` (reported as `-`), never a fabricated zero.
const ML_STATUS_TEMPLATE: &str = r#"
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
      d[name + '_awaiting'] = s.tool ? !!s.tool.awaiting_text : false;
      d[name + '_selection'] = s.selection ? s.selection.count : null;
      d[name + '_rows'] = (s.selection && s.selection.rows)
        ? s.selection.rows.map(function (r) { return String(r.key) + ':' + String(r.value); })
        : [];
      d[name + '_error'] = s.error;
      d[name + '_delta_from_start'] =
        (typeof d.s_ml_start_count === 'number' && typeof s.entityCount === 'number')
          ? (s.entityCount - d.s_ml_start_count)
          : null;
      if (s.camera && s.camera.center && s.camera.center.length === 2) {
        d[name + '_camera'] = [s.camera.center[0], s.camera.center[1], s.camera.scale];
      }
    }).catch(function (err) { d[name + '_failed'] = String(err); });
    return name + '-dispatched';
  } catch (err) {
    try { d[name + '_failed'] = 'fatal:' + err; } catch (_) {}
    return 'error:' + err;
  }
})()
"#;

/// One synthetic click at a logical-pixel position, skipped once the phase has
/// aborted. Synthetic, **not a real mouse**.
const ML_CLICK_TEMPLATE: &str = r#"
(function () {
  var d = window.__yacr_draw = window.__yacr_draw || {};
  var label = '__LABEL__';
  if (d.mleader_abort) { d[label + '_skipped'] = d.mleader_abort; return 'aborted'; }
  try {
    var el = document.querySelector('.native-input');
    if (!el) { d[label + '_failed'] = 'no-element'; return 'no-element'; }
    var r = el.getBoundingClientRect();
    if (!(r.width > 20 && r.height > 20)) {
      d[label + '_failed'] = 'tiny-rect:' + r.width + 'x' + r.height;
      return 'tiny-rect:' + r.width + 'x' + r.height;
    }
    var fx = (__X__ * (r.width / 926.0)) / r.width;
    var fy = (__Y__ * (r.height / 341.0)) / r.height;
    d[label + '_frac'] = [fx, fy];
    if (!(fx >= 0 && fx <= 1 && fy >= 0 && fy <= 1)) {
      d[label + '_failed'] = 'outside-canvas:' + fx + ',' + fy;
      return 'outside-canvas:' + fx + ',' + fy;
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
    return label + '-dispatched';
  } catch (err) {
    try { d[label + '_failed'] = 'fatal:' + err; } catch (_) {}
    return 'error:' + err;
  }
})()
"#;

/// One click-free synthetic pointer move at a logical-pixel position.
///
/// Unlike the click/text steps this one **keeps running after an abort**: a
/// `pointermove` captures no point, changes no selection and completes nothing,
/// so it cannot fake a failed step, and parking the viewport-spanning crosshair
/// keeps the captured frame (including a failure frame) readable.
/// Synthetic, **not a real mouse**.
const ML_MOVE_TEMPLATE: &str = r#"
(function () {
  var d = window.__yacr_draw = window.__yacr_draw || {};
  var label = '__LABEL__';
  try {
    var el = document.querySelector('.native-input');
    if (!el) { d[label + '_failed'] = 'no-element'; return 'no-element'; }
    var r = el.getBoundingClientRect();
    if (!(r.width > 20 && r.height > 20)) {
      d[label + '_failed'] = 'tiny-rect:' + r.width + 'x' + r.height;
      return 'tiny-rect';
    }
    var fx = (__X__ * (r.width / 926.0)) / r.width;
    var fy = (__Y__ * (r.height / 341.0)) / r.height;
    d[label + '_frac'] = [fx, fy];
    if (!(fx >= 0 && fx <= 1 && fy >= 0 && fy <= 1)) {
      d[label + '_failed'] = 'outside-canvas:' + fx + ',' + fy;
      return 'outside-canvas';
    }
    el.setPointerCapture = function () {};
    el.hasPointerCapture = function () { return false; };
    el.releasePointerCapture = function () {};
    var x = r.left + r.width * fx;
    var y = r.top + r.height * fy;
    el.dispatchEvent(new PointerEvent('pointermove', {
      pointerId: 1, pointerType: 'mouse', isPrimary: true,
      button: -1, buttons: 0,
      clientX: x, clientY: y, bubbles: true, cancelable: true
    }));
    d[label + '_px'] = [Math.round(x - r.left), Math.round(y - r.top)];
    d[label + '_dispatched'] = true;
    return label + '-moved';
  } catch (err) {
    try { d[label + '_failed'] = 'fatal:' + err; } catch (_) {}
    return 'error:' + err;
  }
})()
"#;

/// Submit the annotation text through the real `cad_set_tool_text` IPC,
/// skipped once the phase has aborted.
///
/// `__TEXT__` is the terminal annotation string and `__LABEL__` the report slot
/// for the admission ack. The ack only means "queued on the shared
/// `cad_tool_texts` queue"; the completion and any refusal come back through
/// `cad_status`.
const ML_TEXT_TEMPLATE: &str = r#"
(function () {
  var d = window.__yacr_draw = window.__yacr_draw || {};
  var label = '__LABEL__';
  if (d.mleader_abort) { d[label + '_skipped'] = d.mleader_abort; return 'aborted'; }
  d.mleader_text_submitted = '__TEXT__';
  try {
    var internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {
      d[label + '_failed'] = 'no-tauri-internals';
      return 'no-tauri-internals';
    }
    internals.invoke('cad_set_tool_text', { text: '__TEXT__' }).then(function (ack) {
      d[label + '_ack'] = (ack && typeof ack.accepted !== 'undefined')
        ? 'accepted=' + ack.accepted
        : String(JSON.stringify(ack));
    }).catch(function (err) { d[label + '_failed'] = String(err); });
    return label + '-dispatched';
  } catch (err) {
    try { d[label + '_failed'] = 'fatal:' + err; } catch (_) {}
    return 'error:' + err;
  }
})()
"#;

/// One expectation gate over the recorded reads; the first mismatch aborts the
/// phase and logs the reason at once.
const ML_EXPECT_TEMPLATE: &str = r#"
(function () {
  var d = window.__yacr_draw = window.__yacr_draw || {};
  var label = '__LABEL__';
  if (d.mleader_abort) { d[label + '_skipped'] = d.mleader_abort; return 'aborted'; }
  var checks = [__CHECKS__];
  for (var i = 0; i < checks.length; i++) {
    if (d[checks[i][0]] !== checks[i][1]) {
      d.mleader_abort = label + ':' + checks[i][0] + '=' + String(d[checks[i][0]]) +
        '_expected_' + String(checks[i][1]);
      try {
        console.log('[probe] draw evidence mleader abort: ' + d.mleader_abort);
      } catch (err) {}
      return 'aborted';
    }
  }
  d[label + '_ok'] = true;
  return 'ok';
})()
"#;

/// The `MLEADER` report fragment: one single-line, whitespace-free field list
/// built from the **real** `cad_status` reads the phase recorded in
/// `window.__yacr_draw`, stored in `window.__yacr_mleader_summary` so the
/// combined report in [`super::draw_edit`] can append it without duplicating the
/// reads (the P6b `hatch_*` precedent).
///
/// `-` means "absent / never read"; `-1` a delta that could not be computed
/// because one side of the read is missing. `mleader_pass` is derived from the
/// reads — completed (tool `null` ⇒ `awaiting_text` false), the real
/// `entityCount` delta of exactly one, no `error`, and no earlier abort — and is
/// `0` whenever any part is missing. `mleader_select_isolation` restates the
/// authored layout reason the select click can only resolve to the new leader
/// (the point, and its clearance from the measured furniture); it is a layout
/// note, not a status read. The trailing channel lines are the honest statement
/// that `cad_status` exposes neither per-entity geometry nor the created
/// annotation string.
const ML_SUMMARY_TEMPLATE: &str = r#"
(function () {
  var d = window.__yacr_draw || {};
  function v(x) {
    if (x === null || typeof x === 'undefined') { return '-'; }
    try { return String(x).replace(/\s+/g, '_'); } catch (err) { return '?'; }
  }
  function n(x) { return (typeof x === 'number') ? x : -1; }
  function flat(x) {
    try { return String(x).replace(/\s+/g, '_'); } catch (err) { return '?'; }
  }
  function arr(x) {
    if (!x || !x.length) { return '-'; }
    try { return x.map(function (k) { return flat(k); }).join(','); } catch (err) { return '?'; }
  }
  var reads = ['s_ml_start', 's_ml_arrow', 's_ml_landing', 's_ml_text', 's_ml_select'];
  var missing = [];
  for (var i = 0; i < reads.length; i++) {
    if (!d[reads[i] + '_done']) { missing.push(reads[i]); }
  }
  var gates = ['ml_gate_start', 'ml_gate_arrow', 'ml_gate_landing', 'ml_gate_text',
    'ml_gate_select'];
  var gates_missing = [];
  for (var g = 0; g < gates.length; g++) {
    if (!d[gates[g] + '_ok']) { gates_missing.push(gates[g]); }
  }
  var failed = [];
  for (var key in d) {
    if (/_failed$/.test(key) && d[key] && /^(ml_|s_ml_|mleader_)/.test(key)) {
      failed.push(key + '=' + d[key]);
    }
  }
  var before = n(d.s_ml_start_count);
  var after = n(d.s_ml_text_count);
  var delta = (before >= 0 && after >= 0) ? (after - before) : -1;
  var delta_ok = (delta === 1) ? 1 : 0;
  var completed = (d.s_ml_text_tool === null && d.s_ml_text_awaiting === false) ? 1 : 0;
  var error_clear = (d.s_ml_text_error === null) ? 1 : 0;
  var abort = (typeof d.mleader_abort === 'undefined') ? null : d.mleader_abort;
  var pass = (abort === null && gates_missing.length === 0 && completed === 1 &&
    delta_ok === 1 && error_clear === 1) ? 1 : 0;
  var text = 'mleader_synthetic=1_synthetic_dom_pointer_events_not_a_real_mouse' +
    ' mleader_input=synthetic_dom_pointer_events_not_a_real_mouse' +
    ' mleader_text_admission=cad_set_tool_text_ipc_admit_tool_text_shared_cad_tool_texts_queue' +
    ' mleader_awaiting_text_channel=tool.awaiting_text' +
    ' mleader_status_missing=' + (missing.length ? missing.join(',') : '-') +
    ' mleader_gate_missing=' + (gates_missing.length ? gates_missing.join(',') : '-') +
    ' mleader_step_failed=' + (failed.length ? failed.join(',') : '-') +
    ' mleader_aborted_at=' + v(abort) +
    ' mleader_camera=' + arr(d.s_ml_start_camera) +
    ' mleader_start_ack=' + v(d.ml_start_ack) +
    ' mleader_start_tool=' + v(d.s_ml_start_tool) +
    ' mleader_start_points=' + v(d.s_ml_start_points) +
    ' mleader_start_awaiting=' + v(d.s_ml_start_awaiting) +
    ' mleader_arrow_px=' + arr(d.ml_arrow_px) +
    ' mleader_arrow_frac=' + arr(d.ml_arrow_frac) +
    ' mleader_arrow_tool=' + v(d.s_ml_arrow_tool) +
    ' mleader_arrow_points=' + v(d.s_ml_arrow_points) +
    ' mleader_arrow_awaiting=' + v(d.s_ml_arrow_awaiting) +
    ' mleader_landing_px=' + arr(d.ml_landing_px) +
    ' mleader_landing_frac=' + arr(d.ml_landing_frac) +
    ' mleader_landing_tool=' + v(d.s_ml_landing_tool) +
    ' mleader_landing_points=' + v(d.s_ml_landing_points) +
    ' mleader_landing_awaiting=' + v(d.s_ml_landing_awaiting) +
    ' mleader_landing_ready=' + v(d.s_ml_landing_ready) +
    ' mleader_text_value=' + v(d.mleader_text_submitted) +
    ' mleader_text_ack=' + v(d.ml_text_ack) +
    ' mleader_before_count=' + v(d.s_ml_start_count) +
    ' mleader_after_count=' + v(d.s_ml_text_count) +
    ' mleader_delta=' + delta +
    ' mleader_delta_ok=' + delta_ok +
    ' mleader_after_tool=' + v(d.s_ml_text_tool) +
    ' mleader_after_awaiting=' + v(d.s_ml_text_awaiting) +
    ' mleader_after_error=' + v(d.s_ml_text_error) +
    ' mleader_completed=' + completed +
    ' mleader_error_clear=' + error_clear +
    ' mleader_select_px=' + arr(d.ml_select_px) +
    ' mleader_select_count=' + v(d.s_ml_select_selection) +
    ' mleader_select_rows=' + arr(d.s_ml_select_rows) +
    ' mleader_select_isolation=px_298_165_is_the_displayed_segment_midpoint_41px_from_the_hatch_border_78px_from_the_ring_no_coincident_stroke' +
    ' mleader_park_px=' + arr(d.ml_park_px) +
    ' mleader_pass=' + pass +
    ' mleader_geometry_channel=none_cad_status_has_no_per_entity_geometry_multileader_rows_are_style_roots_lines_display' +
    ' mleader_text_value_channel=none_the_created_string_is_not_in_selection_rows_pinned_by_crate_tests';
  try { window.__yacr_mleader_summary = ' ' + text; } catch (err) {}
  try { console.log('[probe] draw evidence mleader summary: ' + text); } catch (err) {}
  return text;
})()
"#;
