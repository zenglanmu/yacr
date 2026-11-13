// Capability gating: the shipped table refuses every host operation by name, and
// refuses it *before* the bridge is asked.
//
// The spy counter is the point of this suite. A refusal that still sends a
// request would be a real bug (the host would have to reject what the client
// already knew it could not do), and it would look identical from the plugin side.
//
// Three refusal families are asserted separately because they mean different
// things: `pending` (reserved, not served yet), `unsupported` (never on this
// carrier) and `descriptor_missing` (the host does not know the operation at all).

import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  API_OPERATIONS,
  CadApiError,
  createCadApi,
  handle,
  objectId,
  parsePluginManifest,
  TransactionManager,
} from '../src/index.ts';
import type {
  CadApiHostInfo,
  CadApiOperation,
  CadApiRequest,
  CadHostBridge,
} from '../src/index.ts';
import { createDocument } from '../src/application-services.ts';
import { createDatabase } from '../src/database-services.ts';
import { createWire } from '../src/internal/capability.ts';

interface SpyBridge {
  readonly bridge: CadHostBridge;
  readonly calls: CadApiRequest[];
}

function hostInfo(
  operations: readonly CadApiOperation[],
  options: {
    readonly apiVersion?: number;
    readonly documentOwner?: CadApiHostInfo['documentOwner'];
  } = {},
): CadApiHostInfo {
  return {
    apiVersion: options.apiVersion ?? 1,
    carrier: 'native-below',
    documentOwner: options.documentOwner ?? 'native',
    operations,
  };
}

function spyBridge(
  operations: readonly CadApiOperation[],
  answer: (request: CadApiRequest) => unknown = () => {
    throw new Error('the bridge must not be called for a refused operation');
  },
  options: {
    readonly apiVersion?: number;
    readonly documentOwner?: CadApiHostInfo['documentOwner'];
  } = {},
): SpyBridge {
  const calls: CadApiRequest[] = [];
  const bridge: CadHostBridge = {
    async hostInfo(): Promise<CadApiHostInfo> {
      return hostInfo(operations, options);
    },
    async request<Result>(request: CadApiRequest): Promise<Result> {
      calls.push(request);
      return answer(request) as Result;
    },
  };
  return { bridge, calls };
}

function supported(operations: readonly CadApiOperation[]): readonly CadApiOperation[] {
  return operations.map((row) => ({
    namespace: row.namespace,
    operation: row.operation,
    status: 'supported' as const,
  }));
}

function withStatus(
  operations: readonly CadApiOperation[],
  operation: string,
  status: CadApiOperation['status'],
): readonly CadApiOperation[] {
  return operations.map((row) =>
    row.operation === operation
      ? {
          namespace: row.namespace,
          operation: row.operation,
          status,
          ...(status === 'supported' ? {} : { reason: `${status}: test row` }),
        }
      : row,
  );
}

function renderError(error: unknown): string {
  if (error instanceof CadApiError) return `${error.name}(${error.code}): ${error.message}`;
  if (error instanceof Error) return `${error.name}: ${error.message}`;
  return String(error);
}

function expectSyncCode(run: () => unknown, code: string): CadApiError {
  let caught: unknown;
  try {
    run();
  } catch (error) {
    caught = error;
  }
  if (!(caught instanceof CadApiError)) {
    throw new Error(`expected CadApiError(${code}), got ${renderError(caught)}`);
  }
  assert.equal(caught.code, code);
  return caught;
}

async function expectRejectCode(run: () => Promise<unknown>, code: string): Promise<CadApiError> {
  let caught: unknown;
  try {
    await run();
  } catch (error) {
    caught = error;
  }
  if (!(caught instanceof CadApiError)) {
    throw new Error(`expected CadApiError(${code}), got ${renderError(caught)}`);
  }
  assert.equal(caught.code, code);
  return caught;
}

