//! `YACR_PROBE_DRAW_EVIDENCE` P6b `HATCH` creation phases.
//!
//! Split out of [`super::draw`] as a sibling module so every evidence file stays
//! well under the 1000-line code-file gate (`scripts/check-file-size.py`, no
//! whitelist). This is the same evidence-only, default-OFF, env-gated driver: it
//! dispatches the **real** `HATCH` selector through the normal `cad_command`
//! funnel, sets the real typed parameters through `cad_set_tool_parameter`, and
//! sends synthetic DOM pointer events to `.native-input`; then it reads the
//! **real** `cad_status` before/after each commit. Every reported number is a
//! real `cad_status` read, and every reported `*_px` is the pixel the synthetic
//! event was actually dispatched at.
//!
//! `HATCH` is a **pre-selection** tool (ADR 0018 decision D): it captures no
//! points (`PointArity::Exact(0)` plus `requires_selection()`), so the boundary
//! is the selection snapshot taken when the tool starts. Each phase therefore
//! draws its boundary first, selects it, then starts `HATCH` and `CONFIRM`s.
//!
//! Phases, each chosen so the frame would look **wrong** if the fill rule were
//! wrong:
//! 1. **Solid with a hole.** Two concentric `CIRCLE`s (outer r = 80 logical px,
//!    inner r = 34), both window-selected, `HATCH` with the defaults (solid,
//!    `Normal`), `CONFIRM`. Expected: **one** new entity; the fill covers the
//!    annulus and leaves the inner disc **empty**. A wrong hole rule fills the
//!    inner disc and is visible immediately.
//! 2. **User-defined pattern.** A closed `RECTANG`, selected, pattern type `1`
//!    with spacing `0.05` and angle `30`, `CONFIRM`. Expected: diagonal parallel
//!    line families inside the boundary, not a solid wash.
//! 3. **Style dispatch on screen.** The phase-1 geometry again (same radii, same
//!    solid fill) with style `2` (`Ignore`). Expected: the inner disc is filled
//!    too, because `Ignore` keeps only the depth-1 ring. `Outer` is deliberately
//!    **not** used here: on a two-ring donut `Outer` and `Normal` produce the
//!    same area (ADR 0018 §0.2 — `Outer` only differs at nesting depth >= 3), so
//!    it would add no visual signal; its numerics are pinned by the contract
//!    tests.
//!
//! ## Why the view is panned first
//!
//! The earlier phases of this driver fill the visible extent with ~70 entities,
//! and a hatch frame is only readable if the measured regions contain nothing
//! but the hatch. The driver therefore performs two **middle-button drags**
//! (a real 2D pan: [`crate::evidence::pan`] and the host's pointer arbitration
//! both use the same gesture) that move the camera two viewport widths to the
//! right, into empty world space. The pan is not assumed: `s_hatch_pan_before`
//! and `s_hatch_pan_after` read the real `cad_status.camera` and the report
//! prints both, so a pan that did not happen is visible in the evidence instead
//! of silently producing a contaminated frame.
//!
//! All hatch-phase coordinates are **logical-pixel** positions on the CAD
//! surface, not world points: after a pan the driver does not know the camera,
//! and the pixel read of the captured frame is the measurement. Every click
//! records both the fraction it dispatched at and the integer pixel inside
//! `.native-input`, so a reviewer can reproduce it from the report alone.
//!
//! ## Honest limits
//!
//! Synthetic input, **not a real mouse**: the events are DOM `PointerEvent`
//! dispatches from Rust through `webview.eval`, never the kernel/evdev/XInput
//! path. `cad_status` has **no geometry channel**: it exposes no fill area, no
//! loop count and no per-entity geometry, so this driver proves "committed, one
//! entity added, tool cleared, no error", and the pixel read of the captured
//! frame proves "ink present / absent where the rule says". The style *numerics*
//! (`Normal={1,3}`, `Outer={1}`, `Ignore={1,2,3}`) and the DXF export are pinned
//! by the `cad-app`/`cad-domain` contract tests, not by this channel. The
//! screenshot is the application window's own drawable only, from one bounded
//! local run (GNOME/Mutter + XWayland, scale 1); nothing here is a
//! cross-platform or vendor claim.
//!
//! ## What the captured frame shows, and one finding it exposed
//!
//! Every phase commits exactly one hatch entity and the pixel read agrees with
//! the rule (see the run report). The frames also show, around each **circular**
//! hatch, four large arcs plus four straight overshoot segments that are neither
//! the fill (measured 100 % inside the annulus, 0 % inside the hole) nor the
//! boundary circles (analytic conics at r = 80 px / 34 px, unchanged). The
//! rectangular hatch, whose boundary polyline carries **no bulges**, has no such
//! artifact. Replaying `cad_geometry`'s bulge-arc construction in screen space
//! reproduces the observed petals (76 of 96 sampled points land on that ink,
//! bounding box matching), while the mirrored construction used by the hatch
//! fill sampler reproduces the circle. That points at the display of the derived
//! **bulged boundary polyline** rather than at the fill or at hatch creation.
//! This driver does **not** fix it (it is a renderer change outside P6b's
//! scope, with its own blast radius) and does not measure it further; it is
//! reported as a finding so it is not mistaken for expected hatch ink.
//!
//! ## Control path for the parameters
//!
//! Pattern type, spacing, angle and style travel through
//! `cad_set_tool_parameter` — the **same** IPC the status-bar steppers call, and
//! the same path the FILLET/CHAMFER/ARRAY phases already use. Two of the values
//! are not reachable by clicking a stepper: `hatchSpacing` steps by `0.5` with a
//! minimum of `0.5` (the pattern would be a single line), so the driver sets
//! `0.05`; and the driver never clicks the stepper widgets themselves, because
//! the status bar renders them at whatever position the layout gives them and a
//! blind pixel click would be a guess. The stepper's own admission path and
//! range check are what is exercised (`accepted=true` means queued; the applied
//! value is read back from `cad_status.tool.params`).

