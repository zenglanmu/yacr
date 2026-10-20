//! `YACR_PROBE_DRAW_EVIDENCE` P5f baseline/continue dimension phases
//! (`DIMBASELINE` / `DIMCONTINUE`).
//!
//! Split out as a sibling module for the same reason as
//! [`super::draw_mleader`]: every evidence file stays well under the 1000-line
//! code-file gate (`scripts/check-file-size.py`, no whitelist). This is the same
//! evidence-only, default-OFF, env-gated driver: it dispatches the **real**
//! `DIMLINEAR`/`DIMBASELINE`/`DIMCONTINUE` selectors through the normal
//! `cad_command` funnel, feeds every pick as synthetic DOM pointer events on
//! `.native-input`, and reads the **real** `cad_status` before/after each commit.
//! Every reported number is a real `cad_status` read; nothing here is synthesized.
//!
//! ## What the phase drives (ADR 0017 decision C, P5f)
//!
//! 1. Create the **linear base dimension** the chain commands need. `DIMLINEAR`
//!    is the same command the P5c phase ([`super::draw_dimension`]) drives — two
//!    extension-line origins, then the dimension-line location, `CONFIRM`. Its
//!    three click positions are recorded so the base pick lands exactly on the
//!    dimension line the synthesis draws (`q1..q2` at the location's own Y for a
//!    horizontal dimension: the line is emitted unconditionally by
//!    `cad_domain::linear_synthesis`).
//! 2. **DIMBASELINE**: start the real selector, click the base dimension's
//!    dimension line (the leading entity pick), assert the pick advanced the
//!    capture (`points` 0 → 1, tool still `draw.dimbaseline`, no `error`), click
//!    **three measured points**, `CONFIRM`, and read `entityCount` **+3** (one
//!    row per measured point, all in one transaction) with `tool` `null` and no
//!    `error`.
//! 3. **DIMCONTINUE**: the same shape against the **same** base dimension —
//!    start, base pick (assert the capture advanced), **three chained points**
//!    that advance along the base's direction, `CONFIRM`, read `entityCount`
//!    **+3**.
//! 4. Click-select the created ink and log the real `selection.rows` the
//!    single-selection projection returns. Only the read itself is gated: the
//!    selected **identity** (`count`/`rows`) is reported verbatim and is not part
//!    of `bc_pass`, because several created dimensions share that band and which
//!    one `pick_closest` returns is not what this phase claims.
//!
//! ## Why the picks are logical pixels, not world points
//!
//! The P5c/P5d/P5e phases target **world** points because they run before the
//! view moves. This phase runs after the P6b hatch phase, which pans the camera
//! twice (see [`super::draw_hatch`]); the camera at this instant is run data, not
//! something this file can author against. The three base clicks, the base pick
//! and the six measured points are therefore given in **nominal logical pixels**
//! and scaled onto the live `.native-input` rect (the P6b/P7c template), and the
//! summary prints the real camera plus every `*_px`/`*_frac` each event hit.
//! The chain geometry itself is derived by the command from the **created**
//! base's own definition points (rotation, origins, dimension line), so this
//! driver never has to know their world values:
//!
//! * the base origins are clicked on one screen row and the location pick is
//!   offset only vertically, so `DIMLINEAR` resolves `rotation = 0` (measures X);
//! * `DIMBASELINE` stacks row `i` at `base.location + n̂ · (i · DIMDLI)` with `n̂`
//!   the base normal pointing away from that row's measured geometry;
//! * `DIMCONTINUE` chains from `base.second` along the base direction, so the
//!   chained points are clicked to the **right** of the base's second origin and
//!   strictly increasing in X (a non-advancing segment is a named refusal).
//!
//! All clicks land in the **bottom band** of the panned frame (below the
//! full-width strokes the P7c run measured at y ≈ 269.5/291.5), which carries no
//! fixture or earlier-phase ink in either the panned or the un-panned view.
//!
//! ## Honest limits
//!
//! Synthetic input, **not a real mouse**: the events are DOM `PointerEvent`
//! dispatches from Rust through `webview.eval`, never the kernel/evdev/XInput
//! path. `cad_status` has **no per-entity geometry channel** — it exposes no
//! dimension vertices, no dimension-row list and no `DIMDLI` — so this driver
//! proves
//! "base pick captured, N measured points captured, one transaction added N
//! entities, tool cleared, no error". The row/segment geometry (and every named
//! refusal) is pinned by the `cad-app` crate contract tests
//! (`tests/drawing/dimension_chain.rs`), not by this channel.
//!
//! The stacked rows are also **expected to leave the frame**: with the pinned
//! `Standard` row's `DIMDLI` (3.75 world, ≈745 logical px at the ≈0.005032
//! world/px the earlier phases log) the first row already sits ≈745 px from the
//! base's dimension line inside a 341-px canvas. The driver therefore asserts the
//! real count and says so in the summary; it does **not** claim the stacked rows
//! are visible. If the fixture carries a different `Standard` row the spacing
//! changes, which is exactly why the number is not asserted here.
//!
//! ## Failure behaviour
//!
//! A step that fails its expectation sets `window.__yacr_draw.baseline_continue_abort`
//! and logs
//! `[probe] draw evidence baseline_continue abort: <step> expected=<…> got=<…>`
//! immediately. Every later **state-changing** dispatch (command, click) checks
//! that flag and **skips** instead of pretending the step ran; only the
//! read-only `cad_status` reads continue, so the summary can still report the
//! real final state. The summary reports `bc_aborted_at=` and `bc_pass=0`, so a
//! run that stopped early cannot read as a pass.

