// Command-window echo formatting: the scrollback rendered above the command
// input.
//
// `cad_status.commandLog` is the only source: a bounded, typed and **text-free**
// array (newest last), cleared by the host when the document is replaced and
// capped at 200 entries. Nothing here reads CAD state or invents a value, a point
// or a message: every label comes from the catalogs, an error's wording is looked
// up by its stable code, and a `result` entry with neither a measurement nor a
// point renders nothing at all.
//
// Angle precedent: **none exists in the React chrome.** No module under
// `apps/desktop-ui/src` formats a measured angle (measurement readouts do not
// live in the JS layer), and the only degree mark in the tree is the DXF `%%d`
// control code mapped in `crates/cad-representation/src/text/mtext.rs`. Angles
// are therefore printed in degrees with the same fixed decimal count as the
// linear values plus an explicit degree sign, so an angle can never be misread as
// a length.
//
// Entries and measurements come off the host wire, so they are typed `unknown`
// and narrowed here; a row is only produced from what the entry really carries.
import type { Translator } from '../i18n.ts';

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
const DELTA_AXES: readonly (readonly [axis: number, key: string])[] = Object.freeze([
  [0, 'delta_x'],
  [1, 'delta_y'],
  [2, 'delta_z'],
]);

/**
 * Precision tokens that are shown as a meta chip; `analytic` is not a claim.
 *
 * A missing key means "this build does not know the token", which is rendered as
 * the token itself (see [`precisionLabel`]).
 */
const PRECISION_KEYS: Readonly<Record<string, string | undefined>> = Object.freeze({
  approximate: 'command.log.precision.approximate',
  unknown: 'command.log.precision.unknown',
});

/** The tone a line is styled by; the component owns the look. */
export type CommandLogTone = 'command' | 'value' | 'delta' | 'error';

/** One rendered line of a row. Only the fields the tone uses are non-null. */
export interface CommandLogLine {
  readonly tone: CommandLogTone;
  readonly label: string | null;
  readonly text: string | null;
  readonly unit: string | null;
  readonly meta: string | null;
}

/** The host's own row counter. */
export type CommandLogSequence = number | string | null;

/** One echoed command line. */
export interface CommandLogCommandRow {
  readonly sequence: CommandLogSequence;
  readonly kind: 'command';
  readonly lines: readonly CommandLogLine[];
}

/** One measurement result, already split into its value/delta lines. */
export interface CommandLogResultRow {
  readonly sequence: CommandLogSequence;
  readonly kind: 'result';
  readonly lines: readonly CommandLogLine[];
}

/** One refused command; `code` is the diagnostic shown in `title` only. */
export interface CommandLogErrorRow {
  readonly sequence: CommandLogSequence;
  readonly kind: 'error';
  readonly lines: readonly CommandLogLine[];
  readonly code: string | null;
}

/** A rendered row of the scrollback. */
export type CommandLogRow = CommandLogCommandRow | CommandLogResultRow | CommandLogErrorRow;

/** A point as the host reports it: at least two finite ordinates. */
export type LogPoint = readonly number[];

/**
 * A measured number with a fixed decimal count, or `null` when it is not a
 * finite number (the wire cannot carry one, so this is a contract violation and
 * must never render as a made-up value).
 */
export function formatMeasurementValue(value: unknown): string | null {
  if (typeof value !== 'number' || !Number.isFinite(value)) return null;
  // `(-0).toFixed(4)` is "-0.0000": an exact zero reads as zero.
  return (value === 0 ? 0 : value).toFixed(MEASUREMENT_DECIMALS);
}

/** The XY-plane bearing of `from` → `to`, in degrees CCW from +X, in `[0, 360)`. */
export function xyAngleDegrees(from: LogPoint, to: LogPoint): number | null {
  const dx = to[0] - from[0];
  const dy = to[1] - from[1];
  if (!Number.isFinite(dx) || !Number.isFinite(dy)) return null;
  const degrees = (Math.atan2(dy, dx) * 180) / Math.PI;
  return (degrees + 360) % 360;
}

/** One ordinate delta of `from` → `to`, or `null` when that ordinate is absent. */
export function pointDelta(from: LogPoint, to: LogPoint, axis: number): number | null {
  const delta = to[axis] - from[axis];
  return Number.isFinite(delta) ? delta : null;
}

