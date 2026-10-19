//! `YACR_PROBE_DRAW_EVIDENCE` P5e dimension phases (DIMANGULAR / DIMORDINATE).
//!
//! Split out of [`super::draw`] as a sibling module so every evidence file stays
//! well under the 1000-line code-file gate (`scripts/check-file-size.py`, no
//! whitelist). This is the same evidence-only, default-OFF, env-gated driver: it
//! dispatches the **real** `DIMANGULAR`/`DIMORDINATE` selectors through the normal
//! `cad_command` funnel and synthetic DOM pointer events on `.native-input`, then
//! reads the **real** `cad_status` before/after each commit. Every reported number
//! is a real `cad_status` read; nothing here is synthesized.
//!
//! What each phase proves (ADR 0017 decision C, P5e):
//! * **DIMANGULAR** is the first dimension kind whose **two leading steps are
//!   entity picks** (`DrawToolKind::DimAngular.picked_step_count() == 2`). The
//!   driver therefore first creates the geometry it needs — two **non-parallel
//!   lines sharing a vertex** — with two real `LINE` runs (`+2` entities), then
//!   starts the real `DIMANGULAR` selector, clicks **each line's body** (inside the
//!   pick tolerance, away from the shared vertex), clicks the dimension-arc
//!   location and `CONFIRM`s. `entityCount` must rise by exactly one (the new
//!   `AcDbDimension`) and `tool` must return to `null` with no error.
//! * **DIMORDINATE** captures two **free points** (feature point, leader endpoint;
//!   `Exact(2)`, no entity pick), then `CONFIRM` ⇒ `entityCount` **+1**.
//!
//! The arc-location pick also **selects which of the four angles is measured**
//! (ADR 0017 U-1: our declared rule is "the sector that contains the location
//! pick"). The driver places the two lines at 0° and 45° from the shared vertex
//! and picks a location in the middle of the 0°..45° sector, so the intended
//! measurement is unambiguous **for a human reading the frame**. `cad_status` has
//! **no geometry channel** (no definition points, no measured value, no sector),
//! so this driver does **not** claim the sector it reports was the sector measured
//! — that is pinned by the `cad-domain`/`cad-app` crate contract tests, plus the
//! committed QCAD fixture that closed U-8.
//!
//! The phases run **after** the P5d radial phases ([`super::draw_dimension_radial`],
//! which end at driver t=163 s) and **before** the combined single-line report
//! ([`super::draw_edit::schedule_draw_report`], moved to t=180 s), so the report's
//! `dimangular_*`/`dimordinate_*` fields carry the real before/after reads and the
//! capture instant lands after both commands have committed.
//!
//! Synthetic input, **not a real mouse**: the events are dispatched on
//! `.native-input` and never touch the kernel/evdev/XInput path. Both dimensions
//! are placed in the otherwise-empty **left band** of the visible extent (world
//! x ≈ −0.64..−0.03, y ≈ 0.02..1.20), left of the fixture's x = 0..3 verticals and
//! above/below the P5d centre-mark circle at (−0.30, 0.50), so the captured CAD
//! frame shows fresh ink exactly where the two new dimensions land.
//!
//! The last step is a **click-free pointer move** to a far corner: the renderer
//! always draws a viewport-spanning crosshair through the last cursor position, so
//! parking it at world (3.70, −0.30) keeps that crosshair (and its pickbox) out of
//! both measured regions instead of confusing the presence check.

use std::time::Instant;

use super::draw::{
    draw_click_world_js, draw_command_js, draw_confirm_js, draw_move_world_js, draw_status_js,
    schedule_draw_ms,
};

