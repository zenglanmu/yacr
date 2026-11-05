// Contract tests for the plugin intercept point in the command funnel
// (ADR 0030 D5): a veto happens after admission/chrome-local resolution and
// before any `cad_command` IPC, and it never becomes a silent success.
import test from 'node:test';
import assert from 'node:assert/strict';

import { queueCadCommand } from '../src/cad/cad-command.ts';
import { commandMessageFor } from '../src/commands/command-message.ts';
import { setCommandCatalog } from '../src/commands/commands.ts';
import { createTranslator } from '../src/i18n.ts';
import { HOST_NAMES } from './helpers.ts';

setCommandCatalog(HOST_NAMES);

const loaded = {
  documentLabel: 'fixture.dwg', entityCount: 3, loading: false,
  canOpen: true, canNew: true, canSave: true, canPlot: true, canFit: true,
  selection: { count: 1, empty: false },
};

function recorder() {
  const calls = [];
  const invoke = async (command, args) => {
    calls.push({ command, args });
    return { accepted: true };
  };
  return { calls, invoke };
}

test('a handled interceptor returns blocked before Rust is invoked', async () => {
  const { calls, invoke } = recorder();
  let asked = null;
  const result = await queueCadCommand(invoke, 'MOVE', loaded, false, command => {
    asked = command;
    return { messageKey: 'vendor.editing.unsupported' };
  });

  assert.deepEqual(result, {
    kind: 'blocked',
    command: 'MOVE',
    messageKey: 'vendor.editing.unsupported',
  });
  assert.equal(asked, 'MOVE', 'the canonical admitted command is offered');
  assert.equal(calls.length, 0, 'a veto must not reach cad_command');
});

test('a null decision still invokes the router command', async () => {
  const { calls, invoke } = recorder();
  const result = await queueCadCommand(invoke, 'LINE', loaded, false, () => null);

  assert.deepEqual(result, { kind: 'accepted', command: 'LINE' });
  assert.deepEqual(calls, [{ command: 'cad_command', args: { command: 'LINE' } }]);
});

test('a throwing interceptor is "not handled" and the command still runs', async () => {
  const { calls, invoke } = recorder();
  const result = await queueCadCommand(invoke, 'LINE', loaded, false, () => {
    throw new Error('plugin exploded');
  });

  assert.deepEqual(result, { kind: 'accepted', command: 'LINE' });
  assert.equal(calls.length, 1);
});

test('a chrome-local PROPERTIES selector is never offered to an interceptor', async () => {
  const { calls, invoke } = recorder();
  let asked = 0;
  const result = await queueCadCommand(invoke, 'PROPERTIES', loaded, false, () => {
    asked += 1;
    return { messageKey: 'vendor.editing.unsupported' };
  });

  assert.deepEqual(result, { kind: 'local', command: 'PROPERTIES' });
  assert.equal(asked, 0, 'there is no router command to veto');
  assert.equal(calls.length, 0);
});

test('commandMessageFor translates a blocked result with the command name', () => {
  const t = createTranslator({
    en: { 'vendor.editing.unsupported': 'Blocked {command} by the plugin.' },
  }, 'en');

  assert.deepEqual(
    commandMessageFor({ kind: 'blocked', command: 'MOVE', messageKey: 'vendor.editing.unsupported' }, t),
    { kind: 'error', text: 'Blocked MOVE by the plugin.' },
  );
});
