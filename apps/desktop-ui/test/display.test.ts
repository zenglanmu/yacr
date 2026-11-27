// The Shadows/Fog tray wire: only the documented record shape is accepted, and
// a missing scalar falls back to the host's documented default instead of
// inventing a new one. The panels dispatch `SHADOWS`/`FOG`; the read never
// writes.

import assert from 'node:assert/strict';
import test from 'node:test';

import { DISPLAY_COMMAND, parseDisplayAnswer, readDisplay } from '../src/domains/concept/display.ts';

test('a full display answer keeps the host values verbatim', () => {
  const answer = parseDisplayAnswer({
    shadows: { enabled: true, azimuth: 90, elevation: 22.5 },
    fog: { enabled: true, start: 5, end: 50, color: [9, 8, 7] },
  });
  assert.deepEqual(answer, {
    shadows: { enabled: true, azimuth: 90, elevation: 22.5 },
    fog: { enabled: true, start: 5, end: 50, color: [9, 8, 7] },
  });
});

test('a sparse answer falls back to the documented defaults', () => {
  const answer = parseDisplayAnswer({ shadows: {}, fog: {} });
  assert.deepEqual(answer, {
    shadows: { enabled: false, azimuth: 135, elevation: 45 },
    fog: { enabled: false, start: 100, end: 1000, color: [200, 210, 220] },
  });
});

test('non-numeric and out-of-range channels are normalised, not trusted', () => {
  const answer = parseDisplayAnswer({
    shadows: { enabled: 'yes', azimuth: '90', elevation: null },
    fog: { enabled: true, start: -5, end: 12, color: [999, NaN, 3.7] },
  });
  assert.equal(answer.shadows.enabled, false);
  assert.equal(answer.shadows.azimuth, 135);
  assert.equal(answer.shadows.elevation, 45);
  assert.equal(answer.fog.start, -5);
  assert.deepEqual(answer.fog.color, [255, 210, 4]);
});

test('a non-object answer is a named transport fault', () => {
  assert.throws(() => parseDisplayAnswer(null), /display\.invalid_answer/);
  assert.throws(() => parseDisplayAnswer([1, 2]), /display\.invalid_answer/);
});

test('the read goes through the one frozen command', async () => {
  const seen: string[] = [];
  const answer = await readDisplay(async (command: string) => {
    seen.push(command);
    return { shadows: { enabled: true }, fog: { enabled: false, start: 1, end: 2 } };
  });
  assert.deepEqual(seen, [DISPLAY_COMMAND]);
  assert.equal(answer.shadows.enabled, true);
  assert.equal(answer.fog.end, 2);
});
