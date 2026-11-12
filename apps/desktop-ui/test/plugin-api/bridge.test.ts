// Contract tests for the host bridge and the assembled plugin client.
//
// The bridge is the only place the app calls the `cad_api_status` wire command
// and the only place a plugin request could reach the host. Both are asserted
// here with a fake `invoke` that counts calls, because the two ways this seam
// can lie are "asked the host anyway" and "answered without asking" — a call
// count is what distinguishes them.
//
// These tests are `.ts` by filename only: they run under Node's type stripping.

import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { test } from 'node:test';

import { createCadHostBridge, IMPLEMENTED_OPERATIONS } from '../../src/plugin-api/bridge.ts';
import { openPluginApi } from '../../src/plugin-api/session.ts';

const CONTRACT_URL = new URL('../../../../crates/cad-ui-core/api/plugin-api.json', import.meta.url);
const payload = JSON.parse(await readFile(CONTRACT_URL, 'utf8'));

/** A minimal `cad_status` stand-in: only `hostProfile` is read by the adapter. */
function statusWith(surfaceMode, documentOwner) {
  return { hostProfile: { surfaceMode, documentOwner } };
}

/** An `invoke` double that records every command and answers `answer`. */
function countingInvoke(answer) {
  const commands = [];
  const invoke = async (command) => {
    commands.push(command);
    return answer;
  };
  return { invoke, commands };
}

/** The stable code of a rejected `CadApiError`. */
function codeOf(error) {
  return error instanceof Error ? error.code : undefined;
}

test('hostInfo() calls cad_api_status once and merges the status snapshot', async () => {
  const { invoke, commands } = countingInvoke(payload);
  const bridge = createCadHostBridge({ invoke, status: () => statusWith('dom-canvas', 'wasm') });
  const info = await bridge.hostInfo();
  assert.deepEqual(commands, ['cad_api_status']);
  assert.equal(info.apiVersion, 1);
  assert.equal(info.carrier, 'dom-canvas');
  assert.equal(info.documentOwner, 'wasm');
  assert.equal(info.operations.length, 20);
});

test('hostInfo() reports unknown/none before any status is observed', async () => {
  const { invoke } = countingInvoke(payload);
  const info = await createCadHostBridge({ invoke, status: () => null }).hostInfo();
  assert.equal(info.carrier, 'unknown');
  assert.equal(info.documentOwner, 'none');
});

test('a host refusal propagates verbatim', async () => {
  const refusal = new Error('unsupported-runtime: cad_api_status');
  const invoke = async () => {
    throw refusal;
  };
  await assert.rejects(
    createCadHostBridge({ invoke, status: () => null }).hostInfo(),
    (error) => {
      assert.ok(error instanceof Error);
      assert.equal(error.message, refusal.message);
      assert.equal(error, refusal);
      return true;
    },
  );
});

test('a malformed host answer is refused with the cad_api_status prefix', async () => {
  const { invoke } = countingInvoke({ apiVersion: 9, namespaces: ['Nope'], operations: [] });
  await assert.rejects(
    createCadHostBridge({ invoke, status: () => null }).hostInfo(),
    (error) => {
      assert.ok(error instanceof Error);
      assert.ok(error.message.startsWith('cad_api_status: '), error.message);
      return true;
    },
  );
});

test('no operation is implemented in this build', () => {
  assert.deepEqual(IMPLEMENTED_OPERATIONS, []);
});

test('every supported descriptor row has an adapter implementation', async () => {
  // The one rule that keeps the descriptor honest: a row may only be
  // `supported` if this adapter can actually serve it. Vacuously true today
  // (all 20 rows are pending), and red the moment a row flips without a case in
  // `bridge.ts` - which is exactly when it must be caught.
  const contract = JSON.parse(await readFile(CONTRACT_URL, 'utf8'));
  const supportedRows = contract.operations.filter((row) => row.status === 'supported');
  for (const row of supportedRows) {
    assert.ok(
      IMPLEMENTED_OPERATIONS.includes(`${row.namespace}.${row.operation}`),
      `${row.namespace}.${row.operation} is advertised as supported but has no adapter implementation`,
    );
  }
});

test('a pending row is refused by name and the host is never called', async () => {
  const { invoke, commands } = countingInvoke(payload);
  const bridge = createCadHostBridge({ invoke, status: () => null });
  await assert.rejects(
    bridge.request({
      namespace: 'DatabaseServices',
      operation: 'transaction.commit',
      parameters: {},
    }),
    (error) => {
      assert.equal(codeOf(error), 'api.adapter-missing: DatabaseServices.transaction.commit');
      return true;
    },
  );
  assert.deepEqual(commands, []);
});

test('an operation outside the contract is refused by name, host untouched', async () => {
  const { invoke, commands } = countingInvoke(payload);
  const bridge = createCadHostBridge({ invoke, status: () => null });
  await assert.rejects(
    bridge.request({ namespace: 'Geometry', operation: 'geometry.doesNotExist', parameters: {} }),
    (error) => {
      assert.equal(codeOf(error), 'api.adapter-missing: Geometry.geometry.doesNotExist');
      return true;
    },
  );
  assert.deepEqual(commands, []);
});

test('a malformed request is api.invalid_argument: request, host untouched', async () => {
  const { invoke, commands } = countingInvoke(payload);
  const bridge = createCadHostBridge({ invoke, status: () => null });
  for (const request of [
    null,
    undefined,
    {},
    { namespace: 'Nope', operation: 'geometry.point3d' },
    { namespace: 'Geometry' },
    { namespace: 'Geometry', operation: '' },
  ]) {
    await assert.rejects(bridge.request(request), (error) => {
      assert.equal(codeOf(error), 'api.invalid_argument: request');
      return true;
    });
  }
  assert.deepEqual(commands, []);
});

test('openPluginApi assembles the client from the real contract JSON', async () => {
  const { invoke, commands } = countingInvoke(payload);
  const session = await openPluginApi({ invoke, status: () => statusWith('native-below', 'native') });

  assert.deepEqual(commands, ['cad_api_status'], 'one snapshot call per session');
  assert.equal(session.host.operations.length, 20);
  assert.equal(session.host.carrier, 'native-below');

  // Pure, always-local operations work without touching the host.
  assert.deepEqual(session.api.geometry.point3d(1, 2, 3), { x: 1, y: 2, z: 3 });
  assert.deepEqual(commands, ['cad_api_status'], 'pure calls never reach the wire');

  // A pending operation is refused by the package's capability gate.
  assert.throws(
    () => session.api.application.documentManager.mdiActiveDocument,
    (error) => {
      assert.equal(
        codeOf(error),
        'api.pending: ApplicationServices.documentManager.mdiActiveDocument',
      );
      return true;
    },
  );
});

test('the plugin-api modules import no Tauri package and hold no CJK copy', async () => {
  const dir = new URL('../../src/plugin-api/', import.meta.url);
  const entries = await readdir(dir, { withFileTypes: true });
  const names = entries.filter((entry) => entry.isFile() && /\.(ts|tsx)$/.test(entry.name));
  assert.ok(names.length > 0, 'the plugin-api directory must contain shipped modules');
  for (const entry of names) {
    const source = await readFile(new URL(entry.name, dir), 'utf8');
    assert.doesNotMatch(source, /@tauri-apps\/api/, `${entry.name}: no direct Tauri import`);
    assert.doesNotMatch(
      source,
      /[\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff]/,
      `${entry.name}: no hardcoded CJK`,
    );
  }
});
