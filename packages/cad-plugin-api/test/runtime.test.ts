// The `Runtime` namespace: command registration, validation and host hooks.
//
// Registration is client-local, so these tests never touch the wire: a fake bridge
// answers `hostInfo()` and any `request`/`mutate` call would be a bug. What is
// asserted is the contract the loader depends on — the exact refusals, the closed
// flag set, and the two hooks a host installs to own the funnel.

import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  API_OPERATIONS,
  CadApiError,
  CommandFlags,
  createCadApi,
} from '../src/index.ts';
import type {
  CadApiHostInfo,
  CadApiOptions,
  CadApiRequest,
  CadHostBridge,
  CommandRegistration,
  PluginContext,
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
      throw new Error('the runtime namespace must not read from the host');
    },
    async mutate<Result>(_request: CadApiRequest): Promise<Result> {
      throw new Error('the runtime namespace must not mutate the host');
    },
  };
}

const handler = (_ctx: PluginContext): void => undefined;

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

test('without a host hook, registration refuses instead of promising a dispatch', async () => {
  const api = await createCadApi(silentBridge());
  const error = expectSyncCode(
    () => api.runtime.registerCommand('HELLO', CommandFlags.Modal, handler),
    'unsupported-runtime: cad_api.Runtime.registerCommand',
  );
  assert.ok(error.message.length > 0);
});

test('a host hook receives the registration exactly as declared', async () => {
  const seen: CommandRegistration[] = [];
  const options: CadApiOptions = {
    onCommandRegister: (registration) => {
      seen.push(registration);
    },
  };
  const api = await createCadApi(silentBridge(), options);
  const registered = api.runtime.registerCommand('HELLO_WORLD', CommandFlags.Modal, handler);

  assert.equal(registered.name, 'HELLO_WORLD');
  assert.equal(registered.flags, 'modal');
  assert.equal(registered.handler, handler);
  assert.equal(Object.isFrozen(registered), true);
  assert.equal(seen.length, 1);
  assert.equal(seen[0], registered, 'the hook sees the same object registerCommand returned');
});

test('a duplicate name is refused and the hook is not called twice', async () => {
  const seen: CommandRegistration[] = [];
  const api = await createCadApi(silentBridge(), {
    onCommandRegister: (registration) => {
      seen.push(registration);
    },
  });
  api.runtime.registerCommand('HELLO', CommandFlags.Modal, handler);
  expectSyncCode(
    () => api.runtime.registerCommand('HELLO', CommandFlags.Transparent, handler),
    'api.invalid_argument: command.duplicate',
  );
  assert.equal(seen.length, 1);
});

test('the command grammar and the flag set are closed', async () => {
  const api = await createCadApi(silentBridge(), { onCommandRegister: () => undefined });
  for (const name of ['line', '1ABC', '', 'A-B', `A${'B'.repeat(32)}`]) {
    expectSyncCode(
      () => api.runtime.registerCommand(name, CommandFlags.Modal, handler),
      'api.invalid_argument: command',
    );
  }
  expectSyncCode(
    () =>
      api.runtime.registerCommand(
        'HELLO',
        'bogus' as unknown as CommandFlags,
        handler,
      ),
    'api.invalid_argument: flags',
  );
  expectSyncCode(
    () =>
      api.runtime.registerCommand('HELLO', CommandFlags.Modal, 'nope' as unknown as typeof handler),
    'api.invalid_argument: handler',
  );
});

test('unregister removes the registration, reports it and frees the name', async () => {
  const removed: string[] = [];
  const api = await createCadApi(silentBridge(), {
    onCommandRegister: () => undefined,
    onCommandUnregister: (name) => {
      removed.push(name);
    },
  });
  api.runtime.registerCommand('HELLO', CommandFlags.Modal, handler);
  api.runtime.unregisterCommand('HELLO');
  assert.deepEqual(removed, ['HELLO']);

  // The name is free again: re-registering works, and after the second removal the
  // unknown-name refusal is what a stale unregister gets.
  api.runtime.registerCommand('HELLO', CommandFlags.Modal, handler);
  api.runtime.unregisterCommand('HELLO');
  expectSyncCode(() => api.runtime.unregisterCommand('HELLO'), 'api.invalid_argument: command.unknown');
  expectSyncCode(() => api.runtime.unregisterCommand('lower'), 'api.invalid_argument: command');
});

test('hooks must be functions when supplied', async () => {
  await expectRejectsCode(
    () => createCadApi(silentBridge(), { onCommandRegister: 'nope' as never }),
    'api.invalid_argument: options.onCommandRegister',
  );
  await expectRejectsCode(
    () => createCadApi(silentBridge(), { onCommandUnregister: 7 as never }),
    'api.invalid_argument: options.onCommandUnregister',
  );
});
