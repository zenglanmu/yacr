//! `YACR_PROBE_DRAW_EVIDENCE` P5c dimension-creation phases (DIMLINEAR /
//! DIMALIGNED).
//!
//! Split out of [`super::draw`] as a sibling module so every evidence file stays
//! well under the 1000-line code-file gate (`scripts/check-file-size.py`, no
//! whitelist). This is the same evidence-only, default-OFF, env-gated driver: it
//! dispatches the **real** `DIMLINEAR`/`DIMALIGNED` selectors through the normal
//! `cad_command` funnel and synthetic DOM pointer events on `.native-input`, then
//! reads the **real** `cad_status` before/after each commit. Every reported number
//! is a real `cad_status` read; nothing here is synthesized.
//!
//! What each phase proves (ADR 0017 decision C, P5c):
//! * **DIMLINEAR**: start the real `DIMLINEAR` selector, feed the three synthetic
//!   picks in AutoCAD's capture order — the two extension-line origins, then the
//!   dimension-line location — read the captured-point count (`Exact(3)`), then
//!   `CONFIRM`, and read `cad_status`: `entityCount` must rise by exactly one
//!   (the new `AcDbDimension`) and `tool` must return to `null` with no error.
//! * **DIMALIGNED**: the same three-pick funnel for the aligned dimension ⇒
//!   `entityCount` **+1** and `tool` `null`.
//!
//! The phases run **after** the P4 edit-command phases (see [`super::draw_edit`],
//! which ends at driver t=135.5 s) and **before** the combined single-line report
//! ([`super::draw_edit::schedule_draw_report`]), so the report's
//! `dimlinear_*`/`dimaligned_*` fields carry the real before/after reads and the
//! capture instant lands after both dimensions have committed.
//!
//! Synthetic input, **not a real mouse**: the events are dispatched on
//! `.native-input` and never touch the kernel/evdev/XInput path. `cad_status`
//! exposes **no** per-entity geometry and **no** vertex count, so the dimension's
//! real geometry is not observable through the status channel; the honest change
//! observable is `entityCount` (plus the surfaced technical `error`), and the
//! geometry is pinned by the `cad-app` crate contract tests
//! (`tests/drawing/dimension.rs`). The two dimensions are placed in the
//! otherwise-empty **right band** of the visible extent (world x ≈ 3.2..3.8, to
//! the right of the fixture's x = 0..3 verticals) so the captured CAD frame shows
//! fresh ink exactly where the new dimensions land.

use std::time::Instant;

use super::draw::{
    draw_click_world_js, draw_command_js, draw_confirm_js, draw_status_js, schedule_draw_ms,
};

/// Schedule the P5c dimension-creation phases relative to the driver start.
///
/// They start at driver t=137 s (after the P4 phases, which end at t=135.5 s) and
/// step every 500 ms; the combined report is scheduled after them at t=146 s (see
/// [`super::draw_edit::schedule_draw_report`]). Each phase reads the real
/// `cad_status` before and after its commit; a dispatch that merely posts is not
/// a pass.
///
/// Source points are world coordinates in the empty right band of the visible
/// extent (camera centre (1.5, 0.5), ~0.005032 world/logical-pixel; the
/// `.native-input` rect is 926×341, so world x ≈ 3.2..3.8 maps inside the canvas
/// while the fixture's four verticals live at x = 0..3). DIMLINEAR's location
/// pick is offset more vertically than horizontally from the origins' midpoint,
/// so it resolves to a **horizontal** dimension (measures X); DIMALIGNED's
/// origins are a slanted pair, so its measurement is the true origin-to-origin
/// distance.
pub(super) fn schedule_dimension_phases(webview: &tauri::Webview, start: &Instant) {
    // ---- DIMLINEAR: two extension-line origins + the dimension-line location ----
    schedule_draw_ms(start, 137_000, "cad_command DIMLINEAR", || {
        webview.eval(draw_command_js("DIMLINEAR", "dimlinear"))
    });
    schedule_draw_ms(
        start,
        137_500,
        "dimlinear-active cad_status (tool active, points 0)",
        || webview.eval(draw_status_js("s_dimlinear_active")),
    );
    let dl_o1 = draw_click_world_js(3.25, 0.45, "dimlinear_o1");
    schedule_draw_ms(
        start,
        138_000,
        "dimlinear origin1 click at world (3.25, 0.45)",
        || webview.eval(dl_o1),
    );
    let dl_o2 = draw_click_world_js(3.75, 0.45, "dimlinear_o2");
    schedule_draw_ms(
        start,
        138_500,
        "dimlinear origin2 click at world (3.75, 0.45)",
        || webview.eval(dl_o2),
    );
    let dl_loc = draw_click_world_js(3.5, 0.8, "dimlinear_loc");
    schedule_draw_ms(
        start,
        139_000,
        "dimlinear location click at world (3.5, 0.8)",
        || webview.eval(dl_loc),
    );
    schedule_draw_ms(
        start,
        139_500,
        "dimlinear-points cad_status (points 3, ready)",
        || webview.eval(draw_status_js("s_dimlinear_points")),
    );
    schedule_draw_ms(start, 140_000, "cad_command CONFIRM (dimlinear)", || {
        webview.eval(draw_confirm_js("confirm_dimlinear"))
    });
    schedule_draw_ms(
        start,
        140_500,
        "dimlinear-after cad_status (entityCount +1, tool null)",
        || webview.eval(draw_status_js("s_dimlinear_after")),
    );

    // ---- DIMALIGNED: two extension-line origins + the dimension-line location ----
    schedule_draw_ms(start, 141_000, "cad_command DIMALIGNED", || {
        webview.eval(draw_command_js("DIMALIGNED", "dimaligned"))
    });
    schedule_draw_ms(
        start,
        141_500,
        "dimaligned-active cad_status (tool active, points 0)",
        || webview.eval(draw_status_js("s_dimaligned_active")),
    );
    let da_o1 = draw_click_world_js(3.3, 0.1, "dimaligned_o1");
    schedule_draw_ms(
        start,
        142_000,
        "dimaligned origin1 click at world (3.3, 0.1)",
        || webview.eval(da_o1),
    );
    let da_o2 = draw_click_world_js(3.7, 0.3, "dimaligned_o2");
    schedule_draw_ms(
        start,
        142_500,
        "dimaligned origin2 click at world (3.7, 0.3)",
        || webview.eval(da_o2),
    );
    let da_loc = draw_click_world_js(3.2, 0.35, "dimaligned_loc");
    schedule_draw_ms(
        start,
        143_000,
        "dimaligned location click at world (3.2, 0.35)",
        || webview.eval(da_loc),
    );
    schedule_draw_ms(
        start,
        143_500,
        "dimaligned-points cad_status (points 3, ready)",
        || webview.eval(draw_status_js("s_dimaligned_points")),
    );
    schedule_draw_ms(start, 144_000, "cad_command CONFIRM (dimaligned)", || {
        webview.eval(draw_confirm_js("confirm_dimaligned"))
    });
    schedule_draw_ms(
        start,
        144_500,
        "dimaligned-after cad_status (entityCount +1, tool null)",
        || webview.eval(draw_status_js("s_dimaligned_after")),
    );
}
