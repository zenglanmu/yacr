// Transaction semantics: nesting, the depth ceiling, single close, the commit
// envelope, savepoint behaviour, local abort, the closed creatable-type set, the
// duplicate-edit rule, and decode-by-`typeName`.
//
// These run against a fake bridge that advertises every DatabaseServices row as
// `supported`, because this build ships them as `pending` (the capability suite
// proves that path). The fake records requests, so the assertions are about what
// actually crossed the wire — not about internal state.

import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  API_OPERATIONS,
  CadApiError,
  MAX_TRANSACTION_DEPTH,
  objectId,
  SUPPORTED_NEW_ENTITY_TYPES,
  TransactionManager,
  vector3d,
} from '../src/index.ts';
import type {
  CadApiHostInfo,
  CadApiOperation,
  CadApiRequest,
  CadHostBridge,
  TransactionOptions,
} from '../src/index.ts';
import { appendEdit } from '../src/transaction.ts';
import { createWire } from '../src/internal/capability.ts';

const DOCUMENT = 'doc-1';

interface FakeBridge {
  readonly bridge: CadHostBridge;
  readonly calls: CadApiRequest[];
}

function databaseOperations(): readonly CadApiOperation[] {
  return API_OPERATIONS.filter((row) => row.namespace === 'DatabaseServices').map((row) => ({
    namespace: row.namespace,
    operation: row.operation,
    status: 'supported' as const,
  }));
}

function hostInfo(): CadApiHostInfo {
  return {
    apiVersion: 1,
    carrier: 'native-below',
    documentOwner: 'native',
    operations: databaseOperations(),
  };
}

function fakeBridge(
  answer: (request: CadApiRequest) => unknown = () => ({ revision: 1, created: [] }),
): FakeBridge {
  const calls: CadApiRequest[] = [];
  const bridge: CadHostBridge = {
    async hostInfo(): Promise<CadApiHostInfo> {
      return hostInfo();
    },
    async request<Result>(request: CadApiRequest): Promise<Result> {
      calls.push(request);
      return answer(request) as Result;
    },
  };
  return { bridge, calls };
}