test('every gated v1 call refuses with its exact pending code and no request', async () => {
  const spy = spyBridge(API_OPERATIONS);
  const api = await createCadApi(spy.bridge);

  const editorCalls: readonly (readonly [string, () => Promise<unknown>])[] = [
    ['editor.getPoint', () => api.editor.getPoint()],
    ['editor.getEntity', () => api.editor.getEntity()],
    ['editor.getSelection', () => api.editor.getSelection()],
    ['editor.getString', () => api.editor.getString()],
    ['editor.getDouble', () => api.editor.getDouble()],
    ['editor.getInteger', () => api.editor.getInteger()],
    ['editor.getKeyword', () => api.editor.getKeyword({ keywords: ['YES', 'NO'] })],
    ['editor.writeMessage', () => api.editor.writeMessage('hello')],
    ['editor.selectAll', () => api.editor.selectAll()],
  ];
  for (const [operation, run] of editorCalls) {
    await expectRejectCode(run, `api.pending: EditorInput.${operation}`);
  }

  const wire = createWire(hostInfo(API_OPERATIONS), spy.bridge);

  const database = createDatabase(wire, { id: 'db-1', version: '3' });
  await expectRejectCode(
    () => database.getObjectId(handle('2A')),
    'api.pending: DatabaseServices.database.getObjectId',
  );
  await expectRejectCode(
    () => database.symbolTables,
    'api.pending: DatabaseServices.database.symbolTables',
  );
  await expectRejectCode(
    () => database.clayer,
    'api.pending: DatabaseServices.database.symbolTables',
  );

  const document = createDocument(wire, {
    id: 'doc-1',
    name: 'drawing.dwg',
    isReadOnly: false,
    database: { id: 'db-1', version: '3' },
  });
  await expectRejectCode(
    () => document.lockDocument(),
    'api.pending: ApplicationServices.document.lock',
  );

  // `startTransaction` is synchronous, so its refusal is thrown, not rejected.
  const manager = new TransactionManager(wire, 'doc-1');
  expectSyncCode(
    () => manager.startTransaction({ expectedRevision: 1 }),
    'api.pending: DatabaseServices.transaction.start',
  );

  assert.equal(spy.calls.length, 0, `no request may escape on a refusal: ${JSON.stringify(spy.calls)}`);
});

test('transaction writes refuse by name, while abort stays local', async () => {
  // `transaction.start` is advertised so a transaction can be opened; every other
  // DatabaseServices row stays exactly as shipped (`pending`).
  const operations = withStatus(API_OPERATIONS, 'transaction.start', 'supported');
  const spy = spyBridge(operations);
  const wire = createWire(hostInfo(operations), spy.bridge);
  const manager = new TransactionManager(wire, 'doc-1');
  const transaction = manager.startTransaction({ expectedRevision: 1 });

  await expectRejectCode(
    () => transaction.getObject(objectId('2A'), 'read'),
    'api.pending: DatabaseServices.transaction.getObject',
  );
  expectSyncCode(
    () =>
      transaction.addNewlyCreatedDBObject(
        { typeName: 'Line', layer: 'A', space: 'model', parameters: {} },
        true,
      ),
    'api.pending: DatabaseServices.transaction.addNewlyCreated',
  );
  await expectRejectCode(
    () => transaction.commit(),
    'api.pending: DatabaseServices.transaction.commit',
  );
  assert.equal(transaction.closed, false, 'a gate refusal must not close the transaction');

  // Abort never touches the wire, so a pending `transaction.abort` row does not
  // block the local rollback.
  await transaction.abort();
  assert.equal(transaction.closed, true);
  assert.equal(spy.calls.length, 0);
});

test('an operation the host does not name is descriptor_missing, not pending', async () => {
  const operations = API_OPERATIONS.filter((row) => row.namespace !== 'DatabaseServices');
  const spy = spyBridge(operations);
  const wire = createWire(hostInfo(operations), spy.bridge);
  await expectRejectCode(
    () => wire.call('DatabaseServices', 'database.getObjectId', {}),
    'api.descriptor_missing: DatabaseServices.database.getObjectId',
  );
  // Geometry has no rows at all: the helpers are package features, so asking the
  // host for a Geometry operation is a contract mismatch.
  await expectRejectCode(
    () => wire.call('Geometry', 'geometry.point3d', {}),
    'api.descriptor_missing: Geometry.geometry.point3d',
  );
  assert.equal(spy.calls.length, 0);
});

test('only an explicit unsupported row produces unsupported-runtime', async () => {
  const operations = withStatus(API_OPERATIONS, 'editor.selectAll', 'unsupported');
  const spy = spyBridge(operations);
  const api = await createCadApi(spy.bridge);
  await expectRejectCode(
    () => api.editor.selectAll(),
    'unsupported-runtime: cad_api.EditorInput.editor.selectAll',
  );
  assert.equal(spy.calls.length, 0);
});

