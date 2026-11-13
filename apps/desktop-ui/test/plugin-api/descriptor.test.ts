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

test('the shipped contract parses with 17 rows at apiVersion 1', () => {
  assert.equal(payload.apiVersion, 1);
  assert.equal(payload.operations.length, 17);
  assert.deepEqual(payload.namespaces, API_NAMESPACES);
  assert.equal(new Set(payload.operations.map((row) => `${row.namespace}.${row.operation}`)).size, 17);
  // The descriptor lists host-served operations only. Six are served in this
  // build; widening the set means landing the host operation and the adapter
  // `case` (or mutating implementation) first — bridge.test.ts enforces that half.
  assert.deepEqual(
    payload.operations
      .filter((row) => row.status !== 'pending')
      .map((row) => `${row.namespace}.${row.operation}`),
    [
      'ApplicationServices.documentManager.mdiActiveDocument',
      'ApplicationServices.documentManager.documents',
      'DatabaseServices.database.getObjectId',
      'DatabaseServices.database.symbolTables',
      'DatabaseServices.database.revision',
      'DatabaseServices.transaction.getObject',
      'DatabaseServices.transaction.commit',
      'EditorInput.editor.getPoint',
      'EditorInput.editor.getEntity',
      'EditorInput.editor.getSelection',
      'EditorInput.editor.getString',
      'EditorInput.editor.getDouble',
      'EditorInput.editor.getInteger',
      'EditorInput.editor.getKeyword',
      'EditorInput.editor.selectAll',
      'EditorInput.editor.currentSelection',
    ],
  );
  assert.deepEqual(
    payload.operations
      .filter((row) => row.status === 'pending')
      .map((row) => `${row.namespace}.${row.operation}`),
    ['ApplicationServices.document.lock'],
    'the document lock is the one remaining reservation',
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
  // Row 0 is the one served read, so the pending cases pick a pending row by name.
  const missing = copy();
  const missingIndex = rowIndex(missing, 'ApplicationServices', 'document.lock');
  delete missing.operations[missingIndex].reason;
  assertRejected(missing, 'no reason');

  const wrong = copy();
  const wrongIndex = rowIndex(wrong, 'ApplicationServices', 'document.lock');
  wrong.operations[wrongIndex].reason = 'api.pending: DatabaseServices.transaction.commit';
  assertRejected(wrong, 'wrong pending reason');
});

test('a supported row must not carry a reason', () => {
  // Row 0 is the served read; adding a reason back to it is the breakage this
  // pins (any other supported row would exercise the same branch).
  const value = copy();
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
