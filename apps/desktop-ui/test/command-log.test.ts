// Command-window echo contract: the pure formatting of every typed result the
// host can emit, the honest cases (no measurement, no points, no unit), the
// localized refusal lines and the rendered structure of the scrollback.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  COMMAND_LOG_ERROR_CODES,
  MEASUREMENT_DECIMALS,
  commandLogView,
  formatCommandLogEntry,
  formatMeasurementValue,
  xyAngleDegrees,
} from '../src/commands/command-log.ts';
import { createTranslator } from '../src/i18n.ts';
import { readCatalog, readCss, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');
const t = createTranslator({ 'zh-CN': zh, en }, 'en');
const tzh = createTranslator({ 'zh-CN': zh, en }, 'zh-CN');

const CATALOG_KEYS = [
  'command.log.label',
  'command.log.prompt',
  'command.log.distance',
  'command.log.angle',
  'command.log.delta_x',
  'command.log.delta_y',
  'command.log.delta_z',
  'command.log.length',
  'command.log.area',
  // ID's point row (2026-10-26): the coordinate label, the two space labels and
  // the six snap labels the echo can name.
  'command.log.point',
  'command.log.space.model',
  'command.log.space.paper',
  'command.log.snap.endpoint',
  'command.log.snap.midpoint',
  'command.log.snap.center',
  'command.log.snap.quadrant',
  'command.log.snap.perpendicular',
  'command.log.snap.intersection',
  'command.log.unit_squared',
  'command.log.precision.approximate',
  'command.log.precision.unknown',
  'command.log.error',
  ...COMMAND_LOG_ERROR_CODES.map(code => `command.log.error.${code}`),
];

function measurement(overrides = {}) {
  return {
    algorithm: 'distance',
    value: 5,
    unit: null,
    precision: 'analytic',
    points: [[1, 2, 0], [4, 6, 0]],
    ...overrides,
  };
}

function result(overrides = {}) {
  return { sequence: 2, kind: 'result', command: null, code: null, measurement: measurement(), ...overrides };
}

// The frozen point shape an ID result carries (`cad_host`'s command log): the
// picked position, the space it belongs to, the object snap that produced it, the
// precision token, the drawing unit and the screen distance (unused here).
function POINT(overrides = {}) {
  return {
    position: [12.5, -3, 0],
    space: 'model',
    snap: 'endpoint',
    precision: 'analytic',
    unit: null,
    pixelDistance: 41,
    ...overrides,
  };
}

function pointResult(overrides = {}) {
  return {
    sequence: 4,
    kind: 'result',
    command: null,
    code: null,
    measurement: null,
    point: POINT(),
    ...overrides,
  };
}

test('a measured number always carries the same fixed decimal count', () => {
  assert.equal(MEASUREMENT_DECIMALS, 4);
  assert.equal(formatMeasurementValue(5), '5.0000');
  assert.equal(formatMeasurementValue(1 / 3), '0.3333');
  assert.equal(formatMeasurementValue(-2.5), '-2.5000');
  // An exact zero never reads as a negative number.
  assert.equal(formatMeasurementValue(-0), '0.0000');
  // Not a finite number: no value is invented.
  for (const value of [null, undefined, Number.NaN, Number.POSITIVE_INFINITY, '5']) {
    assert.equal(formatMeasurementValue(value), null);
  }
});

test('the XY angle is a CCW bearing from +X and agrees with atan2', () => {
  assert.equal(xyAngleDegrees([0, 0], [1, 0]), 0);
  assert.equal(xyAngleDegrees([0, 0], [0, 1]), 90);
  assert.equal(xyAngleDegrees([0, 0], [-1, 0]), 180);
  assert.equal(xyAngleDegrees([0, 0], [0, -1]), 270);
  assert.equal(formatMeasurementValue(xyAngleDegrees([1, 2], [4, 6])), '53.1301');
});

test('a two-point distance echoes the AutoCAD group with computed angle and deltas', () => {
  const row = formatCommandLogEntry(result(), t);
  assert.equal(row.kind, 'result');
  assert.equal(row.sequence, 2);
  assert.deepEqual(row.lines, [
    { tone: 'value', label: en['command.log.distance'], text: '5.0000', unit: null, meta: null },
    { tone: 'delta', label: en['command.log.angle'], text: '53.1301\u00B0', unit: null, meta: null },
    { tone: 'delta', label: en['command.log.delta_x'], text: '3.0000', unit: null, meta: null },
    { tone: 'delta', label: en['command.log.delta_y'], text: '4.0000', unit: null, meta: null },
    { tone: 'delta', label: en['command.log.delta_z'], text: '0.0000', unit: null, meta: null },
  ]);
});

test('the same group reads in the active language', () => {
  const row = formatCommandLogEntry(result(), tzh);
  assert.deepEqual(
    row.lines.map(line => line.label),
    [zh['command.log.distance'], zh['command.log.angle'], zh['command.log.delta_x'],
      zh['command.log.delta_y'], zh['command.log.delta_z']],
  );
});

test('a distance without exactly two usable points prints the value alone', () => {
  const alone = { tone: 'value', label: en['command.log.distance'], text: '5.0000', unit: null, meta: null };
  for (const points of [[], [[1, 2, 0]], [[1, 2, 0], [4, 6, 0], [7, 8, 0]], [[1, 2, 0], [null, 6, 0]]]) {
    const row = formatCommandLogEntry(result({ measurement: measurement({ points }) }), t);
    assert.deepEqual(row.lines, [alone], `points ${JSON.stringify(points)} must not gain a fabricated group`);
  }
});

test('a known unit is shown and drawing units (null) stay unit-less', () => {
  const metric = formatCommandLogEntry(result({ measurement: measurement({ unit: 'mm' }) }), t);
  assert.equal(metric.lines[0].unit, 'mm');
  assert.equal(metric.lines[2].unit, 'mm', 'the deltas carry the same unit as the distance');
  const drawing = formatCommandLogEntry(result({ measurement: measurement({ unit: null }) }), t);
  assert.equal(drawing.lines[0].unit, null);
});

test('polyline, angle and area each print their own labelled line', () => {
  const polyline = formatCommandLogEntry(
    result({ measurement: measurement({ algorithm: 'polyline', value: 10, points: [[0, 0, 0], [10, 0, 0]] }) }),
    t,
  );
  assert.deepEqual(polyline.lines, [
    { tone: 'value', label: en['command.log.length'], text: '10.0000', unit: null, meta: null },
  ]);

  // The backend's Angle3Points value is already in degrees, so it is printed with
  // the degree mark and no unit context.
  const angle = formatCommandLogEntry(
    result({ measurement: measurement({ algorithm: 'angle', value: 90, points: [[1, 0, 0], [0, 0, 0], [0, 1, 0]] }) }),
    t,
  );
  assert.deepEqual(angle.lines, [
    { tone: 'value', label: en['command.log.angle'], text: '90.0000\u00B0', unit: null, meta: null },
  ]);

  // An area's unit is squared; drawing units stay absent.
  const area = formatCommandLogEntry(
    result({ measurement: measurement({ algorithm: 'area', value: 12.5, unit: 'mm', points: [[0, 0, 0], [5, 0, 0], [0, 2, 0]] }) }),
    t,
  );
  assert.equal(area.lines[0].label, en['command.log.area']);
  assert.equal(area.lines[0].text, '12.5000');
  assert.equal(area.lines[0].unit, en['command.log.unit_squared'].replace('{unit}', 'mm'));
  const areaInDrawingUnits = formatCommandLogEntry(
    result({ measurement: measurement({ algorithm: 'area', value: 12.5, unit: null }) }),
    t,
  );
  assert.equal(areaInDrawingUnits.lines[0].unit, null);
});

test('an unknown algorithm token is shown by its raw key, never a fabricated label', () => {
  const row = formatCommandLogEntry(
    result({ measurement: measurement({ algorithm: 'volume', value: 3, points: [] }) }),
    t,
  );
  assert.equal(row.lines[0].label, 'volume');
  assert.equal(row.lines[0].text, '3.0000');
});

test('the precision token becomes a meta chip only when it is not analytic', () => {
  assert.equal(formatCommandLogEntry(result(), t).lines[0].meta, null);
  const approximate = formatCommandLogEntry(
    result({ measurement: measurement({ precision: 'approximate' }) }),
    t,
  );
  assert.equal(approximate.lines[0].meta, en['command.log.precision.approximate']);
  assert.equal(approximate.lines[1].meta, null, 'the chip belongs to the value line only');
  const unknown = formatCommandLogEntry(result({ measurement: measurement({ precision: 'unknown' }) }), t);
  assert.equal(unknown.lines[0].meta, en['command.log.precision.unknown']);
  // A token this build does not know is shown as itself, never relabelled.
  const future = formatCommandLogEntry(result({ measurement: measurement({ precision: 'partial' }) }), t);
  assert.equal(future.lines[0].meta, 'partial');
});

test('a result without a measurement renders nothing instead of a fake value', () => {
  assert.equal(formatCommandLogEntry(result({ measurement: null }), t), null);
  const rows = commandLogView([
    { sequence: 1, kind: 'command', command: 'DIST', code: null, measurement: null },
    result({ sequence: 2, measurement: null }),
    { sequence: 3, kind: 'command', command: 'LINE', code: null, measurement: null },
  ], t);
  assert.deepEqual(rows.map(row => row.sequence), [1, 3], 'the empty result leaves no row behind');
});

test('an ID point echoes its three ordinates with the measurement formatter', () => {
  const row = formatCommandLogEntry(pointResult(), t);
  assert.equal(row.kind, 'result');
  assert.equal(row.sequence, 4);
  // One value line only: the coordinates, the drawing unit, and the space/snap
  // chip. No distance, no deltas — ID measures nothing.
  assert.deepEqual(row.lines, [{
    tone: 'value',
    label: en['command.log.point'],
    text: '12.5000, -3.0000, 0.0000',
    unit: null,
    meta: `${en['command.log.space.model']} \u00b7 ${en['command.log.snap.endpoint']}`,
  }]);
  // The ordinates use the measurement formatter, so they agree with every other
  // measured number in the window (fixed decimals, never a negative zero).
  assert.equal(row.lines[0].text.split(', ')[2], formatMeasurementValue(0));
  // Paper space and a reported unit are the host's own values.
  const paper = formatCommandLogEntry(pointResult({
    point: { ...POINT(), space: 'paper', unit: 'mm', snap: null },
  }), t);
  assert.equal(paper.lines[0].unit, 'mm');
  assert.equal(paper.lines[0].meta, en['command.log.space.paper']);
  // A coordinate read off sampled geometry must say so: `analytic` adds nothing to
  // the chip (it is the default), `approximate` and an unknown token do.
  const approximate = formatCommandLogEntry(pointResult({
    point: { ...POINT(), precision: 'approximate' },
  }), t);
  assert.equal(approximate.lines[0].meta,
    `${en['command.log.space.model']} \u00b7 ${en['command.log.snap.endpoint']} \u00b7 ${en['command.log.precision.approximate']}`);
  const unmodelled = formatCommandLogEntry(pointResult({
    point: { ...POINT(), precision: 'sampled' },
  }), t);
  assert.equal(unmodelled.lines[0].meta, `${en['command.log.space.model']} \u00b7 ${en['command.log.snap.endpoint']} \u00b7 sampled`);
});

test('a point row never invents an ordinate, a space label or a snap label', () => {
  // Only the reported prefix is printed: an unusable ordinate ends the run, never
  // replaced by a zero.
  const twoD = formatCommandLogEntry(pointResult({ point: { ...POINT(), position: [1, 2] } }), t);
  assert.equal(twoD.lines[0].text, '1.0000, 2.0000');
  // ... and a point with fewer than two usable ordinates prints no row at all
  // (`[1, null, 0]` stops at the bad ordinate, leaving one number — not a point).
  for (const position of [[], [1], [1, null, 0], [null, 2], 'oops', undefined]) {
    assert.equal(
      formatCommandLogEntry(pointResult({ point: { ...POINT(), position } }), t),
      null,
      `position ${JSON.stringify(position)} must not produce a coordinate`,
    );
  }
  // Tokens this build does not know are shown as themselves, never guessed at —
  // the same rule the unknown algorithm token follows.
  const unknown = formatCommandLogEntry(
    pointResult({ point: { ...POINT(), space: 'layout2', snap: 'nearest' } }),
    t,
  );
  assert.equal(unknown.lines[0].meta, 'layout2 \u00b7 nearest');
  // An absent space/snap adds nothing to the chip — not even a separator.
  const bare = formatCommandLogEntry(pointResult({ point: { ...POINT(), space: null, snap: null } }), t);
  assert.equal(bare.lines[0].meta, null);
  const snapOnly = formatCommandLogEntry(pointResult({ point: { ...POINT(), space: null } }), t);
  assert.equal(snapOnly.lines[0].meta, en['command.log.snap.endpoint']);
});

test('a point row reads in the active language and names every known snap', () => {
  const row = formatCommandLogEntry(pointResult({ point: { ...POINT(), snap: 'midpoint' } }), tzh);
  assert.equal(row.lines[0].label, zh['command.log.point']);
  assert.equal(row.lines[0].meta, `${zh['command.log.space.model']} \u00b7 ${zh['command.log.snap.midpoint']}`);
  // Every snap the wire can carry has its own sentence; none collapses onto
  // another (an intersection is not a "center").
  const keys = ['endpoint', 'midpoint', 'center', 'quadrant', 'perpendicular', 'intersection']
    .map(snap => formatCommandLogEntry(pointResult({ point: { ...POINT(), snap } }), t).lines[0].meta);
  assert.equal(new Set(keys).size, keys.length);
  assert.equal(keys.includes(undefined), false);
});

test('a command entry is echoed with the catalog prompt, in a meta tone', () => {
  const row = formatCommandLogEntry(
    { sequence: 1, kind: 'command', command: 'DIST', code: null, measurement: null },
    t,
  );
  assert.deepEqual(row, {
    sequence: 1,
    kind: 'command',
    lines: [{ tone: 'command', label: en['command.log.prompt'], text: 'DIST', unit: null, meta: null }],
  });
  // A selector name is required; nothing is echoed for an entry without one.
  assert.equal(
    formatCommandLogEntry({ sequence: 1, kind: 'command', command: '', code: null, measurement: null }, t),
    null,
  );
});

test('an error entry shows the localized code text and keeps the code out of the visible copy', () => {
  const row = formatCommandLogEntry(
    { sequence: 2, kind: 'error', command: null, code: 'unsupported', measurement: null },
    t,
  );
  assert.equal(row.kind, 'error');
  assert.equal(row.code, 'unsupported');
  assert.deepEqual(row.lines, [
    { tone: 'error', label: null, text: en['command.log.error.unsupported'], unit: null, meta: null },
  ]);
  assert.equal(row.lines[0].text.includes('unsupported'), false);
  assert.equal(
    formatCommandLogEntry({ sequence: 3, kind: 'error', command: null, code: 'cancelled', measurement: null }, tzh)
      .lines[0].text,
    zh['command.log.error.cancelled'],
  );
});

test('an unknown or absent error code falls back to the generic refusal line', () => {
  const unknown = formatCommandLogEntry(
    { sequence: 2, kind: 'error', command: null, code: 'future_code', measurement: null },
    t,
  );
  assert.equal(unknown.lines[0].text, en['command.log.error']);
  assert.equal(unknown.code, 'future_code', 'the diagnostic code is still carried for the title');
  const absent = formatCommandLogEntry(
    { sequence: 3, kind: 'error', command: null, code: null, measurement: null },
    t,
  );
  assert.equal(absent.lines[0].text, en['command.log.error']);
  assert.equal(absent.code, null);
});

test('malformed entries and a non-array log render nothing', () => {
  for (const entry of [null, undefined, {}, { kind: 'future' }, 'DIST', 7]) {
    assert.equal(formatCommandLogEntry(entry, t), null);
  }
  assert.deepEqual(commandLogView(null, t), []);
  assert.deepEqual(commandLogView(undefined, t), []);
  assert.deepEqual(commandLogView({ length: 0 }, t), []);
});

test('every echo key exists in both catalogs with the same placeholders', () => {
  // 32 keys: label, prompt, four result labels (Distance/Angle/Length/Area), the
  // point label with its two space and six snap labels, three deltas, the
  // squared-unit template, two precision chips, the generic refusal and ten codes.
  assert.equal(CATALOG_KEYS.length, 32);
  assert.equal(COMMAND_LOG_ERROR_CODES.length, 10);
  for (const key of CATALOG_KEYS) {
    assert.equal(typeof en[key], 'string', `en missing ${key}`);
    assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
    assert.ok(en[key].trim().length > 0, `en empty ${key}`);
    assert.ok(zh[key].trim().length > 0, `zh-CN empty ${key}`);
  }
  // The squared-unit template is the only one with a placeholder, and both
  // catalogs spell it the same way.
  assert.equal(en['command.log.unit_squared'], '{unit}\u00B2');
  assert.equal(zh['command.log.unit_squared'], '{unit}\u00B2');
});

test('the command line renders the host log in a bounded, non-focusable scrollback', async () => {
  const source = await readModule('components/CommandLine.tsx');
  // The data is the host field, formatted by the new module — no parallel state.
  assert.ok(source.includes('shell.status?.commandLog'));
  assert.ok(source.includes('commandLogView(logEntries, t)'));
  assert.ok(!source.includes('setCommandLog'), 'the echo is never mirrored in local state');
  // Newest at the bottom, following only while the view is already at the bottom.
  assert.ok(source.includes("node.scrollTop = node.scrollHeight"));
  assert.ok(source.includes('logStick.current'));
  assert.ok(source.includes('onScroll={trackLogScroll}'));
  // Output, not a control: no tab stop, no focus ring, no clear affordance.
  assert.ok(source.includes('role="log"'));
  assert.ok(source.includes('className="commandlog"'));
  assert.ok(!source.includes('tabIndex'), 'the scrollback must stay unreachable by keyboard');
  assert.ok(!source.includes('commandLogClear'));
  assert.ok(!source.includes('command.log.clear'));
  // The diagnostic code survives only as the hover title.
  assert.ok(source.includes("row.kind === 'error' ? row.code ?? undefined : undefined"));
  assert.ok(source.includes('commandlog__meta'), 'the precision chip is rendered');
});

test('the scrollback is bounded to a few mono lines and never reflows the ribbon', async () => {
  const css = await readCss();
  const block = css.slice(css.indexOf('.commandlog {'), css.indexOf('.commandlog {') + 600);
  assert.ok(block.includes('max-height: 120px'), 'a bounded height keeps the log from growing');
  assert.ok(block.includes('overflow-y: auto'), 'the log scrolls itself');
  assert.ok(block.includes('font-family: var(--font-mono)'));
  assert.ok(block.includes('font-size: 11px'), 'the existing mono step, not a new one');
  assert.ok(block.includes('background: var(--chrome-3)'));
  assert.ok(block.includes('border: 1px solid var(--line)'));
  const meta = css.slice(css.indexOf('.commandlog__meta {'), css.indexOf('.commandlog__meta {') + 400);
  assert.ok(meta.includes('border: 1px solid var(--line-soft)'), 'the chip reuses the shared badge surface');
  assert.ok(meta.includes('font-size: 10px'), 'the existing badge step');
});
