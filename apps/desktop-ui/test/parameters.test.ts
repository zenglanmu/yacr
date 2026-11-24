// Parameters Manager: the pure wire model, the store state machine and the
// chrome wiring (ADR-0043 D9).
//
// No DOM is rendered. The model tests pin the normalisation and the exact
// `-PARAMETERS …` spellings the panel sends; the store tests pin single flight,
// the never-optimistic read-back and the host's own refusal retention; the
// wiring tests pin the ribbon entry, the two chrome-local words, the catalog
// rows and the case-preserving typed tail.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  PARAMETERS_CLOSE_COMMAND,
  PARAMETERS_COMMAND,
  createParamCommand,
  deleteParamCommand,
  formatParamNumber,
  formatParamValue,
  isKindToken,
  isParametersCloseCommand,
  isParametersCommand,
  paramKindKey,
  paramStatusKey,
  paramTargetToken,
  parseParamListAnswer,
  renameParamCommand,
  setParamValueCommand,
} from '../src/params/params.ts';
import { createParamsStore } from '../src/params/params-store.ts';
import {
  closeParamsPanel,
  openParamsPanel,
  paramsPanelSnapshot,
  resetParamsPanel,
  toggleParamsPanel,
} from '../src/params/panel.ts';
import { localSelectorFor } from '../src/commands/local-selectors.ts';
import { LOCAL_CHROME_COMMANDS } from '../src/commands/gating.ts';
import {
  admitCommandText,
  setCommandCatalog,
} from '../src/commands/commands.ts';
import { queueCadCommand } from '../src/cad/cad-command.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { RIBBON_TABS } from '../src/ribbon/ribbon-model.ts';
import { SHELL_ACTIONS } from '../src/ribbon/actions.ts';
import { HOST_NAMES, readCatalog, readModule } from './helpers.ts';

setCommandCatalog(HOST_NAMES);

/** One wire row. */
function wireRow(overrides = {}) {
  return {
    seq: 1,
    name: 'r',
    kind: 'length',
    unit: 'mm',
    value: 20,
    status: 'inherited',
    ...overrides,
  };
}

/** One `cad_param_list` answer. */
function answer(overrides = {}) {
  return { active: null, rows: [wireRow()], total: 1, ...overrides };
}

/** A recorded call plus a handler that answers it. */
function fakeInvoke(handler) {
  const calls = [];
  const invoke = async (command, args) => {
    calls.push([command, args]);
    return handler(command, args, calls);
  };
  return { calls, invoke };
}

const noWait = async () => {};

// ---------------------------------------------------------------------------
// The wire model.
// ---------------------------------------------------------------------------

test('the list answer is normalised without inventing a row or a value', () => {
  const answer = parseParamListAnswer({
    active: 'Small',
    rows: [
      wireRow({ seq: 2, name: 'q', kind: 'number', unit: null, value: 10 }),
      wireRow({ seq: 3, name: 's', status: 'suppressed', value: 7 }),
      // Dropped: no usable sequence.
      wireRow({ seq: 0, name: 'zero' }),
      // Dropped: no name (it can neither be shown nor addressed).
      wireRow({ seq: 4, name: '' }),
      // Dropped: a repeated sequence is one row.
      wireRow({ seq: 2, name: 'again' }),
    ],
    total: 3,
  });
  assert.equal(answer.active, 'Small');
  assert.deepEqual(answer.rows.map(row => [row.seq, row.name]), [[2, 'q'], [3, 's']]);
  assert.equal(answer.rows[0].unit, null);
  assert.equal(answer.rows[1].status, 'suppressed');
  assert.equal(answer.total, 3);
});

