//! `YACR_PROBE_DRAW_EVIDENCE`: the new-drawing-command evidence driver.
//!
//! Evidence-only, default OFF, and independent of every other probe driver
//! (setting only this one does not trigger the input/tool/hover/lifecycle/box/
//! edit passes). It drives the **real** new drawing/edit selectors against a real
//! running host through the normal `cad_command` funnel and synthetic DOM
//! pointer events on `.native-input`, then reads the **real** `cad_status`
//! before/after each commit.
//!
//! What it proves, one phase per claim:
//! 1. **FILLET geometry**: draw two **intersecting** LINEs (a horizontal and a
//!    vertical that cross) with the real `LINE` selector — four synthetic
//!    clicks, two `CONFIRM`s — and read `cad_status`: `entityCount` must rise by
//!    exactly two. The four-parallel-line fixture cannot demonstrate FILLET, so
//!    the driver creates the geometry it needs through the drawing commands.
//! 2. **FILLET end to end**: set the radius through the real IPC
//!    `cad_set_tool_parameter { parameter: 'filletRadius', number: R }`, start
//!    `FILLET`, click one **empty** point (the capture-time entity pick must
//!    refuse it and not advance — a deliberate miss), pick the two lines, dispatch
//!    `CONFIRM`, and read `cad_status`: `entityCount` must rise by exactly one
//!    (the new tangent ARC; the two lines are modified, not added) and `tool` must
//!    return to `null`. The radius is also reported as visible while FILLET is
//!    active (`tool.params.filletRadius`).
//! 3. **CHAMFER geometry**: draw a second intersecting LINE pair (FILLET trims the
//!    first pair, so a fresh pair is drawn for CHAMFER) — `entityCount` +2.
//! 4. **CHAMFER end to end**: set `chamferDistanceA`/`chamferDistanceB` through
//!    the real IPC, start `CHAMFER`, pick the two lines, `CONFIRM`, and read
//!    `entityCount` +1 (the new joining LINE) and `tool` `null`.
//! 5. **POLYGON with a changed side count** (re-run with the new IPC): call the
//!    real IPC `cad_set_tool_parameter` with a non-default value (6) for
//!    `polygonSides` (the old `cad_set_polygon_sides` is gone), start `POLYGON`,
//!    click centre + radius, `CONFIRM`, and read `entityCount` +1 and `tool`
//!    `null`. Separately it asserts the side count is visible while the tool is
//!    active (`tool.params.sides == 6`), and that an out-of-range value (2) is
//!    **refused** with the real error text and leaves the visible value unchanged.
//! 6. **POINT visibility**: start `POINT`, click one point, `CONFIRM`, read
//!    `entityCount` +1, and leave the created POINT on screen.
//! 7. **RECTANG end to end** (same funnel): `entityCount` delta +1.
//! 8. **ARRAYRECT**: click-select one entity, set `arrayRows`/`arrayColumns`
//!    through the real IPC, start `ARRAYRECT`, `CONFIRM`, read `entityCount`
//!    **+5** (`rows × columns − 1` copies; the source is retained).
//! 9. **ARRAYPOLAR**: click-select one entity, set `arrayItems`, start
//!    `ARRAYPOLAR`, click the centre, `CONFIRM`, read `entityCount` **+3**
//!    (`items − 1` copies).
//! 10. **ALIGN**: draw a LINE, click-select it, start `ALIGN`, capture the four
//!     points (source pair, destination pair) with `alignScale` left at its
//!     default `false`, `CONFIRM`. This is an **in-place** transform, so
//!     `entityCount` is **unchanged**; the honest observables are "accepted, no
//!     error, count unchanged" plus the screenshot.
//! 11. **STRETCH**: draw a LINE, start `STRETCH`, capture a crossing window that
//!     encloses **one** endpoint, then the base and displacement points,
//!     `CONFIRM`. Also in-place: `entityCount` unchanged.
//! 12. **EXPLODE**: draw a closed polyline (`RECTANG`), click-select it, then
//!     dispatch the payload-free `EXPLODE` selector: the polyline is deleted and
//!     four LINEs are inserted ⇒ `entityCount` **+3**.
//! 13. **DIMLINEAR** (P5c, ADR 0017 decision C): start the real `DIMLINEAR`
//!     selector, feed three synthetic picks (two extension-line origins, then the
//!     dimension-line location), `CONFIRM`, and read `entityCount` **+1** and
//!     `tool` `null`.
//! 14. **DIMALIGNED** (P5c): the same three-pick funnel ⇒ `entityCount` **+1**.
//! 15. **DIMRADIUS** (P5d): create a `CIRCLE`, start the real `DIMRADIUS`
//!     selector, click the circle's **edge** (the leading entity pick), click the
//!     dimension-line location, `CONFIRM`, and read `entityCount` **+1** (the new
//!     `AcDbDimension`) and `tool` `null`.
//! 16. **DIMDIAMETER** (P5d): the same pick-then-location funnel ⇒ `entityCount`
//!     **+1**.
//! 17. **DIMCENTER** (P5d): create a `CIRCLE`, start the real `DIMCENTER`
//!     selector, click the circle's edge (its single pick), `CONFIRM`, and read
//!     the **real** `entityCount` delta. Its output is plain line geometry, not a
//!     `DIMENSION`; the committed line count follows the drawing's active
//!     `Standard` `DIMCEN`, which `cad_status` does **not** expose, so the report
//!     states the real delta and never asserts a count it cannot read.
//! 18. **DIMANGULAR** (P5e, two-line form): draw two non-parallel `LINE`s that
//!     share a vertex (`+2`), start the real `DIMANGULAR` selector, click **each
//!     line's body** (two entity picks), click the dimension-arc location inside
//!     the intended 0°..45° sector, `CONFIRM`, and read `entityCount` **+1** and
//!     `tool` `null`.
//! 19. **DIMORDINATE** (P5e): feature point, leader endpoint, `CONFIRM` ⇒
//!     `entityCount` **+1**.
//! 20. **HATCH** (P6b, pre-selection): draw two concentric `CIRCLE`s, window-box
//!     select both, `HATCH` (solid, `Normal`), `CONFIRM` ⇒ `entityCount` **+1**;
//!     then a `RECTANG` with a user-defined pattern (spacing 0.05, angle 30) ⇒
//!     **+1**; then the same two circles again with style `2` (`Ignore`) ⇒
//!     **+1**. Each phase reads the real `cad_status` before/after.
//!
//! The P4 edit-command phases (8–12) live in [`super::draw_edit`], the P5c
//! dimension-creation phases (13–14) in [`super::draw_dimension`], the P5d
//! radial-dimension phases (15–17) in [`super::draw_dimension_radial`], the P5e
//! angular/ordinate phases (18–19) in
//! [`super::draw_dimension_angular_ordinate`] and the P6b hatch phases (20) in
//! [`super::draw_hatch`] (this file was split at the 1000-line code-file gate).
//!
//! Synthetic input, **not a real mouse**: the events are dispatched on
//! `.native-input` and never touch the kernel/evdev/XInput path. Every reported
//! number is a real `cad_status` read. `cad_status` exposes **no** per-entity
//! geometry and **no** vertex count, so neither the polygon's real 6-sidedness
//! nor the FILLET/CHAMFER geometry is observable through the status channel; the
//! report says so plainly (`vertex_count_channel=none_...`) instead of claiming a
//! vertex/geometry count it cannot read. The one honest change observable is
//! `entityCount` (plus the surfaced technical `error`), reported verbatim.
//!
//! Fixture note (verified against `fixtures/dwg/synthetic-four-lines.dwg`
//! through the importer contract): the drawing is four mutually parallel
//! vertical LINEs at world x = 0, 1, 2, 3 spanning y = 0..1. The FILLET/CHAMFER
//! phases therefore draw their own crossing pairs first, **before** the
//! PLINE/POLYGON/POINT/RECTANG phases, so the picks resolve on an otherwise empty
//! canvas. Every synthetic click targets a world point inside the visible extent
//! (camera centre (1.5, 0.5), ~0.005032 world/logical-pixel), converted to
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
         phases: LINE pair + FILLET(radius, deliberate miss) + LINE pair + CHAMFER(a/b) + PLINE(3pt+CONFIRM) \
         POLYGON(sides=6, out-of-range=2) POINT(visible) RECTANG + P4 edit commands \
         ARRAYRECT(rows=2,cols=3) ARRAYPOLAR(items=4) ALIGN(4pt) STRETCH(4pt) EXPLODE \
         + P5c dimensions DIMLINEAR(3pt) DIMALIGNED(3pt) \
         + P5d radial dimensions DIMRADIUS(circle+pick+loc) DIMDIAMETER(circle+pick+loc) DIMCENTER(circle+pick) \
         + P5e dimensions DIMANGULAR(2 lines+2 picks+arc loc) DIMORDINATE(feature+leader) \
         + P6b hatch: 2 pan drags, HATCH(solid annulus), HATCH(user-defined pattern), \
         HATCH(style 2 Ignore annulus)"
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
/// * **FILLET geometry** (pair A): t=12s start `LINE`; t=13/14s click the two
///   endpoints of the horizontal; t=15s `CONFIRM`; t=16s read; t=17s start
///   `LINE`; t=18/19s click the two endpoints of the vertical; t=20s `CONFIRM`;
///   t=21s read (`entityCount` +2);
/// * **FILLET**: t=22s `cad_set_tool_parameter filletRadius=0.12`; t=23s start
///   `FILLET`; t=24s read (`tool.params.filletRadius` visible); t=25s click an
///   **empty** point (deliberate miss); t=26s read (refusal in `error`, points
///   unchanged); t=27s pick the horizontal; t=28s read (points = 1); t=29s pick
///   the vertical; t=30s read (points = 2, ready); t=31s `CONFIRM`; t=32s read
///   (`entityCount` +1, `tool` `null`);
/// * **CHAMFER geometry** (pair B): t=33s start `LINE`; t=34/35s click the
///   horizontal endpoints; t=36s `CONFIRM`; t=37s read; t=38s start `LINE`;
///   t=39/40s click the vertical endpoints; t=41s `CONFIRM`; t=42s read
///   (`entityCount` +2);
/// * **CHAMFER**: t=43s set `chamferDistanceA=0.05`; t=44s set
///   `chamferDistanceB=0.05`; t=45s start `CHAMFER`; t=46s read (a/b visible);
///   t=47s pick the horizontal; t=48s read (points = 1); t=49s pick the vertical;
///   t=50s read (points = 2, ready); t=51s `CONFIRM`; t=52s read (`entityCount`
///   +1, `tool` `null`). The FILLET ARC and the CHAMFER LINE are now on screen;
///   the PLINE/POLYGON/POINT/RECTANG phases follow in the t=68..98 span;
/// * **PLINE**: t=68s start `PLINE`; t=69/70/71s click three world points; t=72s
///   read (`points` = 3); t=73s `CONFIRM`; t=74s read (`entityCount` +1);
/// * **POLYGON**: t=76s `cad_set_tool_parameter polygonSides=6`; t=77s start
///   `POLYGON`; t=78s read (`tool.params.sides` = 6); t=79s
///   `cad_set_tool_parameter polygonSides=2` (out of range); t=80s read (refusal
///   in `error`, `sides` still 6); t=81/82s click centre + radius; t=83s read
///   (`points` = 2); t=84s `CONFIRM`; t=85s read (`entityCount` +1, `tool`
///   `null`);
/// * **POINT**: t=87s start `POINT`; t=88s click one world point; t=89s read
///   (`points` = 1); t=90s `CONFIRM`; t=91s read (`entityCount` +1);
/// * **RECTANG**: t=93s start `RECTANG`; t=94/95s click two corners; t=96s read
///   (`points` = 2); t=97s `CONFIRM`; t=98s read (`entityCount` +1);
/// * then the **P4 edit-command phases** (ARRAYRECT / ARRAYPOLAR / ALIGN /
///   STRETCH / EXPLODE) run on a sub-second step (t=100s..135.5s), the **P5c
///   dimension-creation phases** (DIMLINEAR / DIMALIGNED, [`super::draw_dimension`])
///   at t=137s..144.5s, the **P5d radial-dimension phases** (DIMRADIUS /
///   DIMDIAMETER / DIMCENTER, [`super::draw_dimension_radial`]) at
///   t=145s..163s, the **P5e angular/ordinate phases** (DIMANGULAR /
///   DIMORDINATE, [`super::draw_dimension_angular_ordinate`]) at
///   t=163.5s..177.5s, the **P6b hatch phases** (HATCH, [`super::draw_hatch`]) at
///   t=178s..204.5s, and the combined single-line report lands at t=205s — the
///   P4 phases and report in [`super::draw_edit`], which documents the per-step
///   timeline. An external `--capture-at` should land **after** every hatch phase
///   commits and the cursor is parked (≈ driver t=204s, i.e. a launch delay of
///   ≈ 212s) so a reviewer sees the arrays, the EXPLODE result, the P5c
///   dimensions, the P5d circles + radial dimensions, the P5e angular/ordinate
///   dimensions **and** the three hatch frames on screen.
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

    // FILLET geometry (pair A): a horizontal and a vertical LINE that cross at
    // (1.5, 0.6). Drawn first, on the otherwise empty canvas.
    schedule_draw(
        &start,
        12,
        "cad_command LINE (fillet pair A horizontal)",
        || webview.eval(draw_start_js("LINE")),
    );
    let a_h1 = draw_click_world_js(0.4, 0.6, "a_h1");
    schedule_draw(&start, 13, "click a_h1 at world (0.4, 0.6)", || {
        webview.eval(a_h1)
    });
    let a_h2 = draw_click_world_js(2.6, 0.6, "a_h2");
    schedule_draw(&start, 14, "click a_h2 at world (2.6, 0.6)", || {
        webview.eval(a_h2)
    });
    schedule_draw(&start, 15, "cad_command CONFIRM (a_h)", || {
        webview.eval(draw_confirm_js("confirm_a_h"))
    });
    schedule_draw(&start, 16, "a_h-after cad_status", || {
        webview.eval(draw_status_js("s_line_a_h_after"))
    });
    schedule_draw(
        &start,
        17,
        "cad_command LINE (fillet pair A vertical)",
        || webview.eval(draw_start_js("LINE")),
    );
    let a_v1 = draw_click_world_js(1.5, 0.2, "a_v1");
    schedule_draw(&start, 18, "click a_v1 at world (1.5, 0.2)", || {
        webview.eval(a_v1)
    });
    let a_v2 = draw_click_world_js(1.5, 0.8, "a_v2");
    schedule_draw(&start, 19, "click a_v2 at world (1.5, 0.8)", || {
        webview.eval(a_v2)
    });
    schedule_draw(&start, 20, "cad_command CONFIRM (a_v)", || {
        webview.eval(draw_confirm_js("confirm_a_v"))
    });
    schedule_draw(&start, 21, "a_v-after cad_status (entityCount +2)", || {
        webview.eval(draw_status_js("s_line_a_v_after"))
    });

    // FILLET: set the radius, prove it is visible, prove an empty-space pick is
    // refused without advancing, then pick the two lines and commit.
    schedule_draw(
        &start,
        22,
        "cad_set_tool_parameter filletRadius=0.12",
        || {
            webview.eval(draw_set_parameter_number_js(
                "filletRadius",
                0.12,
                "fillet_radius",
            ))
        },
    );
    schedule_draw(&start, 23, "cad_command FILLET", || {
        webview.eval(draw_start_js("FILLET"))
    });
    schedule_draw(
        &start,
        24,
        "fillet-active cad_status (radius must read 0.12)",
        || webview.eval(draw_status_js("s_fillet_active")),
    );
    let fillet_miss = draw_click_world_js(2.3, 0.35, "fillet_miss");
    schedule_draw(
        &start,
        25,
        "fillet deliberate miss click at empty world (2.3, 0.35)",
        || webview.eval(fillet_miss),
    );
    schedule_draw(
        &start,
        26,
        "fillet-miss cad_status (refusal + capture must not advance)",
        || webview.eval(draw_status_js("s_fillet_miss")),
    );
    let fillet_pick1 = draw_click_world_js(2.3, 0.6, "fillet_pick1");
    schedule_draw(
        &start,
        27,
        "fillet pick 1 at world (2.3, 0.6) (pair A horizontal)",
        || webview.eval(fillet_pick1),
    );
    schedule_draw(&start, 28, "fillet-pick1 cad_status", || {
        webview.eval(draw_status_js("s_fillet_pick1"))
    });
    let fillet_pick2 = draw_click_world_js(1.5, 0.35, "fillet_pick2");
    schedule_draw(
        &start,
        29,
        "fillet pick 2 at world (1.5, 0.35) (pair A vertical)",
        || webview.eval(fillet_pick2),
    );
    schedule_draw(&start, 30, "fillet-pick2 cad_status", || {
        webview.eval(draw_status_js("s_fillet_pick2"))
    });
    schedule_draw(&start, 31, "cad_command CONFIRM (fillet)", || {
        webview.eval(draw_confirm_js("confirm_fillet"))
    });
    schedule_draw(
        &start,
        32,
        "fillet-after cad_status (entityCount +1)",
        || webview.eval(draw_status_js("s_fillet_after")),
    );

    // CHAMFER geometry (pair B): a second crossing pair, because FILLET trimmed
    // the first. Crosses at (2.3, 0.35).
    schedule_draw(
        &start,
        33,
        "cad_command LINE (chamfer pair B horizontal)",
        || webview.eval(draw_start_js("LINE")),
    );
    let b_h1 = draw_click_world_js(0.6, 0.35, "b_h1");
    schedule_draw(&start, 34, "click b_h1 at world (0.6, 0.35)", || {
        webview.eval(b_h1)
    });
    let b_h2 = draw_click_world_js(2.8, 0.35, "b_h2");
    schedule_draw(&start, 35, "click b_h2 at world (2.8, 0.35)", || {
        webview.eval(b_h2)
    });
    schedule_draw(&start, 36, "cad_command CONFIRM (b_h)", || {
        webview.eval(draw_confirm_js("confirm_b_h"))
    });
    schedule_draw(&start, 37, "b_h-after cad_status", || {
        webview.eval(draw_status_js("s_line_b_h_after"))
    });
    schedule_draw(
        &start,
        38,
        "cad_command LINE (chamfer pair B vertical)",
        || webview.eval(draw_start_js("LINE")),
    );
    let b_v1 = draw_click_world_js(2.3, 0.2, "b_v1");
    schedule_draw(&start, 39, "click b_v1 at world (2.3, 0.2)", || {
        webview.eval(b_v1)
    });
    let b_v2 = draw_click_world_js(2.3, 0.8, "b_v2");
    schedule_draw(&start, 40, "click b_v2 at world (2.3, 0.8)", || {
        webview.eval(b_v2)
    });
    schedule_draw(&start, 41, "cad_command CONFIRM (b_v)", || {
        webview.eval(draw_confirm_js("confirm_b_v"))
    });
    schedule_draw(&start, 42, "b_v-after cad_status (entityCount +2)", || {
        webview.eval(draw_status_js("s_line_b_v_after"))
    });

    // CHAMFER: set the two distances, prove they are visible, pick the two lines
    // and commit.
    schedule_draw(
        &start,
        43,
        "cad_set_tool_parameter chamferDistanceA=0.05",
        || {
            webview.eval(draw_set_parameter_number_js(
                "chamferDistanceA",
                0.05,
                "chamfer_a",
            ))
        },
    );
    schedule_draw(
        &start,
        44,
        "cad_set_tool_parameter chamferDistanceB=0.05",
        || {
            webview.eval(draw_set_parameter_number_js(
                "chamferDistanceB",
                0.05,
                "chamfer_b",
            ))
        },
    );
    schedule_draw(&start, 45, "cad_command CHAMFER", || {
        webview.eval(draw_start_js("CHAMFER"))
    });
    schedule_draw(
        &start,
        46,
        "chamfer-active cad_status (a/b must read 0.05)",
        || webview.eval(draw_status_js("s_chamfer_active")),
    );
    let chamfer_pick1 = draw_click_world_js(2.5, 0.35, "chamfer_pick1");
    schedule_draw(
        &start,
        47,
        "chamfer pick 1 at world (2.5, 0.35) (pair B horizontal)",
        || webview.eval(chamfer_pick1),
    );
    schedule_draw(&start, 48, "chamfer-pick1 cad_status", || {
        webview.eval(draw_status_js("s_chamfer_pick1"))
    });
    let chamfer_pick2 = draw_click_world_js(2.3, 0.28, "chamfer_pick2");
    schedule_draw(
        &start,
        49,
        "chamfer pick 2 at world (2.3, 0.28) (pair B vertical)",
        || webview.eval(chamfer_pick2),
    );
    schedule_draw(&start, 50, "chamfer-pick2 cad_status", || {
        webview.eval(draw_status_js("s_chamfer_pick2"))
    });
    schedule_draw(&start, 51, "cad_command CONFIRM (chamfer)", || {
        webview.eval(draw_confirm_js("confirm_chamfer"))
    });
    schedule_draw(
        &start,
        52,
        "chamfer-after cad_status (entityCount +1)",
        || webview.eval(draw_status_js("s_chamfer_after")),
    );
    // The FILLET ARC and CHAMFER LINE are now on screen; the PLINE/POLYGON/
    // POINT/RECTANG phases follow, then the P4 edit-command phases in
    // `super::draw_edit`. The recommended external `--capture-at` lands after
    // those P4 phases so the arrays and the EXPLODE result are on screen.

    // PLINE: three points + CONFIRM.
    schedule_draw(&start, 68, "cad_command PLINE", || {
        webview.eval(draw_start_js("PLINE"))
    });
    let pline_a = draw_click_world_js(0.4, 0.3, "pline_a");
    schedule_draw(&start, 69, "pline click A at world (0.4, 0.3)", || {
        webview.eval(pline_a)
    });
    let pline_b = draw_click_world_js(1.5, 0.8, "pline_b");
    schedule_draw(&start, 70, "pline click B at world (1.5, 0.8)", || {
        webview.eval(pline_b)
    });
    let pline_c = draw_click_world_js(2.6, 0.3, "pline_c");
    schedule_draw(&start, 71, "pline click C at world (2.6, 0.3)", || {
        webview.eval(pline_c)
    });
    schedule_draw(&start, 72, "pline-points cad_status", || {
        webview.eval(draw_status_js("s_pline_points"))
    });
    schedule_draw(&start, 73, "cad_command CONFIRM (pline)", || {
        webview.eval(draw_confirm_js("confirm_pline"))
    });
    schedule_draw(&start, 74, "pline-after cad_status", || {
        webview.eval(draw_status_js("s_pline_after"))
    });

    // POLYGON: set a non-default side count through the general IPC, prove it is
    // visible, prove an out-of-range value is refused without changing it, then
    // commit.
    schedule_draw(&start, 76, "cad_set_tool_parameter polygonSides=6", || {
        webview.eval(draw_set_parameter_number_js("polygonSides", 6.0, "sides_6"))
    });
    schedule_draw(&start, 77, "cad_command POLYGON", || {
        webview.eval(draw_start_js("POLYGON"))
    });
    schedule_draw(
        &start,
        78,
        "polygon-active cad_status (sides must read 6)",
        || webview.eval(draw_status_js("s_polygon_active")),
    );
    schedule_draw(
        &start,
        79,
        "cad_set_tool_parameter polygonSides=2 (out of range)",
        || webview.eval(draw_set_parameter_number_js("polygonSides", 2.0, "sides_2")),
    );
    schedule_draw(
        &start,
        80,
        "polygon-bad-sides cad_status (refusal + unchanged value)",
        || webview.eval(draw_status_js("s_polygon_bad")),
    );
    let poly_c = draw_click_world_js(1.5, 0.4, "polygon_center");
    schedule_draw(
        &start,
        81,
        "polygon centre click at world (1.5, 0.4)",
        || webview.eval(poly_c),
    );
    let poly_r = draw_click_world_js(2.1, 0.4, "polygon_radius");
    schedule_draw(
        &start,
        82,
        "polygon radius click at world (2.1, 0.4)",
        || webview.eval(poly_r),
    );
    schedule_draw(&start, 83, "polygon-points cad_status", || {
        webview.eval(draw_status_js("s_polygon_points"))
    });
    schedule_draw(&start, 84, "cad_command CONFIRM (polygon)", || {
        webview.eval(draw_confirm_js("confirm_polygon"))
    });
    schedule_draw(&start, 85, "polygon-after cad_status", || {
        webview.eval(draw_status_js("s_polygon_after"))
    });

    // POINT: one point + CONFIRM.
    schedule_draw(&start, 87, "cad_command POINT", || {
        webview.eval(draw_start_js("POINT"))
    });
    let point_a = draw_click_world_js(2.4, 0.7, "point");
    schedule_draw(&start, 88, "point click at world (2.4, 0.7)", || {
        webview.eval(point_a)
    });
    schedule_draw(&start, 89, "point-points cad_status", || {
        webview.eval(draw_status_js("s_point_points"))
    });
    schedule_draw(&start, 90, "cad_command CONFIRM (point)", || {
        webview.eval(draw_confirm_js("confirm_point"))
    });
    schedule_draw(&start, 91, "point-after cad_status", || {
        webview.eval(draw_status_js("s_point_after"))
    });

    // RECTANG: same funnel, one more `entityCount` delta.
    schedule_draw(&start, 93, "cad_command RECTANG", || {
        webview.eval(draw_start_js("RECTANG"))
    });
    let rect_a = draw_click_world_js(0.3, 0.2, "rectang_a");
    schedule_draw(&start, 94, "rectang click A at world (0.3, 0.2)", || {
        webview.eval(rect_a)
    });
    let rect_b = draw_click_world_js(1.1, 0.7, "rectang_b");
    schedule_draw(&start, 95, "rectang click B at world (1.1, 0.7)", || {
        webview.eval(rect_b)
    });
    schedule_draw(&start, 96, "rectang-points cad_status", || {
        webview.eval(draw_status_js("s_rectang_points"))
    });
    schedule_draw(&start, 97, "cad_command CONFIRM (rectang)", || {
        webview.eval(draw_confirm_js("confirm_rectang"))
    });
    schedule_draw(&start, 98, "rectang-after cad_status", || {
        webview.eval(draw_status_js("s_rectang_after"))
    });

    // P4 edit-command phases (ARRAYRECT / ARRAYPOLAR / ALIGN / STRETCH / EXPLODE)
    // run after the drawing phases above, then the P5c dimension-creation phases
    // (DIMLINEAR / DIMALIGNED) and the P5d radial-dimension phases (DIMRADIUS /
    // DIMDIAMETER / DIMCENTER), followed by the single-line report. They are
    // sub-second-stepped (`schedule_draw_ms`) so the whole pass finishes inside
    // the bounded evidence timeout; see [`super::draw_edit`],
    // [`super::draw_dimension`] and [`super::draw_dimension_radial`].
    super::draw_edit::schedule_draw_edit_phases(&webview, &start);
    super::draw_dimension::schedule_dimension_phases(&webview, &start);
    super::draw_dimension_radial::schedule_radial_dimension_phases(&webview, &start);
    super::draw_dimension_angular_ordinate::schedule_angular_ordinate_phases(&webview, &start);
    super::draw_hatch::schedule_hatch_phases(&webview, &start);
    super::draw_edit::schedule_draw_report(&webview, &start);
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

