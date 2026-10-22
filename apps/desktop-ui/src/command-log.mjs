// Command-window echo formatting: the scrollback rendered above the command
// input.
//
// `cad_status.commandLog` is the only source: a bounded, typed and **text-free**
// array (newest last), cleared by the host when the document is replaced and
// capped at 200 entries. Nothing here reads CAD state or invents a value, a point
// or a message: every label comes from the catalogs, an error's wording is looked
// up by its stable code, and a `result` entry without a measurement renders
// nothing at all.
//
// Angle precedent: **none exists in the React chrome.** No module under
// `apps/desktop-ui/src` formats a measured angle (measurement readouts do not
// live in the JS layer), and the only degree mark in the tree is the DXF `%%d`
// control code mapped in `crates/cad-representation/src/text/mtext.rs`. Angles
// are therefore printed in degrees with the same fixed decimal count as the
// linear values plus an explicit degree sign, so an angle can never be misread as
// a length.

/** Decimal places for every measured number, matching AutoCAD's command echo. */
export const MEASUREMENT_DECIMALS = 4;

/** The degree mark: a unit symbol, not prose, so it is not a catalog value. */
const DEGREE_SIGN = '\u00B0';

/** Stable refusal codes the host emits (`cad_host::command_log::error_code`). */
export const COMMAND_LOG_ERROR_CODES = Object.freeze([
  'unsupported',
  'invalid_input',
  'not_implemented',
  'resource_missing',
  'corrupt_data',
  'gpu_failure',
  'invariant',
  'permission_denied',
  'cancelled',
  'stale_result',
]);

/** Which point ordinate each delta label reads. */
const DELTA_AXES = Object.freeze([
  [0, 'delta_x'],
  [1, 'delta_y'],
  [2, 'delta_z'],
]);

/** Precision tokens that are shown as a meta chip; `analytic` is not a claim. */
const PRECISION_KEYS = Object.freeze({
  approximate: 'command.log.precision.approximate',
  unknown: 'command.log.precision.unknown',
});

/**
 * A measured number with a fixed decimal count, or `null` when it is not a
 * finite number (the wire cannot carry one, so this is a contract violation and
 * must never render as a made-up value).
 */
export function formatMeasurementValue(value) {
  if (!Number.isFinite(value)) return null;
  // `(-0).toFixed(4)` is "-0.0000": an exact zero reads as zero.
  return (value === 0 ? 0 : value).toFixed(MEASUREMENT_DECIMALS);
}

/** The XY-plane bearing of `from` → `to`, in degrees CCW from +X, in `[0, 360)`. */
export function xyAngleDegrees(from, to) {
  const dx = to[0] - from[0];
  const dy = to[1] - from[1];
  if (!Number.isFinite(dx) || !Number.isFinite(dy)) return null;
  const degrees = (Math.atan2(dy, dx) * 180) / Math.PI;
  return (degrees + 360) % 360;
}

/** One ordinate delta of `from` → `to`, or `null` when that ordinate is absent. */
export function pointDelta(from, to, axis) {
  const delta = to[axis] - from[axis];
  return Number.isFinite(delta) ? delta : null;
}

function isPoint(point) {
  return Array.isArray(point) && Number.isFinite(point[0]) && Number.isFinite(point[1]);
}

/** The known display unit symbol, or `null` for drawing units (never invented). */
function displayUnit(unit) {
  return typeof unit === 'string' && unit.length > 0 ? unit : null;
}

/** An area's unit is squared; drawing units stay unit-less. */
function areaUnit(t, unit) {
  const symbol = displayUnit(unit);
  return symbol === null ? null : t('command.log.unit_squared', { unit: symbol });
}

function precisionLabel(t, precision) {
  if (typeof precision !== 'string' || precision === '' || precision === 'analytic') return null;
  const key = PRECISION_KEYS[precision];
  // A token this build does not know is shown as the token itself: relabelling it
  // would read as an accuracy claim nobody made.
  return key ? t(key) : precision;
}

function errorText(t, code) {
  if (typeof code !== 'string' || code === '') return t('command.log.error');
  const key = `command.log.error.${code}`;
  const text = t(key);
  // An unmapped code resolves to the key itself; the generic refusal line is
  // better copy than a machine key, and the raw code stays in the row's title.
  return text === key ? t('command.log.error') : text;
}