test('an unusable number renders as a dash, never as a made-up value', () => {
  const [row] = parseParamListAnswer({
    rows: [wireRow({ value: null, unit: 'mm' })],
    total: 1,
  }).rows;
  assert.equal(row.value, null);
  assert.equal(formatParamValue(row), '\u2014');
  assert.equal(formatParamValue({ ...row, value: 6, unit: 'mm' }), '6 mm');
  assert.equal(formatParamValue({ ...row, value: 0.5, unit: 'rad' }), '0.5 rad');
  assert.equal(formatParamNumber(6), '6');
  assert.equal(formatParamNumber(6.25), '6.25');
  assert.equal(formatParamNumber(Number.NaN), '\u2014');
});

test('the kind and status keys name only the modelled tokens', () => {
  assert.equal(paramKindKey('length'), 'parameters.kind.length');
  assert.equal(paramKindKey('number'), 'parameters.kind.number');
  assert.equal(paramKindKey('angle'), 'parameters.kind.angle');
  assert.equal(paramKindKey('kelvin'), null);
  assert.equal(paramStatusKey('inherited'), 'parameters.status.inherited');
  assert.equal(paramStatusKey('overridden'), 'parameters.status.overridden');
  assert.equal(paramStatusKey('suppressed'), 'parameters.status.suppressed');
  assert.equal(paramStatusKey('mystery'), null);
  assert.equal(isKindToken('length'), true);
  assert.equal(isKindToken('LENGTH'), false, 'the wire token is lowercase');
  assert.equal(isKindToken('kelvin'), false);
});

test('the command builders quote names and refuse a name the grammar cannot carry', () => {
  assert.equal(paramTargetToken('r'), 'r');
  assert.equal(paramTargetToken('My width'), '"My width"');
  assert.equal(paramTargetToken('bad"name'), null);
  assert.equal(paramTargetToken('line\nbreak'), null);

  assert.equal(createParamCommand('My width', 'length', '2 * 3'), '-PARAMETERS NEW "My width" length 2 * 3');
  assert.equal(setParamValueCommand('r', 'q * 2'), '-PARAMETERS SET r q * 2');
  assert.equal(renameParamCommand('r', 'My width'), '-PARAMETERS RENAME r "My width"');
  assert.equal(deleteParamCommand('s'), '-PARAMETERS DELETE s');

  // Nothing is built from an empty name/expression or an unknown kind: the
  // panel reports that instead of sending a malformed line.
  assert.equal(createParamCommand('', 'length', '1'), null);
  assert.equal(createParamCommand('r', 'kelvin', '1'), null);
  assert.equal(createParamCommand('r', 'length', '   '), null);
  assert.equal(setParamValueCommand('r', ''), null);
  assert.equal(renameParamCommand('r', ''), null);
  assert.equal(deleteParamCommand(''), null);
});

test('the two chrome words are exact bare matches', () => {
  assert.equal(PARAMETERS_COMMAND, 'PARAMETERS');
  assert.equal(PARAMETERS_CLOSE_COMMAND, 'PARAMETERSCLOSE');
  for (const text of ['PARAMETERS', 'parameters', '  Parameters  ']) {
    assert.equal(isParametersCommand(text), true, text);
  }
  for (const text of ['PARAMETERS 1', '-PARAMETERS', 'PARAMETERSCLOSE', '', null]) {
    assert.equal(isParametersCommand(text), false, String(text));
  }
  assert.equal(isParametersCloseCommand('parametersclose'), true);
  assert.equal(isParametersCloseCommand('PARAMETERS'), false);
});

// ---------------------------------------------------------------------------
// The store.
// ---------------------------------------------------------------------------

test('the store reads the host answer and distinguishes empty from error', async () => {
  const ready = fakeInvoke(() => answer());
  const store = createParamsStore(ready.invoke, { wait: noWait });
  await store.load();
  assert.equal(ready.calls[0][0], 'cad_param_list');
  assert.equal(store.getState().phase, 'ready');
  assert.equal(store.getState().rows[0].name, 'r');

  const empty = createParamsStore(fakeInvoke(() => ({ active: null, rows: [], total: 0 })).invoke, { wait: noWait });
  await empty.load();
  assert.equal(empty.getState().phase, 'empty');

  const broken = createParamsStore(fakeInvoke(() => { throw 'param.bad_table: nope'; }).invoke, { wait: noWait });
  await broken.load();
  assert.equal(broken.getState().phase, 'error');
  assert.deepEqual(broken.getState().error, { kind: 'read', reason: 'param.bad_table: nope' });
});

