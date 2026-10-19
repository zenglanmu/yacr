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
         POLYGON(sides=6, out-of-range=2) POINT(visible) RECTANG"
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
///   an external `--capture-at` lands **in the t=52..68 gap** so a reviewer sees
///   the rounded corner / the cut corner without the later phases' clutter;
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
/// * t=101s log one single-line report via the eval callback.
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
    // The FILLET ARC and CHAMFER LINE are now on screen. The next phase starts at
    // t=68, so an external `--capture-at` in t=52..68 lands on a host that shows
    // exactly the fillet/chamfer result (fixture + the two trimmed pairs + the
    // arc + the chamfer line), without the later phases' clutter.

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

    // The report is logged from Rust through the eval callback; the JS also
    // `console.log`s it. A dispatch that merely posts is not a pass — the report
    // carries the real before/after reads and the real refusals.
    schedule_draw(&start, 101, "draw evidence report", || {
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

/// Build the named status-read script (`s_base`, `s_fillet_miss`, ...).
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
fn draw_confirm_js(label: &str) -> String {
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

/// Single-line report over `window.__yacr_draw`.
///
/// Fields are whitespace-flattened so the line stays tokenizable; `-` means
/// absent and `-1` a count/value that was never read. Each `*_delta` is only a
/// real number when both counts were read. `polygon_sides_visible` and
/// `polygon_bad_refused` are derived from the real reads (never asserted);
/// `fillet_miss_advanced` is derived from the real capture-point counts.
/// `vertex_count_channel=none_...` is the honest statement that `cad_status`
/// exposes no vertex/geometry count, so neither the polygon's real sidedness nor
/// the FILLET/CHAMFER geometry can be read from the status channel.
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
  var fillet_miss_advanced = n(d.s_fillet_miss_points) > n(d.s_fillet_active_points);
  var status_failed = d.s_base_failed || d.s_line_a_h_after_failed || d.s_line_a_v_after_failed ||
    d.s_fillet_active_failed || d.s_fillet_miss_failed || d.s_fillet_pick1_failed ||
    d.s_fillet_pick2_failed || d.s_fillet_after_failed || d.s_line_b_h_after_failed ||
    d.s_line_b_v_after_failed || d.s_chamfer_active_failed || d.s_chamfer_pick1_failed ||
    d.s_chamfer_pick2_failed || d.s_chamfer_after_failed || d.s_pline_points_failed ||
    d.s_pline_after_failed || d.s_polygon_active_failed || d.s_polygon_bad_failed ||
    d.s_polygon_points_failed || d.s_polygon_after_failed || d.s_point_points_failed ||
    d.s_point_after_failed || d.s_rectang_points_failed || d.s_rectang_after_failed ||
    d.confirm_a_h_failed || d.confirm_a_v_failed || d.confirm_fillet_failed ||
    d.confirm_b_h_failed || d.confirm_b_v_failed || d.confirm_chamfer_failed ||
    d.confirm_pline_failed || d.confirm_polygon_failed || d.confirm_point_failed ||
    d.confirm_rectang_failed;
  var text = 'draw_synthetic=1_not_a_real_mouse' +
    ' baseline_count=' + v(d.s_base_count) +
    ' geom_a_before_count=' + v(d.s_base_count) +
    ' geom_a_after_count=' + v(d.s_line_a_v_after_count) +
    ' geom_a_delta=' + delta(d.s_base_count, d.s_line_a_v_after_count) +
    ' fillet_radius_set_ack=' + v(d['fillet_radius_ack']) +
    ' fillet_active_tool=' + v(d.s_fillet_active_tool) +
    ' fillet_active_radius=' + v(d.s_fillet_active_fillet) +
    ' fillet_miss_points_before=' + v(d.s_fillet_active_points) +
    ' fillet_miss_points_after=' + v(d.s_fillet_miss_points) +
    ' fillet_miss_advanced=' + fillet_miss_advanced +
    ' fillet_miss_error=' + v(d.s_fillet_miss_error) +
    ' fillet_pick1_points=' + v(d.s_fillet_pick1_points) +
    ' fillet_pick2_points=' + v(d.s_fillet_pick2_points) +
    ' fillet_pick2_ready=' + v(d.s_fillet_pick2_ready) +
    ' fillet_confirm_ack=' + v(d.confirm_fillet_ack) +
    ' fillet_after_count=' + v(d.s_fillet_after_count) +
    ' fillet_delta=' + delta(d.s_line_a_v_after_count, d.s_fillet_after_count) +
    ' fillet_after_tool=' + v(d.s_fillet_after_tool) +
    ' fillet_after_error=' + v(d.s_fillet_after_error) +
    ' geom_b_before_count=' + v(d.s_fillet_after_count) +
    ' geom_b_after_count=' + v(d.s_line_b_v_after_count) +
    ' geom_b_delta=' + delta(d.s_fillet_after_count, d.s_line_b_v_after_count) +
    ' chamfer_a_set_ack=' + v(d['chamfer_a_ack']) +
    ' chamfer_b_set_ack=' + v(d['chamfer_b_ack']) +
    ' chamfer_active_tool=' + v(d.s_chamfer_active_tool) +
    ' chamfer_active_a=' + v(d.s_chamfer_active_chamfer_a) +
    ' chamfer_active_b=' + v(d.s_chamfer_active_chamfer_b) +
    ' chamfer_pick1_points=' + v(d.s_chamfer_pick1_points) +
    ' chamfer_pick2_points=' + v(d.s_chamfer_pick2_points) +
    ' chamfer_pick2_ready=' + v(d.s_chamfer_pick2_ready) +
    ' chamfer_confirm_ack=' + v(d.confirm_chamfer_ack) +
    ' chamfer_after_count=' + v(d.s_chamfer_after_count) +
    ' chamfer_delta=' + delta(d.s_line_b_v_after_count, d.s_chamfer_after_count) +
    ' chamfer_after_tool=' + v(d.s_chamfer_after_tool) +
    ' chamfer_after_error=' + v(d.s_chamfer_after_error) +
    ' pline_points=' + v(d.s_pline_points_points) +
    ' pline_after_count=' + v(d.s_pline_after_count) +
    ' pline_delta=' + delta(d.s_chamfer_after_count, d.s_pline_after_count) +
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
    ' fillet_geometry_channel=none_cad_status_has_no_entity_geometry' +
    ' chamfer_geometry_channel=none_cad_status_has_no_entity_geometry' +
    ' point_world=' + arr(d.point_world) +
    ' point_frac=' + arr(d.point_frac) +
    ' camera=' + arr(d.camera) +
    ' rect=' + arr(d.rect) +
    ' clicks=' + n(d.clicks) +
    ' start_failed=' + v(d.start_failed) +
    ' sides_failed=' + v(d.sides_failed) +
    ' confirm_failed=' + v(d.confirm_failed) +
    ' status_failed=' + v(status_failed) +
    ' a_h1_failed=' + v(d.a_h1_failed) +
    ' a_h2_failed=' + v(d.a_h2_failed) +
    ' a_v1_failed=' + v(d.a_v1_failed) +
    ' a_v2_failed=' + v(d.a_v2_failed) +
    ' fillet_miss_failed=' + v(d.fillet_miss_failed) +
    ' fillet_pick1_failed=' + v(d.fillet_pick1_failed) +
    ' fillet_pick2_failed=' + v(d.fillet_pick2_failed) +
    ' b_h1_failed=' + v(d.b_h1_failed) +
    ' b_h2_failed=' + v(d.b_h2_failed) +
    ' b_v1_failed=' + v(d.b_v1_failed) +
    ' b_v2_failed=' + v(d.b_v2_failed) +
    ' chamfer_pick1_failed=' + v(d.chamfer_pick1_failed) +
    ' chamfer_pick2_failed=' + v(d.chamfer_pick2_failed) +
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