use std::time::Instant;

use super::draw::{
    draw_command_js, draw_confirm_js, draw_set_parameter_number_js, draw_status_js,
    schedule_draw_ms,
};

// Nominal CAD surface size the pixel layout is authored against: 926 x 341
// logical pixels (the real `.native-input` rect this window reports). The click
// and drag templates scale these nominal pixels onto the live rect, so a rect a
// pixel or two different keeps the same proportional layout; the report carries
// both the real rect and the integer pixel each event actually hit.

/// Phase 1 (and phase 3, identical geometry) annulus centre, in logical px.
const P1_CENTER: (f64, f64) = (140.0, 180.0);
/// Phase 1 outer radius in logical px: 80 px = 0.4025 world units at scale 1.
const P1_OUTER_R: f64 = 80.0;
/// Phase 1 inner radius in logical px (the hole): 34 px.
const P1_INNER_R: f64 = 34.0;

/// Phase 2 rectangle, in logical px: (340, 40) .. (600, 250).
const P2_RECT: ((f64, f64), (f64, f64)) = ((340.0, 40.0), (600.0, 250.0));

/// Phase 3 annulus centre in logical px (same radii as phase 1).
const P3_CENTER: (f64, f64) = (790.0, 180.0);

/// Schedule the P6b `HATCH` phases relative to the driver start.
///
/// They start at driver t=178 s (after the P5e phases, which end at t=177.5 s)
/// and step every 500 ms; the combined single-line report is scheduled after
/// them at t=205 s (see [`super::draw_edit::schedule_draw_report`]). Each phase
/// reads the real `cad_status` before and after its commit; a dispatch that
/// merely posts is not a pass.
///
/// The timeline:
/// * t=178 s..179.5 s: `cad_status` (camera), two middle-button pan drags,
///   `cad_status` again — the pair of camera reads proves whether the view
///   actually moved to empty space;
/// * t=180 s..187.5 s: phase 1 — outer `CIRCLE`, inner `CIRCLE`, window-box
///   select both, `HATCH` (defaults: solid, `Normal`), `CONFIRM`, read
///   `entityCount` (**+1** for the hatch; each circle is +1 on its own read) and
///   `tool` `null`;
/// * t=188 s..194.5 s: phase 2 — `RECTANG`, click-select it, set
///   `hatchPatternType=1`, `hatchAngle=30`, `hatchSpacing=0.05`, `HATCH`,
///   `CONFIRM`, read `entityCount` **+1**;
/// * t=195 s..203.5 s: phase 3 — set `hatchPatternType=0` and `hatchStyle=2`,
///   two `CIRCLE`s, window-box select both, `HATCH`, `CONFIRM`, read
///   `entityCount` **+1**;
/// * t=204 s: a click-free pointer move to (10, 337) so the viewport-spanning
///   crosshair is parked outside every measured region;
/// * t=204.5 s: build `window.__yacr_hatch_summary`, which the combined report
///   appends as the `hatch_*`/`h1_*`/`h2_*`/`h3_*` fields.
pub(super) fn schedule_hatch_phases(webview: &tauri::Webview, start: &Instant) {
    // ---- Pan the view into empty world space (two middle-button drags) ----
    schedule_draw_ms(
        start,
        178_000,
        "hatch-pan-before cad_status (camera before the pan)",
        || webview.eval(draw_status_js("s_hatch_pan_before")),
    );
    let pan1 = hatch_drag_px_js((880.0, 300.0), (60.0, 300.0), 1, "hatch_pan1");
    schedule_draw_ms(
        start,
        178_500,
        "hatch pan drag 1 (middle button, 880,300 -> 60,300)",
        || webview.eval(pan1),
    );
    let pan2 = hatch_drag_px_js((880.0, 300.0), (60.0, 300.0), 1, "hatch_pan2");
    schedule_draw_ms(
        start,
        179_000,
        "hatch pan drag 2 (middle button, 880,300 -> 60,300)",
        || webview.eval(pan2),
    );
    schedule_draw_ms(
        start,
        179_500,
        "hatch-pan-after cad_status (camera after the pan)",
        || webview.eval(draw_status_js("s_hatch_pan_after")),
    );

    // ---- Phase 1: solid hatch on two concentric circles (the hole) ----
    schedule_draw_ms(
        start,
        180_000,
        "cad_command CIRCLE (hatch phase 1 outer)",
        || webview.eval(draw_command_js("CIRCLE", "h1_outer_circle")),
    );
    let h1_oc = hatch_click_px_js(P1_CENTER.0, P1_CENTER.1, "h1_outer_center");
    schedule_draw_ms(
        start,
        180_500,
        "hatch phase 1 outer circle centre click at px (140, 180)",
        || webview.eval(h1_oc),
    );
    let h1_or = hatch_click_px_js(P1_CENTER.0 + P1_OUTER_R, P1_CENTER.1, "h1_outer_radius");
    schedule_draw_ms(
        start,
        181_000,
        "hatch phase 1 outer circle radius click at px (220, 180)",
        || webview.eval(h1_or),
    );
    schedule_draw_ms(
        start,
        181_500,
        "cad_command CONFIRM (hatch phase 1 outer circle)",
        || webview.eval(draw_confirm_js("confirm_h1_outer_circle")),
    );
    schedule_draw_ms(
        start,
        182_000,
        "hatch-phase1-outer-after cad_status (entityCount +1)",
        || webview.eval(draw_status_js("s_h1_outer_after")),
    );
    schedule_draw_ms(
        start,
        182_500,
        "cad_command CIRCLE (hatch phase 1 inner)",
        || webview.eval(draw_command_js("CIRCLE", "h1_inner_circle")),
    );
    let h1_ic = hatch_click_px_js(P1_CENTER.0, P1_CENTER.1, "h1_inner_center");
    schedule_draw_ms(
        start,
        183_000,
        "hatch phase 1 inner circle centre click at px (140, 180)",
        || webview.eval(h1_ic),
    );
    let h1_ir = hatch_click_px_js(P1_CENTER.0 + P1_INNER_R, P1_CENTER.1, "h1_inner_radius");
    schedule_draw_ms(
        start,
        183_500,
        "hatch phase 1 inner circle radius click at px (174, 180)",
        || webview.eval(h1_ir),
    );
    schedule_draw_ms(
        start,
        184_000,
        "cad_command CONFIRM (hatch phase 1 inner circle)",
        || webview.eval(draw_confirm_js("confirm_h1_inner_circle")),
    );
    schedule_draw_ms(
        start,
        184_500,
        "hatch-phase1-inner-after cad_status (entityCount +2 on the pair)",
        || webview.eval(draw_status_js("s_h1_inner_after")),
    );
    let h1_box = hatch_drag_px_js(
        (
            P1_CENTER.0 - P1_OUTER_R - 12.0,
            P1_CENTER.1 - P1_OUTER_R - 12.0,
        ),
        (
            P1_CENTER.0 + P1_OUTER_R + 12.0,
            P1_CENTER.1 + P1_OUTER_R + 12.0,
        ),
        0,
        "h1_box",
    );
    schedule_draw_ms(
        start,
        185_000,
        "hatch phase 1 window box select (primary drag around both circles)",
        || webview.eval(h1_box),
    );
    schedule_draw_ms(
        start,
        185_500,
        "hatch-phase1-selected cad_status (selection.count must read 2)",
        || webview.eval(draw_status_js("s_h1_select")),
    );
    schedule_draw_ms(start, 186_000, "cad_command HATCH (phase 1)", || {
        webview.eval(draw_command_js("HATCH", "h1_hatch"))
    });
    schedule_draw_ms(
        start,
        186_500,
        "hatch-phase1-active cad_status (tool active, params solid + Normal)",
        || webview.eval(draw_status_js("s_h1_active")),
    );
    schedule_draw_ms(
        start,
        187_000,
        "cad_command CONFIRM (hatch phase 1)",
        || webview.eval(draw_confirm_js("confirm_h1_hatch")),
    );
    schedule_draw_ms(
        start,
        187_500,
        "hatch-phase1-after cad_status (entityCount +1, tool null)",
        || webview.eval(draw_status_js("s_h1_after")),
    );

    // ---- Phase 2: user-defined pattern inside a closed rectangle ----
    schedule_draw_ms(
        start,
        188_000,
        "cad_command RECTANG (hatch phase 2 boundary)",
        || webview.eval(draw_command_js("RECTANG", "h2_rect")),
    );
    let h2_c1 = hatch_click_px_js(P2_RECT.0 .0, P2_RECT.0 .1, "h2_rect_a");
    schedule_draw_ms(
        start,
        188_500,
        "hatch phase 2 rectangle corner A at px (340, 40)",
        || webview.eval(h2_c1),
    );
    let h2_c2 = hatch_click_px_js(P2_RECT.1 .0, P2_RECT.1 .1, "h2_rect_b");
    schedule_draw_ms(
        start,
        189_000,
        "hatch phase 2 rectangle corner B at px (600, 250)",
        || webview.eval(h2_c2),
    );
    schedule_draw_ms(
        start,
        189_500,
        "cad_command CONFIRM (hatch phase 2 rectangle)",
        || webview.eval(draw_confirm_js("confirm_h2_rect")),
    );
    schedule_draw_ms(
        start,
        190_000,
        "hatch-phase2-rect-after cad_status (entityCount +1)",
        || webview.eval(draw_status_js("s_h2_rect_after")),
    );
    let h2_pick = hatch_click_px_js(
        (P2_RECT.0 .0 + P2_RECT.1 .0) / 2.0,
        P2_RECT.0 .1,
        "h2_select",
    );
    schedule_draw_ms(
        start,
        190_500,
        "hatch phase 2 click-select on the rectangle's top edge",
        || webview.eval(h2_pick),
    );
    schedule_draw_ms(
        start,
        191_000,
        "hatch-phase2-selected cad_status (selection.count must read 1)",
        || webview.eval(draw_status_js("s_h2_select")),
    );
    schedule_draw_ms(
        start,
        191_500,
        "cad_set_tool_parameter hatchPatternType=1 (user-defined)",
        || {
            webview.eval(draw_set_parameter_number_js(
                "hatchPatternType",
                1.0,
                "h2_pattern",
            ))
        },
    );
    schedule_draw_ms(
        start,
        192_000,
        "cad_set_tool_parameter hatchAngle=30",
        || webview.eval(draw_set_parameter_number_js("hatchAngle", 30.0, "h2_angle")),
    );
    schedule_draw_ms(
        start,
        192_500,
        "cad_set_tool_parameter hatchSpacing=0.05",
        || {
            webview.eval(draw_set_parameter_number_js(
                "hatchSpacing",
                0.05,
                "h2_spacing",
            ))
        },
    );
    schedule_draw_ms(start, 193_000, "cad_command HATCH (phase 2)", || {
        webview.eval(draw_command_js("HATCH", "h2_hatch"))
    });
    schedule_draw_ms(
        start,
        193_500,
        "hatch-phase2-active cad_status (pattern type 1, angle 30, spacing 0.05)",
        || webview.eval(draw_status_js("s_h2_active")),
    );
    schedule_draw_ms(
        start,
        194_000,
        "cad_command CONFIRM (hatch phase 2)",
        || webview.eval(draw_confirm_js("confirm_h2_hatch")),
    );
    schedule_draw_ms(
        start,
        194_500,
        "hatch-phase2-after cad_status (entityCount +1, tool null)",
        || webview.eval(draw_status_js("s_h2_after")),
    );

    // ---- Phase 3: the phase-1 geometry again, style = 2 (Ignore) ----
    schedule_draw_ms(
        start,
        195_000,
        "cad_set_tool_parameter hatchPatternType=0 (solid again)",
        || {
            webview.eval(draw_set_parameter_number_js(
                "hatchPatternType",
                0.0,
                "h3_pattern",
            ))
        },
    );
    schedule_draw_ms(
        start,
        195_500,
        "cad_set_tool_parameter hatchStyle=2 (Ignore)",
        || webview.eval(draw_set_parameter_number_js("hatchStyle", 2.0, "h3_style")),
    );
    schedule_draw_ms(
        start,
        196_000,
        "cad_command CIRCLE (hatch phase 3 outer)",
        || webview.eval(draw_command_js("CIRCLE", "h3_outer_circle")),
    );
    let h3_oc = hatch_click_px_js(P3_CENTER.0, P3_CENTER.1, "h3_outer_center");
    schedule_draw_ms(
        start,
        196_500,
        "hatch phase 3 outer circle centre click at px (790, 180)",
        || webview.eval(h3_oc),
    );
    let h3_or = hatch_click_px_js(P3_CENTER.0 + P1_OUTER_R, P3_CENTER.1, "h3_outer_radius");
    schedule_draw_ms(
        start,
        197_000,
        "hatch phase 3 outer circle radius click at px (870, 180)",
        || webview.eval(h3_or),
    );
    schedule_draw_ms(
        start,
        197_500,
        "cad_command CONFIRM (hatch phase 3 outer circle)",
        || webview.eval(draw_confirm_js("confirm_h3_outer_circle")),
    );
    schedule_draw_ms(
        start,
        198_000,
        "hatch-phase3-outer-after cad_status (entityCount +1)",
        || webview.eval(draw_status_js("s_h3_outer_after")),
    );
    schedule_draw_ms(
        start,
        198_500,
        "cad_command CIRCLE (hatch phase 3 inner)",
        || webview.eval(draw_command_js("CIRCLE", "h3_inner_circle")),
    );
    let h3_ic = hatch_click_px_js(P3_CENTER.0, P3_CENTER.1, "h3_inner_center");
    schedule_draw_ms(
        start,
        199_000,
        "hatch phase 3 inner circle centre click at px (790, 180)",
        || webview.eval(h3_ic),
    );
    let h3_ir = hatch_click_px_js(P3_CENTER.0 + P1_INNER_R, P3_CENTER.1, "h3_inner_radius");
    schedule_draw_ms(
        start,
        199_500,
        "hatch phase 3 inner circle radius click at px (824, 180)",
        || webview.eval(h3_ir),
    );
    schedule_draw_ms(
        start,
        200_000,
        "cad_command CONFIRM (hatch phase 3 inner circle)",
        || webview.eval(draw_confirm_js("confirm_h3_inner_circle")),
    );
    schedule_draw_ms(
        start,
        200_500,
        "hatch-phase3-inner-after cad_status (entityCount +2 on the pair)",
        || webview.eval(draw_status_js("s_h3_inner_after")),
    );
    let h3_box = hatch_drag_px_js(
        (
            P3_CENTER.0 - P1_OUTER_R - 12.0,
            P3_CENTER.1 - P1_OUTER_R - 12.0,
        ),
        (
            P3_CENTER.0 + P1_OUTER_R + 12.0,
            P3_CENTER.1 + P1_OUTER_R + 12.0,
        ),
        0,
        "h3_box",
    );
    schedule_draw_ms(
        start,
        201_000,
        "hatch phase 3 window box select (primary drag around both circles)",
        || webview.eval(h3_box),
    );
    schedule_draw_ms(
        start,
        201_500,
        "hatch-phase3-selected cad_status (selection.count must read 2)",
        || webview.eval(draw_status_js("s_h3_select")),
    );
    schedule_draw_ms(start, 202_000, "cad_command HATCH (phase 3)", || {
        webview.eval(draw_command_js("HATCH", "h3_hatch"))
    });
    schedule_draw_ms(
        start,
        202_500,
        "hatch-phase3-active cad_status (tool active, style 2 Ignore)",
        || webview.eval(draw_status_js("s_h3_active")),
    );
    schedule_draw_ms(
        start,
        203_000,
        "cad_command CONFIRM (hatch phase 3)",
        || webview.eval(draw_confirm_js("confirm_h3_hatch")),
    );
    schedule_draw_ms(
        start,
        203_500,
        "hatch-phase3-after cad_status (entityCount +1, tool null)",
        || webview.eval(draw_status_js("s_h3_after")),
    );

    // ---- Park the cursor: no click, just move the crosshair out of every
    // measured region (top-left corner, below/left of all hatch geometry). ----
    let park = hatch_click_free_px_js(10.0, 337.0, "hatch_park");
    schedule_draw_ms(
        start,
        204_000,
        "hatch_park click-free pointer move to px (10, 337)",
        || webview.eval(park),
    );

    // ---- Build the hatch report fragment the combined report appends. ----
    schedule_draw_ms(start, 204_500, "hatch summary", || {
        webview.eval(hatch_summary_js())
    });
}

