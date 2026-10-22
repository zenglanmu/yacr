//! `YACR_PROBE_DRAW_EVIDENCE` synthetic DOM **scripts**.
//!
//! Split out of [`super::draw`] to stay under the 1000-line code-file gate
//! (`scripts/check-file-size.py`, no whitelist). This module holds the JS
//! templates and their builder functions only; the phase order, timing and
//! expected `cad_status` observations live in [`super::draw`], which re-exports
//! these builders so existing `super::draw::…` call sites are unchanged.
//!
//! Everything here is evidence-only, default OFF (gated by
//! `YACR_PROBE_DRAW_EVIDENCE`) and dispatches the real `cad_command` /
//! `cad_status` funnels on `.native-input`; synthetic input, **not a real
//! mouse**.

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
