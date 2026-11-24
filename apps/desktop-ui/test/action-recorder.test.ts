// The ADR-0056 Action Recorder panel contract: wire normalisation, the typed
// command lines, the store's armed/empty/error states, and the real Manage panel
// wiring (no file picker, no fake rows).
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { readCatalog, readModule } from './helpers.ts';
import { RIBBON_TABS } from '../src/ribbon/ribbon-model.ts';
import { SHELL_ACTIONS } from '../src/ribbon/actions.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { REFUSAL_COMMAND_CATALOG } from '../src/commands/catalog-refusals.ts';
import { ARGUMENT_COMMANDS } from '../src/commands/gating.ts';
import {
  ACTION_COMMAND,
  ACTION_RECORDER_COMMAND,
  RECORD_CONTINUE_COMMAND,
  isActionRecorderControl,
  isJsonDestination,
  parseActionRecorderAnswer,
  recordDeleteCommand,
  recordPlayCommand,
  recordStartCommand,
  recordStopCommand,
} from '../src/recorder/recorder.ts';
import { createRecorderStore } from '../src/recorder/recorder-store.ts';

const noWait = async () => {};
type Call = [string, Record<string, unknown> | undefined];

function fakeInvoke(handler: (command: string, args?: Record<string, unknown>) => unknown) {
  const calls: Call[] = [];
  return {
    calls,
    invoke: async (command: string, args?: Record<string, unknown>) => {
      calls.push([command, args]);
      return handler(command, args);
    },
  };
}

const ANSWER = {
  recording: null,
  macros: [{ name: 'door', path: '/tmp/door.actm.json', steps: 2 }],
  lastMacro: 'door',
  playback: null,
  lastError: null,
};

test('the answer normalises and keeps the host rows verbatim', () => {
  const answer = parseActionRecorderAnswer(ANSWER);
  assert.deepEqual(answer.recording, null);
  assert.deepEqual(answer.macros, [{ name: 'door', path: '/tmp/door.actm.json', steps: 2 }]);
  assert.equal(answer.lastMacro, 'door');
  assert.equal(answer.playback, null);
  assert.equal(answer.lastError, null);
  // A malformed row is dropped rather than shown with a fabricated count.
  const partial = parseActionRecorderAnswer({
    recording: { name: 'live', steps: 3, path: null },
    macros: [
      { name: 'ok', steps: 1, path: null },
      { name: '', steps: 1 },
      { name: 'bad', steps: -1 },
      { name: 'ok', steps: 9, path: null },
    ],
    lastMacro: null,
    playback: { name: 'ok', nextStep: 2, total: 4 },
    lastError: 'actstop.path: x',
  });
  assert.deepEqual(partial.recording, { name: 'live', path: null, steps: 3 });
  assert.deepEqual(partial.macros.map(row => row.name), ['ok']);
  assert.equal(partial.macros[0].steps, 1, 'the first row wins a duplicate name');
  assert.deepEqual(partial.playback, { name: 'ok', nextStep: 2, total: 4 });
  assert.equal(partial.lastError, 'actstop.path: x');
});

test('the panel builds exact typed command lines', () => {
  assert.equal(ACTION_RECORDER_COMMAND, 'cad_action_recorder');
  assert.equal(ACTION_COMMAND, 'cad_command');
  assert.equal(recordStartCommand(''), 'ACTRECORD');
  assert.equal(recordStartCommand('  My Macro.json  '), 'ACTRECORD My Macro.json');
  assert.equal(recordStopCommand(''), 'ACTSTOP');
  assert.equal(recordStopCommand('/tmp/Mixed Case.actm.json'), 'ACTSTOP /tmp/Mixed Case.actm.json');
  assert.equal(recordPlayCommand('door'), 'ACTMANAGER PLAY door');
  assert.equal(recordDeleteCommand('door'), 'ACTMANAGER DELETE door');
  assert.equal(RECORD_CONTINUE_COMMAND, 'ACTMANAGER CONTINUE');
  assert.equal(isJsonDestination('/tmp/a.actm.json'), true);
  assert.equal(isJsonDestination('/tmp/a.txt'), false);
  for (const name of ['ACTRECORD', 'actstop', '-ACTSTOP', 'ACTUSERINPUT', 'ACTUSERMESSAGE',
    '-ACTUSERMESSAGE', 'ACTMANAGER']) {
    assert.equal(isActionRecorderControl(`${name} tail`), true, name);
  }
  assert.equal(isActionRecorderControl('COLOR 3'), false);
});