function managerFor(fake: FakeBridge): TransactionManager {
  return new TransactionManager(createWire(hostInfo(), fake.bridge), DOCUMENT);
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

function parametersOf(request: CadApiRequest): Readonly<Record<string, unknown>> {
  return request.parameters;
}

/** A host-shaped `Line` answer (no `handle`: the entity was created, not imported). */
function lineAnswer(): unknown {
  return {
    object: {
      typeName: 'Line',
      objectId: '2A',
      handle: null,
      layer: 'WALLS',
      color: { method: 'byLayer' },
      linetype: { method: 'byLayer' },
      startPoint: { x: 0, y: 0, z: 0 },
      endPoint: { x: 10, y: 0, z: 0 },
      length: 10,
    },
  };
}

test('a nested commit is a savepoint release and only the outer one applies', async () => {
  const fake = fakeBridge((request) =>
    request.operation === 'transaction.getObject' ? lineAnswer() : { revision: 5, created: ['2A'] },
  );
  const manager = managerFor(fake);
  const outer = manager.startTransaction({ expectedRevision: 3 });
  const inner = manager.startTransaction();

  assert.equal(outer.depth, 1);
  assert.equal(inner.depth, 2);
  await expectRejectCode(() => outer.commit(), 'api.transaction.nesting_order');
  assert.equal(fake.calls.length, 0, 'a refused nesting order must not reach the host');

  // Reads and buffered writes are allowed at any depth (LIFO gates closes only).
  const line = await inner.getObject(objectId('1F'), 'read');
  assert.equal(line.typeName, 'Line');
  assert.equal(line.handle, null, 'a created entity has no handle and none is invented');
  assert.equal(line.layer, 'WALLS');
  inner.addNewlyCreatedDBObject(
    { typeName: 'Line', layer: 'WALLS', space: 'model', parameters: { start: [0, 0, 0] } },
    true,
  );

  const nested = await inner.commit();
  assert.deepEqual(nested, { applied: false, revision: null, created: [] });
  assert.equal(manager.depth, 1);
  assert.equal(fake.calls.length, 1, 'only the inner read reached the host so far');

  const committed = await outer.commit();
  assert.deepEqual(committed, { applied: true, revision: 5, created: ['2A'] });
  assert.equal(manager.depth, 0);

  const commitRequest = fake.calls[1];
  assert.equal(commitRequest.namespace, 'DatabaseServices');
  assert.equal(commitRequest.operation, 'transaction.commit');
  const parameters = parametersOf(commitRequest);
  assert.equal(parameters['document'], DOCUMENT);
  assert.equal(typeof parameters['transactionId'], 'string');
  assert.equal(parameters['expectedRevision'], 3);
  assert.deepEqual(parameters['edits'], [
    {
      kind: 'insert',
      typeName: 'Line',
      layer: 'WALLS',
      space: 'model',
      parameters: { start: [0, 0, 0] },
    },
  ]);
});

test('an aborted inner transaction contributes no edits to the outer commit', async () => {
  const fake = fakeBridge(() => ({ revision: 9, created: [] }));
  const manager = managerFor(fake);
  const outer = manager.startTransaction({ expectedRevision: 2 });
  const inner = manager.startTransaction();

  inner.addNewlyCreatedDBObject(
    { typeName: 'Circle', layer: 'A', space: 'model', parameters: {} },
    true,
  );
  await inner.abort();
  assert.equal(manager.depth, 1);
  assert.equal(fake.calls.length, 0, 'abort is local: nothing crosses the wire');

  await outer.commit();
  assert.equal(fake.calls.length, 1);
  assert.deepEqual(parametersOf(fake.calls[0])['edits'], []);
});

test('abort is local only, closes the transaction and drops its buffer', async () => {
  const fake = fakeBridge();
  const manager = managerFor(fake);
  const transaction = manager.startTransaction({ expectedRevision: 7 });
  transaction.addNewlyCreatedDBObject(
    { typeName: 'Arc', layer: 'A', space: 'model', parameters: {} },
    true,
  );
  await transaction.abort();
  assert.equal(transaction.closed, true);
  assert.equal(manager.depth, 0);
  assert.equal(fake.calls.length, 0, 'no transaction.abort request is ever sent');
  await expectRejectCode(() => transaction.commit(), 'api.transaction.closed');
  await expectRejectCode(() => transaction.abort(), 'api.transaction.closed');
});

test('add === false records nothing', async () => {
  const fake = fakeBridge(() => ({ revision: 4, created: [] }));
  const manager = managerFor(fake);
  const transaction = manager.startTransaction({ expectedRevision: 3 });
  transaction.addNewlyCreatedDBObject(
    { typeName: 'Line', layer: 'A', space: 'model', parameters: {} },
    false,
  );
  await transaction.commit();
  assert.deepEqual(parametersOf(fake.calls[0])['edits'], []);
});

test('the creatable-type set is closed', () => {
  assert.deepEqual(SUPPORTED_NEW_ENTITY_TYPES, ['Line', 'Circle', 'Arc']);
  const manager = managerFor(fakeBridge());
  const transaction = manager.startTransaction({ expectedRevision: 1 });
  for (const typeName of ['Polyline', 'DBText', 'MText', '', 'line']) {
    expectSyncCode(
      () =>
        transaction.addNewlyCreatedDBObject(
          { typeName, layer: 'A', space: 'model', parameters: {} },
          true,
        ),
      'api.invalid_argument: typeName',
    );
  }
});

test('one transaction may not edit one entity twice', () => {
  const manager = managerFor(fakeBridge());
  const transaction = manager.startTransaction({ expectedRevision: 1 });
  appendEdit(transaction, {
    kind: 'translate',
    objectId: objectId('2A'),
    displacement: vector3d(1, 0, 0),
  });
  // A second write of either kind to the same id is refused, mirroring the host's
  // "an API batch cannot edit one entity more than once".
  expectSyncCode(
    () => appendEdit(transaction, { kind: 'delete', objectId: objectId('2A') }),
    'api.invalid_argument: edits.duplicate',
  );
  appendEdit(transaction, {
    kind: 'translate',
    objectId: objectId('2B'),
    displacement: vector3d(0, 0, 0),
  });
});

test('a savepoint merge applies the same duplicate rule across buffers', async () => {
  const manager = managerFor(fakeBridge());
  const outer = manager.startTransaction({ expectedRevision: 1 });
  const inner = manager.startTransaction();
  appendEdit(outer, { kind: 'delete', objectId: objectId('2A') });
  appendEdit(inner, {
    kind: 'translate',
    objectId: objectId('2A'),
    displacement: vector3d(0, 1, 0),
  });
  await expectRejectCode(() => inner.commit(), 'api.invalid_argument: edits.duplicate');
  assert.equal(inner.closed, false, 'a refused merge leaves the savepoint open');
  assert.equal(manager.depth, 2);
  await inner.abort();
  await outer.abort();
});

test('the transaction stack is capped at MAX_TRANSACTION_DEPTH', () => {
  const manager = managerFor(fakeBridge());
  for (let index = 0; index < MAX_TRANSACTION_DEPTH; index += 1) {
    // Only the outermost transaction owes the revision precondition.
    if (index === 0) manager.startTransaction({ expectedRevision: 0 });
    else manager.startTransaction();
  }
  assert.equal(MAX_TRANSACTION_DEPTH, 8);
  assert.equal(manager.depth, MAX_TRANSACTION_DEPTH);
  expectSyncCode(() => manager.startTransaction(), 'api.transaction.nesting_limit');
});

test('a transaction closes exactly once', async () => {
  const manager = managerFor(fakeBridge(() => ({ revision: 1, created: [] })));
  const transaction = manager.startTransaction({ expectedRevision: 0 });
  assert.equal(transaction.closed, false);
  await transaction.commit();
  assert.equal(transaction.closed, true);
  await expectRejectCode(() => transaction.commit(), 'api.transaction.closed');
  await expectRejectCode(() => transaction.abort(), 'api.transaction.closed');
});

test('expectedRevision is required and validated before the transaction opens', () => {
  const manager = managerFor(fakeBridge());
  const cases: readonly unknown[] = [undefined, {}, { expectedRevision: -1 }, { expectedRevision: 1.5 }];
  for (const options of cases) {
    expectSyncCode(
      () => manager.startTransaction(options as TransactionOptions | undefined),
      'api.invalid_argument: expectedRevision',
    );
  }
  assert.equal(manager.depth, 0);
});

test('a stale revision surfaces as api.revision-conflict with actualRevision', async () => {
  const fake = fakeBridge(() => {
    throw {
      code: 'api.revision-conflict',
      message: 'the drawing moved on',
      details: { actualRevision: 12 },
    };
  });
  const manager = managerFor(fake);
  const transaction = manager.startTransaction({ expectedRevision: 4 });
  const error = await expectRejectCode(() => transaction.commit(), 'api.revision-conflict');
  assert.equal(error.details?.['actualRevision'], 12);
  assert.equal(transaction.closed, true, 'a rejected commit still ends the transaction');
  assert.equal(manager.depth, 0);
});

test('a conflict without host details keeps the code and omits details', async () => {
  const fake = fakeBridge(() => {
    throw { code: 'api.revision-conflict' };
  });
  const manager = managerFor(fake);
  const transaction = manager.startTransaction({ expectedRevision: 1 });
  const error = await expectRejectCode(() => transaction.commit(), 'api.revision-conflict');
  assert.equal(error.details, undefined);
});

test('a commit answer without a usable revision or created list is refused', async () => {
  const cases: readonly (readonly [unknown, string])[] = [
    [{ created: [] }, 'api.invalid_argument: bridge.answer.revision'],
    [
      { revision: Number.POSITIVE_INFINITY, created: [] },
      'api.invalid_argument: bridge.answer.revision',
    ],
    [{ revision: -1, created: [] }, 'api.invalid_argument: bridge.answer.revision'],
    [{ revision: '1', created: [] }, 'api.invalid_argument: bridge.answer.revision'],
    [{ revision: 1 }, 'api.invalid_argument: bridge.answer.created'],
    [{ revision: 1, created: [''] }, 'api.invalid_argument: bridge.answer.created'],
    [{ revision: 1, created: [7] }, 'api.invalid_argument: bridge.answer.created'],
  ];
  for (const [answer, code] of cases) {
    const manager = managerFor(fakeBridge(() => answer));
    const transaction = manager.startTransaction({ expectedRevision: 1 });
    await expectRejectCode(() => transaction.commit(), code);
    assert.equal(transaction.closed, true);
    assert.equal(manager.depth, 0);
  }
});

test('getObject decodes by the host typeName tag and refuses an unknown one', async () => {
  const fake = fakeBridge(lineAnswer);
  const manager = managerFor(fake);
  const transaction = manager.startTransaction({ expectedRevision: 1 });
  const object = await transaction.getObject(objectId('2A'), 'read');
  assert.equal(object.typeName, 'Line');
  assert.equal(object.objectId, '2A');
  const parameters = parametersOf(fake.calls[0]);
  assert.equal(parameters['transactionId'], 'tx-1');
  assert.equal(parameters['objectId'], '2A');
  assert.equal(parameters['mode'], 'read');

  const answers: readonly unknown[] = [
    { object: { objectId: '1F' } },
    { object: { typeName: 'Nope', objectId: '1F' } },
    { object: {} },
  ];
  for (const answer of answers) {
    const unknown = managerFor(fakeBridge(() => answer));
    const next = unknown.startTransaction({ expectedRevision: 1 });
    await expectRejectCode(
      () => next.getObject(objectId('1F'), 'write'),
      'api.invalid_argument: bridge.answer.typeName',
    );
  }
});

test('getObject refuses a missing object and a malformed mode', async () => {
  const manager = managerFor(fakeBridge(() => ({ object: null })));
  const transaction = manager.startTransaction({ expectedRevision: 1 });
  await expectRejectCode(
    () => transaction.getObject(objectId('2A'), 'read'),
    'api.invalid_argument: bridge.answer.object',
  );
  await expectRejectCode(
    () => transaction.getObject(objectId('2A'), 'bogus' as never),
    'api.invalid_argument: mode',
  );
});

test('no public member can push an edit', () => {
  const manager = managerFor(fakeBridge());
  const transaction = manager.startTransaction({ expectedRevision: 1 });
  assert.equal('adoptEdits' in transaction, false, 'the savepoint merge is manager-owned');
  assert.equal('edits' in transaction, false, 'the buffer is not an instance field');
});
