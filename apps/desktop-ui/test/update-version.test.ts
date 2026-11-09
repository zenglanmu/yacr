// Version parsing/comparison: the matrix the update notice depends on.
//
// `.ts` by filename only: Node's type stripping runs it, `tsc` never sees it.
import test from 'node:test';
import assert from 'node:assert/strict';

import {
  compareVersions,
  formatVersion,
  isNewer,
  parseVersion,
} from '../src/update/version.ts';

test('parseVersion accepts an optional v and exactly three numeric parts', () => {
  assert.deepEqual(parseVersion('0.1.6'), [0, 1, 6]);
  assert.deepEqual(parseVersion('v0.1.6'), [0, 1, 6]);
  assert.deepEqual(parseVersion('1.2.3'), [1, 2, 3]);
  assert.deepEqual(parseVersion('v10.20.30'), [10, 20, 30]);
  assert.deepEqual(parseVersion('0.1.10'), [0, 1, 10]);
  assert.deepEqual(parseVersion('1.0.0'), [1, 0, 0]);
});

test('parseVersion rejects placeholders, partials, suffixes, whitespace and garbage', () => {
  for (const input of [
    '', 'v', 'V1.2.3', '1', '1.2', '1.2.3.4', '1.2.3-beta', '1.2.3+build',
    '1.a.3', '-1.2.3', ' 1.2.3', '1.2.3 ', '0..3', '.1.2', 'v.1.2', '0.0.0', 'v0.0.0',
    '../1.2.3', '999999999999999999999999.0.0',
  ]) {
    assert.equal(parseVersion(input), null, JSON.stringify(input));
  }
  // Non-strings are rejected by the runtime guard too.
  for (const input of [null, undefined, 1, {}, []]) {
    assert.equal(parseVersion(input), null, String(input));
  }
});

test('compareVersions is numeric, not lexicographic', () => {
  assert.equal(compareVersions([0, 1, 10], [0, 1, 9]), 1);
  assert.equal(compareVersions([0, 1, 9], [0, 1, 10]), -1);
  assert.equal(compareVersions([0, 1, 6], [0, 1, 6]), 0);
  assert.equal(compareVersions([1, 0, 0], [0, 99, 99]), 1);
  assert.equal(compareVersions([0, 0, 1], [0, 1, 0]), -1);
});

test('formatVersion is the canonical spelling', () => {
  assert.equal(formatVersion([0, 1, 6]), '0.1.6');
  assert.equal(formatVersion(parseVersion('v2.3.4')), '2.3.4');
});

test('isNewer compares the release against the running version', () => {
  assert.equal(isNewer('v0.1.10', 'v0.1.9'), true);
  assert.equal(isNewer('0.1.7', 'v0.1.6'), true);
  assert.equal(isNewer('0.1.6', 'v0.1.6'), false, 'equal versions are not newer');
  assert.equal(isNewer('v0.1.6', '0.1.7'), false, 'older is not newer');
  // Unusable input is never an update: a parse failure must not become a notice.
  assert.equal(isNewer('garbage', '0.1.6'), false);
  assert.equal(isNewer('0.1.7', 'garbage'), false);
  assert.equal(isNewer('0.0.0', '0.1.6'), false);
  assert.equal(isNewer('0.1.7', '0.0.0'), false);
});
