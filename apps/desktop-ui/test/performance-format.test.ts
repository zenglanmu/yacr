// File-info performance formatting: the panel's numbers are real host readings
// or an em dash, never a fabricated zero or a borrowed total.
import test from 'node:test';
import assert from 'node:assert/strict';
import { formatBytes, formatMemoryUsage, formatPercent } from '../src/performance/format.ts';

const DASH = '\u2014';

test('missing and malformed readings render the em dash instead of zero', () => {
  for (const value of [null, undefined, Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY, -1]) {
    assert.equal(formatBytes(value), DASH);
    assert.equal(formatPercent(value), DASH);
  }
});

test('bytes scale in binary units with one decimal from MiB up', () => {
  assert.equal(formatBytes(0), '0 B');
  assert.equal(formatBytes(1023), '1023 B');
  assert.equal(formatBytes(1024), '1 KiB');
  assert.equal(formatBytes(1024 * 1024), '1 MiB');
  assert.equal(formatBytes(412 * 1024 * 1024), '412 MiB');
  assert.equal(formatBytes(1.2 * 1024 ** 3), '1.2 GiB');
  assert.equal(formatBytes(2.5 * 1024 ** 4), '2.5 TiB');
});

test('percent keeps one decimal and treats zero as a real reading', () => {
  assert.equal(formatPercent(0), '0.0%');
  assert.equal(formatPercent(7.5), '7.5%');
  assert.equal(formatPercent(123.456), '123.5%');
});

test('memory usage joins real values and keeps an em dash for a missing side', () => {
  assert.equal(formatMemoryUsage(1.2 * 1024 ** 3, 16 * 1024 ** 3), '1.2 GiB / 16 GiB (7.5%)');
  assert.equal(formatMemoryUsage(0, 1024 ** 3), '0 B / 1 GiB (0.0%)');
  assert.equal(formatMemoryUsage(null, 16 * 1024 ** 3), `${DASH} / 16 GiB`);
  assert.equal(formatMemoryUsage(1024 ** 3, null), `1 GiB / ${DASH}`);
  assert.equal(formatMemoryUsage(null, null), DASH);
  assert.equal(formatMemoryUsage(1024 ** 3, 0), '1 GiB / 0 B');
});
