// Status-bar coordinate readout: the host's real `cursorWorld` in drawing units,
// formatted AutoCAD-style (4 decimals, `Z` only when non-zero) and never the DOM
// pointer's CSS pixels under the "coordinates" label.
import test from 'node:test';
import assert from 'node:assert/strict';
import { COORDINATE_PLACEHOLDER, formatWorldCursor } from '../src/coords.mjs';
import { readCatalog, readModule } from './helpers.mjs';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

test('a world point prints 4 decimals, including negatives', () => {
  assert.deepEqual(formatWorldCursor({ cursorWorld: [12.5, -3.25, 0] }), {
    text: '12.5000, -3.2500',
    hintKey: 'status.coordinate_hint_world',
  });
  assert.deepEqual(formatWorldCursor({ cursorWorld: [0, 0, 0] }), {
    text: '0.0000, 0.0000',
    hintKey: 'status.coordinate_hint_world',
  });
  assert.deepEqual(formatWorldCursor({ cursorWorld: [-1234.56789, 0.00004, 0] }), {
    text: '-1234.5679, 0.0000',
    hintKey: 'status.coordinate_hint_world',
  });
});

test('Z is printed only when it is non-zero, and never as a signed zero', () => {
  const plane = formatWorldCursor({ cursorWorld: [1, 2, 0] });
  assert.equal(plane.text, '1.0000, 2.0000');
  // A real elevation is part of the coordinate.
  assert.equal(formatWorldCursor({ cursorWorld: [1, 2, 3.5] }).text, '1.0000, 2.0000, 3.5000');
  assert.equal(formatWorldCursor({ cursorWorld: [1, 2, -3.5] }).text, '1.0000, 2.0000, -3.5000');
  // `-0` is the same drawing coordinate as `0`: never "-0.0000".
  assert.equal(formatWorldCursor({ cursorWorld: [-0, -0, -0] }).text, '0.0000, 0.0000');
  assert.equal(formatWorldCursor({ cursorWorld: [0, 0, -0] }).text, '0.0000, 0.0000');
  // A Z that rounds to zero is a plane coordinate: the printed form agrees.
  assert.equal(formatWorldCursor({ cursorWorld: [0, 0, -1e-9] }).text, '0.0000, 0.0000');
  // A tiny negative X/Y also prints as `0`, not `-0.0000`.
  assert.equal(formatWorldCursor({ cursorWorld: [-1e-9, 0, 0] }).text, '0.0000, 0.0000');
});

test('no reliable world coordinate renders the em dash, never zero or pixels', () => {
  const none = { text: COORDINATE_PLACEHOLDER, hintKey: 'status.coordinate_hint_none' };
  assert.equal(COORDINATE_PLACEHOLDER, '\u2014, \u2014');
  for (const status of [
    null,
    undefined,
    {},
    { cursorWorld: null },
    // A malformed point is treated like "no coordinate" here; `parseCadStatus`
    // rejects it before it ever reaches the UI.
    { cursorWorld: undefined },
    { cursorWorld: [0, 0] },
    { cursorWorld: [0, 0, 0, 0] },
    { cursorWorld: [0, 0, '0'] },
    { cursorWorld: '0.0000, 0.0000' },
  ]) {
    assert.deepEqual(formatWorldCursor(status), none, JSON.stringify(status));
  }
});

test('the status bar renders the formatted world text and never the pixel fallback', async () => {
  const source = await readModule('components/StatusBar.jsx');
  // One pure formatter, fed by the host status (not by a DOM pointer event).
  assert.ok(source.includes('formatWorldCursor(shell.status)'));
  assert.ok(source.includes('{worldCursor.text}'));
  // The title follows the real state: world hint vs the explicit "none" hint.
  assert.ok(source.includes('t(worldCursor.hintKey)'));
  // The group label stays the catalog's "coordinates" copy.
  assert.ok(source.includes("aria-label={t('status.coordinates')}"));
  // No pixel readout survives anywhere under this label: the old
  // `cursor.x.toFixed(0)` readout and the `shell.cursor` state are both gone.
  assert.ok(!source.includes('shell.cursor'));
  assert.ok(!source.includes('toFixed(0)'));
  assert.ok(!source.includes("t('status.coordinate_hint')"));
});

test('the coordinate catalog keys exist in both locales with real copy', () => {
  for (const key of ['status.coordinates', 'status.coordinate_hint_world', 'status.coordinate_hint_none']) {
    assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
    assert.ok(zh[key].trim().length > 0, `zh-CN empty ${key}`);
    assert.equal(typeof en[key], 'string', `en missing ${key}`);
    assert.ok(en[key].trim().length > 0, `en empty ${key}`);
  }
  assert.match(zh['status.coordinate_hint_world'], /世界坐标/);
  assert.match(en['status.coordinate_hint_world'], /world/i);
});