/// Build one synthetic click at a **logical-pixel** position on the CAD surface.
///
/// The position is scaled from the nominal surface ([`NOMINAL_WIDTH`] ×
/// [`NOMINAL_HEIGHT`]) onto the live `.native-input` rect, so the layout stays
/// proportional if the rect is a pixel or two different, and the report records
/// both the fraction and the integer pixel it actually dispatched at. Synthetic,
/// **not a real mouse**.
fn hatch_click_px_js(x: f64, y: f64, label: &str) -> String {
    HATCH_CLICK_TEMPLATE
        .replace("__X__", &x.to_string())
        .replace("__Y__", &y.to_string())
        .replace("__LABEL__", label)
}

/// Build a click-free pointer move at a **logical-pixel** position.
///
/// Only `pointermove` is dispatched: no press, no release, no selection change.
/// Used to park the renderer's viewport-spanning crosshair outside the regions
/// the pixel read measures. Synthetic, **not a real mouse**.
fn hatch_click_free_px_js(x: f64, y: f64, label: &str) -> String {
    HATCH_MOVE_TEMPLATE
        .replace("__X__", &x.to_string())
        .replace("__Y__", &y.to_string())
        .replace("__LABEL__", label)
}

/// Build one synthetic **drag** at logical-pixel positions.
///
/// `button` is the DOM button number (`0` primary — a 2D window/crossing box
/// selection, `1` middle — a pan). One press, one move to the end point and one
/// release: the UI coalesces the move burst into a single latest-wins move, and
/// the host derives the whole drag from press/move/release, so one move is the
/// same gesture a human hand would produce. Synthetic, **not a real mouse**.
fn hatch_drag_px_js(from: (f64, f64), to: (f64, f64), button: u8, label: &str) -> String {
    let buttons = if button == 1 { "4" } else { "1" };
    HATCH_DRAG_TEMPLATE
        .replace("__X0__", &from.0.to_string())
        .replace("__Y0__", &from.1.to_string())
        .replace("__X1__", &to.0.to_string())
        .replace("__Y1__", &to.1.to_string())
        .replace("__BUTTON__", &button.to_string())
        .replace("__BUTTONS__", buttons)
        .replace("__LABEL__", label)
}

