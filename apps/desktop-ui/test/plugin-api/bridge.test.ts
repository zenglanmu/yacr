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

/**
 * An `invoke` double that records every command (and its args).
 *
 * `answer` is either one value for every command or a `(command, args) => value`
 * function, which is what the two-command flows below need (`cad_api_status`
 * then `cad_api_call`).
 */
function countingInvoke(answer) {
  const commands = [];
  const args = [];
  const invoke = async (command, payload) => {
    commands.push(command);
    args.push(payload);
    return typeof answer === 'function' ? answer(command, payload) : answer;
  };
  return { invoke, commands, args };
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
  assert.equal(info.operations.length, 17);
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

test('this build implements every served operation (reads, live session, prompts)', () => {
  assert.deepEqual(IMPLEMENTED_OPERATIONS, [
    'ApplicationServices.documentManager.mdiActiveDocument',
    'ApplicationServices.documentManager.documents',
    'DatabaseServices.database.symbolTables',
    'DatabaseServices.database.revision',
    'DatabaseServices.database.getObjectId',
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
  ]);
});

test('every supported descriptor row has an adapter implementation', async () => {
  // The one rule that keeps the descriptor honest: a row may only be
  // `supported` if this adapter can actually serve it. It has sixteen rows to
  // check today and goes red the moment another row flips without a `case` (or a
  // mutating implementation) in `bridge.ts` - which is exactly when it must be
  // caught.
  const contract = JSON.parse(await readFile(CONTRACT_URL, 'utf8'));
  const supportedRows = contract.operations.filter((row) => row.status === 'supported');
  for (const row of supportedRows) {
    assert.ok(
      IMPLEMENTED_OPERATIONS.includes(`${row.namespace}.${row.operation}`),
      `${row.namespace}.${row.operation} is advertised as supported but has no adapter implementation`,
    );
  }
});

test('the served read forwards to cad_api_call and returns the host value', async () => {
  const documentValue = {
    document: {
      id: '42',
      name: 'Drawing1',
      isReadOnly: false,
      database: { id: '7', version: '15' },
    },
  };
  const { invoke, commands, args } = countingInvoke((command) =>
    command === 'cad_api_status' ? payload : { ok: true, value: documentValue },
  );
  const bridge = createCadHostBridge({ invoke, status: () => statusWith('native-below', 'native') });
  const result = await bridge.request({
    namespace: 'ApplicationServices',
    operation: 'documentManager.mdiActiveDocument',
    parameters: {},
  });
  assert.deepEqual(result, documentValue);
  assert.deepEqual(commands, ['cad_api_call']);
  assert.deepEqual(args, [
    {
      namespace: 'ApplicationServices',
      operation: 'documentManager.mdiActiveDocument',
      parameters: {},
    },
  ]);
});

test('the served document list forwards to cad_api_call unchanged', async () => {
  const listValue = {
    documents: [
      { id: '42', name: 'Drawing1', isReadOnly: false, database: { id: '7', version: '15' } },
    ],
  };
  const { invoke, args } = countingInvoke((command) =>
    command === 'cad_api_status' ? payload : { ok: true, value: listValue },
  );
  const bridge = createCadHostBridge({ invoke, status: () => statusWith('native-below', 'native') });
  const result = await bridge.request({
    namespace: 'ApplicationServices',
    operation: 'documentManager.documents',
    parameters: {},
  });
  assert.deepEqual(result, listValue);
  assert.deepEqual(args, [
    {
      namespace: 'ApplicationServices',
      operation: 'documentManager.documents',
      parameters: {},
    },
  ]);
});

test('the served revision forwards with its database parameter', async () => {
  const { invoke, args } = countingInvoke((command) =>
    command === 'cad_api_status' ? payload : { ok: true, value: { revision: 12 } },
  );
  const bridge = createCadHostBridge({ invoke, status: () => statusWith('native-below', 'native') });
  const result = await bridge.request({
    namespace: 'DatabaseServices',
    operation: 'database.revision',
    parameters: { database: '7' },
  });
  assert.deepEqual(result, { revision: 12 });
  assert.deepEqual(args, [
    {
      namespace: 'DatabaseServices',
      operation: 'database.revision',
      parameters: { database: '7' },
    },
  ]);
});

test('the served symbol tables forward with their database parameter', async () => {
  const tablesValue = {
    blockTable: {
      name: 'blocks',
      records: [{ name: 'WALL', isLayout: false, origin: [0, 0, 0] }],
    },
    layerTable: {
      name: 'layers',
      records: [
        {
          name: '0',
          color: { aci: 7, rgb: '#ffffff' },
          visible: true,
          locked: false,
          frozen: false,
          linetype: 'Continuous',
        },
      ],
    },
    clayer: '0',
  };
  const { invoke, args } = countingInvoke((command) =>
    command === 'cad_api_status' ? payload : { ok: true, value: tablesValue },
  );
  const bridge = createCadHostBridge({ invoke, status: () => statusWith('native-below', 'native') });
  const result = await bridge.request({
    namespace: 'DatabaseServices',
    operation: 'database.symbolTables',
    parameters: { database: '7' },
  });
  assert.deepEqual(result, tablesValue);
  assert.deepEqual(args, [
    {
      namespace: 'DatabaseServices',
      operation: 'database.symbolTables',
      parameters: { database: '7' },
    },
  ]);
});

test('mutate submits one correlated request and polls until its outcome appears', async () => {
  let submitted = null;
  let polls = 0;
  const { invoke, commands } = countingInvoke((command, request) => {
    if (command === 'cad_api_status') return payload;
    if (command === 'cad_api_submit') {
      submitted = request;
      return { accepted: true };
    }
    if (command === 'cad_api_poll') {
      polls += 1;
      // Two empty polls (nothing completed yet), then our outcome.
      if (polls < 3) return { results: [] };
      return {
        results: [
          { requestId: submitted.requestId, ok: true, value: { revision: 3, created: ['9'] } },
        ],
      };
    }
    return { ok: false, error: `unsupported-runtime: cad_api.${command}` };
  });
  const bridge = createCadHostBridge({
    invoke,
    status: () => statusWith('native-below', 'native'),
    pollDelayMs: 0,
    maxPolls: 10,
  });

  const value = await bridge.mutate({
    namespace: 'DatabaseServices',
    operation: 'transaction.commit',
    parameters: { document: 'doc-1' },
  });

  assert.deepEqual(value, { revision: 3, created: ['9'] });
  assert.equal(polls, 3, 'empty polls are retried, not answered');
  assert.deepEqual(commands, ['cad_api_submit', 'cad_api_poll', 'cad_api_poll', 'cad_api_poll']);
  assert.equal(typeof submitted.requestId, 'string');
  assert.match(submitted.requestId, /^[A-Za-z0-9._-]{1,64}$/);
  assert.equal(submitted.namespace, 'DatabaseServices');
  assert.equal(submitted.operation, 'transaction.commit');
  assert.deepEqual(submitted.parameters, { document: 'doc-1' });
});

test('a mutation outcome refusal keeps the host code', async () => {
  let submitted = null;
  const { invoke } = countingInvoke((command, request) => {
    if (command === 'cad_api_status') return payload;
    if (command === 'cad_api_submit') {
      submitted = request;
      return { accepted: true };
    }
    return {
      results: [{ requestId: submitted.requestId, ok: false, error: 'api.revision-conflict' }],
    };
  });
  const bridge = createCadHostBridge({
    invoke,
    status: () => null,
    pollDelayMs: 0,
    maxPolls: 2,
  });
  await assert.rejects(
    bridge.mutate({
      namespace: 'DatabaseServices',
      operation: 'transaction.commit',
      parameters: {},
    }),
    (error) => {
      assert.equal(codeOf(error), 'api.revision-conflict');
      assert.equal(error.message, 'api.revision-conflict');
      return true;
    },
  );
});

test('an unanswered submission times out by request id', async () => {
  const { invoke } = countingInvoke((command) => {
    if (command === 'cad_api_status') return payload;
    if (command === 'cad_api_submit') return { accepted: true };
    return { results: [] };
  });
  const bridge = createCadHostBridge({
    invoke,
    status: () => null,
    pollDelayMs: 0,
    maxPolls: 3,
  });
  await assert.rejects(
    bridge.mutate({
      namespace: 'DatabaseServices',
      operation: 'transaction.commit',
      parameters: {},
    }),
    (error) => {
      assert.match(codeOf(error), /^api\.timeout: apir-/u);
      return true;
    },
  );
});

test('a malformed submit ack or poll answer is api.invalid_argument: bridge.answer', async () => {
  const answers = [
    [{ accepted: false }, { results: [] }],
    [{ ok: 1 }, { results: [] }],
    [{ accepted: true }, { nope: [] }],
    [{ accepted: true }, { results: 'none' }],
    [{ accepted: true }, { results: [7] }],
  ];
  for (const [ack, poll] of answers) {
    const { invoke } = countingInvoke((command) => {
      if (command === 'cad_api_status') return payload;
      if (command === 'cad_api_submit') return ack;
      return poll;
    });
    const bridge = createCadHostBridge({ invoke, status: () => null, pollDelayMs: 0, maxPolls: 2 });
    await assert.rejects(
      bridge.mutate({
        namespace: 'DatabaseServices',
        operation: 'transaction.commit',
        parameters: {},
      }),
      (error) => {
        assert.equal(codeOf(error), 'api.invalid_argument: bridge.answer');
        return true;
      },
    );
  }
});

test('a poll refusal propagates verbatim', async () => {
  const { invoke } = countingInvoke((command) => {
    if (command === 'cad_api_status') return payload;
    if (command === 'cad_api_submit') return { accepted: true };
    throw new Error('unsupported-runtime: cad_api_poll');
  });
  const bridge = createCadHostBridge({ invoke, status: () => null, pollDelayMs: 0, maxPolls: 2 });
  await assert.rejects(
    bridge.mutate({
      namespace: 'DatabaseServices',
      operation: 'transaction.commit',
      parameters: {},
    }),
    (error) => {
      assert.equal(codeOf(error), 'unsupported-runtime: cad_api_poll');
      return true;
    },
  );
});

test('a mutation this adapter does not implement is refused locally', async () => {
  const { invoke, commands } = countingInvoke(payload);
  const bridge = createCadHostBridge({ invoke, status: () => null });
  await assert.rejects(
    bridge.mutate({
      namespace: 'DatabaseServices',
      operation: 'transaction.start',
      parameters: {},
    }),
    (error) => {
      assert.equal(codeOf(error), 'api.adapter-missing: DatabaseServices.transaction.start');
      return true;
    },
  );
  assert.deepEqual(commands, []);
});

test('the served current selection forwards to cad_api_call', async () => {
  const { invoke, args } = countingInvoke((command) =>
    command === 'cad_api_status' ? payload : { ok: true, value: { objectIds: ['12', '13'] } },
  );
  const bridge = createCadHostBridge({ invoke, status: () => statusWith('native-below', 'native') });
  const result = await bridge.request({
    namespace: 'EditorInput',
    operation: 'editor.currentSelection',
    parameters: {},
  });
  assert.deepEqual(result, { objectIds: ['12', '13'] });
  assert.deepEqual(args, [
    {
      namespace: 'EditorInput',
      operation: 'editor.currentSelection',
      parameters: {},
    },
  ]);
});

test('a host refusal becomes the CadApiError code, verbatim', async () => {
  for (const marker of ['cad_waiting', 'unsupported-runtime: cad_api.ApplicationServices.documentManager.mdiActiveDocument']) {
    const { invoke } = countingInvoke(() => ({ ok: false, error: marker }));
    const bridge = createCadHostBridge({ invoke, status: () => null });
    await assert.rejects(
      bridge.request({
        namespace: 'ApplicationServices',
        operation: 'documentManager.mdiActiveDocument',
        parameters: {},
      }),
      (error) => {
        assert.equal(codeOf(error), marker);
        assert.equal(error.message, marker);
        return true;
      },
    );
  }
});

test('a malformed or contradictory outcome is api.invalid_argument: bridge.answer', async () => {
  const outcomes = [
    null,
    [],
    'ok',
    {},
    { ok: 'yes', value: {} },
    { ok: true },
    { ok: true, error: 'cad_waiting' },
    { ok: false },
    { ok: false, error: '' },
    { ok: false, error: 7 },
    { ok: false, error: 'cad_waiting', value: {} },
  ];
  for (const outcome of outcomes) {
    const { invoke } = countingInvoke(() => outcome);
    const bridge = createCadHostBridge({ invoke, status: () => null });
    await assert.rejects(
      bridge.request({
        namespace: 'ApplicationServices',
        operation: 'documentManager.mdiActiveDocument',
        parameters: {},
      }),
      (error) => {
        assert.equal(codeOf(error), 'api.invalid_argument: bridge.answer');
        return true;
      },
    );
  }
});

test('the served operation is bound to its namespace', async () => {
  const { invoke, commands } = countingInvoke({ ok: true, value: { document: null } });
  const bridge = createCadHostBridge({ invoke, status: () => null });
  await assert.rejects(
    bridge.request({
      namespace: 'DatabaseServices',
      operation: 'documentManager.mdiActiveDocument',
      parameters: {},
    }),
    (error) => {
      assert.equal(
        codeOf(error),
        'api.adapter-missing: DatabaseServices.documentManager.mdiActiveDocument',
      );
      return true;
    },
  );
  assert.deepEqual(commands, [], 'the host is never asked about a foreign namespace');
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
  const { invoke, commands } = countingInvoke((command) =>
    command === 'cad_api_status'
      ? payload
      : {
          ok: true,
          value: {
            document: {
              id: '42',
              name: 'Drawing1',
              isReadOnly: false,
              database: { id: '7', version: '15' },
            },
          },
        },
  );
  const session = await openPluginApi({ invoke, status: () => statusWith('native-below', 'native') });

  assert.deepEqual(commands, ['cad_api_status'], 'one snapshot call per session');
  assert.equal(session.host.operations.length, 17);
  assert.equal(session.host.carrier, 'native-below');

  // Pure, always-local operations work without touching the host.
  assert.deepEqual(session.api.geometry.point3d(1, 2, 3), { x: 1, y: 2, z: 3 });
  assert.deepEqual(commands, ['cad_api_status'], 'pure calls never reach the wire');

  // The one still-pending row is refused by the package's capability gate.
  const document = await session.api.application.documentManager.mdiActiveDocument();
  if (document === null) throw new Error('expected a document');
  await assert.rejects(document.lockDocument(), (error) => {
    assert.equal(codeOf(error), 'api.pending: ApplicationServices.document.lock');
    return true;
  });
});

test('the served read assembles a Document end to end', async () => {
  const { invoke, commands } = countingInvoke((command) =>
    command === 'cad_api_status'
      ? payload
      : {
          ok: true,
          value: {
            document: {
              id: '42',
              name: 'Drawing1',
              isReadOnly: true,
              database: { id: '7', version: '15' },
            },
          },
        },
  );
  const session = await openPluginApi({ invoke, status: () => statusWith('native-below', 'native') });

  const document = await session.api.application.documentManager.mdiActiveDocument();
  if (document === null) throw new Error('expected the served read to resolve a document');
  assert.equal(document.name, 'Drawing1');
  assert.equal(document.isReadOnly, true);
  assert.equal(document.database.id, '7');
  assert.equal(document.database.version, '15');

  // The document's own services still come from the capability-gated client, so a
  // pending row refuses by name even though the document itself is real.
  await assert.rejects(document.lockDocument(), (error) => {
    assert.equal(codeOf(error), 'api.pending: ApplicationServices.document.lock');
    return true;
  });
  assert.deepEqual(commands, ['cad_api_status', 'cad_api_call'], 'one read, no other traffic');
});

test('the served document list assembles documents end to end', async () => {
  const { invoke } = countingInvoke((command, request) => {
    if (command === 'cad_api_status') return payload;
    if (request.operation === 'documentManager.documents') {
      return {
        ok: true,
        value: {
          documents: [
            {
              id: '42',
              name: 'Drawing1',
              isReadOnly: false,
              database: { id: '7', version: '15' },
            },
          ],
        },
      };
    }
    if (request.operation === 'database.revision') return { ok: true, value: { revision: 15 } };
    return { ok: false, error: `unsupported-runtime: cad_api.${request.operation}` };
  });
  const session = await openPluginApi({ invoke, status: () => statusWith('native-below', 'native') });

  const documents = await session.api.application.documentManager.documents();
  assert.equal(documents.length, 1);
  assert.equal(documents[0].name, 'Drawing1');
  assert.equal(documents[0].database.id, '7');

  // The document's services are the same capability-gated client: the revision
  // read is served, but `document.lock` is still pending and refuses by name
  // without reaching the host.
  assert.equal(await documents[0].database.revision(), 15);
  await assert.rejects(documents[0].lockDocument(), (error) => {
    assert.equal(codeOf(error), 'api.pending: ApplicationServices.document.lock');
    return true;
  });
  assert.equal(await documents[0].database.revision(), 15);
});

test('the served symbol tables assemble end to end', async () => {
  const tablesValue = {
    blockTable: {
      name: 'blocks',
      records: [{ name: 'WALL', isLayout: false, origin: [12, 0, 0] }],
    },
    layerTable: {
      name: 'layers',
      records: [
        {
          name: '0',
          color: { aci: 7, rgb: '#ffffff' },
          visible: true,
          locked: false,
          frozen: false,
          linetype: 'Continuous',
        },
        {
          name: 'TRUE_COLOR',
          color: { aci: null, rgb: '#123abc' },
          visible: true,
          locked: false,
          frozen: false,
          linetype: 'Continuous',
        },
      ],
    },
    clayer: '',
  };
  const { invoke, args } = countingInvoke((command, request) => {
    if (command === 'cad_api_status') return payload;
    if (request.operation === 'documentManager.mdiActiveDocument') {
      return {
        ok: true,
        value: {
          document: {
            id: '42',
            name: 'Drawing1',
            isReadOnly: false,
            database: { id: '7', version: '15' },
          },
        },
      };
    }
    if (request.operation === 'database.symbolTables') return { ok: true, value: tablesValue };
    return { ok: false, error: `unsupported-runtime: cad_api.${request.operation}` };
  });
  const session = await openPluginApi({ invoke, status: () => statusWith('native-below', 'native') });
  const document = await session.api.application.documentManager.mdiActiveDocument();
  if (document === null) throw new Error('expected a document');

  const tables = await document.database.symbolTables;
  assert.deepEqual(tables.blockTable.names(), ['WALL']);
  assert.deepEqual(tables.blockTable.record('WALL')?.origin, { x: 12, y: 0, z: 0 });
  assert.deepEqual(tables.layerTable.names(), ['0', 'TRUE_COLOR']);
  assert.equal(tables.layerTable.record('TRUE_COLOR')?.color.aci, null);
  // The active layer name is the host's, including the honest empty string.
  assert.equal(await document.database.clayer, '');
  assert.deepEqual(args.at(-1), {
    namespace: 'DatabaseServices',
    operation: 'database.symbolTables',
    parameters: { database: '7' },
  });
});

test('the served current selection assembles end to end', async () => {
  const { invoke, commands } = countingInvoke((command) =>
    command === 'cad_api_status' ? payload : { ok: true, value: { objectIds: ['12', '13', '12'] } },
  );
  const session = await openPluginApi({ invoke, status: () => statusWith('dom-canvas', 'wasm') });

  const selection = await session.api.editor.currentSelection();
  assert.equal(selection.count, 3);
  assert.deepEqual(selection.getObjectIds(), ['12', '13', '12']);
  // The host deduplicates; the client passes the ids through untouched (a repeated
  // id here is the fake's, and the read never rewrites the answer).
  assert.deepEqual(commands, ['cad_api_status', 'cad_api_call']);
});

test('a commit goes through submit + poll and resolves the host outcome', async () => {
  const documentValue = {
    document: {
      id: '42',
      name: 'Drawing1',
      isReadOnly: false,
      database: { id: '7', version: '15' },
    },
  };
  let submitted = null;
  const { invoke, commands } = countingInvoke((command, request) => {
    if (command === 'cad_api_status') return payload;
    if (command === 'cad_api_call') return { ok: true, value: documentValue };
    if (command === 'cad_api_submit') {
      submitted = request;
      return { accepted: true };
    }
    if (command === 'cad_api_poll') {
      return {
        results: [
          { requestId: submitted.requestId, ok: true, value: { revision: 16, created: ['9'] } },
        ],
      };
    }
    return { ok: false, error: `unsupported-runtime: cad_api.${command}` };
  });
  const session = await openPluginApi({
    invoke,
    status: () => statusWith('native-below', 'native'),
    pollDelayMs: 0,
    maxPolls: 5,
  });

  const document = await session.api.application.documentManager.mdiActiveDocument();
  if (document === null) throw new Error('expected a document');
  const transaction = document.transactionManager.startTransaction({ expectedRevision: 15 });
  transaction.addNewlyCreatedDBObject(
    { typeName: 'Line', layer: '0', space: 'model', parameters: { start: [0, 0, 0] } },
    true,
  );

  const result = await transaction.commit();
  assert.deepEqual(result, { applied: true, revision: 16, created: ['9'] });
  assert.equal(typeof submitted.requestId, 'string');
  assert.equal(submitted.namespace, 'DatabaseServices');
  assert.equal(submitted.operation, 'transaction.commit');
  assert.deepEqual(submitted.parameters, {
    document: '42',
    transactionId: 'tx-1',
    expectedRevision: 15,
    edits: [
      {
        kind: 'insert',
        typeName: 'Line',
        layer: '0',
        space: 'model',
        parameters: { start: [0, 0, 0] },
      },
    ],
  });
  assert.deepEqual(commands, ['cad_api_status', 'cad_api_call', 'cad_api_submit', 'cad_api_poll']);
});

test('the served revision read returns the host number', async () => {
  const { invoke, args } = countingInvoke((command, request) => {
    if (command === 'cad_api_status') return payload;
    if (request.operation === 'documentManager.mdiActiveDocument') {
      return {
        ok: true,
        value: {
          document: {
            id: '42',
            name: 'Drawing1',
            isReadOnly: false,
            database: { id: '7', version: '15' },
          },
        },
      };
    }
    return { ok: true, value: { revision: 12 } };
  });
  const session = await openPluginApi({ invoke, status: () => statusWith('native-below', 'native') });
  const active = await session.api.application.documentManager.mdiActiveDocument();
  if (active === null) throw new Error('expected a document');
  assert.equal(await active.database.revision(), 12);
  assert.deepEqual(args.at(-1), {
    namespace: 'DatabaseServices',
    operation: 'database.revision',
    parameters: { database: '7' },
  });
});

test('the served read resolves null when no document is open', async () => {
  const { invoke } = countingInvoke((command) =>
    command === 'cad_api_status' ? payload : { ok: true, value: { document: null } },
  );
  const session = await openPluginApi({ invoke, status: () => statusWith('dom-canvas', 'wasm') });
  assert.equal(await session.api.application.documentManager.mdiActiveDocument(), null);
});

test('openPluginApi installs the command hooks when the caller supplies them', async () => {
  const registered = [];
  const removed = [];
  const { invoke } = countingInvoke(payload);
  const session = await openPluginApi({
    invoke,
    status: () => statusWith('native-below', 'native'),
    onCommandRegister: (registration) => {
      registered.push(registration.name);
    },
    onCommandUnregister: (name) => {
      removed.push(name);
    },
  });

  session.api.runtime.registerCommand('HELLO', 'modal', () => undefined);
  assert.deepEqual(registered, ['HELLO']);
  session.api.runtime.unregisterCommand('HELLO');
  assert.deepEqual(removed, ['HELLO']);
});

test('openPluginApi without hooks leaves command registration refused', async () => {
  const { invoke } = countingInvoke(payload);
  const session = await openPluginApi({ invoke, status: () => statusWith('native-below', 'native') });
  assert.throws(
    () => session.api.runtime.registerCommand('HELLO', 'modal', () => undefined),
    (error) => {
      assert.equal(codeOf(error), 'unsupported-runtime: cad_api.Runtime.registerCommand');
      return true;
    },
  );
});

test('the live-session reads run end to end over submit + poll', async () => {
  const lineObject = {
    typeName: 'Line',
    objectId: '12',
    handle: '1A',
    layer: 'WALLS',
    color: { method: 'byLayer' },
    linetype: { method: 'byLayer' },
    startPoint: [0, 0, 0],
    endPoint: [10, 0, 0],
    length: 10,
  };
  const submitted = [];
  const { invoke } = countingInvoke((command, request) => {
    if (command === 'cad_api_status') return payload;
    if (command === 'cad_api_call') {
      return {
        ok: true,
        value: {
          document: {
            id: '42',
            name: 'Drawing1',
            isReadOnly: false,
            database: { id: '7', version: '15' },
          },
        },
      };
    }
    if (command === 'cad_api_submit') {
      submitted.push(request);
      return { accepted: true };
    }
    if (command === 'cad_api_poll') {
      const last = submitted.at(-1);
      const value = last.operation === 'database.getObjectId'
        ? { objectId: '12' }
        : { object: lineObject };
      return { results: [{ requestId: last.requestId, ok: true, value }] };
    }
    return { ok: false, error: `unsupported-runtime: cad_api.${command}` };
  });
  const session = await openPluginApi({
    invoke,
    status: () => statusWith('native-below', 'native'),
    pollDelayMs: 0,
    maxPolls: 3,
  });

  const document = await session.api.application.documentManager.mdiActiveDocument();
  if (document === null) throw new Error('expected a document');
  assert.equal(await document.database.getObjectId('1A'), '12');

  const transaction = document.transactionManager.startTransaction({ expectedRevision: 15 });
  const object = await transaction.getObject('12', 'read');
  assert.equal(object.typeName, 'Line');
  assert.equal(object.layer, 'WALLS');

  assert.deepEqual(submitted.map((entry) => entry.operation), [
    'database.getObjectId',
    'transaction.getObject',
  ]);
  assert.deepEqual(submitted[0].parameters, { database: '7', handle: '1A' });
  assert.deepEqual(submitted[1].parameters, {
    document: '42',
    transactionId: 'tx-1',
    objectId: '12',
    mode: 'read',
  });
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
