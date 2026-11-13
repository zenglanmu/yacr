// Contract tests for the plugin command registry and the App funnel wiring.
//
// The registry is the one table the typed-command funnel consults, so what is
// asserted here is exact matching (no prefix/suffix matches), case-insensitivity,
// the first-wins re-registration window, and the failure isolation that keeps a
// plugin bug from breaking every other command.
//
// These tests are `.ts` by filename only: they run under Node's type stripping.

import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';

import {
  currentPluginNoticeForTests,
  resetPluginHostForTests,
} from '../../src/plugins/host.ts';
import {
  pluginCommandFor,
  registerPluginCommand,
  resetPluginCommands,
  runPluginCommand,
  unregisterPluginCommand,
} from '../../src/plugin-api/commands.ts';

function fresh(): void {
  resetPluginCommands();
  resetPluginHostForTests();
}

function command(name, run, flags = 'modal') {
  return { name, flags, run };
}

test('a registered command matches exactly and case-insensitively', () => {
  fresh();
  let ran = 0;
  registerPluginCommand(command('HELLO_WORLD', async () => { ran += 1; }));

  for (const line of ['HELLO_WORLD', 'hello_world', '  HELLO_WORLD', 'HeLLo_WorLd']) {
    const match = pluginCommandFor(line);
    assert.ok(match !== null, `${JSON.stringify(line)} must match`);
    assert.equal(match.name, 'HELLO_WORLD');
  }
  assert.equal(ran, 0, 'matching never runs the command');
});

test('nothing else matches: a host name, arguments, blanks and prefixes', () => {
  fresh();
  registerPluginCommand(command('HELLO_WORLD', async () => undefined));

  // `LINE` is host vocabulary; only a plugin registration could put it here.
  for (const line of ['LINE', 'HELLO', 'HELLO_WORLDX', 'HELLO_WORLD 1', '', '   ']) {
    assert.equal(pluginCommandFor(line), null, `${JSON.stringify(line)} must not match`);
  }
});

test('a re-registration inside the window keeps the first command', () => {
  fresh();
  const first = async () => undefined;
  const second = async () => undefined;
  registerPluginCommand(command('HELLO', first));
  registerPluginCommand(command('HELLO', second));

  const match = pluginCommandFor('HELLO');
  assert.ok(match !== null);
  assert.equal(match.run, first, 'a double load must not flip what the command does');
});

test('unregister and the reset seam both empty the table', () => {
  fresh();
  registerPluginCommand(command('HELLO', async () => undefined));
  unregisterPluginCommand('HELLO');
  assert.equal(pluginCommandFor('HELLO'), null);

  registerPluginCommand(command('HELLO', async () => undefined));
  resetPluginCommands();
  assert.equal(pluginCommandFor('HELLO'), null);
});

test('a failing handler is reported through the shared notice and never rethrown', async () => {
  fresh();
  let calls = 0;
  const failing = async () => {
    calls += 1;
    throw new Error('plugin bug');
  };

  await runPluginCommand(failing, 'HELLO');
  assert.equal(calls, 1, 'the handler ran');
  assert.deepEqual(currentPluginNoticeForTests(), {
    key: 'plugin.command.failed',
    params: { command: 'HELLO' },
  });

  // A successful run reports nothing new.
  resetPluginHostForTests();
  await runPluginCommand(async () => undefined, 'HELLO');
  assert.equal(currentPluginNoticeForTests(), null);
});

test('App.tsx consults the plugin funnel before the host command queue', async () => {
  const source = await readFile(new URL('../../src/App.tsx', import.meta.url), 'utf8');

  // The 1000-line gate is a repository rule; this file is the closest to it.
  // Count like `check-file-size.py` does (splitlines: a trailing newline is not a
  // line), not like `split('\n')`.
  const lines = source.split('\n');
  if (lines.at(-1) === '') lines.pop();
  assert.ok(lines.length <= 999, `App.tsx must stay under the gate, saw ${lines.length} lines`);

  const pluginCall = source.indexOf('pluginCommandFor(text)');
  const queueCall = source.indexOf('queueCadCommand(invoke');
  assert.ok(pluginCall > 0, 'App.tsx must consult pluginCommandFor');
  assert.ok(queueCall > 0, 'App.tsx must still queue host commands');
  assert.ok(
    pluginCall < queueCall,
    'the plugin funnel must run before the host command is queued',
  );
  assert.ok(
    source.includes('runPluginCommand(pluginRun.run, pluginRun.name)'),
    'the funnel must run the matched command through runPluginCommand',
  );
});