use std::time::Instant;

use super::draw::schedule_draw_ms;
use super::draw_baseline_continue_js::{
    bc_click_px_js, bc_command_js, bc_expect_js, bc_move_px_js, bc_status_js, bc_summary_js,
};

/// Base dimension's first extension-line origin, nominal logical px.
///
/// The two origins share one screen row so `DIMLINEAR` sees a purely vertical
/// location offset and resolves a **horizontal** dimension (rotation 0, measures
/// X) — the orientation both chain commands below assume.
const BC_ORIGIN1: (f64, f64) = (250.0, 337.0);
/// Base dimension's second extension-line origin, nominal logical px.
const BC_ORIGIN2: (f64, f64) = (370.0, 337.0);
/// Base dimension-line location, nominal logical px: 22 px above the origins,
/// so the drawn dimension line is the horizontal segment from x = 250 to x = 370
/// at y = 315.
const BC_LOCATION: (f64, f64) = (310.0, 315.0);
/// The base pick for both chain commands: a point on that dimension line, 30 px
/// left of the text's midpoint so the click is on the line itself (the text is
/// offset above the line by `DIMGAP + DIMTXT/2`, whatever the resolved style).
const BC_BASE_PICK: (f64, f64) = (280.0, 315.0);
/// The three measured points for `DIMBASELINE` (one row each). They sit below the
/// base line and their X projections differ from the base's first origin, which
/// is what the command requires of every row.
const BC_BASELINE_POINTS: [(f64, f64); 3] = [(450.0, 330.0), (530.0, 330.0), (610.0, 330.0)];
/// The three chained points for `DIMCONTINUE`: strictly increasing X, all to the
/// right of the base's second origin (px 370), so every segment advances along
/// the base direction instead of folding back.
const BC_CONTINUE_POINTS: [(f64, f64); 3] = [(450.0, 325.0), (530.0, 325.0), (610.0, 325.0)];
/// Informational select click: real ink of the created dimensions.
const BC_SELECT: (f64, f64) = (280.0, 315.0);
/// Click-free pointer park so the viewport-spanning crosshair leaves the phase's
/// ink (the P7c precedent).
const BC_PARK: (f64, f64) = (10.0, 337.0);

