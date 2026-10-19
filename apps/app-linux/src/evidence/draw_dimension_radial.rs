//! `YACR_PROBE_DRAW_EVIDENCE` P5d radial-dimension phases (DIMRADIUS /
//! DIMDIAMETER / DIMCENTER).
//!
//! Split out of [`super::draw_dimension`] as a sibling module so every evidence
//! file stays well under the 1000-line code-file gate
//! (`scripts/check-file-size.py`, no whitelist). This is the same evidence-only,
//! default-OFF, env-gated driver: it dispatches the **real**
//! `DIMRADIUS`/`DIMDIAMETER`/`DIMCENTER` selectors through the normal
//! `cad_command` funnel and synthetic DOM pointer events on `.native-input`, then
//! reads the **real** `cad_status` before/after each commit. Every reported number
//! is a real `cad_status` read; nothing here is synthesized.
//!
//! These are the first creation commands whose **leading step is an entity pick**
//! (ADR 0017 decision C, P5d: `DrawToolKind::picked_step_count() == 1` for the
//! radial pair and DIMCENTER). So each phase first creates the arc/circle it
//! needs to pick — `CIRCLE` (centre + radius point), drawn by this driver since
//! the fixture is four straight LINEs — then:
//! * **DIMRADIUS**: start the real selector, click the circle's edge (the entity
//!   pick; a click the pick resolves to the circle), read the captured-point
//!   count (`Exact(2)`; the pick advances it to 1), click the dimension-line
//!   **location**, `CONFIRM`, and read `cad_status`: `entityCount` must rise by
//!   exactly one (the new `AcDbDimension`) and `tool` must return to `null` with
//!   no error.
//! * **DIMDIAMETER**: the same pick-then-location funnel ⇒ `entityCount` **+1**.
//! * **DIMCENTER**: start the real selector, click the circle's edge (its single
//!   `Exact(1)` pick), `CONFIRM`, and read `cad_status`. Its output is **plain
//!   line geometry**, not a `DIMENSION` entity (ADR 0017 decision D.7), and the
//!   number of committed lines is whatever the drawing's active `Standard`
//!   `DIMCEN` dictates: `cad_status` has **no** DIMSTYLE channel, so this driver
//!   reports the **real** delta and never asserts a count it cannot read. On the
//!   acadrust-authored fixture the source `Standard` carries a non-zero `DIMCEN`
//!   (acadrust seeds `dimcen = 0.09`), so the centre mark's crossed pair of lines
//!   is committed (delta `2`); the `DIMCEN = 0` "nothing, no transaction" case
//!   (ADR 0017 P5b deviation 2 / `cad-db` `DimStyle::standard_metric`) is pinned
//!   by the crate contract tests, not observable here.
//!
//! The **location pick only sets the leader direction**; the text position is
//! derived by the synthesis at the leader midpoint (ADR 0017 decision C/P5d
//! deviation 1). Nothing in this driver's report implies the user positioned the
//! text.
//!
//! The phases run **after** the P5c dimension phases ([`super::draw_dimension`],
//! which end at driver t=144.5 s) and **before** the combined single-line report
//! ([`super::draw_edit::schedule_draw_report`], moved to t=165 s), so the report's
//! `dimradius_*`/`dimdiameter_*`/`dimcenter_*` fields carry the real before/after
//! reads and the capture instant lands after every P5d command has committed.
//!
//! Synthetic input, **not a real mouse**: the events are dispatched on
//! `.native-input` and never touch the kernel/evdev/XInput path. `cad_status`
//! exposes **no** per-entity geometry and **no** vertex count, so neither the
//! dimension's real geometry nor DIMCENTER's real lines are observable through
//! the status channel; the honest change observable is `entityCount` (plus the
//! surfaced technical `error`), and the geometry is pinned by the `cad-app` crate
//! contract tests (`tests/drawing/dimension_radial.rs`,
//! `host/tests/dimension.rs`). The circles and the two dimensions are placed in
//! the otherwise-empty **right band** and **top band** of the visible extent
//! (world x ≈ 2.0..3.5, world y ≈ 1.1..1.2) and the **far-left** band (world
//! x ≈ −0.3), so the captured CAD frame shows fresh ink exactly where the new
//! entities land.

use std::time::Instant;

use super::draw::{
    draw_click_world_js, draw_command_js, draw_confirm_js, draw_status_js, schedule_draw_ms,
};

