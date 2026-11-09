// The persisted update policy: one day throttle, one skipped release, and a
// storage that may throw on any access.
//
// `.ts` by filename only: Node's type stripping runs it, `tsc` never sees it.
import test from 'node:test';
import assert from 'node:assert/strict';

import { CHECK_INTERVAL_MS } from '../src/update/config.ts';
import {
  LAST_CHECK_KEY,
  SKIPPED_VERSION_KEY,
  clearSkipped,
  lastCheckAt,
  markChecked,
  markSkipped,
  shouldCheck,
  skippedVersion,
} from '../src/update/policy.ts';

/** A `Storage` stand-in with the same string-only contract. */
function memoryStorage(initial = {}) {
  const map = new Map(Object.entries(initial).map(([key, value]) => [key, String(value)]));
  return {
    map,
    getItem(key) { return map.has(key) ? map.get(key) : null; },
    setItem(key, value) { map.set(key, String(value)); },
    removeItem(key) { map.delete(key); },
  };
}

/** A storage that throws on every access (private mode / quota / hostile page). */
function throwingStorage() {
  const fail = () => { throw new Error('storage blocked'); };
  return { getItem: fail, setItem: fail, removeItem: fail };
}

test('a missing last check is due, and a successful check is recorded', () => {
  const storage = memoryStorage();
  assert.equal(lastCheckAt(0, storage), null);
  assert.equal(shouldCheck(1, storage), true);

  markChecked(1_000, storage);
  assert.equal(lastCheckAt(0, storage), 1_000);
  assert.equal(storage.map.get(LAST_CHECK_KEY), '1000');
});

test('the check is throttled for exactly one interval', () => {
  const storage = memoryStorage();
  const now = 5_000_000;
  markChecked(now, storage);

  assert.equal(shouldCheck(now, storage), false, 'same instant');
  assert.equal(shouldCheck(now + CHECK_INTERVAL_MS - 1, storage), false, 'one ms early');
  assert.equal(shouldCheck(now + CHECK_INTERVAL_MS, storage), true, 'the boundary is due');
  assert.equal(shouldCheck(now + 2 * CHECK_INTERVAL_MS, storage), true);

  // A future timestamp (clock skew) is not due.
  assert.equal(shouldCheck(now - 1, storage), false);
});

test('a corrupt or negative timestamp counts as no previous check', () => {
  for (const raw of ['not a number', '', '  ', '-1', '1.5', 'NaN', 'Infinity']) {
    const storage = memoryStorage({ [LAST_CHECK_KEY]: raw });
    assert.equal(lastCheckAt(0, storage), null, JSON.stringify(raw));
    assert.equal(shouldCheck(10, storage), true, JSON.stringify(raw));
  }
});

test('the skipped release is remembered and can be cleared', () => {
  const storage = memoryStorage();
  assert.equal(skippedVersion(storage), null);

  markSkipped('0.1.7', storage);
  assert.equal(skippedVersion(storage), '0.1.7');
  assert.equal(storage.map.get(SKIPPED_VERSION_KEY), '0.1.7');

  clearSkipped(storage);
  assert.equal(skippedVersion(storage), null);
  assert.equal(storage.map.has(SKIPPED_VERSION_KEY), false);
});

test('an empty skipped value is ignored', () => {
  const storage = memoryStorage({ [SKIPPED_VERSION_KEY]: '' });
  assert.equal(skippedVersion(storage), null);
  markSkipped('', storage);
  assert.equal(skippedVersion(storage), null);
});

test('a blocked storage never throws and degrades to "no policy"', () => {
  const storage = throwingStorage();
  assert.doesNotThrow(() => markChecked(1_000, storage));
  assert.equal(lastCheckAt(0, storage), null);
  assert.equal(shouldCheck(1_000, storage), true);

  assert.doesNotThrow(() => markSkipped('0.1.7', storage));
  assert.equal(skippedVersion(storage), null);
  assert.doesNotThrow(() => clearSkipped(storage));
});

test('a null storage behaves like a blocked one', () => {
  assert.equal(lastCheckAt(0, null), null);
  assert.equal(shouldCheck(0, null), true);
  assert.equal(skippedVersion(null), null);
  assert.doesNotThrow(() => markChecked(0, null));
  assert.doesNotThrow(() => markSkipped('0.1.7', null));
  assert.doesNotThrow(() => clearSkipped(null));
});