/// Schedule the P5f baseline/continue phases relative to the driver start.
///
/// They start at driver t=213.2 s (after the P7c `MLEADER` phase, which ends at
/// t=212.4 s) and step every 400 ms; the combined single-line report is scheduled
/// after them at t=236 s (see [`super::draw_edit::schedule_draw_report`]). The
/// timeline:
///
/// * t=213.2 s..218.4 s: `DIMLINEAR` base — start, status, origin 1, origin 2,
///   location, `CONFIRM`, status;
/// * t=218.8 s: gate (the base added exactly one entity, tool cleared, no error);
/// * t=219.2 s..221.2 s: `DIMBASELINE` — start, status, base pick on the
///   dimension line, status, gate (the pick advanced the capture);
/// * t=221.6 s..224.8 s: three measured points, one status + gate each, then the
///   last gate requires `points = 4` and `ready = true`;
/// * t=225.2 s..226 s: `CONFIRM`, status, gate (`entityCount` **+3**, tool
///   `null`, no error);
/// * t=226.4 s..228.4 s: `DIMCONTINUE` — start, status, base pick, status, gate;
/// * t=228.8 s..232 s: three chained points, one status + gate each, then the
///   last gate requires `points = 4` and `ready = true`;
/// * t=232.4 s..233.2 s: `CONFIRM`, status, gate (`entityCount` **+3**);
/// * t=233.6 s..234.4 s: informational click-select, status, gate (the read
///   completed; count/rows are reported verbatim, not asserted);
/// * t=234.8 s: click-free pointer move to (10, 337).
///
/// The report fragment is built at t=235.2 s by the script module
/// [`super::draw_baseline_continue_js`].
pub(super) fn schedule_baseline_continue_phases(webview: &tauri::Webview, start: &Instant) {
    // ---- The linear base dimension both chain commands pick ----
    schedule_draw_ms(
        start,
        213_200,
        "cad_command DIMLINEAR (baseline/continue base)",
        || webview.eval(bc_command_js("DIMLINEAR", "bc_base")),
    );
    schedule_draw_ms(
        start,
        213_600,
        "bc-base-start cad_status (tool draw.dimlinear, points 0)",
        || webview.eval(bc_status_js("s_bc_base_start", "s_bc_base_start")),
    );
    schedule_draw_ms(
        start,
        214_000,
        "bc-base-start gate (aborts unless the real selector is active)",
        || {
            webview.eval(bc_expect_js(
                "bc_gate_base_start",
                &[
                    ("s_bc_base_start_done", "true"),
                    ("s_bc_base_start_tool", "'draw.dimlinear'"),
                    ("s_bc_base_start_points", "0"),
                ],
            ))
        },
    );
    let origin1 = bc_click_px_js(BC_ORIGIN1.0, BC_ORIGIN1.1, "bc_origin1");
    schedule_draw_ms(start, 214_400, "bc origin1 click at px (250, 337)", || {
        webview.eval(origin1)
    });
    schedule_draw_ms(start, 214_800, "bc-origin1 cad_status (points 1)", || {
        webview.eval(bc_status_js("s_bc_o1", "s_bc_o1"))
    });
    schedule_draw_ms(
        start,
        215_200,
        "bc-origin1 gate (aborts unless the first origin was captured)",
        || {
            webview.eval(bc_expect_js(
                "bc_gate_o1",
                &[
                    ("bc_origin1_dispatched", "true"),
                    ("s_bc_o1_done", "true"),
                    ("s_bc_o1_points", "1"),
                    ("s_bc_o1_error", "null"),
                ],
            ))
        },
    );
    let origin2 = bc_click_px_js(BC_ORIGIN2.0, BC_ORIGIN2.1, "bc_origin2");
    schedule_draw_ms(start, 215_600, "bc origin2 click at px (370, 337)", || {
        webview.eval(origin2)
    });
    schedule_draw_ms(start, 216_000, "bc-origin2 cad_status (points 2)", || {
        webview.eval(bc_status_js("s_bc_o2", "s_bc_o2"))
    });
    schedule_draw_ms(
        start,
        216_400,
        "bc-origin2 gate (aborts unless the second origin was captured)",
        || {
            webview.eval(bc_expect_js(
                "bc_gate_o2",
                &[
                    ("bc_origin2_dispatched", "true"),
                    ("s_bc_o2_done", "true"),
                    ("s_bc_o2_points", "2"),
                    ("s_bc_o2_error", "null"),
                ],
            ))
        },
    );
    let location = bc_click_px_js(BC_LOCATION.0, BC_LOCATION.1, "bc_location");
    schedule_draw_ms(
        start,
        216_800,
        "bc location click at px (310, 315) (purely vertical offset => horizontal dimension)",
        || webview.eval(location),
    );
    schedule_draw_ms(
        start,
        217_200,
        "bc-location cad_status (points 3, ready)",
        || webview.eval(bc_status_js("s_bc_loc", "s_bc_loc")),
    );
    schedule_draw_ms(
        start,
        217_600,
        "bc-location gate (aborts unless the base is confirmable)",
        || {
            webview.eval(bc_expect_js(
                "bc_gate_loc",
                &[
                    ("bc_location_dispatched", "true"),
                    ("s_bc_loc_done", "true"),
                    ("s_bc_loc_points", "3"),
                    ("s_bc_loc_ready", "true"),
                    ("s_bc_loc_error", "null"),
                ],
            ))
        },
    );
    schedule_draw_ms(
        start,
        218_000,
        "cad_command CONFIRM (base DIMLINEAR)",
        || webview.eval(bc_command_js("CONFIRM", "bc_base_confirm")),
    );
    schedule_draw_ms(
        start,
        218_400,
        "bc-base-after cad_status (entityCount +1, tool null)",
        || webview.eval(bc_status_js("s_bc_base_after", "s_bc_base_start")),
    );
    schedule_draw_ms(
        start,
        218_800,
        "bc-base-after gate (delta 1, tool cleared, no error)",
        || {
            webview.eval(bc_expect_js(
                "bc_gate_base_after",
                &[
                    ("s_bc_base_after_done", "true"),
                    ("s_bc_base_after_delta", "1"),
                    ("s_bc_base_after_tool", "null"),
                    ("s_bc_base_after_error", "null"),
                ],
            ))
        },
    );

    // ---- DIMBASELINE: pick the base on its dimension line, then 3 measured points ----
    schedule_draw_ms(start, 219_200, "cad_command DIMBASELINE", || {
        webview.eval(bc_command_js("DIMBASELINE", "bc_baseline"))
    });
    schedule_draw_ms(
        start,
        219_600,
        "bc-baseline-start cad_status (tool draw.dimbaseline, points 0)",
        || webview.eval(bc_status_js("s_bc_bl_start", "s_bc_bl_start")),
    );
    schedule_draw_ms(
        start,
        220_000,
        "bc-baseline-start gate (aborts unless the real selector is active)",
        || {
            webview.eval(bc_expect_js(
                "bc_gate_bl_start",
                &[
                    ("s_bc_bl_start_done", "true"),
                    ("s_bc_bl_start_tool", "'draw.dimbaseline'"),
                    ("s_bc_bl_start_points", "0"),
                ],
            ))
        },
    );
    let baseline_pick = bc_click_px_js(BC_BASE_PICK.0, BC_BASE_PICK.1, "bc_baseline_pick");
    schedule_draw_ms(
        start,
        220_400,
        "bc baseline base pick at px (280, 315) (on the base dimension line)",
        || webview.eval(baseline_pick),
    );
    schedule_draw_ms(
        start,
        220_800,
        "bc-baseline-pick cad_status (points 1: the base pick advanced the capture)",
        || webview.eval(bc_status_js("s_bc_bl_pick", "s_bc_bl_start")),
    );
    schedule_draw_ms(
        start,
        221_200,
        "bc-baseline-pick gate (base pick captured, tool still active, no error)",
        || {
            webview.eval(bc_expect_js(
                "bc_gate_bl_pick",
                &[
                    ("bc_baseline_pick_dispatched", "true"),
                    ("s_bc_bl_pick_done", "true"),
                    ("s_bc_bl_pick_tool", "'draw.dimbaseline'"),
                    ("s_bc_bl_pick_points", "1"),
                    ("s_bc_bl_pick_error", "null"),
                ],
            ))
        },
    );
    schedule_draw_ms(
        start,
        221_600,
        "bc baseline measured point 1 click at px (450, 330)",
        || {
            webview.eval(bc_click_px_js(
                BC_BASELINE_POINTS[0].0,
                BC_BASELINE_POINTS[0].1,
                "bc_bl_p1",
            ))
        },
    );
    schedule_draw_ms(
        start,
        222_000,
        "bc-baseline-p1 cad_status (points 2)",
        || webview.eval(bc_status_js("s_bc_bl_p1", "s_bc_bl_start")),
    );
    schedule_draw_ms(
        start,
        222_400,
        "bc-baseline-p1 gate (points 2, no error)",
        || {
            webview.eval(bc_expect_js(
                "bc_gate_bl_p1",
                &[
                    ("bc_bl_p1_dispatched", "true"),
                    ("s_bc_bl_p1_done", "true"),
                    ("s_bc_bl_p1_points", "2"),
                    ("s_bc_bl_p1_error", "null"),
                ],
            ))
        },
    );
    schedule_draw_ms(
        start,
        222_800,
        "bc baseline measured point 2 click at px (530, 330)",
        || {
            webview.eval(bc_click_px_js(
                BC_BASELINE_POINTS[1].0,
                BC_BASELINE_POINTS[1].1,
                "bc_bl_p2",
            ))
        },
    );
    schedule_draw_ms(
        start,
        223_200,
        "bc-baseline-p2 cad_status (points 3)",
        || webview.eval(bc_status_js("s_bc_bl_p2", "s_bc_bl_start")),
    );
    schedule_draw_ms(
        start,
        223_600,
        "bc-baseline-p2 gate (points 3, no error)",
        || {
            webview.eval(bc_expect_js(
                "bc_gate_bl_p2",
                &[
                    ("bc_bl_p2_dispatched", "true"),
                    ("s_bc_bl_p2_done", "true"),
                    ("s_bc_bl_p2_points", "3"),
                    ("s_bc_bl_p2_error", "null"),
                ],
            ))
        },
    );
    schedule_draw_ms(
        start,
        224_000,
        "bc baseline measured point 3 click at px (610, 330)",
        || {
            webview.eval(bc_click_px_js(
                BC_BASELINE_POINTS[2].0,
                BC_BASELINE_POINTS[2].1,
                "bc_bl_p3",
            ))
        },
    );
    schedule_draw_ms(
        start,
        224_400,
        "bc-baseline-p3 cad_status (points 4, ready)",
        || webview.eval(bc_status_js("s_bc_bl_p3", "s_bc_bl_start")),
    );
    schedule_draw_ms(
        start,
        224_800,
        "bc-baseline-p3 gate (base pick + 3 measured points, confirmable)",
        || {
            webview.eval(bc_expect_js(
                "bc_gate_bl_p3",
                &[
                    ("bc_bl_p3_dispatched", "true"),
                    ("s_bc_bl_p3_done", "true"),
                    ("s_bc_bl_p3_points", "4"),
                    ("s_bc_bl_p3_ready", "true"),
                    ("s_bc_bl_p3_error", "null"),
                ],
            ))
        },
    );
    schedule_draw_ms(start, 225_200, "cad_command CONFIRM (dimbaseline)", || {
        webview.eval(bc_command_js("CONFIRM", "bc_baseline_confirm"))
    });
    schedule_draw_ms(
        start,
        225_600,
        "bc-baseline-after cad_status (entityCount +3, tool null)",
        || webview.eval(bc_status_js("s_bc_bl_after", "s_bc_bl_start")),
    );
    schedule_draw_ms(
        start,
        226_000,
        "bc-baseline-after gate (delta 3, tool cleared, no error)",
        || {
            webview.eval(bc_expect_js(
                "bc_gate_bl_after",
                &[
                    ("s_bc_bl_after_done", "true"),
                    ("s_bc_bl_after_delta", "3"),
                    ("s_bc_bl_after_tool", "null"),
                    ("s_bc_bl_after_error", "null"),
                ],
            ))
        },
    );

    // ---- DIMCONTINUE: same base, three chained points ----
    schedule_draw_ms(start, 226_400, "cad_command DIMCONTINUE", || {
        webview.eval(bc_command_js("DIMCONTINUE", "bc_continue"))
    });
    schedule_draw_ms(
        start,
        226_800,
        "bc-continue-start cad_status (tool draw.dimcontinue, points 0)",
        || webview.eval(bc_status_js("s_bc_ct_start", "s_bc_ct_start")),
    );
    schedule_draw_ms(
        start,
        227_200,
        "bc-continue-start gate (aborts unless the real selector is active)",
        || {
            webview.eval(bc_expect_js(
                "bc_gate_ct_start",
                &[
                    ("s_bc_ct_start_done", "true"),
                    ("s_bc_ct_start_tool", "'draw.dimcontinue'"),
                    ("s_bc_ct_start_points", "0"),
                ],
            ))
        },
    );
    let continue_pick = bc_click_px_js(BC_BASE_PICK.0, BC_BASE_PICK.1, "bc_continue_pick");
    schedule_draw_ms(
        start,
        227_600,
        "bc continue base pick at px (280, 315) (the same base dimension line)",
        || webview.eval(continue_pick),
    );
    schedule_draw_ms(
        start,
        228_000,
        "bc-continue-pick cad_status (points 1: the base pick advanced the capture)",
        || webview.eval(bc_status_js("s_bc_ct_pick", "s_bc_ct_start")),
    );
    schedule_draw_ms(
        start,
        228_400,
        "bc-continue-pick gate (base pick captured, tool still active, no error)",
        || {
            webview.eval(bc_expect_js(
                "bc_gate_ct_pick",
                &[
                    ("bc_continue_pick_dispatched", "true"),
                    ("s_bc_ct_pick_done", "true"),
                    ("s_bc_ct_pick_tool", "'draw.dimcontinue'"),
                    ("s_bc_ct_pick_points", "1"),
                    ("s_bc_ct_pick_error", "null"),
                ],
            ))
        },
    );
    schedule_draw_ms(
        start,
        228_800,
        "bc continue chained point 1 click at px (450, 325)",
        || {
            webview.eval(bc_click_px_js(
                BC_CONTINUE_POINTS[0].0,
                BC_CONTINUE_POINTS[0].1,
                "bc_ct_p1",
            ))
        },
    );
    schedule_draw_ms(
        start,
        229_200,
        "bc-continue-p1 cad_status (points 2)",
        || webview.eval(bc_status_js("s_bc_ct_p1", "s_bc_ct_start")),
    );
    schedule_draw_ms(
        start,
        229_600,
        "bc-continue-p1 gate (points 2, no error)",
        || {
            webview.eval(bc_expect_js(
                "bc_gate_ct_p1",
                &[
                    ("bc_ct_p1_dispatched", "true"),
                    ("s_bc_ct_p1_done", "true"),
                    ("s_bc_ct_p1_points", "2"),
                    ("s_bc_ct_p1_error", "null"),
                ],
            ))
        },
    );
    schedule_draw_ms(
        start,
        230_000,
        "bc continue chained point 2 click at px (530, 325)",
        || {
            webview.eval(bc_click_px_js(
                BC_CONTINUE_POINTS[1].0,
                BC_CONTINUE_POINTS[1].1,
                "bc_ct_p2",
            ))
        },
    );
    schedule_draw_ms(
        start,
        230_400,
        "bc-continue-p2 cad_status (points 3)",
        || webview.eval(bc_status_js("s_bc_ct_p2", "s_bc_ct_start")),
    );
    schedule_draw_ms(
        start,
        230_800,
        "bc-continue-p2 gate (points 3, no error)",
        || {
            webview.eval(bc_expect_js(
                "bc_gate_ct_p2",
                &[
                    ("bc_ct_p2_dispatched", "true"),
                    ("s_bc_ct_p2_done", "true"),
                    ("s_bc_ct_p2_points", "3"),
                    ("s_bc_ct_p2_error", "null"),
                ],
            ))
        },
    );
    schedule_draw_ms(
        start,
        231_200,
        "bc continue chained point 3 click at px (610, 325)",
        || {
            webview.eval(bc_click_px_js(
                BC_CONTINUE_POINTS[2].0,
                BC_CONTINUE_POINTS[2].1,
                "bc_ct_p3",
            ))
        },
    );
    schedule_draw_ms(
        start,
        231_600,
        "bc-continue-p3 cad_status (points 4, ready)",
        || webview.eval(bc_status_js("s_bc_ct_p3", "s_bc_ct_start")),
    );
    schedule_draw_ms(
        start,
        232_000,
        "bc-continue-p3 gate (base pick + 3 chained points, confirmable)",
        || {
            webview.eval(bc_expect_js(
                "bc_gate_ct_p3",
                &[
                    ("bc_ct_p3_dispatched", "true"),
                    ("s_bc_ct_p3_done", "true"),
                    ("s_bc_ct_p3_points", "4"),
                    ("s_bc_ct_p3_ready", "true"),
                    ("s_bc_ct_p3_error", "null"),
                ],
            ))
        },
    );
    schedule_draw_ms(start, 232_400, "cad_command CONFIRM (dimcontinue)", || {
        webview.eval(bc_command_js("CONFIRM", "bc_continue_confirm"))
    });
    schedule_draw_ms(
        start,
        232_800,
        "bc-continue-after cad_status (entityCount +3, tool null)",
        || webview.eval(bc_status_js("s_bc_ct_after", "s_bc_ct_start")),
    );
    schedule_draw_ms(
        start,
        233_200,
        "bc-continue-after gate (delta 3, tool cleared, no error)",
        || {
            webview.eval(bc_expect_js(
                "bc_gate_ct_after",
                &[
                    ("s_bc_ct_after_done", "true"),
                    ("s_bc_ct_after_delta", "3"),
                    ("s_bc_ct_after_tool", "null"),
                    ("s_bc_ct_after_error", "null"),
                ],
            ))
        },
    );

    // ---- Informational: select the created ink and read the real rows ----
    schedule_draw_ms(
        start,
        233_600,
        "bc select click at px (280, 315) (real created dimension ink)",
        || webview.eval(bc_click_px_js(BC_SELECT.0, BC_SELECT.1, "bc_select")),
    );
    schedule_draw_ms(
        start,
        234_000,
        "bc-selected cad_status (selection count + real rows, reported verbatim)",
        || webview.eval(bc_status_js("s_bc_select", "s_bc_ct_start")),
    );
    schedule_draw_ms(
        start,
        234_400,
        "bc-select gate (the read completed; count/rows are not asserted)",
        || {
            webview.eval(bc_expect_js(
                "bc_gate_select",
                &[
                    ("bc_select_dispatched", "true"),
                    ("s_bc_select_done", "true"),
                ],
            ))
        },
    );

    // ---- Park the crosshair, then build the report fragment ----
    schedule_draw_ms(
        start,
        234_800,
        "bc click-free pointer move to px (10, 337)",
        || webview.eval(bc_move_px_js(BC_PARK.0, BC_PARK.1, "bc_park")),
    );
    schedule_draw_ms(start, 235_200, "baseline/continue summary fragment", || {
        webview.eval(bc_summary_js())
    });
}
