// The cross-language contract test.
//
// `crates/cad-ui-core/api/plugin-api.json` is the shared authority: the Rust host
// reads it to answer `runtime.manifest`, and this package's `API_OPERATIONS` must
// be its exact mirror. It is **not** generated from this file — the host lane owns
// the JSON — so the only acceptable relationship is byte-for-byte semantic
// equality, checked here so drift fails a test rather than reaching a plugin.
//
// The table lists **host-served operations only**. Pure package features (the
// Geometry/Colors helpers, the manifest parser) deliberately have no row, so the
// row count is 20 and every row is `pending` in this build. The assertions below
// pin that shape: a row flipping to `supported` without a host implementation is
// a contract change, and this suite refuses to pass silently.
//
// If the JSON has not landed yet, this suite reports BLOCKED (a skip with an
// explicit message) instead of weakening the comparison. A missing authority file
// is a real gap, not permission to compare nothing.

import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';

import { API_NAMESPACES, API_OPERATIONS, API_VERSION } from '../src/index.ts';

const CONTRACT_URL = new URL(
  '../../../crates/cad-ui-core/api/plugin-api.json',
  import.meta.url,
);

interface ContractFile {
  readonly apiVersion?: unknown;
  readonly namespaces?: unknown;
  readonly operations?: unknown;
}

/**
 * Read namespace names from the contract's `namespaces` array.
 *
 * The authority file may list namespaces as bare strings or as objects carrying
 * the name (`{ "namespace": ... }` / `{ "name": ... }`); both are understood, and
 * anything else fails loudly. The operation rows below are compared exactly, so
 * this tolerance cannot hide a real contract drift.
 */
function namespaceNames(value: unknown): readonly unknown[] {
  if (!Array.isArray(value)) throw new Error('contract "namespaces" must be an array');
  return value.map((entry) => {
    if (typeof entry === 'string') return entry;
    if (typeof entry === 'object' && entry !== null) {
      const record = entry as Record<string, unknown>;
      const name = record['namespace'] ?? record['name'];
      if (typeof name === 'string') return name;
    }
    throw new Error(`contract namespace entry is unreadable: ${JSON.stringify(entry)}`);
  });
}

test('the table is 21 host operations: 18 pending and three served reads', () => {
  assert.equal(API_OPERATIONS.length, 21, 'the v1 descriptor has 21 host-operation rows');
  const supported = API_OPERATIONS.filter((row) => row.status === 'supported').map(
    (row) => `${row.namespace}.${row.operation}`,
  );
  // Three capabilities are served in this build, and these are exactly them.
  // Widening the set is a contract change: the host must serve the operation and
  // the WebView adapter must implement it first.
  assert.deepEqual(supported, [
    'ApplicationServices.documentManager.mdiActiveDocument',
    'ApplicationServices.documentManager.documents',
    'DatabaseServices.database.revision',
  ]);
  const pending = API_OPERATIONS.filter((row) => row.status === 'pending');
  assert.equal(pending.length, 18, 'every other host operation is still pending');
});

test('every capability row carries its canonical reason', () => {
  for (const row of API_OPERATIONS) {
    if (row.status === 'pending') {
      assert.equal(
        row.reason,
        `api.pending: ${row.namespace}.${row.operation}`,
        `${row.namespace}.${row.operation} must carry its canonical pending reason`,
      );
    } else {
      assert.equal(
        row.reason,
        undefined,
        `${row.namespace}.${row.operation} must not carry a reason when it is ${row.status}`,
      );
    }
  }
});

test('pure package features have no descriptor row', () => {
  // The Geometry/Colors helpers and the manifest parser never cross the wire, so
  // they are not capabilities. A row here would advertise host state that does
  // not exist.
  for (const row of API_OPERATIONS) {
    assert.notEqual(row.namespace, 'Geometry', `${row.operation} must not be a Geometry row`);
    assert.notEqual(row.namespace, 'Colors', `${row.operation} must not be a Colors row`);
    assert.notEqual(row.operation, 'runtime.manifest', 'the manifest parser needs no row');
  }
});

test('crates/cad-ui-core/api/plugin-api.json mirrors API_OPERATIONS', async (context) => {
  let text: string;
  try {
    text = await readFile(CONTRACT_URL, 'utf8');
  } catch (error) {
    if ((error as { code?: unknown }).code === 'ENOENT') {
      context.skip(
        'BLOCKED: crates/cad-ui-core/api/plugin-api.json is missing (owned by the Rust lane)',
      );
      return;
    }
    throw error;
  }

  let contract: ContractFile;
  try {
    contract = JSON.parse(text) as ContractFile;
  } catch (error) {
    throw new Error(`contract file is not valid JSON: ${String(error)}`);
  }

  assert.equal(
    contract.apiVersion,
    API_VERSION,
    `contract apiVersion drifted: json=${String(contract.apiVersion)} package=${API_VERSION}`,
  );

  assert.deepEqual(
    namespaceNames(contract.namespaces),
    [...API_NAMESPACES],
    'contract namespaces drifted from API_NAMESPACES',
  );

  assert.ok(Array.isArray(contract.operations), 'contract "operations" must be an array');
  const rows = contract.operations as readonly unknown[];
  assert.equal(
    rows.length,
    API_OPERATIONS.length,
    `operation count drifted: json=${rows.length} package=${API_OPERATIONS.length}`,
  );

  for (let index = 0; index < API_OPERATIONS.length; index += 1) {
    const expected = API_OPERATIONS[index];
    const expectedRow: Record<string, unknown> = {
      namespace: expected.namespace,
      operation: expected.operation,
      status: expected.status,
    };
    if (expected.reason !== undefined) expectedRow['reason'] = expected.reason;
    const actual = rows[index];
    assert.deepEqual(
      actual,
      expectedRow,
      `operation row ${index} drifted:\n  json:    ${JSON.stringify(actual)}\n  package: ${JSON.stringify(expectedRow)}`,
    );
  }
});