test('the served active-document read calls the bridge and decodes the document', async () => {
  const spy = spyBridge(API_OPERATIONS, () => ({
    document: {
      id: '42',
      name: 'Drawing1',
      isReadOnly: false,
      database: { id: '7', version: '15' },
    },
  }));
  const api = await createCadApi(spy.bridge);
  const document = await api.application.documentManager.mdiActiveDocument();
  if (document === null) throw new Error('expected the served read to resolve a document');
  assert.equal(document.name, 'Drawing1');
  assert.equal(document.isReadOnly, false);
  assert.equal(document.database.id, '7');
  assert.equal(document.database.version, '15');
  assert.deepEqual(spy.calls, [
    {
      namespace: 'ApplicationServices',
      operation: 'documentManager.mdiActiveDocument',
      parameters: {},
    },
  ]);
});

test('a document manager with no open document answers null, not a failure', async () => {
  const spy = spyBridge(API_OPERATIONS, () => ({ document: null }));
  const api = await createCadApi(spy.bridge);
  assert.equal(await api.application.documentManager.mdiActiveDocument(), null);
  assert.equal(spy.calls.length, 1);
});

test('a malformed document answer is refused field by field', async () => {
  const cases: readonly (readonly [unknown, string])[] = [
    [{}, 'api.invalid_argument: bridge.answer.document'],
    [{ document: 7 }, 'api.invalid_argument: bridge.answer.document'],
    [
      { document: { name: 'x', isReadOnly: false, database: { id: '7', version: '1' } } },
      'api.invalid_argument: bridge.answer.document.id',
    ],
    [
      { document: { id: '42', isReadOnly: false, database: { id: '7', version: '1' } } },
      'api.invalid_argument: bridge.answer.document.name',
    ],
    [
      { document: { id: '42', name: 'x', isReadOnly: 'no', database: { id: '7', version: '1' } } },
      'api.invalid_argument: bridge.answer.document.isReadOnly',
    ],
    [
      { document: { id: '42', name: 'x', isReadOnly: false } },
      'api.invalid_argument: bridge.answer.document.database',
    ],
    [
      { document: { id: '42', name: 'x', isReadOnly: false, database: { version: '1' } } },
      'api.invalid_argument: bridge.answer.document.database.id',
    ],
    [
      { document: { id: '42', name: 'x', isReadOnly: false, database: { id: '7' } } },
      'api.invalid_argument: bridge.answer.document.database.version',
    ],
  ];
  for (const [answer, code] of cases) {
    const api = await createCadApi(spyBridge(API_OPERATIONS, () => answer).bridge);
    await expectRejectCode(() => api.application.documentManager.mdiActiveDocument(), code);
  }
});

test('a cad_waiting refusal reaches the caller unchanged', async () => {
  const spy = spyBridge(API_OPERATIONS, () => {
    throw { code: 'cad_waiting', message: 'cad_waiting' };
  });
  const api = await createCadApi(spy.bridge);
  await expectRejectCode(
    () => api.application.documentManager.mdiActiveDocument(),
    'cad_waiting',
  );
});

test('the served document list calls the bridge and decodes every descriptor', async () => {
  const spy = spyBridge(API_OPERATIONS, () => ({
    documents: [
      { id: '42', name: 'Drawing1', isReadOnly: false, database: { id: '7', version: '15' } },
    ],
  }));
  const api = await createCadApi(spy.bridge);
  const documents = await api.application.documentManager.documents();
  assert.equal(documents.length, 1);
  const document = documents[0];
  assert.equal(document.name, 'Drawing1');
  assert.equal(document.isReadOnly, false);
  assert.equal(document.database.id, '7');
  assert.equal(document.database.version, '15');
  assert.deepEqual(spy.calls, [
    {
      namespace: 'ApplicationServices',
      operation: 'documentManager.documents',
      parameters: {},
    },
  ]);
});

test('an empty document list is the real "no document open" answer', async () => {
  const spy = spyBridge(API_OPERATIONS, () => ({ documents: [] }));
  const api = await createCadApi(spy.bridge);
  assert.deepEqual(await api.application.documentManager.documents(), []);
  assert.equal(spy.calls.length, 1);
});

