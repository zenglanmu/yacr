// Contract tests for the `cad_api_status` decoder.
//
// The authority is the host's own JSON (`crates/cad-ui-core/api/plugin-api.json`),
// read here as text exactly as the wire answer carries it. A decoder that lets a
// malformed payload through is how a plugin ends up calling a capability the host
// never granted, so every rejection below is asserted on both the stable prefix
// and the detail that names the broken field.
//
// These tests are `.ts` by filename only: they run under Node's type stripping.

import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';

import { API_NAMESPACES } from '../../../../packages/cad-plugin-api/src/index.ts';
import { hostInfoFrom, parseApiStatusPayload } from '../../src/plugin-api/descriptor.ts';

const CONTRACT_URL = new URL('../../../../crates/cad-ui-core/api/plugin-api.json', import.meta.url);
const payload = parseApiStatusPayload(JSON.parse(await readFile(CONTRACT_URL, 'utf8')));

/** A fresh deep copy, so a mutation in one case cannot leak into another. */
function copy() {
  return JSON.parse(JSON.stringify(payload));
}

/** The array index of one row, looked up by name rather than hardcoded. */
function rowIndex(target, namespace, operation) {
  const index = target.operations.findIndex(
    (row) => row.namespace === namespace && row.operation === operation,
  );
  assert.notEqual(index, -1, `${namespace}.${operation} must exist in the contract`);
  return index;
}

/** Assert the one error shape the decoder produces, with a detail substring. */
function assertRejected(value, detail) {
  assert.throws(
    () => parseApiStatusPayload(value),
    (error) => {
      assert.ok(error instanceof Error, 'the decoder throws an Error');
      assert.ok(
        error.message.startsWith('cad_api_status: '),
        `expected the cad_api_status prefix, got: ${error.message}`,
      );
      assert.ok(
        error.message.includes(detail),
        `expected the detail to mention "${detail}", got: ${error.message}`,
      );
      return true;
    },
  );
}

test('the shipped contract parses with 20 rows at apiVersion 1', () => {
  assert.equal(payload.apiVersion, 1);
  assert.equal(payload.operations.length, 20);
  assert.deepEqual(payload.namespaces, API_NAMESPACES);
  assert.equal(new Set(payload.operations.map((row) => `${row.namespace}.${row.operation}`)).size, 20);
  // The descriptor lists host-served operations only, and this build serves none:
  // a `supported` row here would promise a host state that does not exist.
  assert.deepEqual(
    payload.operations.filter((row) => row.status !== 'pending'),
    [],
    'every row is pending in this build',
  );
});

test('a non-object payload is rejected, including an array', () => {
  assertRejected(null, 'not an object');
  assertRejected([], 'not an object');
  assertRejected('cad_api_status', 'not an object');
});

test('apiVersion must be an integer of at least 1', () => {
  for (const apiVersion of [0, 1.5, '1', undefined]) {
    const value = copy();
    value.apiVersion = apiVersion;
    assertRejected(value, 'apiVersion');
  }
});

test('namespaces must be the six known names, with no duplicates', () => {
  const unknown = copy();
  unknown.namespaces = ['Geometry', 'Nope'];
  assertRejected(unknown, 'unknown namespace');

  const duplicate = copy();
  duplicate.namespaces = [...API_NAMESPACES, 'Colors'];
  assertRejected(duplicate, 'duplicate namespace');

  const empty = copy();
  empty.namespaces = [];
  assertRejected(empty, 'namespaces');
});

test('an unknown status is rejected rather than treated as supported', () => {
  const value = copy();
  value.operations[0].status = 'maybe';
  assertRejected(value, 'unknown status');
});

test('a pending row must carry its reason and it must be the canonical one', () => {
  const missing = copy();
  delete missing.operations[0].reason;
  assertRejected(missing, 'no reason');

  const wrong = copy();
  wrong.operations[0].reason = 'api.pending: DatabaseServices.transaction.commit';
  assertRejected(wrong, 'wrong pending reason');
});

test('a supported row must not carry a reason', () => {
  // The shipped table has no supported rows left, so the branch is exercised by
  // flipping one row: a host that upgrades a row and keeps its reason is broken.
  const value = copy();
  value.operations[0].status = 'supported';
  value.operations[0].reason = 'because';
  assertRejected(value, 'carries a reason');
});

test('an unsupported row needs the stable unsupported-runtime prefix', () => {
  const value = copy();
  value.operations[0].status = 'unsupported';
  value.operations[0].reason = 'because this carrier cannot';
  assertRejected(value, 'stable prefix');
});

test('operation names must be dotted camelCase segments', () => {
  for (const operation of ['Point3d', 'geometry', 'geometry.', 'geometry.Point3d', '']) {
    const value = copy();
    value.operations[rowIndex(value, 'EditorInput', 'editor.getPoint')].operation = operation;
    assertRejected(value, 'malformed operation name');
  }
});

test('duplicate rows are rejected', () => {
  const value = copy();
  value.operations.push({ ...value.operations[0] });
  assertRejected(value, 'duplicate operation row');
});

test('hostInfoFrom reports unknown/none before a status is observed', () => {
  const info = hostInfoFrom(payload, null);
  assert.equal(info.apiVersion, 1);
  assert.equal(info.carrier, 'unknown');
  assert.equal(info.documentOwner, 'none');
  assert.equal(info.operations, payload.operations, 'rows are carried verbatim');
});

test('hostInfoFrom follows the reported dom-canvas carrier', () => {
  const info = hostInfoFrom(payload, {
    hostProfile: { surfaceMode: 'dom-canvas', documentOwner: 'wasm' },
  });
  assert.equal(info.carrier, 'dom-canvas');
  assert.equal(info.documentOwner, 'wasm');
});

test('hostInfoFrom treats every other observed surface mode as native-below', () => {
  for (const status of [
    { hostProfile: { surfaceMode: 'native-below', documentOwner: 'native' } },
    {},
  ]) {
    const info = hostInfoFrom(payload, status);
    assert.equal(info.carrier, 'native-below');
    assert.equal(info.documentOwner, 'native');
  }
});
