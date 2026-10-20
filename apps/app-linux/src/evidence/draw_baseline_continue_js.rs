//! `YACR_PROBE_DRAW_EVIDENCE` P5f baseline/continue **scripts**.
//!
//! Split out of [`super::draw_baseline_continue`] for the same reason as every
//! other evidence split: each file stays well under the 1000-line code-file gate
//! (`scripts/check-file-size.py`, no whitelist). This module holds the synthetic
//! DOM builder functions and their JS templates only; the phase itself (what is
//! dispatched, in which order, and what each step expects) lives in
//! [`super::draw_baseline_continue`].
//!
//! Everything here is evidence-only, default OFF (gated by
//! `YACR_PROBE_DRAW_EVIDENCE`) and dispatches the **real** `cad_command` /
//! `cad_status` funnels on `.native-input`; synthetic input, **not a real
//! mouse**.

/// Build the phase summary script (the combined report appends its fragment).
pub(super) fn bc_summary_js() -> String {
    BC_SUMMARY_TEMPLATE.to_string()
}

/// Build the named status read for this phase (`s_bc_bl_pick`, ...).
///
/// `base` is the read the delta is derived against (`s_bc_bl_start` for the
/// baseline's post-commit read), so the pass criterion compares the real
/// `entityCount` before and after the commit instead of a hard-coded total.
pub(super) fn bc_status_js(name: &str, base: &str) -> String {
    BC_STATUS_TEMPLATE
        .replace("__NAME__", name)
        .replace("__BASE__", base)
}

/// One labeled `cad_command` dispatch, skipped once the phase has aborted.
pub(super) fn bc_command_js(command: &str, label: &str) -> String {
    BC_COMMAND_TEMPLATE
        .replace("__COMMAND__", command)
        .replace("__LABEL__", label)
}

/// One synthetic click at a **logical-pixel** position on the CAD surface.
///
/// Same diagnostics as the P7c click (`*_frac`, `*_px`, `*_dispatched`,
/// `*_failed`, `d.clicks`); the position is scaled from the nominal surface onto
/// the live `.native-input` rect. Synthetic, **not a real mouse**.
pub(super) fn bc_click_px_js(x: f64, y: f64, label: &str) -> String {
    BC_CLICK_TEMPLATE
        .replace("__X__", &x.to_string())
        .replace("__Y__", &y.to_string())
        .replace("__LABEL__", label)
}

/// Build a click-free pointer move at a **logical-pixel** position.
pub(super) fn bc_move_px_js(x: f64, y: f64, label: &str) -> String {
    BC_MOVE_TEMPLATE
        .replace("__X__", &x.to_string())
        .replace("__Y__", &y.to_string())
        .replace("__LABEL__", label)
}

/// Build one expectation gate over the recorded reads.
///
/// `checks` are `(recorded key, JS literal expected value)` pairs compared with
/// `!==` against `window.__yacr_draw[key]`. The first mismatch sets
/// `baseline_continue_abort` and logs the reason immediately; every later
/// state-changing dispatch checks that flag and skips.
pub(super) fn bc_expect_js(label: &str, checks: &[(&str, &str)]) -> String {
    let list = checks
        .iter()
        .map(|(key, value)| format!("['{key}', {value}]"))
        .collect::<Vec<_>>()
        .join(",");
    BC_EXPECT_TEMPLATE
        .replace("__LABEL__", label)
        .replace("__CHECKS__", &list)
}