/// Millisecond-resolution variant of [`schedule_draw`], shared with
/// [`super::draw_edit`].
///
/// The P4 edit-command phases use a sub-second step so the whole pass fits inside
/// the bounded evidence timeout; the dispatch semantics (sleep to the target,
/// run, log under the draw prefix) are identical to [`schedule_draw`]. Only the
/// **dispatch** is reported here; the observable outcome comes from the
/// `cad_status` reads.
pub(super) fn schedule_draw_ms(
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
        Ok(()) => eprintln!("[probe] draw evidence step: {label} -> dispatched"),
        Err(error) => eprintln!("[probe] draw evidence step: {label} -> dispatch error: {error}"),
    }
}

/// Build the named status-read script (`s_base`, `s_fillet_miss`, ...).
pub(super) fn draw_status_js(name: &str) -> String {
    DRAW_STATUS_TEMPLATE.replace("__NAME__", name)
}

/// Build the start script for one draw selector (`PLINE` / `POLYGON` / ...).
pub(super) fn draw_start_js(tool: &str) -> String {
    DRAW_START_TEMPLATE.replace("__TOOL__", tool)
}

/// Build a labeled `cad_command` dispatch for any selector (`LINE` / `ARRAYRECT`
/// / `EXPLODE` / ...), recording the ack under `__LABEL__`.
///
/// Unlike [`draw_start_js`] this carries a per-command label, so the P4 edit
/// phases report one ack per selector (including the payload-free `EXPLODE`)
/// instead of overwriting a single shared `start_ack`.
pub(super) fn draw_command_js(command: &str, label: &str) -> String {
    DRAW_COMMAND_TEMPLATE
        .replace("__COMMAND__", command)
        .replace("__LABEL__", label)
}