test('cad_waiting is one retry and then a "not ready" state, not an empty table', async () => {
  let reads = 0;
  const { invoke, calls } = fakeInvoke(() => {
    reads += 1;
    if (reads === 1) throw 'cad_waiting';
    return answer();
  });
  const store = createParamsStore(invoke, { wait: noWait });
  await store.load();
  assert.equal(calls.filter(([command]) => command === 'cad_param_list').length, 2);
  assert.equal(store.getState().phase, 'ready');
  // A host that never becomes ready settles in `waiting`, never `empty`.
  const stalled = createParamsStore(fakeInvoke(() => { throw 'cad_waiting'; }).invoke, { wait: noWait });
  await stalled.load();
  assert.equal(stalled.getState().phase, 'waiting');
  assert.equal(stalled.getState().error, null);
});

test('create sends one quoted -PARAMETERS line and re-reads the host table', async () => {
  let rows = [];
  const { invoke, calls } = fakeInvoke((command) => {
    if (command === 'cad_param_list') return answer({ rows, total: rows.length });
    if (command === 'cad_command') {
      rows = [wireRow({ seq: 1, name: 'My width', value: 6 })];
      return { accepted: true };
    }
    throw new Error(`unexpected ${command}`);
  });
  const store = createParamsStore(invoke, { wait: noWait });
  await store.load();
  const ack = await store.create('My width', 'length', '2 * 3');
  assert.deepEqual(ack, { kind: 'ok' });
  assert.deepEqual(calls[1], ['cad_command', { command: '-PARAMETERS NEW "My width" length 2 * 3' }]);
  assert.deepEqual(store.getState().rows.map(row => row.name), ['My width']);
  assert.equal(store.getState().busy, false);
});

test('set, rename and delete send their own lines and read the result back', async () => {
  const { invoke, calls } = fakeInvoke((command) => {
    if (command === 'cad_param_list') return answer();
    return { accepted: true };
  });
  const store = createParamsStore(invoke, { wait: noWait });
  await store.load();
  assert.deepEqual(await store.setValue('r', 'q * 2'), { kind: 'ok' });
  assert.deepEqual(await store.rename('r', 'My width'), { kind: 'ok' });
  assert.deepEqual(await store.remove('My width'), { kind: 'ok' });
  const sent = calls.filter(([command]) => command === 'cad_command').map(([, args]) => args.command);
  assert.deepEqual(sent, [
    '-PARAMETERS SET r q * 2',
    '-PARAMETERS RENAME r "My width"',
    '-PARAMETERS DELETE "My width"',
  ]);
});

test('a rename that the first read-back cannot see gets one follow-up read', async () => {
  let renamed = false;
  let readsAfterCommand = 0;
  let pending = false;
  const { invoke, calls } = fakeInvoke((command) => {
    if (command === 'cad_param_list') {
      if (pending) {
        readsAfterCommand += 1;
        if (readsAfterCommand >= 2) renamed = true;
      }
      return answer({ rows: [wireRow({ name: renamed ? 'w' : 'r' })], total: 1 });
    }
    pending = true;
    return { accepted: true };
  });
  const store = createParamsStore(invoke, { wait: noWait });
  await store.load();
  assert.deepEqual(await store.rename('r', 'w'), { kind: 'ok' });
  assert.equal(calls.filter(([command]) => command === 'cad_param_list').length, 3);
  assert.equal(store.getState().rows[0].name, 'w');
});