test('the local argument grammar admits the real forms only', () => {
  const spec = (name: string) => ARGUMENT_COMMANDS[name];
  assert.ok(spec('ACTRECORD'));
  assert.ok(spec('ACTSTOP'));
  assert.ok(spec('-ACTSTOP'));
  assert.ok(spec('ACTUSERINPUT'));
  assert.ok(spec('ACTUSERMESSAGE'));
  assert.ok(spec('-ACTUSERMESSAGE'));
  assert.ok(spec('ACTMANAGER'));
  assert.equal(spec('ACTRECORD').parse('a path.json'), true);
  assert.equal(spec('ACTRECORD').parse('   '), false);
  assert.equal(spec('ACTUSERMESSAGE').parse('hello'), true);
  assert.equal(spec('ACTUSERMESSAGE').parse(''), false);
  assert.equal(spec('ACTMANAGER').parse('LIST'), true);
  assert.equal(spec('ACTMANAGER').parse('PLAY door'), true);
  assert.equal(spec('ACTMANAGER').parse('DELETE door'), true);
  assert.equal(spec('ACTMANAGER').parse('CONTINUE'), true);
  assert.equal(spec('ACTMANAGER').parse('WAT'), false);
  assert.equal(spec('ACTMANAGER').parse('PLAY'), false);
});

test('the store reads, distinguishes empty from error and never fabricates rows', async () => {
  const ready = fakeInvoke(() => ANSWER);
  const store = createRecorderStore(ready.invoke, { wait: noWait });
  await store.load();
  assert.equal(ready.calls[0][0], 'cad_action_recorder');
  assert.equal(store.getState().phase, 'ready');
  assert.equal(store.getState().answer.macros[0].name, 'door');

  const empty = createRecorderStore(
    fakeInvoke(() => ({ recording: null, macros: [], lastMacro: null, playback: null })).invoke,
    { wait: noWait },
  );
  await empty.load();
  assert.equal(empty.getState().phase, 'empty');

  const broken = createRecorderStore(
    fakeInvoke(() => { throw 'unsupported-runtime: action-recorder'; }).invoke,
    { wait: noWait },
  );
  await broken.load();
  assert.equal(broken.getState().phase, 'error');
  assert.deepEqual(broken.getState().error, {
    kind: 'read',
    reason: 'unsupported-runtime: action-recorder',
  });
});

test('every button sends one typed line and re-reads the host state', async () => {
  const { invoke, calls } = fakeInvoke(command => command === 'cad_action_recorder'
    ? ANSWER
    : { accepted: true });
  const store = createRecorderStore(invoke, { wait: noWait });
  await store.load();
  assert.deepEqual(await store.start('/tmp/door.json'), { kind: 'ok' });
  assert.deepEqual(await store.stop('/tmp/door.json'), { kind: 'ok' });
  assert.deepEqual(await store.play('door'), { kind: 'ok' });
  assert.deepEqual(await store.remove('door'), { kind: 'ok' });
  assert.deepEqual(await store.resume(), { kind: 'ok' });
  const sent = calls.filter(([command]) => command === 'cad_command').map(([, args]) => args?.command);
  assert.deepEqual(sent, [
    'ACTRECORD /tmp/door.json',
    'ACTSTOP /tmp/door.json',
    'ACTMANAGER PLAY door',
    'ACTMANAGER DELETE door',
    'ACTMANAGER CONTINUE',
  ]);
  // One read after each write: no optimistic state.
  assert.equal(calls.filter(([command]) => command === 'cad_action_recorder').length, 6);
});

