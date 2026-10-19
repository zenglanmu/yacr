//! `YACR_PROBE_DRAW_EVIDENCE` P4 edit-command phases (ARRAYRECT / ARRAYPOLAR /
//! ALIGN / STRETCH / EXPLODE) and the combined single-line draw report.
//!
//! Split out of [`super::draw`] at the 1000-line code-file gate. This is the same
//! evidence-only, default-OFF, env-gated driver: it dispatches the **real**
//! selectors through the normal `cad_command`/`cad_set_tool_parameter` funnel and
//! synthetic DOM pointer events on `.native-input`, then reads the **real**
//! `cad_status` before/after each commit. Every reported number is a real
//! `cad_status` read; nothing here is synthesized.
//!
//! What each phase proves:
//! * **ARRAYRECT**: click-select one entity, set `arrayRows`/`arrayColumns` through
//!   the real IPC, start `ARRAYRECT`, `CONFIRM`, read `entityCount` **+5**
//!   (`rows × columns − 1`; the source is retained).
//! * **ARRAYPOLAR**: click-select one entity, set `arrayItems`, start
//!   `ARRAYPOLAR`, click the centre, `CONFIRM`, read `entityCount` **+3**
//!   (`items − 1`).
//! * **ALIGN**: draw a LINE, click-select it, start `ALIGN`, capture the four
//!   points (source pair, destination pair) with `alignScale` left at its default
//!   `false`, `CONFIRM`. This is an **in-place** transform, so `entityCount` must
//!   be **unchanged**; the honest observables are "accepted, no error, count
//!   unchanged" plus the screenshot — the report says that plainly and never
//!   implies a geometry delta.
//! * **STRETCH**: draw a LINE, start `STRETCH`, capture a crossing window that
//!   encloses **one** endpoint, then the base and displacement points, `CONFIRM`.
//!   Also in-place: `entityCount` unchanged.
//! * **EXPLODE**: draw a closed polyline (`RECTANG`), click-select it, then
//!   dispatch the payload-free `EXPLODE` selector: the polyline is deleted and
//!   four LINEs are inserted ⇒ `entityCount` **+3**.
//!
//! Synthetic input, **not a real mouse**: the events are dispatched on
//! `.native-input` and never touch the kernel/evdev/XInput path. `cad_status`
//! exposes no per-entity geometry, so the array/align/stretch geometry is not
//! observable through the status channel; only `entityCount` (plus the surfaced
//! technical `error`) is reported as the honest change observable.
//!
//! The phases are sub-second-stepped ([`super::draw::schedule_draw_ms`]) so the
//! whole pass finishes inside the bounded evidence timeout; the report lands last.

use std::time::Instant;

use super::draw::{
    draw_click_world_js, draw_command_js, draw_confirm_js, draw_set_parameter_number_js,
    draw_status_js, schedule_draw_ms,
};

