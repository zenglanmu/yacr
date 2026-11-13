// The editor prompts: the mutating lane, the parameter projection and the local
// message sink.
//
// A prompt completes only when the user answers it, so every prompt (and the
// immediate `selectAll`) travels on the submit lane: the fake bridge records the
// mutating lane separately, and any read-lane traffic would be a bug. `writeMessage`
// is the opposite case — client-local, no wire at all.

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { API_OPERATIONS, CadApiError, createCadApi } from '../src/index.ts';
import type {
  CadApiHostInfo,
  CadApiRequest,
  CadHostBridge,
  PromptResult,
} from '../src/index.ts';
import { createEditor } from '../src/editor-input.ts';
import { createWire } from '../src/internal/capability.ts';

const HOST: CadApiHostInfo = {
  apiVersion: 1,
  carrier: 'native-below',
  documentOwner: 'native',
  operations: API_OPERATIONS.map((row) => ({
    namespace: row.namespace,
    operation: row.operation,
    status: 'supported' as const,
  })),
};

interface Fake {
  readonly bridge: CadHostBridge;
  readonly calls: CadApiRequest[];
  readonly mutations: CadApiRequest[];
}

function fake(answer: (request: CadApiRequest) => unknown = () => ({ status: 'Cancel' })): Fake {
  const calls: CadApiRequest[] = [];
  const mutations: CadApiRequest[] = [];
  const bridge: CadHostBridge = {
    async hostInfo(): Promise<CadApiHostInfo> {
      return HOST;
    },
    async request<Result>(request: CadApiRequest): Promise<Result> {
      calls.push(request);
      return answer(request) as Result;
    },
    async mutate<Result>(request: CadApiRequest): Promise<Result> {
      mutations.push(request);
      return answer(request) as Result;
    },
  };
  return { bridge, calls, mutations };
}