/// Schedule the P5d radial-dimension phases relative to the driver start.
///
/// They start at driver t=145 s (after the P5c phases, which end at t=144.5 s)
/// and step every 500 ms; the combined report is scheduled after them at t=165 s
/// (see [`super::draw_edit::schedule_draw_report`]). Each phase reads the real
/// `cad_status` before and after its commit; a dispatch that merely posts is not
/// a pass.
///
/// Source points are world coordinates in free bands of the visible extent
/// (camera centre (1.5, 0.5), ~0.005032 world/logical-pixel; the `.native-input`
/// rect is ~926×341, so world x ≈ −0.83..3.83, y ≈ −0.36..1.36, while the
/// fixture's four verticals live at x = 0..3, y = 0..1). Each circle's edge pick
/// lands exactly on its rightmost quadrant point (a snap target **and** on the
/// circle), so the snap-then-pick path resolves the circle with no displacement;
/// the radial location pick is offset along the world +X direction from the
/// centre, so the leader points right.
pub(super) fn schedule_radial_dimension_phases(webview: &tauri::Webview, start: &Instant) {
    // ---- DIMRADIUS: create a circle, pick it, then the dimension-line location ----
    schedule_draw_ms(
        start,
        145_000,
        "cad_command CIRCLE (dimradius target)",
        || webview.eval(draw_command_js("CIRCLE", "rad_circle")),
    );
    let rad_c = draw_click_world_js(3.30, 1.10, "rad_circle_center");
    schedule_draw_ms(
        start,
        145_500,
        "dimradius circle centre click at world (3.30, 1.10)",
        || webview.eval(rad_c),
    );
    let rad_r = draw_click_world_js(3.48, 1.10, "rad_circle_radius");
    schedule_draw_ms(
        start,
        146_000,
        "dimradius circle radius click at world (3.48, 1.10)",
        || webview.eval(rad_r),
    );
    schedule_draw_ms(
        start,
        146_500,
        "cad_command CONFIRM (dimradius circle)",
        || webview.eval(draw_confirm_js("confirm_rad_circle")),
    );
    schedule_draw_ms(
        start,
        147_000,
        "dimradius-target-after cad_status (circle +1)",
        || webview.eval(draw_status_js("s_rad_circle_after")),
    );
    schedule_draw_ms(start, 147_500, "cad_command DIMRADIUS", || {
        webview.eval(draw_command_js("DIMRADIUS", "dimradius"))
    });
    schedule_draw_ms(
        start,
        148_000,
        "dimradius-active cad_status (tool active, points 0)",
        || webview.eval(draw_status_js("s_dimradius_active")),
    );
    let dr_pick = draw_click_world_js(3.48, 1.10, "dimradius_pick");
    schedule_draw_ms(
        start,
        148_500,
        "dimradius edge pick at world (3.48, 1.10) (on the circle)",
        || webview.eval(dr_pick),
    );
    schedule_draw_ms(
        start,
        149_000,
        "dimradius-pick cad_status (points 1, still needs the location)",
        || webview.eval(draw_status_js("s_dimradius_pick")),
    );
    let dr_loc = draw_click_world_js(3.55, 1.10, "dimradius_loc");
    schedule_draw_ms(
        start,
        149_500,
        "dimradius location click at world (3.55, 1.10) (sets the direction)",
        || webview.eval(dr_loc),
    );
    schedule_draw_ms(
        start,
        150_000,
        "dimradius-points cad_status (points 2, ready)",
        || webview.eval(draw_status_js("s_dimradius_points")),
    );
    schedule_draw_ms(start, 150_500, "cad_command CONFIRM (dimradius)", || {
        webview.eval(draw_confirm_js("confirm_dimradius"))
    });
    schedule_draw_ms(
        start,
        151_000,
        "dimradius-after cad_status (entityCount +1, tool null)",
        || webview.eval(draw_status_js("s_dimradius_after")),
    );

    // ---- DIMDIAMETER: create a circle, pick it, then the dimension-line location ----
    schedule_draw_ms(
        start,
        151_500,
        "cad_command CIRCLE (dimdiameter target)",
        || webview.eval(draw_command_js("CIRCLE", "dia_circle")),
    );
    let dia_c = draw_click_world_js(2.00, 1.15, "dia_circle_center");
    schedule_draw_ms(
        start,
        152_000,
        "dimdiameter circle centre click at world (2.00, 1.15)",
        || webview.eval(dia_c),
    );
    let dia_r = draw_click_world_js(2.15, 1.15, "dia_circle_radius");
    schedule_draw_ms(
        start,
        152_500,
        "dimdiameter circle radius click at world (2.15, 1.15)",
        || webview.eval(dia_r),
    );
    schedule_draw_ms(
        start,
        153_000,
        "cad_command CONFIRM (dimdiameter circle)",
        || webview.eval(draw_confirm_js("confirm_dia_circle")),
    );
    schedule_draw_ms(
        start,
        153_500,
        "dimdiameter-target-after cad_status (circle +1)",
        || webview.eval(draw_status_js("s_dia_circle_after")),
    );
    schedule_draw_ms(start, 154_000, "cad_command DIMDIAMETER", || {
        webview.eval(draw_command_js("DIMDIAMETER", "dimdiameter"))
    });
    schedule_draw_ms(
        start,
        154_500,
        "dimdiameter-active cad_status (tool active, points 0)",
        || webview.eval(draw_status_js("s_dimdiameter_active")),
    );
    let dd_pick = draw_click_world_js(2.15, 1.15, "dimdiameter_pick");
    schedule_draw_ms(
        start,
        155_000,
        "dimdiameter edge pick at world (2.15, 1.15) (on the circle)",
        || webview.eval(dd_pick),
    );
    schedule_draw_ms(
        start,
        155_500,
        "dimdiameter-pick cad_status (points 1, still needs the location)",
        || webview.eval(draw_status_js("s_dimdiameter_pick")),
    );
    let dd_loc = draw_click_world_js(2.30, 1.15, "dimdiameter_loc");
    schedule_draw_ms(
        start,
        156_000,
        "dimdiameter location click at world (2.30, 1.15) (sets the direction)",
        || webview.eval(dd_loc),
    );
    schedule_draw_ms(
        start,
        156_500,
        "dimdiameter-points cad_status (points 2, ready)",
        || webview.eval(draw_status_js("s_dimdiameter_points")),
    );
    schedule_draw_ms(start, 157_000, "cad_command CONFIRM (dimdiameter)", || {
        webview.eval(draw_confirm_js("confirm_dimdiameter"))
    });
    schedule_draw_ms(
        start,
        157_500,
        "dimdiameter-after cad_status (entityCount +1, tool null)",
        || webview.eval(draw_status_js("s_dimdiameter_after")),
    );

    // ---- DIMCENTER: create a circle, pick it, CONFIRM (plain-line centre mark) ----
    schedule_draw_ms(
        start,
        158_000,
        "cad_command CIRCLE (dimcenter target)",
        || webview.eval(draw_command_js("CIRCLE", "center_circle")),
    );
    let cen_c = draw_click_world_js(-0.30, 0.50, "center_circle_center");
    schedule_draw_ms(
        start,
        158_500,
        "dimcenter circle centre click at world (-0.30, 0.50)",
        || webview.eval(cen_c),
    );
    let cen_r = draw_click_world_js(-0.10, 0.50, "center_circle_radius");
    schedule_draw_ms(
        start,
        159_000,
        "dimcenter circle radius click at world (-0.10, 0.50)",
        || webview.eval(cen_r),
    );
    schedule_draw_ms(
        start,
        159_500,
        "cad_command CONFIRM (dimcenter circle)",
        || webview.eval(draw_confirm_js("confirm_center_circle")),
    );
    schedule_draw_ms(
        start,
        160_000,
        "dimcenter-target-after cad_status (circle +1)",
        || webview.eval(draw_status_js("s_center_circle_after")),
    );
    schedule_draw_ms(start, 160_500, "cad_command DIMCENTER", || {
        webview.eval(draw_command_js("DIMCENTER", "dimcenter"))
    });
    schedule_draw_ms(
        start,
        161_000,
        "dimcenter-active cad_status (tool active, points 0)",
        || webview.eval(draw_status_js("s_dimcenter_active")),
    );
    let dc_pick = draw_click_world_js(-0.10, 0.50, "dimcenter_pick");
    schedule_draw_ms(
        start,
        161_500,
        "dimcenter edge pick at world (-0.10, 0.50) (on the circle)",
        || webview.eval(dc_pick),
    );
    schedule_draw_ms(
        start,
        162_000,
        "dimcenter-pick cad_status (points 1, ready)",
        || webview.eval(draw_status_js("s_dimcenter_pick")),
    );
    schedule_draw_ms(start, 162_500, "cad_command CONFIRM (dimcenter)", || {
        webview.eval(draw_confirm_js("confirm_dimcenter"))
    });
    schedule_draw_ms(
        start,
        163_000,
        "dimcenter-after cad_status (real delta; plain lines, not a DIMENSION)",
        || webview.eval(draw_status_js("s_dimcenter_after")),
    );
}