/// Read the real `cad_status` and record the fields this phase compares.
///
/// `*_delta` is only a number when both `entityCount` reads exist; a missing side
/// leaves it `null` (reported as `-`), never a fabricated zero.
pub(super) const BC_STATUS_TEMPLATE: &str = r#"
(function () {
  var d = window.__yacr_draw = window.__yacr_draw || {};
  var name = '__NAME__';
  var base = '__BASE__';
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
      d[name + '_selection'] = s.selection ? s.selection.count : null;
      d[name + '_rows'] = (s.selection && s.selection.rows)
        ? s.selection.rows.map(function (r) { return String(r.key) + ':' + String(r.value); })
        : [];
      d[name + '_error'] = s.error;
      d[name + '_delta'] =
        (typeof d[base + '_count'] === 'number' && typeof s.entityCount === 'number')
          ? (s.entityCount - d[base + '_count'])
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

/// Dispatch any real selector through the public `cad_command` command,
/// recording the ack under `__LABEL__` and skipped once the phase has aborted.
pub(super) const BC_COMMAND_TEMPLATE: &str = r#"
(function () {
  var d = window.__yacr_draw = window.__yacr_draw || {};
  var label = '__LABEL__';
  if (d.baseline_continue_abort) { d[label + '_skipped'] = d.baseline_continue_abort; return 'aborted'; }
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

/// One synthetic click at a logical-pixel position, skipped once the phase has
/// aborted. Synthetic, **not a real mouse**.
pub(super) const BC_CLICK_TEMPLATE: &str = r#"
(function () {
  var d = window.__yacr_draw = window.__yacr_draw || {};
  var label = '__LABEL__';
  if (d.baseline_continue_abort) { d[label + '_skipped'] = d.baseline_continue_abort; return 'aborted'; }
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
/// Unlike the click/command steps this one **keeps running after an abort**: a
/// `pointermove` captures no point, changes no selection and completes nothing,
/// so it cannot fake a failed step, and parking the crosshair keeps the captured
/// frame readable. Synthetic, **not a real mouse**.
pub(super) const BC_MOVE_TEMPLATE: &str = r#"
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

/// One expectation gate over the recorded reads; the first mismatch aborts the
/// phase and logs it at once in the established single-line shape.
pub(super) const BC_EXPECT_TEMPLATE: &str = r#"
(function () {
  var d = window.__yacr_draw = window.__yacr_draw || {};
  var label = '__LABEL__';
  if (d.baseline_continue_abort) { d[label + '_skipped'] = d.baseline_continue_abort; return 'aborted'; }
  var checks = [__CHECKS__];
  for (var i = 0; i < checks.length; i++) {
    if (d[checks[i][0]] !== checks[i][1]) {
      d.baseline_continue_abort = label + ' expected=' + String(checks[i][1]) +
        ' got=' + String(d[checks[i][0]]);
      try {
        console.log('[probe] draw evidence baseline_continue abort: ' + d.baseline_continue_abort);
      } catch (err) {}
      return 'aborted';
    }
  }
  d[label + '_ok'] = true;
  return 'ok';
})()
"#;

/// The baseline/continue report fragment: one single-line, whitespace-free field
/// list built from the **real** `cad_status` reads the phase recorded in
/// `window.__yacr_draw`, stored in `window.__yacr_baseline_continue_summary` so
/// the combined report in [`super::draw_edit`] can append it without duplicating
/// the reads (the P6b/P7c precedent), and logged on its own as
/// `[probe] draw evidence baseline_continue summary:`.
///
/// `-` means "absent / never read"; `-1` a count or delta that could not be
/// computed because one side of the read is missing. `bc_base_created`,
/// `bc_baseline_ok`, `bc_continue_ok` and `bc_pass` are **derived** from the
/// reads (never asserted by omission):
///
/// * `bc_base_created`: the base `DIMLINEAR` added exactly one entity, the tool
///   cleared and `error` is `null`;
/// * `bc_baseline_ok`: the base pick advanced the capture (`points` 0 → 1), the
///   three measured points landed (`points` 4), the commit added exactly three
///   entities, the tool cleared and `error` is `null`;
/// * `bc_continue_ok`: the same shape for the chained run;
/// * `bc_pass`: all three plus no abort and no missing gate.
///
/// `bc_baseline_points`/`bc_continue_points` are the **measured** points after
/// the base pick (`tool.points − 1`); `*_tool_points` is the raw status value,
/// which includes the base pick's own point. The trailing channel fields are the
/// honest statement that `cad_status` exposes neither per-entity geometry nor
/// `DIMDLI`, so the stacked rows' placement is not asserted here.
pub(super) const BC_SUMMARY_TEMPLATE: &str = r#"
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
  function delta(before_key, after_key) {
    var before = n(d[before_key + '_count']);
    var after = n(d[after_key + '_count']);
    return (before >= 0 && after >= 0) ? (after - before) : -1;
  }
  function measured(points_key) {
    var points = n(d[points_key + '_points']);
    return (points >= 0) ? (points - 1) : -1;
  }
  var reads = ['s_bc_base_start', 's_bc_o1', 's_bc_o2', 's_bc_loc', 's_bc_base_after',
    's_bc_bl_start', 's_bc_bl_pick', 's_bc_bl_p1', 's_bc_bl_p2', 's_bc_bl_p3',
    's_bc_bl_after', 's_bc_ct_start', 's_bc_ct_pick', 's_bc_ct_p1', 's_bc_ct_p2',
    's_bc_ct_p3', 's_bc_ct_after', 's_bc_select'];
  var missing = [];
  for (var i = 0; i < reads.length; i++) {
    if (!d[reads[i] + '_done']) { missing.push(reads[i]); }
  }
  var gates = ['bc_gate_base_start', 'bc_gate_o1', 'bc_gate_o2', 'bc_gate_loc',
    'bc_gate_base_after', 'bc_gate_bl_start', 'bc_gate_bl_pick', 'bc_gate_bl_p1',
    'bc_gate_bl_p2', 'bc_gate_bl_p3', 'bc_gate_bl_after', 'bc_gate_ct_start',
    'bc_gate_ct_pick', 'bc_gate_ct_p1', 'bc_gate_ct_p2', 'bc_gate_ct_p3',
    'bc_gate_ct_after', 'bc_gate_select'];
  var gates_missing = [];
  for (var g = 0; g < gates.length; g++) {
    if (!d[gates[g] + '_ok']) { gates_missing.push(gates[g]); }
  }
  var failed = [];
  for (var key in d) {
    if (/_failed$/.test(key) && d[key] && /^(bc_|s_bc_)/.test(key)) {
      failed.push(key + '=' + d[key]);
    }
  }
  var base_delta = delta('s_bc_base_start', 's_bc_base_after');
  var base_created = (base_delta === 1 && d.s_bc_base_after_tool === null &&
    d.s_bc_base_after_error === null) ? 1 : 0;
  var bl_delta = delta('s_bc_bl_start', 's_bc_bl_after');
  var bl_pick_advanced = (d.s_bc_bl_start_points === 0 && d.s_bc_bl_pick_points === 1) ? 1 : 0;
  var bl_points = measured('s_bc_bl_p3');
  var bl_ok = (bl_pick_advanced === 1 && bl_points === 3 && bl_delta === 3 &&
    d.s_bc_bl_after_tool === null && d.s_bc_bl_after_error === null) ? 1 : 0;
  var ct_delta = delta('s_bc_ct_start', 's_bc_ct_after');
  var ct_pick_advanced = (d.s_bc_ct_start_points === 0 && d.s_bc_ct_pick_points === 1) ? 1 : 0;
  var ct_points = measured('s_bc_ct_p3');
  var ct_ok = (ct_pick_advanced === 1 && ct_points === 3 && ct_delta === 3 &&
    d.s_bc_ct_after_tool === null && d.s_bc_ct_after_error === null) ? 1 : 0;
  var abort = (typeof d.baseline_continue_abort === 'undefined') ? null : d.baseline_continue_abort;
  var pass = (abort === null && gates_missing.length === 0 && base_created === 1 &&
    bl_ok === 1 && ct_ok === 1) ? 1 : 0;
  var text =
    'bc_synthetic=1_synthetic_dom_pointer_events_not_a_real_mouse' +
    ' bc_input=synthetic_dom_pointer_events_not_a_real_mouse' +
    ' bc_camera=' + arr(d.s_bc_base_start_camera) +
    ' bc_status_missing=' + (missing.length ? missing.join(',') : '-') +
    ' bc_gate_missing=' + (gates_missing.length ? gates_missing.join(',') : '-') +
    ' bc_step_failed=' + (failed.length ? failed.join(',') : '-') +
    ' bc_aborted_at=' + v(abort) +
    ' bc_origin1_px=' + arr(d.bc_origin1_px) +
    ' bc_origin1_frac=' + arr(d.bc_origin1_frac) +
    ' bc_origin2_px=' + arr(d.bc_origin2_px) +
    ' bc_origin2_frac=' + arr(d.bc_origin2_frac) +
    ' bc_location_px=' + arr(d.bc_location_px) +
    ' bc_location_frac=' + arr(d.bc_location_frac) +
    ' bc_base_before_count=' + v(d.s_bc_base_start_count) +
    ' bc_base_after_count=' + v(d.s_bc_base_after_count) +
    ' bc_base_delta=' + base_delta +
    ' bc_base_after_tool=' + v(d.s_bc_base_after_tool) +
    ' bc_base_after_error=' + v(d.s_bc_base_after_error) +
    ' bc_base_created=' + base_created +
    ' bc_base_pick_px=' + arr(d.bc_baseline_pick_px) +
    ' bc_base_pick_frac=' + arr(d.bc_baseline_pick_frac) +
    ' bc_baseline_pick=' + v(d.s_bc_bl_pick_points) +
    ' bc_baseline_pick_advanced=' + bl_pick_advanced +
    ' bc_baseline_pick_error=' + v(d.s_bc_bl_pick_error) +
    ' bc_baseline_point1_px=' + arr(d.bc_bl_p1_px) +
    ' bc_baseline_point2_px=' + arr(d.bc_bl_p2_px) +
    ' bc_baseline_point3_px=' + arr(d.bc_bl_p3_px) +
    ' bc_baseline_tool_points=' + v(d.s_bc_bl_p3_points) +
    ' bc_baseline_points=' + bl_points +
    ' bc_baseline_ready=' + v(d.s_bc_bl_p3_ready) +
    ' bc_baseline_before_count=' + v(d.s_bc_bl_start_count) +
    ' bc_baseline_after_count=' + v(d.s_bc_bl_after_count) +
    ' bc_baseline_delta=' + bl_delta +
    ' bc_baseline_after_tool=' + v(d.s_bc_bl_after_tool) +
    ' bc_baseline_after_error=' + v(d.s_bc_bl_after_error) +
    ' bc_baseline_ok=' + bl_ok +
    ' bc_continue_pick_px=' + arr(d.bc_continue_pick_px) +
    ' bc_continue_pick_frac=' + arr(d.bc_continue_pick_frac) +
    ' bc_continue_pick=' + v(d.s_bc_ct_pick_points) +
    ' bc_continue_pick_advanced=' + ct_pick_advanced +
    ' bc_continue_pick_error=' + v(d.s_bc_ct_pick_error) +
    ' bc_continue_point1_px=' + arr(d.bc_ct_p1_px) +
    ' bc_continue_point2_px=' + arr(d.bc_ct_p2_px) +
    ' bc_continue_point3_px=' + arr(d.bc_ct_p3_px) +
    ' bc_continue_tool_points=' + v(d.s_bc_ct_p3_points) +
    ' bc_continue_points=' + ct_points +
    ' bc_continue_ready=' + v(d.s_bc_ct_p3_ready) +
    ' bc_continue_before_count=' + v(d.s_bc_ct_start_count) +
    ' bc_continue_after_count=' + v(d.s_bc_ct_after_count) +
    ' bc_continue_delta=' + ct_delta +
    ' bc_continue_after_tool=' + v(d.s_bc_ct_after_tool) +
    ' bc_continue_after_error=' + v(d.s_bc_ct_after_error) +
    ' bc_continue_ok=' + ct_ok +
    ' bc_select_px=' + arr(d.bc_select_px) +
    ' bc_select_count=' + v(d.s_bc_select_selection) +
    ' bc_select_rows=' + arr(d.s_bc_select_rows) +
    ' bc_park_px=' + arr(d.bc_park_px) +
    ' bc_pass=' + pass +
    ' bc_geometry_channel=none_cad_status_has_no_per_entity_geometry_no_vertex_or_row_list' +
    ' bc_row_spacing_channel=none_cad_status_has_no_dimli_so_the_stacked_rows_are_not_asserted' +
    ' bc_select_note=informational_real_rows_of_the_picked_created_dimension_not_a_pass_criterion';
  try { window.__yacr_baseline_continue_summary = ' ' + text; } catch (err) {}
  try { console.log('[probe] draw evidence baseline_continue summary: ' + text); } catch (err) {}
  return text;
})()
"#;
