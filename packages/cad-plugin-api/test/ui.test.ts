// The `Ui` namespace: declarative contributions, validation and host hooks.
//
// Registration is client-local, so these tests never touch the wire: the fake
// bridge answers `hostInfo()` and any `request`/`mutate` call would be a bug.
// What is asserted is the contract the host adapter depends on — the exact
// refusals, the frozen normalized shape, and the register/unregister hooks.

import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  API_OPERATIONS,
  CadApiError,
  createCadApi,
} from '../src/index.ts';
import type {
  CadApiHostInfo,
  CadApiOptions,
  CadApiRequest,
  CadHostBridge,
  UiContribution,
  UiRegistration,
} from '../src/index.ts';

const HOST: CadApiHostInfo = {
  apiVersion: 1,
  carrier: 'native-below',
  documentOwner: 'native',
  operations: API_OPERATIONS,
};

function silentBridge(): CadHostBridge {
  return {
    async hostInfo(): Promise<CadApiHostInfo> {
      return HOST;
    },
    async request<Result>(_request: CadApiRequest): Promise<Result> {
      throw new Error('the ui namespace must not read from the host');
    },
    async mutate<Result>(_request: CadApiRequest): Promise<Result> {
      throw new Error('the ui namespace must not mutate the host');
    },
  };
}

function renderError(error: unknown): string {
  if (error instanceof CadApiError) return `${error.name}(${error.code}): ${error.message}`;
  if (error instanceof Error) return `${error.name}: ${error.message}`;
  return String(error);
}

async function expectRejectsCode(run: () => Promise<unknown>, code: string): Promise<void> {
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
}