test('a refused write keeps the last good rows and the host reason', async () => {
  const { invoke } = fakeInvoke((command) => {
    if (command === 'cad_param_list') return answer();
    throw 'param.duplicate_name: parameter name "r" already belongs to sequence 1';
  });
  const store = createParamsStore(invoke, { wait: noWait });
  await store.load();
  const ack = await store.create('r', 'length', '3');
  assert.deepEqual(ack, {
    kind: 'error',
    reason: 'param.duplicate_name: parameter name "r" already belongs to sequence 1',
  });
  assert.deepEqual(store.getState().error, {
    kind: 'mutation',
    reason: 'param.duplicate_name: parameter name "r" already belongs to sequence 1',
  });
  assert.deepEqual(store.getState().rows.map(row => row.name), ['r']);
});

test('an unaddressable name is invalid before any IPC', async () => {
  const { invoke, calls } = fakeInvoke(() => answer());
  const store = createParamsStore(invoke, { wait: noWait });
  await store.load();
  assert.deepEqual(await store.create('bad"name', 'length', '1'), { kind: 'invalid' });
  assert.equal(calls.filter(([command]) => command === 'cad_command').length, 0);
});

test('a second write while one is in flight is busy, not queued', async () => {
  let release;
  const gate = new Promise<void>(resolve => { release = resolve; });
  const { invoke, calls } = fakeInvoke(async (command) => {
    if (command === 'cad_param_list') return answer();
    await gate;
    return { accepted: true };
  });
  const store = createParamsStore(invoke, { wait: noWait });
  await store.load();
  const first = store.create('w', 'length', '1');
  assert.deepEqual(await store.create('x', 'length', '2'), { kind: 'busy' });
  assert.equal(calls.filter(([command]) => command === 'cad_command').length, 1);
  release();
  assert.deepEqual(await first, { kind: 'ok' });
});

// ---------------------------------------------------------------------------
// Chrome wiring.
// ---------------------------------------------------------------------------

test('the ribbon entry is the Parametric tab Manage panel, a shell action', () => {
  const parametric = RIBBON_TABS.find(tab => tab.id === 'parametric');
  assert.ok(parametric, 'the Parametric tab exists');
  const panel = parametric.panels.find(entry => entry.id === 'parametric_manage');
  assert.ok(panel, 'the Manage panel exists');
  assert.equal(panel.titleKey, 'ribbon.panel.parametric_manage');
  assert.equal(Boolean(panel.pending), false, 'the panel is wired now');
  assert.equal(panel.items.length, 1);
  assert.equal(panel.items[0].action, SHELL_ACTIONS.parameters);
  assert.equal(panel.items[0].action, 'parameters');
  assert.equal(panel.items[0].command, undefined, 'it never sends a router command');
  assert.equal(panel.items[0].labelKey, 'parameters.open');
});

test('App and Workspace open one chrome-local panel, never a router command', async () => {
  const app = await readModule('App.tsx');
  // The action table lives in `chrome/panel-actions.ts` (the shell is at the code
  // gate); App consults it instead of growing one branch per panel.
  const actions = await readModule('chrome/panel-actions.ts');
  assert.match(actions, /import \{ openBimPanel \} from '\.\.\/bim\/panel\.ts';/);
  assert.match(actions, /import \{ openParamsPanel \} from '\.\.\/params\/panel\.ts';/);
  assert.match(actions, /parameters: openParamsPanel/);
  assert.match(app, /import \{ PANEL_ACTIONS \} from '\.\/chrome\/panel-actions\.ts';/);
  assert.match(app, /else if \(PANEL_ACTIONS\[action\]\) PANEL_ACTIONS\[action\]\(\);/);
  assert.doesNotMatch(app, /parameters[^\n]*runCommand/);

  const workspace = await readModule('components/Workspace.tsx');
  assert.match(workspace, /import \{ ParamsPanel \} from '\.\/ParamsPanel\.tsx';/);
  assert.match(workspace, /const paramsOpen = useParamsPanelOpen\(\);/);
  assert.match(workspace, /const showParams = palettesAvailable && paramsOpen;/);
  assert.match(workspace, /windowKey="params"/);
  assert.match(workspace, /showParams && !shell\.mobile/);
  assert.match(workspace, /showParams && shell\.mobile/);
});