/// Build the summary script the combined report appends.
fn hatch_summary_js() -> String {
    HATCH_SUMMARY_TEMPLATE.to_string()
}

/// One synthetic click at a logical-pixel position on the CAD surface.
///
/// Same diagnostics as the world-point click in [`super::draw`] (`*_frac`,
/// `*_px`, `*_dispatched`, `*_failed`, `d.clicks`), but the position is given in
/// nominal surface pixels and scaled onto the live rect instead of being derived
/// from `cad_status.camera`. Synthetic, **not a real mouse**.
const HATCH_CLICK_TEMPLATE: &str = r#"
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
const HATCH_MOVE_TEMPLATE: &str = r#"
(function () {
  var d = window.__yacr_draw = window.__yacr_draw || {};
  var label = '__LABEL__';
  try {
    var el = document.querySelector('.native-input');
    if (!el) { d[label + '_failed'] = 'no-element'; return 'no-element'; }
    var r = el.getBoundingClientRect();
    if (!(r.width > 20 && r.height > 20)) {
      d[label + '_failed'] = 'tiny-rect';
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

/// One synthetic drag (press, one move, release) at logical-pixel positions.
///
/// `__BUTTON__` is the DOM button (`0` primary box window, `1` middle pan) and
/// `__BUTTONS__` its `PointerEvent.buttons` mask (`1` / `4`). Synthetic, **not a
/// real mouse**.
const HATCH_DRAG_TEMPLATE: &str = r#"
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
    var fx0 = __X0__ / 926.0, fy0 = __Y0__ / 341.0;
    var fx1 = __X1__ / 926.0, fy1 = __Y1__ / 341.0;
    d[label + '_frac'] = [fx0, fy0, fx1, fy1];
    if (!(fx0 >= 0 && fx0 <= 1 && fy0 >= 0 && fy0 <= 1 &&
          fx1 >= 0 && fx1 <= 1 && fy1 >= 0 && fy1 <= 1)) {
      d[label + '_failed'] = 'outside-canvas';
      return 'outside-canvas';
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
    var x0 = r.left + r.width * fx0, y0 = r.top + r.height * fy0;
    var x1 = r.left + r.width * fx1, y1 = r.top + r.height * fy1;
    fire('pointerdown', x0, y0, __BUTTON__, __BUTTONS__);
    fire('pointermove', x1, y1, -1, __BUTTONS__);
    fire('pointerup', x1, y1, __BUTTON__, 0);
    d[label + '_px'] = [Math.round(x0 - r.left), Math.round(y0 - r.top),
                        Math.round(x1 - r.left), Math.round(y1 - r.top)];
    d[label + '_dispatched'] = true;
    return label + '-dispatched';
  } catch (err) {
    try { d[label + '_failed'] = 'fatal:' + err; } catch (_) {}
    return 'error:' + err;
  }
})()
"#;

/// The hatch report fragment: one single-line, whitespace-free field list built
/// from the **real** `cad_status` reads the phases recorded in
/// `window.__yacr_draw`, stored in `window.__yacr_hatch_summary` so the combined
/// report in [`super::draw_edit`] can append it without duplicating the reads.
///
/// `-` means "absent / never read"; `-1` a delta that could not be computed
/// because one side of the read is missing. `hatch_status_missing` and
/// `hatch_step_failed` are derived from the recorded reads and dispatch errors
/// (never asserted), so a failed status read or a click that could not be
/// dispatched is visible instead of being smoothed over. The trailing
/// `hatch_geometry_channel=none_...` lines are the honest statement that
/// `cad_status` exposes no fill area, no loop count and no per-entity geometry.
const HATCH_SUMMARY_TEMPLATE: &str = r#"
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
  var reads = ['s_hatch_pan_before', 's_hatch_pan_after',
    's_h1_outer_after', 's_h1_inner_after', 's_h1_select',
    's_h1_active', 's_h1_after',
    's_h2_rect_after', 's_h2_select', 's_h2_active', 's_h2_after',
    's_h3_outer_after', 's_h3_inner_after', 's_h3_select',
    's_h3_active', 's_h3_after'];
  var missing = [];
  for (var i = 0; i < reads.length; i++) {
    if (!d[reads[i] + '_done']) { missing.push(reads[i]); }
  }
  var failed = [];
  for (var key in d) {
    if (/_failed$/.test(key) && d[key]) { failed.push(key + '=' + d[key]); }
  }
  var text = 'hatch_synthetic=1_synthetic_dom_pointer_events_not_a_real_mouse' +
    ' hatch_input=synthetic_dom_pointer_events_not_a_real_mouse' +
    ' hatch_status_missing=' + (missing.length ? missing.join(',') : '-') +
    ' hatch_step_failed=' + (failed.length ? failed.join(',') : '-') +
    ' hatch_pan_before_camera=' + arr(d.s_hatch_pan_before_camera) +
    ' hatch_pan_after_camera=' + arr(d.s_hatch_pan_after_camera) +
    ' hatch_pan1_px=' + arr(d.hatch_pan1_px) +
    ' hatch_pan2_px=' + arr(d.hatch_pan2_px) +
    ' h1_outer_circle_ack=' + v(d['h1_outer_circle_ack']) +
    ' h1_outer_center_px=' + arr(d.h1_outer_center_px) +
    ' h1_outer_radius_px=' + arr(d.h1_outer_radius_px) +
    ' h1_inner_circle_ack=' + v(d['h1_inner_circle_ack']) +
    ' h1_inner_center_px=' + arr(d.h1_inner_center_px) +
    ' h1_inner_radius_px=' + arr(d.h1_inner_radius_px) +
    ' h1_box_px=' + arr(d.h1_box_px) +
    ' h1_select_count=' + v(d.s_h1_select_selection) +
    ' h1_before_count=' + v(d.s_h1_inner_after_count) +
    ' h1_start_ack=' + v(d['h1_hatch_ack']) +
    ' h1_active_tool=' + v(d.s_h1_active_tool) +
    ' h1_active_points=' + v(d.s_h1_active_points) +
    ' h1_active_ready=' + v(d.s_h1_active_ready) +
    ' h1_active_pattern_type=' + v(d.s_h1_active_hatch_pattern_type) +
    ' h1_active_style=' + v(d.s_h1_active_hatch_style) +
    ' h1_confirm_ack=' + v(d.confirm_h1_hatch_ack) +
    ' h1_after_count=' + v(d.s_h1_after_count) +
    ' h1_delta=' + delta(d.s_h1_inner_after_count, d.s_h1_after_count) +
    ' h1_after_tool=' + v(d.s_h1_after_tool) +
    ' h1_after_error=' + v(d.s_h1_after_error) +
    ' h2_rect_ack=' + v(d['h2_rect_ack']) +
    ' h2_rect_corner_a_px=' + arr(d.h2_rect_a_px) +
    ' h2_rect_corner_b_px=' + arr(d.h2_rect_b_px) +
    ' h2_select_px=' + arr(d.h2_select_px) +
    ' h2_select_count=' + v(d.s_h2_select_selection) +
    ' h2_before_count=' + v(d.s_h2_rect_after_count) +
    ' h2_pattern_set_ack=' + v(d['h2_pattern_ack']) +
    ' h2_angle_set_ack=' + v(d['h2_angle_ack']) +
    ' h2_spacing_set_ack=' + v(d['h2_spacing_ack']) +
    ' h2_start_ack=' + v(d['h2_hatch_ack']) +
    ' h2_active_tool=' + v(d.s_h2_active_tool) +
    ' h2_active_pattern_type=' + v(d.s_h2_active_hatch_pattern_type) +
    ' h2_active_spacing=' + v(d.s_h2_active_hatch_spacing) +
    ' h2_active_angle=' + v(d.s_h2_active_hatch_angle) +
    ' h2_active_style=' + v(d.s_h2_active_hatch_style) +
    ' h2_confirm_ack=' + v(d.confirm_h2_hatch_ack) +
    ' h2_after_count=' + v(d.s_h2_after_count) +
    ' h2_delta=' + delta(d.s_h2_rect_after_count, d.s_h2_after_count) +
    ' h2_after_tool=' + v(d.s_h2_after_tool) +
    ' h2_after_error=' + v(d.s_h2_after_error) +
    ' h3_pattern_set_ack=' + v(d['h3_pattern_ack']) +
    ' h3_style_set_ack=' + v(d['h3_style_ack']) +
    ' h3_outer_circle_ack=' + v(d['h3_outer_circle_ack']) +
    ' h3_inner_circle_ack=' + v(d['h3_inner_circle_ack']) +
    ' h3_outer_center_px=' + arr(d.h3_outer_center_px) +
    ' h3_outer_radius_px=' + arr(d.h3_outer_radius_px) +
    ' h3_inner_center_px=' + arr(d.h3_inner_center_px) +
    ' h3_inner_radius_px=' + arr(d.h3_inner_radius_px) +
    ' h3_box_px=' + arr(d.h3_box_px) +
    ' h3_select_count=' + v(d.s_h3_select_selection) +
    ' h3_before_count=' + v(d.s_h3_inner_after_count) +
    ' h3_start_ack=' + v(d['h3_hatch_ack']) +
    ' h3_active_tool=' + v(d.s_h3_active_tool) +
    ' h3_active_style=' + v(d.s_h3_active_hatch_style) +
    ' h3_active_pattern_type=' + v(d.s_h3_active_hatch_pattern_type) +
    ' h3_confirm_ack=' + v(d.confirm_h3_hatch_ack) +
    ' h3_after_count=' + v(d.s_h3_after_count) +
    ' h3_delta=' + delta(d.s_h3_inner_after_count, d.s_h3_after_count) +
    ' h3_after_tool=' + v(d.s_h3_after_tool) +
    ' h3_after_error=' + v(d.s_h3_after_error) +
    ' hatch_park_px=' + arr(d.hatch_park_px) +
    ' hatch_geometry_channel=none_cad_status_has_no_fill_area_loop_count_or_entity_geometry' +
    ' hatch_style_channel=codes_only_0_normal_1_outer_2_ignore_pinned_by_contract_tests' +
    ' hatch_export_channel=none_pinned_by_contract_tests';
  try { window.__yacr_hatch_summary = ' ' + text; } catch (err) {}
  try { console.log('[probe] hatch evidence summary: ' + text); } catch (err) {}
  return text;
})()
"#;