function expectThrows(run: () => unknown, pattern: RegExp): void {
  let caught: unknown;
  try {
    run();
  } catch (error) {
    caught = error;
  }
  assert.ok(caught !== null, 'expected a throw');
  assert.match(String(caught), pattern);
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

function contribution(overrides: Partial<UiContribution> = {}): UiContribution {
  return {
    id: 'acme.roof-tools',
    toolbar: [{
      id: 'acme.roof',
      title: 'Roof',
      items: [
        { id: 'ROOF', label: 'Roof', icon: 'line', command: 'LINE' },
        { id: 'DORMER', label: 'Dormer', pendingReason: 'kernel has no dormer op' },
      ],
    }],
    ...overrides,
  };
}

test('without a host hook, registration refuses instead of promising chrome', async () => {
  const api = await createCadApi(silentBridge());
  expectSyncCode(
    () => api.ui.register(contribution()),
    'unsupported-runtime: cad_api.Ui.register',
  );
});

test('the hook receives the normalized, frozen contribution', async () => {
  const seen: UiRegistration[] = [];
  const options: CadApiOptions = {
    onUiRegister: (registration) => {
      seen.push(registration);
    },
  };
  const api = await createCadApi(silentBridge(), options);
  const registered = api.ui.register(contribution());

  assert.equal(registered.id, 'acme.roof-tools');
  assert.equal(registered.domain, '*', 'an omitted domain targets every profile');
  assert.equal(Object.isFrozen(registered), true);
  assert.equal(Object.isFrozen(registered.toolbar), true);
  assert.equal(Object.isFrozen(registered.toolbar?.[0]), true);
  assert.equal(Object.isFrozen(registered.toolbar?.[0]?.items), true);
  assert.equal(Object.isFrozen(registered.toolbar?.[0]?.items[0]), true);
  assert.equal(seen.length, 1);
  assert.equal(seen[0], registered, 'the hook sees the same object register returned');

  const tool = registered.toolbar?.[0]?.items[0];
  assert.equal(tool?.label, 'Roof');
  assert.equal(tool?.command, 'LINE');
  assert.equal(Object.prototype.hasOwnProperty.call(tool, 'pendingReason'), false);
});

test('a duplicate contribution id is refused and the hook is not called twice', async () => {
  const seen: UiRegistration[] = [];
  const api = await createCadApi(silentBridge(), {
    onUiRegister: (registration) => {
      seen.push(registration);
    },
  });
  api.ui.register(contribution());
  expectSyncCode(
    () => api.ui.register(contribution({ domain: 'concept' })),
    'api.invalid_argument: ui.duplicate',
  );
  assert.equal(seen.length, 1);
});

test('a hook throw leaves nothing registered', async () => {
  let attempts = 0;
  const api = await createCadApi(silentBridge(), {
    onUiRegister: () => {
      attempts += 1;
      if (attempts === 1) throw new Error('host refused');
    },
  });
  expectThrows(() => api.ui.register(contribution()), /host refused/);
  api.ui.register(contribution());
  assert.equal(attempts, 2, 'the failed attempt did not pin the id');
});

test('unregister calls the inverse hook and an unknown id is refused', async () => {
  const removed: string[] = [];
  const api = await createCadApi(silentBridge(), {
    onUiRegister: () => undefined,
    onUiUnregister: (id) => {
      removed.push(id);
    },
  });
  api.ui.register(contribution());
  api.ui.unregister('acme.roof-tools');
  assert.deepEqual(removed, ['acme.roof-tools']);
  expectSyncCode(() => api.ui.unregister('acme.roof-tools'), 'api.invalid_argument: ui.unknown');
  expectSyncCode(() => api.ui.unregister('bad id'), 'api.invalid_argument: ui.id');
});

test('a malformed contribution is refused by field and never reaches the hook', async () => {
  let calls = 0;
  const api = await createCadApi(silentBridge(), {
    onUiRegister: () => {
      calls += 1;
    },
  });

  const cases: Array<[() => unknown, string]> = [
    [() => api.ui.register({ id: 'x' } as unknown as UiContribution), 'api.invalid_argument: ui.slots'],
    [() => api.ui.register({ id: 'bad id', toolbar: contribution().toolbar } as UiContribution), 'api.invalid_argument: id'],
    [() => api.ui.register({ id: 'x', domain: 'Concept', toolbar: contribution().toolbar } as UiContribution), 'api.invalid_argument: domain'],
    [() => api.ui.register({
      id: 'x',
      toolbar: [{ id: 'g', title: 'G', items: [] }],
    }), 'api.invalid_argument: toolbar[0].items'],
    [() => api.ui.register({
      id: 'x',
      toolbar: [{ id: 'g', title: 'G', items: [{ id: 'i', label: 'I' }] }],
    } as unknown as UiContribution), 'api.invalid_argument: toolbar[0].items[0].dispatch'],
    [() => api.ui.register({
      id: 'x',
      toolbar: [{ id: 'g', title: 'G', items: [{ id: 'i', label: 'I', command: 'LINE', pendingReason: 'no' }] }],
    } as unknown as UiContribution), 'api.invalid_argument: toolbar[0].items[0].dispatch'],
    [() => api.ui.register({
      id: 'x',
      toolbar: [{ id: 'g', title: 'G', items: [{ id: 'i', label: 'I', command: ' LINE' }] }],
    }), 'api.invalid_argument: toolbar[0].items[0].command'],
    [() => api.ui.register({
      id: 'x',
      toolbar: [{ id: 'g', title: 'G', items: [
        { id: 'i', label: 'I', command: 'LINE' },
        { id: 'i', label: 'J', command: 'ARC' },
      ] }],
    }), 'api.invalid_argument: toolbar[0].items[1].id.duplicate'],
    [() => api.ui.register({
      id: 'x',
      tray: [{ id: 'p', title: 'P' }],
    } as unknown as UiContribution), 'api.invalid_argument: tray[0].pendingReason'],
  ];

  for (const [run, code] of cases) expectSyncCode(run, code);
  assert.equal(calls, 0);
});

test('menu, toolbar and tray all normalize; command text may contain spaces', async () => {
  const api = await createCadApi(silentBridge(), { onUiRegister: () => undefined });
  const registered = api.ui.register({
    id: 'acme.all',
    domain: 'concept',
    menu: [{ id: 'roof', title: 'Roof', items: [{ id: 'ZOOMW', label: 'Window', command: 'ZOOM WINDOW' }] }],
    toolbar: [{ id: 'roof-tools', title: 'Roof Tools', items: [{ id: 'LINE', label: 'Line', command: 'LINE' }] }],
    tray: [{ id: 'roof-panel', title: 'Roof', pendingReason: 'no roof panel yet' }],
  });
  assert.equal(registered.domain, 'concept');
  assert.equal(registered.menu?.[0]?.items[0]?.command, 'ZOOM WINDOW');
  assert.equal(registered.tray?.[0]?.pendingReason, 'no roof panel yet');
});

test('a non-function hook is refused at assembly time', async () => {
  await expectRejectsCode(
    () => createCadApi(silentBridge(), { onUiRegister: 1 as unknown as () => void }),
    'api.invalid_argument: options.onUiRegister',
  );
});