test('a malformed document list is refused field by field', async () => {
  const cases: readonly (readonly [unknown, string])[] = [
    [{}, 'api.invalid_argument: bridge.answer.documents'],
    [{ documents: null }, 'api.invalid_argument: bridge.answer.documents'],
    [{ documents: 5 }, 'api.invalid_argument: bridge.answer.documents'],
    [{ documents: [7] }, 'api.invalid_argument: bridge.answer.documents'],
    [
      { documents: [{ name: 'x', isReadOnly: false, database: { id: '7', version: '1' } }] },
      'api.invalid_argument: bridge.answer.documents.id',
    ],
    [
      { documents: [{ id: '1', name: 'x', isReadOnly: false }] },
      'api.invalid_argument: bridge.answer.documents.database',
    ],
    [
      { documents: [{ id: '1', name: 'x', isReadOnly: false, database: { id: '7' } }] },
      'api.invalid_argument: bridge.answer.documents.database.version',
    ],
  ];
  for (const [answer, code] of cases) {
    const api = await createCadApi(spyBridge(API_OPERATIONS, () => answer).bridge);
    await expectRejectCode(() => api.application.documentManager.documents(), code);
  }
});

test('the served revision read sends the database id and returns a number', async () => {
  const spy = spyBridge(API_OPERATIONS, (request) => {
    assert.equal(request.operation, 'database.revision');
    return { revision: 12 };
  });
  const wire = createWire(hostInfo(API_OPERATIONS), spy.bridge);
  const database = createDatabase(wire, { id: '7', version: '15' });
  assert.equal(await database.revision(), 12);
  assert.deepEqual(spy.calls, [
    {
      namespace: 'DatabaseServices',
      operation: 'database.revision',
      parameters: { database: '7' },
    },
  ]);
});

test('a malformed revision answer is refused, never coerced', async () => {
  const answers: readonly unknown[] = [
    {},
    { revision: '12' },
    { revision: -1 },
    { revision: 1.5 },
    { revision: Number.POSITIVE_INFINITY },
    { revision: Number.NaN },
    { revision: null },
  ];
  for (const answer of answers) {
    const spy = spyBridge(API_OPERATIONS, () => answer);
    const wire = createWire(hostInfo(API_OPERATIONS), spy.bridge);
    const database = createDatabase(wire, { id: '7', version: '15' });
    await expectRejectCode(
      () => database.revision(),
      'api.invalid_argument: bridge.answer.revision',
    );
  }
});

test('a descriptor that serves document operations owes a document owner', async () => {
  const operations = withStatus(API_OPERATIONS, 'database.getObjectId', 'supported');
  const contradiction = spyBridge(operations, undefined, { documentOwner: 'none' });
  const error = await expectRejectCode(
    () => createCadApi(contradiction.bridge),
    'api.descriptor_invalid: documentOwner',
  );
  assert.equal(contradiction.calls.length, 0);
  assert.ok(error.message.length > 0);

  // The same rows with an owner are a legitimate descriptor.
  const owned = spyBridge(supported(API_OPERATIONS), () => ({
    objectId: null,
    revision: 1,
    created: [],
    accepted: true,
  }));
  await createCadApi(owned.bridge);
});

test('a host from another API generation is refused before anything is wired', async () => {
  const outdated = spyBridge(API_OPERATIONS, undefined, { apiVersion: 0 });
  await expectRejectCode(() => createCadApi(outdated.bridge), 'api.host-outdated: 0');
  assert.equal(outdated.calls.length, 0);

  const future = spyBridge(API_OPERATIONS, undefined, { apiVersion: 2 });
  await expectRejectCode(() => createCadApi(future.bridge), 'api.host-outdated: 2');
});

test('a malformed host snapshot is refused with the offending field', async () => {
  const notAnObject: CadHostBridge = {
    hostInfo: async () => null as never,
    request: async <Result>(_request: CadApiRequest): Promise<Result> => ({}) as Result,
  };
  await expectRejectCode(() => createCadApi(notAnObject), 'api.invalid_argument: hostInfo');

  const unknownNamespace: CadHostBridge = {
    hostInfo: async () =>
      ({
        apiVersion: 1,
        carrier: 'native-below',
        documentOwner: 'native',
        operations: [{ namespace: 'Nope', operation: 'x.y', status: 'pending', reason: 'r' }],
      }) as never,
    request: async <Result>(_request: CadApiRequest): Promise<Result> => ({}) as Result,
  };
  await expectRejectCode(
    () => createCadApi(unknownNamespace),
    'api.invalid_argument: hostInfo.operations[0].namespace',
  );
});