test('PARAMETERS opens and PARAMETERSCLOSE closes the same store, idempotently', () => {
  resetParamsPanel();
  assert.deepEqual(localSelectorFor('PARAMETERS', 'desktop'), { kind: 'parameters' });
  assert.deepEqual(localSelectorFor('  parameters ', 'mobile'), { kind: 'parameters' });
  assert.deepEqual(localSelectorFor('PARAMETERSCLOSE', 'desktop'), { kind: 'parametersClose' });
  // A dashed line is the router's command-line form and never a local selector.
  assert.equal(localSelectorFor('-PARAMETERS SET r 2', 'desktop'), null);
  assert.equal(localSelectorFor('PARAMETERS 1', 'desktop'), null);

  assert.equal(paramsPanelSnapshot(), false);
  openParamsPanel();
  openParamsPanel();
  assert.equal(paramsPanelSnapshot(), true, 'a repeat open is a no-op, not a toggle');
  toggleParamsPanel();
  assert.equal(paramsPanelSnapshot(), false);
  closeParamsPanel();
  closeParamsPanel();
  assert.equal(paramsPanelSnapshot(), false, 'a repeat close is a success');
});

test('the three names are catalogued once with their catalog copy', async () => {
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  const rows = {
    PARAMETERS: 'draw.kind.parameters',
    PARAMETERSCLOSE: 'draw.kind.parametersclose',
    '-PARAMETERS': 'draw.kind.dashparameters',
  };
  for (const [id, key] of Object.entries(rows)) {
    const matches = COMMAND_CATALOG.filter(row => row.id === id);
    assert.equal(matches.length, 1, `${id} must be catalogued once`);
    assert.equal(matches[0].labelKey, key);
    for (const [locale, catalog] of [['zh-CN', zh], ['en', en]]) {
      assert.equal(typeof catalog[key], 'string', `${locale} missing ${key}`);
      assert.equal(typeof catalog[`${key}.hint`], 'string', `${locale} missing ${key}.hint`);
    }
    // The two bare words are chrome-local; the dashed one is a router selector.
    assert.equal(
      LOCAL_CHROME_COMMANDS.includes(id),
      id !== '-PARAMETERS',
      `${id} chrome-local classification`,
    );
  }
});

test('admission preserves a typed parameter name and expression verbatim', async () => {
  const loaded = {
    documentLabel: 'fixture.dwg',
    entityCount: 1,
    loading: false,
    canOpen: true,
    canNew: true,
    canSave: true,
    canPlot: true,
    canFit: true,
  };
  // The bare words need no vocabulary and no document.
  setCommandCatalog([]);
  assert.equal(admitCommandText('PARAMETERS', { status: null }).kind, 'admitted');
  assert.equal(admitCommandText('PARAMETERSCLOSE', { status: null }).kind, 'admitted');
  setCommandCatalog(HOST_NAMES);

  const admitted = admitCommandText('-PARAMETERS SET "My Width" q * 2', { status: loaded });
  assert.equal(admitted.kind, 'admitted');
  assert.equal(admitted.command, '-PARAMETERS SET "My Width" q * 2');

  const { invoke, calls } = fakeInvoke(() => ({ accepted: true }));
  const result = await queueCadCommand(invoke, '-PARAMETERS SET "My Width" q * 2', loaded, false);
  assert.equal(result.kind, 'accepted');
  assert.deepEqual(calls, [['cad_command', { command: '-PARAMETERS SET "My Width" q * 2' }]]);

  // A shape that cannot be a -PARAMETERS line is refused before IPC.
  for (const line of ['-PARAMETERS FROB r', '-PARAMETERS DELETE', '-PARAMETERS NEW r length']) {
    assert.equal(admitCommandText(line, { status: loaded }).kind, 'unsupported', line);
  }
});
