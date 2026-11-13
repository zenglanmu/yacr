// Contract tests for the plugin loader.
//
// Loading is the host's half of the lifecycle: import, validate, assemble the
// client with the command hooks, activate, and — on any failure — leave nothing
// behind. These tests pin exactly that, with an already-loaded module namespace
// (`source.module`) so nothing touches the network, plus one importer-path test.
//
// These tests are `.ts` by filename only: they run under Node's type stripping.

import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';

import { loadPluginModule } from '../../src/plugin-api/loader.ts';
import { pluginCommandFor, resetPluginCommands } from '../../src/plugin-api/commands.ts';

const CONTRACT_URL = new URL('../../../../crates/cad-ui-core/api/plugin-api.json', import.meta.url);
const payload = JSON.parse(await readFile(CONTRACT_URL, 'utf8'));

/** The chrome's `cad_status` stand-in; only `hostProfile` is read by the adapter. */
function status() {
  return { hostProfile: { surfaceMode: 'native-below', documentOwner: 'native' } };
}

/** A fake host that serves `cad_api_status` and nothing else. */
function deps(extra = {}) {
  const calls = [];
  const invoke = async (command) => {
    calls.push(command);
    if (command === 'cad_api_status') return payload;
    throw new Error(`unsupported-runtime: ${command}`);
  };
  return { invoke, status, calls, ...extra };
}

function fresh() {
  resetPluginCommands();
}

function codeOf(error) {
  return error instanceof Error ? error.code : undefined;
}

async function expectReject(promise, code) {
  let caught = null;
  try {
    await promise;
  } catch (error) {
    caught = error;
  }
  assert.ok(caught !== null, `expected a rejection with ${code}`);
  assert.equal(codeOf(caught), code);
  return caught;
}

test('a loaded plugin activates, registers a command and deactivates idempotently', async () => {
  fresh();
  let ran = 0;
  let deactivations = 0;
  const module = {
    default: {
      id: 'sample-hello',
      apiVersion: 1,
      activate: (ctx) => {
        assert.equal(ctx.pluginId, 'sample-hello');
        ctx.api.runtime.registerCommand('HELLO', 'modal', () => {
          ran += 1;
        });
      },
      deactivate: () => {
        deactivations += 1;
      },
    },
  };

  const active = await loadPluginModule({ module }, deps());
  assert.equal(active.plugin.id, 'sample-hello');
  assert.equal(active.context.pluginId, 'sample-hello');

  const match = pluginCommandFor('HELLO');
  assert.ok(match !== null, 'activate must have registered the command');
  await match.run();
  assert.equal(ran, 1);

  await active.deactivate();
  assert.equal(deactivations, 1);
  assert.equal(pluginCommandFor('HELLO'), null, 'deactivate removes its commands');
  await active.deactivate();
  assert.equal(deactivations, 1, 'deactivate is idempotent');
});

test('a url source is resolved through the injected importer', async () => {
  fresh();
  const seen = [];
  const module = { default: { id: 'sample-hello', apiVersion: 1 } };
  const active = await loadPluginModule(
    { url: 'plugin://sample/index.js' },
    deps({
      importer: async (url) => {
        seen.push(url);
        return module;
      },
    }),
  );
  assert.deepEqual(seen, ['plugin://sample/index.js']);
  assert.equal(active.plugin.id, 'sample-hello');
  await active.deactivate();
});

test('a failed import is api.plugin.load_failed: import and registers nothing', async () => {
  fresh();
  await expectReject(
    loadPluginModule(
      { url: 'plugin://gone/index.js' },
      deps({
        importer: async () => {
          throw new Error('404');
        },
      }),
    ),
    'api.plugin.load_failed: import',
  );
  assert.equal(pluginCommandFor('HELLO'), null);
});

test('an unusable default export is api.plugin.load_failed: module', async () => {
  fresh();
  for (const module of [{}, { default: 7 }, { default: { id: 'Bad ID', apiVersion: 1 } }]) {
    await expectReject(
      loadPluginModule({ module }, deps()),
      'api.plugin.load_failed: module',
    );
    assert.equal(pluginCommandFor('HELLO'), null);
  }
});

test('another contract generation is api.host-outdated before anything is wired', async () => {
  fresh();
  let activated = 0;
  await expectReject(
    loadPluginModule(
      {
        module: {
          default: {
            id: 'sample-hello',
            apiVersion: 2,
            activate: () => {
              activated += 1;
            },
          },
        },
      },
      deps(),
    ),
    'api.host-outdated: 2',
  );
  assert.equal(activated, 0, 'a plugin for another generation never activates');
});

test('a rejected activate leaves no command registered', async () => {
  fresh();
  const module = {
    default: {
      id: 'sample-hello',
      apiVersion: 1,
      activate: (ctx) => {
        ctx.api.runtime.registerCommand('HELLO', 'modal', () => undefined);
        throw new Error('boom');
      },
    },
  };

  await expectReject(loadPluginModule({ module }, deps()), 'api.plugin.load_failed: activate');
  assert.equal(pluginCommandFor('HELLO'), null, 'a half-active plugin must not survive');
});
