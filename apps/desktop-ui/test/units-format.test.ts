// Stored `UNITS` readout formatting: one helper, every length/angle format and
// precision, the base-angle/rotation direction, and the two real consumers
// (status-bar coordinate, command-window echo).
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  LEGACY_UNITS_FORMAT,
  formatAngle,
  formatLength,
  parseCadUnits,
  unitsFormat,
} from '../src/cad/units-format.ts';
import type { CadUnits } from '../src/cad/units-format.ts';
import { formatWorldCursor } from '../src/cad/coords.ts';
import { formatCommandLogEntry } from '../src/commands/command-log.ts';
import { createTranslator } from '../src/i18n.ts';
import { readCatalog } from './helpers.ts';

const en = await readCatalog('en');
const t = createTranslator({ 'zh-CN': en, en }, 'en');

function units(overrides: Partial<CadUnits> = {}): CadUnits {
  return {
    lengthFormat: 'decimal',
    lengthPrecision: 4,
    angleFormat: 'decimal_degrees',
    anglePrecision: 4,
    angleBaseDegrees: 0,
    angleClockwise: false,
    ...overrides,
  };
}

test('the legacy fallback is the fixed four-decimal form', () => {
  assert.deepEqual(unitsFormat(null), LEGACY_UNITS_FORMAT);
  assert.equal(formatLength(12.34567, LEGACY_UNITS_FORMAT), '12.3457');
  assert.equal(formatAngle(45, LEGACY_UNITS_FORMAT, false), '45.0000\u00B0');
});

test('every linear format uses the stored form and precision', () => {
  assert.equal(formatLength(12.345, unitsFormat(units({ lengthPrecision: 2 }))), '12.35');
  assert.equal(
    formatLength(12.345, unitsFormat(units({ lengthFormat: 'scientific' }))),
    '1.2345E+01',
  );
  assert.equal(
    formatLength(12.345, unitsFormat(units({ lengthFormat: 'engineering' }))),
    '1\'-0.3450"',
  );
  assert.equal(
    formatLength(12.375, unitsFormat(units({ lengthFormat: 'architectural', lengthPrecision: 3 }))),
    '1\'-0 3/8"',
  );
  assert.equal(
    formatLength(12.375, unitsFormat(units({ lengthFormat: 'fractional', lengthPrecision: 3 }))),
    '12 3/8',
  );
});

test('linear precision is exact, and a rounded zero never prints a sign', () => {
  assert.equal(formatLength(-0, unitsFormat(units({ lengthPrecision: 2 }))), '0.00');
  assert.equal(formatLength(-1.5, unitsFormat(units({ lengthPrecision: 2 }))), '-1.50');
  assert.equal(formatLength(Number.NaN, LEGACY_UNITS_FORMAT), null);
});

test('every angular format uses the stored form and precision', () => {
  assert.equal(
    formatAngle(45, unitsFormat(units({ anglePrecision: 2 })), false),
    '45.00\u00B0',
  );
  assert.equal(
    formatAngle(45.5, unitsFormat(units({ angleFormat: 'degrees_minutes_seconds', anglePrecision: 0 })), false),
    '45d30\'0"',
  );
  assert.equal(
    formatAngle(45, unitsFormat(units({ angleFormat: 'gradians', anglePrecision: 2 })), false),
    '50.00g',
  );
  assert.equal(
    formatAngle(180, unitsFormat(units({ angleFormat: 'radians' })), false),
    '3.1416r',
  );
  assert.equal(
    formatAngle(45, unitsFormat(units({ angleFormat: 'surveyors_units', anglePrecision: 0 }))),
    'N 45d0\'0" E',
  );
  assert.equal(
    formatAngle(135, unitsFormat(units({ angleFormat: 'surveyors_units', anglePrecision: 0 }))),
    'S 45d0\'0" E',
  );
});

test('absolute angles apply the base angle and clockwise direction; relative ones do not', () => {
  const settings = units({ angleBaseDegrees: 90, angleClockwise: true, anglePrecision: 2 });
  assert.equal(formatAngle(0, unitsFormat(settings), true), '90.00\u00B0');
  assert.equal(formatAngle(90, unitsFormat(settings), true), '0.00\u00B0');
  assert.equal(formatAngle(0, unitsFormat(settings), false), '0.00\u00B0');
});

test('a malformed units block is refused instead of guessed', () => {
  assert.equal(parseCadUnits(undefined), null);
  assert.equal(parseCadUnits(null), null);
  for (const broken of [
    {},
    { ...units(), lengthFormat: 'parsecs' },
    { ...units(), lengthPrecision: 9 },
    { ...units(), anglePrecision: -1 },
    { ...units(), angleBaseDegrees: Number.NaN },
    { ...units(), angleClockwise: 'yes' },
  ]) {
    assert.throws(() => parseCadUnits(broken), /CAD_STATUS_INVALID/, JSON.stringify(broken));
  }
});

test('the status-bar coordinate readout consumes the stored length settings', () => {
  const readout = formatWorldCursor({
    cursorWorld: [12.5, -3.25, 0],
    units: units({ lengthPrecision: 2 }),
  });
  assert.equal(readout.text, '12.50, -3.25');
  const architectural = formatWorldCursor({
    cursorWorld: [12.375, 0, 0],
    units: units({ lengthFormat: 'architectural', lengthPrecision: 3 }),
  });
  assert.equal(architectural.text, '1\'-0 3/8", 0\'-0"');
});

test('the command-window echo consumes the stored length and angle settings', () => {
  const result = {
    sequence: 1,
    kind: 'result',
    measurement: {
      algorithm: 'distance',
      value: 12.345,
      unit: 'mm',
      precision: 'analytic',
      points: [[0, 0, 0], [9.9994, 7.0837, 0]],
    },
  };
  const row = formatCommandLogEntry(
    result,
    t,
    units({ lengthPrecision: 2, anglePrecision: 1 }),
  );
  assert.ok(row && row.kind === 'result');
  const lines = row.lines;
  assert.equal(lines[0].text, '12.35');
  assert.equal(lines[1].text, '35.3\u00B0');
  assert.equal(lines[2].text, '10.00');
});