function isPoint(point: unknown): point is LogPoint {
  return Array.isArray(point) && Number.isFinite(point[0]) && Number.isFinite(point[1]);
}

/** The known display unit symbol, or `null` for drawing units (never invented). */
function displayUnit(unit: unknown): string | null {
  return typeof unit === 'string' && unit.length > 0 ? unit : null;
}

/** An area's unit is squared; drawing units stay unit-less. */
function areaUnit(t: Translator, unit: unknown): string | null {
  const symbol = displayUnit(unit);
  return symbol === null ? null : t('command.log.unit_squared', { unit: symbol });
}

function precisionLabel(t: Translator, precision: unknown): string | null {
  if (typeof precision !== 'string' || precision === '' || precision === 'analytic') return null;
  const key = PRECISION_KEYS[precision];
  // A token this build does not know is shown as the token itself: relabelling it
  // would read as an accuracy claim nobody made.
  return key ? t(key) : precision;
}

function errorText(t: Translator, code: unknown): string {
  if (typeof code !== 'string' || code === '') return t('command.log.error');
  const key = `command.log.error.${code}`;
  const text = t(key);
  // An unmapped code resolves to the key itself; the generic refusal line is
  // better copy than a machine key, and the raw code stays in the row's title.
  return text === key ? t('command.log.error') : text;
}

/** The host's row counter when it is a usable scalar (the wire always sends one). */
function sequenceOf(entry: Record<string, unknown>): CommandLogSequence {
  const value = entry.sequence;
  return typeof value === 'number' || typeof value === 'string' ? value : null;
}