function renderError(error: unknown): string {
  if (error instanceof CadApiError) return `${error.name}(${error.code}): ${error.message}`;
  if (error instanceof Error) return `${error.name}: ${error.message}`;
  return String(error);
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

test('every prompt uses the mutating lane and decodes its Ok answer', async () => {
  const cases: readonly (readonly [
    string,
    (host: Fake) => Promise<unknown>,
    unknown,
    (result: unknown) => void,
  ])[] = [
    [
      'editor.getPoint',
      (host) => createEditor(createWire(HOST, host.bridge)).getPoint(),
      { status: 'Ok', point: { x: 1, y: 2, z: 3 } },
      (result) => assert.deepEqual(result, { status: 'Ok', value: { x: 1, y: 2, z: 3 } }),
    ],
    [
      'editor.getEntity',
      (host) => createEditor(createWire(HOST, host.bridge)).getEntity(),
      { status: 'Ok', objectId: '12' },
      (result) => assert.deepEqual(result, { status: 'Ok', value: '12' }),
    ],
    [
      'editor.getSelection',
      (host) => createEditor(createWire(HOST, host.bridge)).getSelection(),
      { status: 'Ok', objectIds: ['12', '13'] },
      (result) => {
        const selection = result as PromptResult<{ count: number; getObjectIds(): readonly string[] }>;
        if (selection.status !== 'Ok') throw new Error('expected Ok');
        assert.equal(selection.value.count, 2);
        assert.deepEqual(selection.value.getObjectIds(), ['12', '13']);
      },
    ],
    [
      'editor.getString',
      (host) => createEditor(createWire(HOST, host.bridge)).getString(),
      { status: 'Ok', value: 'hello' },
      (result) => assert.deepEqual(result, { status: 'Ok', value: 'hello' }),
    ],
    [
      'editor.getDouble',
      (host) => createEditor(createWire(HOST, host.bridge)).getDouble(),
      { status: 'Ok', value: 2.5 },
      (result) => assert.deepEqual(result, { status: 'Ok', value: 2.5 }),
    ],
    [
      'editor.getInteger',
      (host) => createEditor(createWire(HOST, host.bridge)).getInteger(),
      { status: 'Ok', value: 3 },
      (result) => assert.deepEqual(result, { status: 'Ok', value: 3 }),
    ],
    [
      'editor.getKeyword',
      (host) => createEditor(createWire(HOST, host.bridge)).getKeyword({ keywords: ['YES'] }),
      { status: 'Keyword', keyword: 'YES' },
      (result) => assert.deepEqual(result, { status: 'Keyword', keyword: 'YES' }),
    ],
  ];

  for (const [operation, run, answer, check] of cases) {
    const host = fake(() => answer);
    check(await run(host));
    assert.equal(host.calls.length, 0, `${operation} must not use the read lane`);
    assert.equal(host.mutations.length, 1, `${operation} must submit once`);
    assert.equal(host.mutations[0].namespace, 'EditorInput');
    assert.equal(host.mutations[0].operation, operation);
  }
});

test('Cancel, None and an error status decode without inventing a value', async () => {
  const cases: readonly (readonly [unknown, PromptResult<unknown>])[] = [
    [{ status: 'Cancel' }, { status: 'Cancel' }],
    [{ status: 'None' }, { status: 'None' }],
    [{ status: 'Error', message: 'bad input' }, { status: 'Error', message: 'bad input' }],
  ];
  for (const [answer, expected] of cases) {
    const host = fake(() => answer);
    const result = await createEditor(createWire(HOST, host.bridge)).getString();
    assert.deepEqual(result, expected);
  }
});

test('the prompt body carries only the declared, present fields', async () => {
  const host = fake(() => ({ status: 'Cancel' }));
  const wire = createWire(HOST, host.bridge);
  const editor = createEditor(wire);

  await editor.getPoint();
  await editor.getString({ message: 'name?', defaultValue: 'x', allowSpaces: true });
  await editor.getEntity({ message: 'pick', allowNone: true, rejectMessage: 'no' });
  await editor.getKeyword({ message: 'yn', keywords: ['YES', 'NO'], defaultKeyword: 'YES' });
  await editor.getDouble({ message: 'v', defaultValue: 1.5, minimum: 0, maximum: 9 });

  assert.deepEqual(
    host.mutations.map((request) => request.parameters),
    [
      {},
      { message: 'name?', defaultValue: 'x' },
      { message: 'pick', rejectMessage: 'no' },
      { message: 'yn', keywords: ['YES', 'NO'] },
      { message: 'v', defaultValue: 1.5 },
    ],
  );
});

test('a malformed prompt answer is refused by field', async () => {
  const cases: readonly (readonly [unknown, string, (editor: ReturnType<typeof createEditor>) => Promise<unknown>])[] = [
    [{}, 'api.invalid_argument: bridge.answer.status', (editor) => editor.getString()],
    [
      { status: 'Maybe' },
      'api.invalid_argument: bridge.answer.status',
      (editor) => editor.getString(),
    ],
    [{ status: 'Ok' }, 'api.invalid_argument: bridge.answer.point', (editor) => editor.getPoint()],
    [
      { status: 'Ok', point: [1, 2] },
      'api.invalid_argument: bridge.answer.point',
      (editor) => editor.getPoint(),
    ],
    [
      { status: 'Ok', value: 'two' },
      'api.invalid_argument: bridge.answer.value',
      (editor) => editor.getDouble(),
    ],
    [
      { status: 'Ok', value: 1.5 },
      'api.invalid_argument: bridge.answer.value',
      (editor) => editor.getInteger(),
    ],
  ];
  for (const [answer, code, run] of cases) {
    const host = fake(() => answer);
    const editor = createEditor(createWire(HOST, host.bridge));
    await expectRejectCode(() => run(editor), code);
  }
});

test('selectAll submits an empty body and needs an Ok status', async () => {
  const ok = fake(() => ({ status: 'Ok', objectIds: ['12', '13'] }));
  const selection = await createEditor(createWire(HOST, ok.bridge)).selectAll();
  assert.deepEqual(selection.getObjectIds(), ['12', '13']);
  assert.deepEqual(ok.mutations, [
    { namespace: 'EditorInput', operation: 'editor.selectAll', parameters: {} },
  ]);
  assert.equal(ok.calls.length, 0);

  const notOk = fake(() => ({ status: 'Cancel' }));
  await expectRejectCode(
    () => createEditor(createWire(HOST, notOk.bridge)).selectAll(),
    'api.invalid_argument: bridge.answer.status',
  );
});

test('writeMessage is client-local: sink or refusal, never a wire call', async () => {
  const host = fake();
  const unsinked = await createCadApi(host.bridge);
  await expectRejectCode(
    () => unsinked.editor.writeMessage('hello'),
    'unsupported-runtime: cad_api.EditorInput.editor.writeMessage',
  );

  const messages: string[] = [];
  const sunk = await createCadApi(host.bridge, {
    onMessage: (text) => {
      messages.push(text);
    },
  });
  await sunk.editor.writeMessage('hello');
  assert.deepEqual(messages, ['hello']);
  assert.equal(host.calls.length, 0);
  assert.equal(host.mutations.length, 0, 'writeMessage never touches the wire');
});