/// Schedule the P4 edit-command phases relative to the driver start (seconds ×
/// 1000 ms), in the task's order: ARRAYRECT, ARRAYPOLAR, ALIGN, STRETCH, EXPLODE.
///
/// The phases start at t=100 s (after the drawing phases) and step every 500 ms so
/// the whole pass, including the final report at t=137 s, fits inside the bounded
/// evidence timeout. Each phase reads the real `cad_status` before and after its
/// commit; a dispatch that merely posts is not a pass.
///
/// Source geometry is placed in free regions of the visible extent (camera centre
/// (1.5, 0.5), ~0.005032 world/logical-pixel; the fixture is four parallel
/// vertical LINEs at x = 0, 1, 2, 3 spanning y = 0..1): the array sources at
/// y = 1.05 (above the fixture tops), and the ALIGN/STRETCH/EXPLODE geometry in
/// the y = 0.05..0.15 band between the fixture verticals.
pub(super) fn schedule_draw_edit_phases(webview: &tauri::Webview, start: &Instant) {
    // ---- ARRAYRECT: one selected entity -> 2×3 grid (+5 copies) ----
    schedule_draw_ms(
        start,
        100_000,
        "cad_command LINE (arrayrect source)",
        || webview.eval(draw_command_js("LINE", "arr_line")),
    );
    let arr_a = draw_click_world_js(0.4, 1.05, "arr_src_a");
    schedule_draw_ms(
        start,
        100_500,
        "arrayrect source click A at world (0.4, 1.05)",
        || webview.eval(arr_a),
    );
    let arr_b = draw_click_world_js(0.7, 1.05, "arr_src_b");
    schedule_draw_ms(
        start,
        101_000,
        "arrayrect source click B at world (0.7, 1.05)",
        || webview.eval(arr_b),
    );
    schedule_draw_ms(
        start,
        101_500,
        "cad_command CONFIRM (arrayrect source)",
        || webview.eval(draw_confirm_js("confirm_arr_src")),
    );
    schedule_draw_ms(start, 102_000, "arrayrect-source-after cad_status", || {
        webview.eval(draw_status_js("s_arr_src_after"))
    });
    let arr_sel = draw_click_world_js(0.55, 1.05, "arr_select");
    schedule_draw_ms(
        start,
        102_500,
        "arrayrect click-select at world (0.55, 1.05)",
        || webview.eval(arr_sel),
    );
    schedule_draw_ms(start, 103_000, "arrayrect-selected cad_status", || {
        webview.eval(draw_status_js("s_arr_select"))
    });
    schedule_draw_ms(start, 103_500, "cad_set_tool_parameter arrayRows=2", || {
        webview.eval(draw_set_parameter_number_js("arrayRows", 2.0, "arr_rows"))
    });
    schedule_draw_ms(
        start,
        104_000,
        "cad_set_tool_parameter arrayColumns=3",
        || {
            webview.eval(draw_set_parameter_number_js(
                "arrayColumns",
                3.0,
                "arr_cols",
            ))
        },
    );
    schedule_draw_ms(
        start,
        104_500,
        "cad_set_tool_parameter arrayRowSpacing=0.25",
        || {
            webview.eval(draw_set_parameter_number_js(
                "arrayRowSpacing",
                0.25,
                "arr_row_sp",
            ))
        },
    );
    schedule_draw_ms(
        start,
        105_000,
        "cad_set_tool_parameter arrayColumnSpacing=0.3",
        || {
            webview.eval(draw_set_parameter_number_js(
                "arrayColumnSpacing",
                0.3,
                "arr_col_sp",
            ))
        },
    );
    schedule_draw_ms(start, 105_500, "cad_command ARRAYRECT", || {
        webview.eval(draw_command_js("ARRAYRECT", "arrayrect"))
    });
    schedule_draw_ms(start, 106_000, "arrayrect-active cad_status", || {
        webview.eval(draw_status_js("s_arrayrect_active"))
    });
    schedule_draw_ms(start, 106_500, "cad_command CONFIRM (arrayrect)", || {
        webview.eval(draw_confirm_js("confirm_arrayrect"))
    });
    schedule_draw_ms(start, 107_000, "arrayrect-after cad_status", || {
        webview.eval(draw_status_js("s_arrayrect_after"))
    });

    // ---- ARRAYPOLAR: one selected entity around a captured centre (+3) ----
    schedule_draw_ms(
        start,
        108_000,
        "cad_command LINE (arraypolar source)",
        || webview.eval(draw_command_js("LINE", "pol_line")),
    );
    let pol_a = draw_click_world_js(2.6, 1.05, "pol_src_a");
    schedule_draw_ms(
        start,
        108_500,
        "arraypolar source click A at world (2.6, 1.05)",
        || webview.eval(pol_a),
    );
    let pol_b = draw_click_world_js(2.9, 1.05, "pol_src_b");
    schedule_draw_ms(
        start,
        109_000,
        "arraypolar source click B at world (2.9, 1.05)",
        || webview.eval(pol_b),
    );
    schedule_draw_ms(
        start,
        109_500,
        "cad_command CONFIRM (arraypolar source)",
        || webview.eval(draw_confirm_js("confirm_pol_src")),
    );
    schedule_draw_ms(start, 110_000, "arraypolar-source-after cad_status", || {
        webview.eval(draw_status_js("s_pol_src_after"))
    });
    let pol_sel = draw_click_world_js(2.75, 1.05, "pol_select");
    schedule_draw_ms(
        start,
        110_500,
        "arraypolar click-select at world (2.75, 1.05)",
        || webview.eval(pol_sel),
    );
    schedule_draw_ms(start, 111_000, "arraypolar-selected cad_status", || {
        webview.eval(draw_status_js("s_pol_select"))
    });
    schedule_draw_ms(
        start,
        111_500,
        "cad_set_tool_parameter arrayItems=4",
        || webview.eval(draw_set_parameter_number_js("arrayItems", 4.0, "pol_items")),
    );
    schedule_draw_ms(start, 112_000, "cad_command ARRAYPOLAR", || {
        webview.eval(draw_command_js("ARRAYPOLAR", "arraypolar"))
    });
    schedule_draw_ms(start, 112_500, "arraypolar-active cad_status", || {
        webview.eval(draw_status_js("s_arraypolar_active"))
    });
    let polar_center = draw_click_world_js(2.6, 0.75, "polar_center");
    schedule_draw_ms(
        start,
        113_000,
        "arraypolar centre click at world (2.6, 0.75)",
        || webview.eval(polar_center),
    );
    schedule_draw_ms(start, 113_500, "arraypolar-points cad_status", || {
        webview.eval(draw_status_js("s_arraypolar_points"))
    });
    schedule_draw_ms(start, 114_000, "cad_command CONFIRM (arraypolar)", || {
        webview.eval(draw_confirm_js("confirm_arraypolar"))
    });
    schedule_draw_ms(start, 114_500, "arraypolar-after cad_status", || {
        webview.eval(draw_status_js("s_arraypolar_after"))
    });

    // ---- ALIGN: four captured points, in-place (count unchanged) ----
    schedule_draw_ms(start, 115_500, "cad_command LINE (align source)", || {
        webview.eval(draw_command_js("LINE", "align_line"))
    });
    let align_a = draw_click_world_js(0.3, 0.05, "align_src_a");
    schedule_draw_ms(
        start,
        116_000,
        "align source click A at world (0.3, 0.05)",
        || webview.eval(align_a),
    );
    let align_b = draw_click_world_js(0.7, 0.05, "align_src_b");
    schedule_draw_ms(
        start,
        116_500,
        "align source click B at world (0.7, 0.05)",
        || webview.eval(align_b),
    );
    schedule_draw_ms(start, 117_000, "cad_command CONFIRM (align source)", || {
        webview.eval(draw_confirm_js("confirm_align_src"))
    });
    schedule_draw_ms(start, 117_500, "align-source-after cad_status", || {
        webview.eval(draw_status_js("s_align_src_after"))
    });
    let align_sel = draw_click_world_js(0.5, 0.05, "align_select");
    schedule_draw_ms(
        start,
        118_000,
        "align click-select at world (0.5, 0.05)",
        || webview.eval(align_sel),
    );
    schedule_draw_ms(start, 118_500, "align-selected cad_status", || {
        webview.eval(draw_status_js("s_align_select"))
    });
    schedule_draw_ms(start, 119_000, "cad_command ALIGN", || {
        webview.eval(draw_command_js("ALIGN", "align"))
    });
    schedule_draw_ms(
        start,
        119_500,
        "align-active cad_status (alignScale false)",
        || webview.eval(draw_status_js("s_align_active")),
    );
    let align_s1 = draw_click_world_js(0.5, 0.05, "align_s1");
    schedule_draw_ms(
        start,
        120_000,
        "align source1 click at world (0.5, 0.05)",
        || webview.eval(align_s1),
    );
    let align_d1 = draw_click_world_js(0.5, 0.25, "align_d1");
    schedule_draw_ms(
        start,
        120_500,
        "align dest1 click at world (0.5, 0.25)",
        || webview.eval(align_d1),
    );
    let align_s2 = draw_click_world_js(0.7, 0.05, "align_s2");
    schedule_draw_ms(
        start,
        121_000,
        "align source2 click at world (0.7, 0.05)",
        || webview.eval(align_s2),
    );
    let align_d2 = draw_click_world_js(0.7, 0.25, "align_d2");
    schedule_draw_ms(
        start,
        121_500,
        "align dest2 click at world (0.7, 0.25)",
        || webview.eval(align_d2),
    );
    schedule_draw_ms(start, 122_000, "align-points cad_status", || {
        webview.eval(draw_status_js("s_align_points"))
    });
    schedule_draw_ms(start, 122_500, "cad_command CONFIRM (align)", || {
        webview.eval(draw_confirm_js("confirm_align"))
    });
    schedule_draw_ms(
        start,
        123_000,
        "align-after cad_status (count unchanged)",
        || webview.eval(draw_status_js("s_align_after")),
    );

    // ---- STRETCH: crossing window on one endpoint, in-place (count unchanged) ----
    schedule_draw_ms(start, 124_000, "cad_command LINE (stretch source)", || {
        webview.eval(draw_command_js("LINE", "stretch_line"))
    });
    let st_a = draw_click_world_js(1.4, 0.05, "stretch_src_a");
    schedule_draw_ms(
        start,
        124_500,
        "stretch source click A at world (1.4, 0.05)",
        || webview.eval(st_a),
    );
    let st_b = draw_click_world_js(1.8, 0.05, "stretch_src_b");
    schedule_draw_ms(
        start,
        125_000,
        "stretch source click B at world (1.8, 0.05)",
        || webview.eval(st_b),
    );
    schedule_draw_ms(
        start,
        125_500,
        "cad_command CONFIRM (stretch source)",
        || webview.eval(draw_confirm_js("confirm_stretch_src")),
    );
    schedule_draw_ms(start, 126_000, "stretch-source-after cad_status", || {
        webview.eval(draw_status_js("s_stretch_src_after"))
    });
    schedule_draw_ms(start, 126_500, "cad_command STRETCH", || {
        webview.eval(draw_command_js("STRETCH", "stretch"))
    });
    schedule_draw_ms(start, 127_000, "stretch-active cad_status", || {
        webview.eval(draw_status_js("s_stretch_active"))
    });
    let st_w1 = draw_click_world_js(1.3, -0.05, "stretch_w1");
    schedule_draw_ms(
        start,
        127_500,
        "stretch window corner 1 at world (1.3, -0.05)",
        || webview.eval(st_w1),
    );
    let st_w2 = draw_click_world_js(1.55, 0.15, "stretch_w2");
    schedule_draw_ms(
        start,
        128_000,
        "stretch window corner 2 at world (1.55, 0.15)",
        || webview.eval(st_w2),
    );
    let st_base = draw_click_world_js(1.4, 0.05, "stretch_base");
    schedule_draw_ms(
        start,
        128_500,
        "stretch base click at world (1.4, 0.05)",
        || webview.eval(st_base),
    );
    let st_delta = draw_click_world_js(1.4, 0.25, "stretch_delta");
    schedule_draw_ms(
        start,
        129_000,
        "stretch displacement click at world (1.4, 0.25)",
        || webview.eval(st_delta),
    );
    schedule_draw_ms(start, 129_500, "stretch-points cad_status", || {
        webview.eval(draw_status_js("s_stretch_points"))
    });
    schedule_draw_ms(start, 130_000, "cad_command CONFIRM (stretch)", || {
        webview.eval(draw_confirm_js("confirm_stretch"))
    });
    schedule_draw_ms(
        start,
        130_500,
        "stretch-after cad_status (count unchanged)",
        || webview.eval(draw_status_js("s_stretch_after")),
    );

    // ---- EXPLODE: closed polyline -> four LINEs (+3) ----
    schedule_draw_ms(
        start,
        131_500,
        "cad_command RECTANG (explode source)",
        || webview.eval(draw_command_js("RECTANG", "explode_rect")),
    );
    let ex_a = draw_click_world_js(2.3, 0.05, "explode_rect_a");
    schedule_draw_ms(
        start,
        132_000,
        "explode rect corner A at world (2.3, 0.05)",
        || webview.eval(ex_a),
    );
    let ex_b = draw_click_world_js(2.7, 0.15, "explode_rect_b");
    schedule_draw_ms(
        start,
        132_500,
        "explode rect corner B at world (2.7, 0.15)",
        || webview.eval(ex_b),
    );
    schedule_draw_ms(
        start,
        133_000,
        "cad_command CONFIRM (explode source)",
        || webview.eval(draw_confirm_js("confirm_explode_rect")),
    );
    schedule_draw_ms(start, 133_500, "explode-source-after cad_status", || {
        webview.eval(draw_status_js("s_explode_src_after"))
    });
    let ex_sel = draw_click_world_js(2.5, 0.05, "explode_select");
    schedule_draw_ms(
        start,
        134_000,
        "explode click-select at world (2.5, 0.05)",
        || webview.eval(ex_sel),
    );
    schedule_draw_ms(start, 134_500, "explode-selected cad_status", || {
        webview.eval(draw_status_js("s_explode_select"))
    });
    schedule_draw_ms(start, 135_000, "cad_command EXPLODE", || {
        webview.eval(draw_command_js("EXPLODE", "explode"))
    });
    schedule_draw_ms(start, 135_500, "explode-after cad_status", || {
        webview.eval(draw_status_js("s_explode_after"))
    });
}