test('a void operation must acknowledge with { accepted: true }', async () => {
  const operations = withStatus(API_OPERATIONS, 'editor.writeMessage', 'supported');
  const answers: readonly (readonly [unknown, boolean])[] = [
    [{ accepted: true }, true],
    [{ accepted: false }, false],
    [{}, false],
    [{ ok: 1 }, false],
    [undefined, false],
    [null, false],
    [{ accepted: 'true' }, false],
  ];
  for (const [answer, accepted] of answers) {
    const spy = spyBridge(operations, () => answer);
    const api = await createCadApi(spy.bridge);
    if (accepted) {
      await api.editor.writeMessage('hello');
      assert.equal(spy.calls.length, 1);
    } else {
      await expectRejectCode(() => api.editor.writeMessage('hello'), 'api.invalid_argument: bridge.answer');
    }
  }
});

test('document.lock is a void operation and needs the same ack', async () => {
  const operations = withStatus(API_OPERATIONS, 'document.lock', 'supported');
  const spy = spyBridge(operations, () => ({ accepted: true }));
  const api = await createCadApi(spy.bridge);
  const wire = createWire(hostInfo(operations), spy.bridge);
  const document = createDocument(wire, {
    id: 'doc-1',
    name: 'drawing.dwg',
    isReadOnly: false,
    database: { id: 'db-1', version: '3' },
  });
  await document.lockDocument();
  assert.deepEqual(spy.calls[0].parameters, { document: 'doc-1' });
});

test('a supported operation reaches the bridge and its answer is mapped', async () => {
  const operations = supported(API_OPERATIONS);
  const spy = spyBridge(operations, (request) => {
    assert.equal(request.operation, 'editor.getString');
    return { status: 'Ok', value: 'hello' };
  });
  const api = await createCadApi(spy.bridge);
  const answer = await api.editor.getString({ message: 'name?' });
  assert.deepEqual(answer, { status: 'Ok', value: 'hello' });
  assert.equal(spy.calls.length, 1);
  assert.deepEqual(spy.calls[0].parameters, { message: 'name?' });
});

test('a null answer and an unknown prompt status are refused, never coerced', async () => {
  const operations = supported(API_OPERATIONS);
  const nullSpy = spyBridge(operations, () => null);
  const nullApi = await createCadApi(nullSpy.bridge);
  await expectRejectCode(
    () => nullApi.editor.getString(),
    'api.invalid_argument: bridge.answer',
  );

  const oddSpy = spyBridge(operations, () => ({ status: 'Maybe' }));
  const oddApi = await createCadApi(oddSpy.bridge);
  await expectRejectCode(
    () => oddApi.editor.getString(),
    'api.invalid_argument: bridge.answer.status',
  );
});

test('a shaped bridge rejection keeps its stable code', async () => {
  const operations = supported(API_OPERATIONS);
  const spy = spyBridge(operations, () => {
    throw { code: 'unsupported-runtime: cad_api.EditorInput.editor.getString', message: 'no' };
  });
  const api = await createCadApi(spy.bridge);
  await expectRejectCode(
    () => api.editor.getString(),
    'unsupported-runtime: cad_api.EditorInput.editor.getString',
  );
});

test('Geometry, Colors and the manifest parser never touch the bridge', async () => {
  const spy = spyBridge(API_OPERATIONS);
  const api = await createCadApi(spy.bridge);

  assert.deepEqual(api.geometry.point3d(1, 2, 3), { x: 1, y: 2, z: 3 });
  assert.deepEqual(api.geometry.vector3d(0, 0, 1), { x: 0, y: 0, z: 1 });
  assert.equal(api.geometry.matrix3dIdentity().values.length, 16);
  assert.equal(api.colors.aciToRgb(1), '#ff0000');
  assert.equal(api.colors.aciToRgb(300), undefined);
  assert.equal(api.colors.aciToRgb(0), undefined, '0 is ByBlock, not a colour');
  assert.deepEqual(
    parsePluginManifest(
      JSON.stringify({
        manifestVersion: 1,
        id: 'sample-hello',
        name: 'Sample Hello',
        version: '0.1.0',
        apiVersion: 1,
        entry: 'index.js',
      }),
    ).id,
    'sample-hello',
  );

  assert.equal(spy.calls.length, 0);
});