test('a refused command keeps the host reason and the last good rows', async () => {
  const { invoke } = fakeInvoke(command => {
    if (command === 'cad_action_recorder') return ANSWER;
    throw 'actstop.path: ACTSTOP needs a destination ending in `.json`';
  });
  const store = createRecorderStore(invoke, { wait: noWait });
  await store.load();
  const ack = await store.stop('');
  assert.deepEqual(ack, {
    kind: 'error',
    reason: 'actstop.path: ACTSTOP needs a destination ending in `.json`',
  });
  assert.equal(store.getState().answer.macros[0].name, 'door');
  assert.equal(store.getState().error?.kind, 'command');
});

test('the Manage ribbon panel is real, chrome-local and has no file picker', async () => {
  const manage = RIBBON_TABS.find(tab => tab.id === 'manage');
  assert.ok(manage);
  const panel = manage.panels.find(entry => entry.id === 'action_recorder');
  assert.ok(panel);
  assert.equal(panel.titleKey, 'ribbon.panel.action_recorder');
  assert.equal(Boolean(panel.pending), false, 'the panel is wired now');
  assert.equal(panel.items.length, 1);
  assert.equal(panel.items[0].action, SHELL_ACTIONS.actionRecorder);
  assert.equal(panel.items[0].action, 'actionRecorder');
  assert.equal(panel.items[0].command, undefined, 'it never sends a router command');
  assert.equal(panel.items[0].labelKey, 'recorder.title');

  const fixture = JSON.parse(await readFile(
    new URL('./fixtures/autocad-2024-drafting.json', import.meta.url), 'utf8'));
  assert.equal(fixture.pendingPanels.includes('action_recorder'), false);

  const panelActions = await readModule('chrome/panel-actions.ts');
  assert.match(panelActions, /import \{ openRecorderPanel \} from '\.\.\/recorder\/panel\.ts';/);
  assert.match(panelActions, /actionRecorder: openRecorderPanel/);
  const workspace = await readModule('components/Workspace.tsx');
  assert.match(workspace, /<ActionRecorderPanel \/>/);
  assert.match(workspace, /<ActionRecorderPanel variant="floating" \/>/);
  const component = await readModule('components/ActionRecorderPanel.tsx');
  assert.doesNotMatch(component, /type="file"/, 'no file picker exists');
  assert.doesNotMatch(component, /showOpenFilePicker|showSaveFilePicker/);
});

test('the seven names are real catalog rows, not refusal rows, and are localized', async () => {
  const names = ['ACTRECORD', 'ACTSTOP', '-ACTSTOP', 'ACTUSERINPUT', 'ACTUSERMESSAGE',
    '-ACTUSERMESSAGE', 'ACTMANAGER'];
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  for (const name of names) {
    assert.equal(COMMAND_CATALOG.filter(row => row.id === name).length, 1, name);
    assert.equal(REFUSAL_COMMAND_CATALOG.some(row => row.id === name), false, name);
    const slug = `draw.kind.${name.startsWith('-') ? `dash${name.slice(1)}` : name}`.toLowerCase();
    assert.equal(typeof en[slug], 'string', slug);
    assert.equal(typeof en[`${slug}.hint`], 'string', slug);
    assert.equal(typeof zh[slug], 'string', slug);
    assert.equal(typeof zh[`${slug}.hint`], 'string', slug);
    assert.doesNotMatch(en[slug], /not implemented/, slug);
  }
  // The panel copy exists in both catalogs; no hardcoded Chinese lives in TSX.
  for (const key of ['recorder.title', 'recorder.start', 'recorder.stop', 'recorder.empty',
    'recorder.play', 'recorder.delete', 'recorder.continue', 'recorder.note']) {
    assert.equal(typeof en[key], 'string', key);
    assert.equal(typeof zh[key], 'string', key);
  }
});