/// Schedule the combined single-line report relative to the driver start.
///
/// It lands after every phase (default t=137 s), so the arrays and the EXPLODE
/// result are already committed; the report carries the real before/after reads
/// and the real refusals.
pub(super) fn schedule_draw_report(webview: &tauri::Webview, start: &Instant) {
    schedule_draw_ms(start, 137_000, "draw evidence report", || {
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
///
/// The P4 edit-command fields follow the same discipline: `arrayrect_delta` /
/// `arraypolar_delta` / `explode_delta` are the real `entityCount` deltas;
/// `align_delta` / `stretch_delta` are expected to be `0` (in-place transforms)
/// and the report states "accepted, no error, count unchanged" rather than
/// implying a geometry change it cannot observe.
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
    d.s_arr_src_after_failed || d.s_arr_select_failed || d.s_arrayrect_active_failed ||
    d.s_arrayrect_after_failed || d.s_pol_src_after_failed || d.s_pol_select_failed ||
    d.s_arraypolar_active_failed || d.s_arraypolar_points_failed || d.s_arraypolar_after_failed ||
    d.s_align_src_after_failed || d.s_align_select_failed || d.s_align_active_failed ||
    d.s_align_points_failed || d.s_align_after_failed || d.s_stretch_src_after_failed ||
    d.s_stretch_active_failed || d.s_stretch_points_failed || d.s_stretch_after_failed ||
    d.s_explode_src_after_failed || d.s_explode_select_failed || d.s_explode_after_failed ||
    d.confirm_a_h_failed || d.confirm_a_v_failed || d.confirm_fillet_failed ||
    d.confirm_b_h_failed || d.confirm_b_v_failed || d.confirm_chamfer_failed ||
    d.confirm_pline_failed || d.confirm_polygon_failed || d.confirm_point_failed ||
    d.confirm_rectang_failed || d.confirm_arr_src_failed || d.confirm_arrayrect_failed ||
    d.confirm_pol_src_failed || d.confirm_arraypolar_failed || d.confirm_align_src_failed ||
    d.confirm_align_failed || d.confirm_stretch_src_failed || d.confirm_stretch_failed ||
    d.confirm_explode_rect_failed;
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
    ' arrayrect_line_ack=' + v(d['arr_line_ack']) +
    ' arrayrect_rows_set_ack=' + v(d['arr_rows_ack']) +
    ' arrayrect_columns_set_ack=' + v(d['arr_cols_ack']) +
    ' arrayrect_row_spacing_set_ack=' + v(d['arr_row_sp_ack']) +
    ' arrayrect_column_spacing_set_ack=' + v(d['arr_col_sp_ack']) +
    ' arrayrect_select_count=' + v(d.s_arr_select_selection) +
    ' arrayrect_before_count=' + v(d.s_arr_src_after_count) +
    ' arrayrect_start_ack=' + v(d['arrayrect_ack']) +
    ' arrayrect_active_tool=' + v(d.s_arrayrect_active_tool) +
    ' arrayrect_active_rows=' + v(d.s_arrayrect_active_array_rows) +
    ' arrayrect_active_columns=' + v(d.s_arrayrect_active_array_columns) +
    ' arrayrect_confirm_ack=' + v(d.confirm_arrayrect_ack) +
    ' arrayrect_after_count=' + v(d.s_arrayrect_after_count) +
    ' arrayrect_delta=' + delta(d.s_arr_src_after_count, d.s_arrayrect_after_count) +
    ' arrayrect_after_tool=' + v(d.s_arrayrect_after_tool) +
    ' arrayrect_after_error=' + v(d.s_arrayrect_after_error) +
    ' arraypolar_line_ack=' + v(d['pol_line_ack']) +
    ' arraypolar_items_set_ack=' + v(d['pol_items_ack']) +
    ' arraypolar_select_count=' + v(d.s_pol_select_selection) +
    ' arraypolar_before_count=' + v(d.s_pol_src_after_count) +
    ' arraypolar_start_ack=' + v(d['arraypolar_ack']) +
    ' arraypolar_active_tool=' + v(d.s_arraypolar_active_tool) +
    ' arraypolar_active_items=' + v(d.s_arraypolar_active_array_items) +
    ' arraypolar_points=' + v(d.s_arraypolar_points_points) +
    ' arraypolar_points_ready=' + v(d.s_arraypolar_points_ready) +
    ' arraypolar_confirm_ack=' + v(d.confirm_arraypolar_ack) +
    ' arraypolar_after_count=' + v(d.s_arraypolar_after_count) +
    ' arraypolar_delta=' + delta(d.s_pol_src_after_count, d.s_arraypolar_after_count) +
    ' arraypolar_after_tool=' + v(d.s_arraypolar_after_tool) +
    ' arraypolar_after_error=' + v(d.s_arraypolar_after_error) +
    ' align_line_ack=' + v(d['align_line_ack']) +
    ' align_select_count=' + v(d.s_align_select_selection) +
    ' align_before_count=' + v(d.s_align_src_after_count) +
    ' align_start_ack=' + v(d['align_ack']) +
    ' align_active_tool=' + v(d.s_align_active_tool) +
    ' align_active_scale=' + v(d.s_align_active_align_scale) +
    ' align_points=' + v(d.s_align_points_points) +
    ' align_points_ready=' + v(d.s_align_points_ready) +
    ' align_confirm_ack=' + v(d.confirm_align_ack) +
    ' align_after_count=' + v(d.s_align_after_count) +
    ' align_delta=' + delta(d.s_align_src_after_count, d.s_align_after_count) +
    ' align_in_place_unchanged=' + (delta(d.s_align_src_after_count, d.s_align_after_count) === 0) +
    ' align_after_tool=' + v(d.s_align_after_tool) +
    ' align_after_error=' + v(d.s_align_after_error) +
    ' stretch_line_ack=' + v(d['stretch_line_ack']) +
    ' stretch_before_count=' + v(d.s_stretch_src_after_count) +
    ' stretch_start_ack=' + v(d['stretch_ack']) +
    ' stretch_active_tool=' + v(d.s_stretch_active_tool) +
    ' stretch_points=' + v(d.s_stretch_points_points) +
    ' stretch_points_ready=' + v(d.s_stretch_points_ready) +
    ' stretch_confirm_ack=' + v(d.confirm_stretch_ack) +
    ' stretch_after_count=' + v(d.s_stretch_after_count) +
    ' stretch_delta=' + delta(d.s_stretch_src_after_count, d.s_stretch_after_count) +
    ' stretch_in_place_unchanged=' + (delta(d.s_stretch_src_after_count, d.s_stretch_after_count) === 0) +
    ' stretch_after_tool=' + v(d.s_stretch_after_tool) +
    ' stretch_after_error=' + v(d.s_stretch_after_error) +
    ' explode_rect_ack=' + v(d['explode_rect_ack']) +
    ' explode_select_count=' + v(d.s_explode_select_selection) +
    ' explode_before_count=' + v(d.s_explode_src_after_count) +
    ' explode_ack=' + v(d['explode_ack']) +
    ' explode_after_count=' + v(d.s_explode_after_count) +
    ' explode_delta=' + delta(d.s_explode_src_after_count, d.s_explode_after_count) +
    ' explode_after_tool=' + v(d.s_explode_after_tool) +
    ' explode_after_error=' + v(d.s_explode_after_error) +
    ' vertex_count_channel=none_cad_status_has_no_vertex_or_entity_geometry' +
    ' fillet_geometry_channel=none_cad_status_has_no_entity_geometry' +
    ' chamfer_geometry_channel=none_cad_status_has_no_entity_geometry' +
    ' array_geometry_channel=none_cad_status_has_no_entity_geometry' +
    ' align_geometry_channel=none_cad_status_has_no_entity_geometry' +
    ' stretch_geometry_channel=none_cad_status_has_no_entity_geometry' +
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
    ' rectang_b_failed=' + v(d.rectang_b_failed) +
    ' arr_line_failed=' + v(d.arr_line_failed) +
    ' arr_src_a_failed=' + v(d.arr_src_a_failed) +
    ' arr_src_b_failed=' + v(d.arr_src_b_failed) +
    ' arr_select_failed=' + v(d.arr_select_failed) +
    ' arr_rows_failed=' + v(d.arr_rows_failed) +
    ' arr_cols_failed=' + v(d.arr_cols_failed) +
    ' arr_row_sp_failed=' + v(d.arr_row_sp_failed) +
    ' arr_col_sp_failed=' + v(d.arr_col_sp_failed) +
    ' arrayrect_failed=' + v(d.arrayrect_failed) +
    ' pol_line_failed=' + v(d.pol_line_failed) +
    ' pol_src_a_failed=' + v(d.pol_src_a_failed) +
    ' pol_src_b_failed=' + v(d.pol_src_b_failed) +
    ' pol_select_failed=' + v(d.pol_select_failed) +
    ' pol_items_failed=' + v(d.pol_items_failed) +
    ' arraypolar_failed=' + v(d.arraypolar_failed) +
    ' polar_center_failed=' + v(d.polar_center_failed) +
    ' align_line_failed=' + v(d.align_line_failed) +
    ' align_src_a_failed=' + v(d.align_src_a_failed) +
    ' align_src_b_failed=' + v(d.align_src_b_failed) +
    ' align_select_failed=' + v(d.align_select_failed) +
    ' align_failed=' + v(d.align_failed) +
    ' align_s1_failed=' + v(d.align_s1_failed) +
    ' align_d1_failed=' + v(d.align_d1_failed) +
    ' align_s2_failed=' + v(d.align_s2_failed) +
    ' align_d2_failed=' + v(d.align_d2_failed) +
    ' stretch_line_failed=' + v(d.stretch_line_failed) +
    ' stretch_src_a_failed=' + v(d.stretch_src_a_failed) +
    ' stretch_src_b_failed=' + v(d.stretch_src_b_failed) +
    ' stretch_failed=' + v(d.stretch_failed) +
    ' stretch_w1_failed=' + v(d.stretch_w1_failed) +
    ' stretch_w2_failed=' + v(d.stretch_w2_failed) +
    ' stretch_base_failed=' + v(d.stretch_base_failed) +
    ' stretch_delta_failed=' + v(d.stretch_delta_failed) +
    ' explode_rect_failed=' + v(d.explode_rect_failed) +
    ' explode_rect_a_failed=' + v(d.explode_rect_a_failed) +
    ' explode_rect_b_failed=' + v(d.explode_rect_b_failed) +
    ' explode_select_failed=' + v(d.explode_select_failed) +
    ' explode_failed=' + v(d.explode_failed);
  try { console.log('[probe] draw evidence report: ' + text); } catch (err) {}
  return text;
})()
"#;