/// Build the `cad_set_tool_parameter` script for one numeric parameter value.
///
/// `__PARAM__` is the parameter's stable machine key (e.g. `polygonSides`),
/// `__NUMBER__` the numeric value and `__LABEL__` the report slot name.
pub(super) fn draw_set_parameter_number_js(parameter: &str, value: f64, label: &str) -> String {
    DRAW_SET_PARAMETER_TEMPLATE
        .replace("__PARAM__", parameter)
        .replace("__NUMBER__", &value.to_string())
        .replace("__LABEL__", label)
}

/// Build one synthetic click at a **world** point: the script reads the real
/// `cad_status.camera` and the live `.native-input` rect and converts the world
/// point to rect fractions before dispatching the pointer events. Synthetic,
/// **not a real mouse**.
pub(super) fn draw_click_world_js(wx: f64, wy: f64, label: &str) -> String {
    DRAW_CLICK_TEMPLATE
        .replace("__WX__", &wx.to_string())
        .replace("__WY__", &wy.to_string())
        .replace("__LABEL__", label)
}

/// Build one **click-free** synthetic pointer move at a **world** point.
///
/// Same world→rect-fraction conversion as [`draw_click_world_js`], but it
/// dispatches only `pointermove` (button `-1`, buttons `0`): no press, no release
/// and no `d.clicks` increment. Used by the P5e phases to park the cursor so the
/// renderer's viewport-spanning crosshair does not run through the regions the
/// presence check measures. Synthetic, **not a real mouse**.
pub(super) fn draw_move_world_js(wx: f64, wy: f64, label: &str) -> String {
    DRAW_MOVE_TEMPLATE
        .replace("__WX__", &wx.to_string())
        .replace("__WY__", &wy.to_string())
        .replace("__LABEL__", label)
}