function distanceLines(t: Translator, measurement: Record<string, unknown>, formatted: string): CommandLogLine[] {
  const unit = displayUnit(measurement.unit);
  const lines: CommandLogLine[] = [{
    tone: 'value',
    label: t('command.log.distance'),
    text: formatted,
    unit,
    meta: precisionLabel(t, measurement.precision),
  }];
  const points = Array.isArray(measurement.points) ? measurement.points : [];
  const from = points[0];
  const to = points[1];
  // The multi-line group needs the **exact** pair the host captured. Any other
  // count means there is no bearing to derive, so the value is printed alone —
  // points are never fabricated to fill the group.
  if (points.length !== 2 || !isPoint(from) || !isPoint(to)) return lines;
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

function measurementLines(t: Translator, measurement: unknown): CommandLogLine[] {
  if (!measurement || typeof measurement !== 'object') return [];
  const record = measurement as Record<string, unknown>;
  const { algorithm } = record;
  if (typeof algorithm !== 'string' || algorithm === '') return [];
  const formatted = formatMeasurementValue(record.value);
  if (formatted === null) return [];
  if (algorithm === 'distance') return distanceLines(t, record, formatted);

  const meta = precisionLabel(t, record.precision);
  const single = (label: string, text: string, unit: string | null): CommandLogLine[] => [
    { tone: 'value', label, text, unit, meta },
  ];
  if (algorithm === 'angle') return single(t('command.log.angle'), `${formatted}${DEGREE_SIGN}`, null);
  if (algorithm === 'polyline') return single(t('command.log.length'), formatted, displayUnit(record.unit));
  if (algorithm === 'area') return single(t('command.log.area'), formatted, areaUnit(t, record.unit));
  // A future algorithm token is shown by its raw key — the properties palette's
  // rule for an unknown machine key, never an invented label.
  return single(algorithm, formatted, displayUnit(record.unit));
}

/**
 * The two drawing spaces a point can be reported in, with their own labels.
 *
 * A token outside this set is rendered as the token itself (see [`tokenLabel`]):
 * the same rule the algorithm and precision tokens follow, so an unreleased space
 * can never be relabelled into a claim about where the point lives.
 */
const SPACE_KEYS: Readonly<Record<string, string | undefined>> = Object.freeze({
  model: 'command.log.space.model',
  paper: 'command.log.space.paper',
});

/**
 * The object snaps a point result can report, with their own labels.
 *
 * These are the **command-window** spellings, deliberately separate from the
 * status bar's `snap.kind.*` labels: the echo names the snap that produced this
 * stored coordinate ("the point you asked for was an endpoint"), which is a result
 * fact, not the live candidate hint. An unknown snap token is rendered raw.
 */
const POINT_SNAP_KEYS: Readonly<Record<string, string | undefined>> = Object.freeze({
  endpoint: 'command.log.snap.endpoint',
  midpoint: 'command.log.snap.midpoint',
  center: 'command.log.snap.center',
  quadrant: 'command.log.snap.quadrant',
  perpendicular: 'command.log.snap.perpendicular',
  intersection: 'command.log.snap.intersection',
});

/**
 * A machine token's label, or the token itself, or `null` when there is none.
 *
 * A token this build does not know is printed verbatim: relabelling it would
 * invent a fact, and dropping it would hide one the host reported.
 */
function tokenLabel(
  t: Translator,
  keys: Readonly<Record<string, string | undefined>>,
  token: unknown,
): string | null {
  if (typeof token !== 'string' || token === '') return null;
  const key = keys[token];
  return key ? t(key) : token;
}

/**
 * The value line of a reported **point** (`entry.point`), or nothing.
 *
 * ID echoes the coordinates of one picked point: the wire carries
 * `{ position, space, snap, precision, unit, pixelDistance }`. The line prints the
 * three ordinates with the same fixed decimal count every measurement uses,
 * joined by `", "` (AutoCAD's own `X, Y, Z` shape), the drawing unit when the host
 * reported one, and a meta chip naming the space and — when there was one — the
 * object snap that produced the point.
 *
 * Nothing is invented: an ordinate that is not a finite number ends the run rather
 * than becoming a zero, and a point with fewer than two usable ordinates produces
 * no line at all (a coordinate needs at least X and Y to mean anything). The meta
 * chip names the space, the object snap when there was one, and the precision
 * token when the source geometry is not analytic (`analytic` prints nothing) — a
 * coordinate picked off sampled geometry says so, exactly like a measurement row.
 */
function pointLines(t: Translator, point: unknown): CommandLogLine[] {
  if (!point || typeof point !== 'object') return [];
  const record = point as Record<string, unknown>;
  const position = Array.isArray(record.position) ? record.position : [];
  const ordinates: string[] = [];
  for (const value of position.slice(0, 3)) {
    const formatted = formatMeasurementValue(value);
    if (formatted === null) break;
    ordinates.push(formatted);
  }
  if (ordinates.length < 2) return [];
  const meta = [
    tokenLabel(t, SPACE_KEYS, record.space),
    tokenLabel(t, POINT_SNAP_KEYS, record.snap),
    precisionLabel(t, record.precision),
  ].filter((part): part is string => part !== null).join(' \u00b7 ');
  return [{
    tone: 'value',
    label: t('command.log.point'),
    text: ordinates.join(', '),
    unit: displayUnit(record.unit),
    meta: meta === '' ? null : meta,
  }];
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
export function formatCommandLogEntry(entry: unknown, t: Translator): CommandLogRow | null {
  if (!entry || typeof entry !== 'object') return null;
  const record = entry as Record<string, unknown>;
  if (record.kind === 'command') {
    const command = record.command;
    if (typeof command !== 'string' || command.trim() === '') return null;
    return {
      sequence: sequenceOf(record),
      kind: 'command',
      lines: [{
        tone: 'command',
        label: t('command.log.prompt'),
        text: command,
        unit: null,
        meta: null,
      }],
    };
  }
  if (record.kind === 'result') {
    // A result with neither a measurement nor a point has nothing to print:
    // render nothing rather than a placeholder that reads as a measurement. A
    // result that carries both (a command that measures *and* reports a point)
    // prints the measurement group first, then the point line.
    const lines = [...measurementLines(t, record.measurement), ...pointLines(t, record.point)];
    if (lines.length === 0) return null;
    return { sequence: sequenceOf(record), kind: 'result', lines };
  }
  if (record.kind === 'error') {
    return {
      sequence: sequenceOf(record),
      kind: 'error',
      lines: [{ tone: 'error', label: null, text: errorText(t, record.code), unit: null, meta: null }],
      code: typeof record.code === 'string' && record.code !== '' ? record.code : null,
    };
  }
  return null;
}

/** Every renderable row of the host log, oldest first. */
export function commandLogView(entries: unknown, t: Translator): CommandLogRow[] {
  if (!Array.isArray(entries)) return [];
  const rows: CommandLogRow[] = [];
  for (const entry of entries) {
    const row = formatCommandLogEntry(entry, t);
    if (row) rows.push(row);
  }
  return rows;
}