function distanceLines(t, measurement, formatted) {
  const unit = displayUnit(measurement.unit);
  const lines = [{
    tone: 'value',
    label: t('command.log.distance'),
    text: formatted,
    unit,
    meta: precisionLabel(t, measurement.precision),
  }];
  const points = Array.isArray(measurement.points) ? measurement.points : [];
  // The multi-line group needs the **exact** pair the host captured. Any other
  // count means there is no bearing to derive, so the value is printed alone —
  // points are never fabricated to fill the group.
  if (points.length !== 2 || !isPoint(points[0]) || !isPoint(points[1])) return lines;
  const [from, to] = points;
  const angle = xyAngleDegrees(from, to);
  if (angle !== null) {
    lines.push({
      tone: 'delta',
      label: t('command.log.angle'),
      text: `${formatMeasurementValue(angle)}${DEGREE_SIGN}`,
      unit: null,
      meta: null,
    });
  }
  for (const [axis, key] of DELTA_AXES) {
    const delta = pointDelta(from, to, axis);
    // A point that carries no such ordinate yields no delta line rather than a
    // zero the host never measured.
    if (delta === null) continue;
    lines.push({ tone: 'delta', label: t(`command.log.${key}`), text: formatMeasurementValue(delta), unit, meta: null });
  }
  return lines;
}

function measurementLines(t, measurement) {
  if (!measurement || typeof measurement !== 'object') return [];
  const { algorithm } = measurement;
  if (typeof algorithm !== 'string' || algorithm === '') return [];
  const formatted = formatMeasurementValue(measurement.value);
  if (formatted === null) return [];
  if (algorithm === 'distance') return distanceLines(t, measurement, formatted);

  const meta = precisionLabel(t, measurement.precision);
  const single = (label, text, unit) => [{ tone: 'value', label, text, unit, meta }];
  if (algorithm === 'angle') return single(t('command.log.angle'), `${formatted}${DEGREE_SIGN}`, null);
  if (algorithm === 'polyline') return single(t('command.log.length'), formatted, displayUnit(measurement.unit));
  if (algorithm === 'area') return single(t('command.log.area'), formatted, areaUnit(t, measurement.unit));
  // A future algorithm token is shown by its raw key — the properties palette's
  // rule for an unknown machine key, never an invented label.
  return single(algorithm, formatted, displayUnit(measurement.unit));
}

/**
 * One rendered echo row, or `null` when there is nothing honest to show.
 *
 * Row shapes (the component only styles them):
 *   command: `{ sequence, kind: 'command', lines: [{ tone: 'command', label, text }] }`
 *   result:  `{ sequence, kind: 'result',  lines: [{ tone, label, text, unit, meta }] }`
 *   error:   `{ sequence, kind: 'error',   lines: [{ tone: 'error', text }], code }`
 *
 * `code` is the diagnostic value: the component shows it in `title` only.
 */
export function formatCommandLogEntry(entry, t) {
  if (!entry || typeof entry !== 'object') return null;
  if (entry.kind === 'command') {
    if (typeof entry.command !== 'string' || entry.command.trim() === '') return null;
    return {
      sequence: entry.sequence,
      kind: 'command',
      lines: [{
        tone: 'command',
        label: t('command.log.prompt'),
        text: entry.command,
        unit: null,
        meta: null,
      }],
    };
  }
  if (entry.kind === 'result') {
    // A result without a measurement has no value to print: render nothing
    // rather than a placeholder that reads as a measurement.
    const lines = measurementLines(t, entry.measurement);
    if (lines.length === 0) return null;
    return { sequence: entry.sequence, kind: 'result', lines };
  }
  if (entry.kind === 'error') {
    return {
      sequence: entry.sequence,
      kind: 'error',
      lines: [{ tone: 'error', label: null, text: errorText(t, entry.code), unit: null, meta: null }],
      code: typeof entry.code === 'string' && entry.code !== '' ? entry.code : null,
    };
  }
  return null;
}

/** Every renderable row of the host log, oldest first. */
export function commandLogView(entries, t) {
  if (!Array.isArray(entries)) return [];
  const rows = [];
  for (const entry of entries) {
    const row = formatCommandLogEntry(entry, t);
    if (row) rows.push(row);
  }
  return rows;
}