/// Schedule the P5e `DIMANGULAR` + `DIMORDINATE` phases relative to the driver
/// start.
///
/// They start at driver t=163.5 s (after the P5d phases, which end at t=163 s) and
/// step every 500 ms; the combined report is scheduled after them at t=180 s (see
/// [`super::draw_edit::schedule_draw_report`]). Each phase reads the real
/// `cad_status` before and after its commit; a dispatch that merely posts is not a
/// pass.
///
/// Source points are world coordinates in the empty left band of the visible
/// extent (camera centre (1.5, 0.5), ~0.005032 world/logical-pixel; the
/// `.native-input` rect is ~926×341, so world x ≈ −0.83..3.83, y ≈ −0.36..1.36,
/// while the fixture's four verticals live at x = 0..3, y = 0..1).
///
/// The angular pair shares the vertex `V = (−0.60, 0.90)`: line 1 is the 0° ray to
/// `(−0.05, 0.90)`, line 2 the 45° ray to `(−0.35, 1.15)`. The body clicks sit on
/// each segment (0.30 and ~0.177 world units from `V`, far outside the 3-logical-
/// pixel pickbox), and the location pick sits at 22.5° / 0.25 world units from `V`
/// — the middle of the intended 0°..45° sector, and ~19 px from either ray so the
/// 12-logical-pixel snap aperture cannot pull it onto a ray.
pub(super) fn schedule_angular_ordinate_phases(webview: &tauri::Webview, start: &Instant) {
    // ---- DIMANGULAR geometry: two lines sharing the vertex (-0.60, 0.90) ----
    schedule_draw_ms(
        start,
        163_500,
        "cad_command LINE (dimangular line 1)",
        || webview.eval(draw_command_js("LINE", "ang_line1")),
    );
    let l1_v = draw_click_world_js(-0.60, 0.90, "ang_l1_v");
    schedule_draw_ms(
        start,
        164_000,
        "dimangular line 1 vertex click at world (-0.60, 0.90)",
        || webview.eval(l1_v),
    );
    let l1_e = draw_click_world_js(-0.05, 0.90, "ang_l1_end");
    schedule_draw_ms(
        start,
        164_500,
        "dimangular line 1 end click at world (-0.05, 0.90)",
        || webview.eval(l1_e),
    );
    schedule_draw_ms(
        start,
        165_000,
        "cad_command CONFIRM (dimangular line 1)",
        || webview.eval(draw_confirm_js("confirm_ang_line1")),
    );
    schedule_draw_ms(
        start,
        165_500,
        "dimangular-line1-after cad_status (entityCount +1)",
        || webview.eval(draw_status_js("s_ang_line1_after")),
    );
    schedule_draw_ms(
        start,
        166_000,
        "cad_command LINE (dimangular line 2)",
        || webview.eval(draw_command_js("LINE", "ang_line2")),
    );
    let l2_v = draw_click_world_js(-0.60, 0.90, "ang_l2_v");
    schedule_draw_ms(
        start,
        166_500,
        "dimangular line 2 vertex click at world (-0.60, 0.90) (shared vertex)",
        || webview.eval(l2_v),
    );
    let l2_e = draw_click_world_js(-0.35, 1.15, "ang_l2_end");
    schedule_draw_ms(
        start,
        167_000,
        "dimangular line 2 end click at world (-0.35, 1.15) (45 deg from line 1)",
        || webview.eval(l2_e),
    );
    schedule_draw_ms(
        start,
        167_500,
        "cad_command CONFIRM (dimangular line 2)",
        || webview.eval(draw_confirm_js("confirm_ang_line2")),
    );
    schedule_draw_ms(
        start,
        168_000,
        "dimangular-line2-after cad_status (entityCount +2 on the pair)",
        || webview.eval(draw_status_js("s_ang_line2_after")),
    );

    // ---- DIMANGULAR: two entity picks, then the arc location ----
    schedule_draw_ms(start, 168_500, "cad_command DIMANGULAR", || {
        webview.eval(draw_command_js("DIMANGULAR", "dimangular"))
    });
    schedule_draw_ms(
        start,
        169_000,
        "dimangular-active cad_status (tool active, points 0)",
        || webview.eval(draw_status_js("s_dimangular_active")),
    );
    let angular_pick1 = draw_click_world_js(-0.30, 0.90, "dimangular_pick1");
    schedule_draw_ms(
        start,
        169_500,
        "dimangular line 1 body pick at world (-0.30, 0.90) (on line 1)",
        || webview.eval(angular_pick1),
    );
    schedule_draw_ms(
        start,
        170_000,
        "dimangular-pick1 cad_status (points 1, still an entity pick)",
        || webview.eval(draw_status_js("s_dimangular_pick1")),
    );
    let angular_pick2 = draw_click_world_js(-0.475, 1.025, "dimangular_pick2");
    schedule_draw_ms(
        start,
        170_500,
        "dimangular line 2 body pick at world (-0.475, 1.025) (on line 2)",
        || webview.eval(angular_pick2),
    );
    schedule_draw_ms(
        start,
        171_000,
        "dimangular-pick2 cad_status (points 2, now a free point)",
        || webview.eval(draw_status_js("s_dimangular_pick2")),
    );
    let angular_loc = draw_click_world_js(-0.369, 0.996, "dimangular_loc");
    schedule_draw_ms(
        start,
        171_500,
        "dimangular arc location click at world (-0.369, 0.996) (22.5 deg, inside the 0..45 sector)",
        || webview.eval(angular_loc),
    );
    schedule_draw_ms(
        start,
        172_000,
        "dimangular-points cad_status (points 3, ready)",
        || webview.eval(draw_status_js("s_dimangular_points")),
    );
    schedule_draw_ms(start, 172_500, "cad_command CONFIRM (dimangular)", || {
        webview.eval(draw_confirm_js("confirm_dimangular"))
    });
    schedule_draw_ms(
        start,
        173_000,
        "dimangular-after cad_status (entityCount +1, tool null)",
        || webview.eval(draw_status_js("s_dimangular_after")),
    );

    // ---- DIMORDINATE: feature point, leader endpoint, CONFIRM ----
    schedule_draw_ms(start, 173_500, "cad_command DIMORDINATE", || {
        webview.eval(draw_command_js("DIMORDINATE", "dimordinate"))
    });
    schedule_draw_ms(
        start,
        174_000,
        "dimordinate-active cad_status (tool active, points 0)",
        || webview.eval(draw_status_js("s_dimordinate_active")),
    );
    let ordinate_feature = draw_click_world_js(-0.45, 0.10, "dimordinate_feature");
    schedule_draw_ms(
        start,
        174_500,
        "dimordinate feature click at world (-0.45, 0.10)",
        || webview.eval(ordinate_feature),
    );
    schedule_draw_ms(
        start,
        175_000,
        "dimordinate-feature cad_status (points 1)",
        || webview.eval(draw_status_js("s_dimordinate_feature")),
    );
    let ordinate_leader = draw_click_world_js(-0.20, 0.18, "dimordinate_leader");
    schedule_draw_ms(
        start,
        175_500,
        "dimordinate leader click at world (-0.20, 0.18) (|dy| < |dx| => Y ordinate)",
        || webview.eval(ordinate_leader),
    );
    schedule_draw_ms(
        start,
        176_000,
        "dimordinate-points cad_status (points 2, ready)",
        || webview.eval(draw_status_js("s_dimordinate_points")),
    );
    schedule_draw_ms(start, 176_500, "cad_command CONFIRM (dimordinate)", || {
        webview.eval(draw_confirm_js("confirm_dimordinate"))
    });
    schedule_draw_ms(
        start,
        177_000,
        "dimordinate-after cad_status (entityCount +1, tool null)",
        || webview.eval(draw_status_js("s_dimordinate_after")),
    );

    // ---- Park the cursor: no click, just move the crosshair out of both bands ----
    let park = draw_move_world_js(3.70, -0.30, "dim_park");
    schedule_draw_ms(
        start,
        177_500,
        "dim_park click-free pointer move to world (3.70, -0.30) (moves the crosshair away)",
        || webview.eval(park),
    );
}