/// Read the real `cad_status` and record it under `__NAME__`.
///
/// `points` is the tool's captured-point count, `ready` its confirmability,
/// `sides` the POLYGON `tool.params.sides`, `fillet`/`chamfer_a`/`chamfer_b` the
/// FILLET/CHAMFER typed parameters (or `null`), and `error` the real technical
/// error string (the surfaced refusal, if any).
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
      var params = (s.tool && s.tool.params) ? s.tool.params : null;
      d[name + '_sides'] = (params && typeof params.sides !== 'undefined') ? params.sides : null;
      d[name + '_fillet'] = (params && typeof params.filletRadius !== 'undefined') ? params.filletRadius : null;
      d[name + '_chamfer_a'] = (params && typeof params.chamferDistanceA !== 'undefined') ? params.chamferDistanceA : null;
      d[name + '_chamfer_b'] = (params && typeof params.chamferDistanceB !== 'undefined') ? params.chamferDistanceB : null;
      d[name + '_array_rows'] = (params && typeof params.arrayRows !== 'undefined') ? params.arrayRows : null;
      d[name + '_array_columns'] = (params && typeof params.arrayColumns !== 'undefined') ? params.arrayColumns : null;
      d[name + '_array_items'] = (params && typeof params.arrayItems !== 'undefined') ? params.arrayItems : null;
      d[name + '_align_scale'] = (params && typeof params.alignScale !== 'undefined') ? params.alignScale : null;
      d[name + '_selection'] = s.selection ? s.selection.count : null;
      d[name + '_error'] = s.error;
      d[name + '_hatch_pattern_type'] = (params && typeof params.hatchPatternType !== 'undefined') ? params.hatchPatternType : null;
      d[name + '_hatch_spacing'] = (params && typeof params.hatchSpacing !== 'undefined') ? params.hatchSpacing : null;
      d[name + '_hatch_angle'] = (params && typeof params.hatchAngle !== 'undefined') ? params.hatchAngle : null;
      d[name + '_hatch_style'] = (params && typeof params.hatchStyle !== 'undefined') ? params.hatchStyle : null;
      if (s.camera && s.camera.center && s.camera.center.length === 2) {
        d.camera = [s.camera.center[0], s.camera.center[1], s.camera.scale];
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

/// Dispatch any real selector through the public `cad_command` command,
/// recording the ack under `__LABEL__`.
///
/// `__COMMAND__` is the canonical selector (`LINE` / `ARRAYRECT` / `ARRAYPOLAR` /
/// `ALIGN` / `STRETCH` / `RECTANG` / `EXPLODE`). This is [`DRAW_START_TEMPLATE`]
/// with a per-command label so the P4 edit phases never conflate two acks; the
/// payload-free `EXPLODE` selector rides the exact same funnel.
const DRAW_COMMAND_TEMPLATE: &str = r#"
(function () {
  var d = window.__yacr_draw = window.__yacr_draw || {};
  var label = '__LABEL__';
  try {
    var internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {
      d[label + '_failed'] = 'no-tauri-internals';
      return 'no-tauri-internals';
    }
    internals.invoke('cad_command', { command: '__COMMAND__' }).then(function (ack) {
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

/// Call the real `cad_set_tool_parameter` IPC with one numeric value.
///
/// `__PARAM__` is the parameter's stable machine key (`polygonSides`,
/// `filletRadius`, `chamferDistanceA`/`B`), `__NUMBER__` the numeric value,
/// `__LABEL__` the report slot name (e.g. `sides_6`). This deliberately bypasses
/// the frontend range guard and reaches the host: the host admits it into the
/// bounded queue and the owner tick applies `CadSession::set_tool_parameter`,
/// whose authoritative per-parameter check refuses an out-of-range value
/// explicitly (never a silent clamp). `{accepted:true}` is queue admission only;
/// the real outcome surfaces through `cad_status.error`.
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

/// Build the labeled confirm script for one commit (`confirm_fillet`, ...).
///
/// Drawing tools never auto-commit, so the captured points alone leave
/// `entityCount` unchanged; `CONFIRM` (Enter) is what commits the entity. Each
/// commit carries its own label so the report never conflates two confirms.
/// Synthetic input, **not a real mouse**.
pub(super) fn draw_confirm_js(label: &str) -> String {
    DRAW_CONFIRM_TEMPLATE.replace("__LABEL__", label)
}

/// Confirm the started draw tool through the public `cad_command` command.
///
/// `__LABEL__` names the commit so the ack is recorded per phase.
const DRAW_CONFIRM_TEMPLATE: &str = r#"
(function () {
  var d = window.__yacr_draw = window.__yacr_draw || {};
  var label = '__LABEL__';
  try {
    var internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {
      d.confirm_failed = 'no-tauri-internals';
      return 'no-tauri-internals';
    }
    internals.invoke('cad_command', { command: 'CONFIRM' }).then(function (ack) {
      d[label + '_ack'] = (ack && typeof ack.accepted !== 'undefined')
        ? 'accepted=' + ack.accepted
        : String(JSON.stringify(ack));
    }).catch(function (err) { d[label + '_failed'] = String(err); });
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

/// One **click-free** synthetic pointer move at a **world** point.
///
/// Same camera/rect conversion and the same recorded diagnostics as
/// [`DRAW_CLICK_TEMPLATE`] (`__LABEL___world`, `__LABEL___frac`, `__LABEL___px`),
/// but only `pointermove` is dispatched and `d.clicks` is left untouched: no
/// press, no release, no selection change. `__WX__`/`__WY__` are world
/// coordinates; `__LABEL__` names the move. Synthetic, **not a real mouse**.
const DRAW_MOVE_TEMPLATE: &str = r#"
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
      var x = r.left + r.width * fx;
      var y = r.top + r.height * fy;
      el.dispatchEvent(new PointerEvent('pointermove', {
        pointerId: 1, pointerType: 'mouse', isPrimary: true,
        button: -1, buttons: 0,
        clientX: x, clientY: y, bubbles: true, cancelable: true
      }));
      d[label + '_px'] = [Math.round(x - r.left), Math.round(y - r.top)];
      d[label + '_dispatched'] = true;
    }).catch(function (err) { d[label + '_failed'] = String(err); });
    return label + '-moved';
  } catch (err) {
    try { d[label + '_failed'] = 'fatal:' + err; } catch (_) {}
    return 'error:' + err;
  }
})()
"#;
