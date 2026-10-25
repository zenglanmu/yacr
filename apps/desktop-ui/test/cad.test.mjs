import test from 'node:test';
import assert from 'node:assert/strict';
import { commandEnabled, handleCommandKey, importProgressView, parseCadStatus, queueCadCommand, requestImportCancel, startCadPolling } from '../src/cad.mjs';
import { setCommandCatalog } from '../src/commands.mjs';
import { readAppSource, readComponentsSource } from './helpers.mjs';

// The admission vocabulary the host reports; without it every command would be
// explicitly unsupported, so load a representative set once.
setCommandCatalog(['OPEN', 'NEW', 'SAVE', 'PLOT', 'FIT', 'LINE', 'MOVE', 'CONFIRM', 'CANCEL', 'UNDO']);

const empty = { documentLabel: null, entityCount: null, loading: false, importProgress: null, error: null, canOpen: true, canNew: true, gridEnabled: true, orthoEnabled: false, canSave: false, canPlot: false, canFit: false, backend: null, adapter: null, camera: null, importCount: 0, frameCount: 0, selection: { count: 0, empty: true, rows: [], mixedKeys: [] } };
const loaded = { ...empty, documentLabel: 'fixture.dwg', entityCount: 3, canSave: true, canPlot: true, canFit: true, backend: 'Vulkan', adapter: 'hardware', camera: { center: [10, 20], scale: 2 }, importCount: 1, frameCount: 10 };
const importing = { ...empty, loading: true, canOpen: false, canNew: false, importProgress: { phase: 'entities', entitiesDone: 40, entitiesTotal: 200, bytes: 4096, cancellable: true } };
const settle = () => new Promise(resolve => setImmediate(resolve));

test('status validates explicit empty document and rejects unknown or malformed JSON', () => {
  assert.equal(parseCadStatus(empty), empty);
  assert.equal(parseCadStatus(loaded), loaded);
  assert.equal(parseCadStatus(importing), importing);
  for (const value of [null, {}, [], 'unknown', { ...empty, entityCount: undefined }, { ...empty, loading: 'false' }, { ...empty, canNew: undefined }, { ...empty, canSave: 'true' }, { ...empty, canPlot: 1 }, { ...empty, gridEnabled: undefined }, { ...empty, gridEnabled: 'true' }, { ...empty, gridEnabled: 1 }, { ...empty, orthoEnabled: undefined }, { ...empty, orthoEnabled: 'true' }, { ...empty, orthoEnabled: 1 }, { ...empty, camera: { center: [0], scale: 1 } }, { ...empty, camera: { center: [0, 0], scale: 0 } }, { ...empty, frameCount: -1 }, { ...empty, snapEnabled: 'true' }, { ...empty, snapEnabled: 1 }, { ...empty, hoverPreview: 'true' }, { ...empty, snapKind: 1 }, { ...empty, snapKind: {} }]) {
    assert.throws(() => parseCadStatus(value), /CAD_STATUS_INVALID/);
  }
  // The display-grid switch is required and always present (unlike the optional
  // `snapEnabled`/`hoverPreview`), so a host that omits it is rejected outright
  // rather than rendered as a guessed off chip.
  const gridOff = { ...empty, gridEnabled: false };
  assert.equal(parseCadStatus(gridOff), gridOff);
  // `orthoEnabled` is required the same way (the ORTHO chip is a real toggle, so
  // a guessed default is never acceptable), and a real boolean is carried as-is.
  const orthoOn = { ...empty, orthoEnabled: true };
  assert.equal(parseCadStatus(orthoOn), orthoOn);
  assert.equal(parseCadStatus(empty).orthoEnabled, false);
  // The optional object-snap flag is accepted when it is a real boolean and
  // tolerated when absent (older host).
  const snapOff = { ...empty, snapEnabled: false };
  assert.equal(parseCadStatus(snapOff), snapOff);
  assert.equal(parseCadStatus(empty), empty);
  // The optional current snap kind is accepted as a real string or `null`.
  const snapKind = { ...empty, snapKind: 'endpoint' };
  assert.equal(parseCadStatus(snapKind), snapKind);
  const snapKindNull = { ...empty, snapKind: null };
  assert.equal(parseCadStatus(snapKindNull), snapKindNull);
});

test('import progress is validated and never rendered from a malformed block', () => {
  const bad = [
    { ...empty, importProgress: {} },
    { ...empty, importProgress: { phase: 'importing', entitiesDone: 0, entitiesTotal: null, bytes: null, cancellable: true } },
    { ...empty, importProgress: { phase: null, entitiesDone: -1, entitiesTotal: null, bytes: null, cancellable: true } },
    { ...empty, importProgress: { phase: null, entitiesDone: 0, entitiesTotal: 1.5, bytes: null, cancellable: true } },
    { ...empty, importProgress: { phase: null, entitiesDone: 0, entitiesTotal: null, bytes: null, cancellable: 'yes' } },
  ];
  for (const value of bad) assert.throws(() => parseCadStatus(value), /CAD_STATUS_INVALID/);
  const indeterminate = { ...empty, loading: true, importProgress: { phase: null, entitiesDone: 0, entitiesTotal: null, bytes: null, cancellable: true } };
  assert.equal(parseCadStatus(indeterminate), indeterminate);
});

test('selection block is validated: empty, single, multi are well-formed and a malformed one is rejected', () => {
  const row = (key, value, extra = {}) => ({ key, value, group: 'entity', mixed: false, editable: false, kind: 'text', options: [], ...extra });
  const single = { count: 1, empty: false, rows: [row('id', '1'), row('length', '5.000000', { group: 'geometry', kind: 'number' })], mixedKeys: [] };
  const multi = {
    count: 3,
    empty: false,
    rows: [row('id', null, { mixed: true }), row('type', null, { mixed: true })],
    mixedKeys: ['id', 'type'],
  };
  for (const selection of [empty.selection, single, multi]) {
    const status = { ...empty, selection };
    assert.equal(parseCadStatus(status), status);
  }
  const bad = [
    { ...empty, selection: null },
    { ...empty, selection: undefined },
    { ...empty, selection: [] },
    { ...empty, selection: 'none' },
    { ...empty, selection: { count: 1, empty: true, rows: [], mixedKeys: [] } },
    { ...empty, selection: { count: 0, empty: false, rows: [], mixedKeys: [] } },
    { ...empty, selection: { count: -1, empty: false, rows: [], mixedKeys: [] } },
    { ...empty, selection: { count: 1.5, empty: false, rows: [], mixedKeys: [] } },
    { ...empty, selection: { count: 1, empty: false, rows: null, mixedKeys: [] } },
    { ...empty, selection: { count: 1, empty: false, rows: [], mixedKeys: null } },
    { ...empty, selection: { count: 1, empty: false, rows: [{ key: '', value: 'x', group: 'entity', mixed: false, editable: false, kind: 'text', options: [] }], mixedKeys: [] } },
    { ...empty, selection: { count: 1, empty: false, rows: [{ key: 'id' }], mixedKeys: [] } },
    { ...empty, selection: { count: 1, empty: false, rows: [{ key: 'id', value: 3, group: 'entity', mixed: false, editable: false, kind: 'text', options: [] }], mixedKeys: [] } },
    { ...empty, selection: { count: 1, empty: false, rows: ['id'], mixedKeys: [] } },
    // rows without a selection are malformed.
    { ...empty, selection: { count: 0, empty: true, rows: [{ key: 'id', value: '1', group: 'entity', mixed: false, editable: false, kind: 'text', options: [] }], mixedKeys: [] } },
    // a multi-selection blindly using untyped rows is rejected.
    { ...empty, selection: { count: 2, empty: false, rows: [{ key: 'id', value: '1' }], mixedKeys: [] } },
    // mixed keys are only meaningful for a real multi-selection.
    { ...empty, selection: { count: 1, empty: false, rows: [], mixedKeys: ['id'] } },
    { ...empty, selection: { count: 2, empty: false, rows: [], mixedKeys: [3] } },
  ];
  for (const value of bad) assert.throws(() => parseCadStatus(value), /CAD_STATUS_INVALID/);
});

test('an unknown row key is tolerated and never coerced to a fabricated state', () => {
  const row = (key, value) => ({ key, value, group: 'future_group', mixed: false, editable: false, kind: 'text', options: [] });
  const status = {
    ...empty,
    selection: { count: 1, empty: false, rows: [row('future_key', 'raw'), row('id', '7')], mixedKeys: [] },
  };
  assert.equal(parseCadStatus(status), status);
});

test('import progress view is determinate only with a real total and never invents 100%', () => {
  const t = (key, params) => (params ? `${key}(${Object.values(params).join('/')})` : key);
  assert.equal(importProgressView(null, t), null);
  const determinate = importProgressView(importing.importProgress, t);
  assert.equal(determinate.determinate, true);
  assert.equal(determinate.ratio, 0.2);
  assert.equal(determinate.count, 'import.progress.count(40/200)');
  assert.equal(determinate.detail, 'import.progress.count(40/200)import.detail.separatorimport.progress.bytes(4096)');
  assert.equal(determinate.phase, 'import.phase.entities');
  assert.equal(determinate.cancellable, true);
  const indeterminate = importProgressView({ phase: null, entitiesDone: 7, entitiesTotal: null, bytes: null, cancellable: true }, t);
  assert.equal(indeterminate.determinate, false);
  assert.equal(indeterminate.ratio, null);
  assert.equal(indeterminate.count, 'import.progress.indeterminate(7)');
  assert.equal(indeterminate.phase, 'import.phase.unknown');
  // A ratio beyond the observed total is clamped, never shown as past 100%.
  assert.equal(importProgressView({ phase: 'entities', entitiesDone: 9, entitiesTotal: 4, bytes: null, cancellable: false }, t).ratio, 1);
});

test('cancel is offered only for a live cancellable import and is admission, not completion', async () => {
  assert.deepEqual(await requestImportCancel(() => assert.fail('cancel reached IPC while unavailable'), loaded), { kind: 'unavailable' });
  assert.deepEqual(await requestImportCancel(() => assert.fail('cancel reached IPC while terminal'), { ...importing, importProgress: { ...importing.importProgress, cancellable: false } }), { kind: 'unavailable' });
  const calls = [];
  const accepted = await requestImportCancel(async (...args) => { calls.push(args); return { accepted: true }; }, importing);
  assert.deepEqual(calls, [['cad_command', { command: 'CANCEL' }]]);
  assert.deepEqual(accepted, { kind: 'accepted' });
  for (const invoke of [async () => { throw Error('QUEUE_FULL'); }, async () => ({ accepted: false }), async () => null]) {
    const result = await requestImportCancel(invoke, importing);
    assert.equal(result.kind, 'failed');
    assert.ok(result.error);
  }
});

test('the progress affordance reads only real status fields and offers cancel through the real path', async () => {
  const components = await readComponentsSource();
  const source = await readAppSource();
  assert.ok(components.includes('importProgressView(shell.importProgress, t)'));
  assert.ok(components.includes('role="progressbar"'));
  assert.ok(components.includes("is-indeterminate"));
  assert.ok(components.includes("shell.cancelImport()"));
  assert.ok(source.includes("invoke('cad_command', { command: 'CANCEL' })"));
  // No fabricated progress: no CSS/JS animation drives the bar to 100% on its own.
  assert.ok(!source.includes('import.progress.complete'));
});

test('command admission is not completion and invokes only the frozen command IPC', async () => {
  const calls = [];
  const result = await queueCadCommand(async (...args) => { calls.push(args); return { accepted: true }; }, ' fit ', loaded, false);
  assert.deepEqual(calls, [['cad_command', { command: 'FIT' }]]);
  assert.deepEqual(result, { kind: 'accepted', command: 'FIT' });
  assert.equal('completed' in result, false);
  assert.equal(loaded.importCount, 1);
  const open = await queueCadCommand(async (name, args) => { assert.equal(name, 'cad_command'); assert.equal(args.command, 'OPEN'); return { accepted: true }; }, 'OPEN', empty, false);
  assert.equal(open.kind, 'accepted');
  // An interactive tool admitted by the loaded host vocabulary now reaches IPC.
  const line = await queueCadCommand(async (name, args) => { assert.equal(name, 'cad_command'); assert.equal(args.command, 'LINE'); return { accepted: true }; }, ' line ', loaded, false);
  assert.deepEqual(line, { kind: 'accepted', command: 'LINE' });
});

test('unsupported, unavailable, full queue, IPC failure and malformed acceptance do not clear or pretend success', async () => {
  assert.deepEqual(
    await queueCadCommand(() => assert.fail('unknown command reached IPC'), 'NOT_A_COMMAND', loaded, false),
    { kind: 'unsupported', command: 'NOT_A_COMMAND', reasonKey: 'command.unsupported' },
  );
  // `NEW` used to be refused as "host-owned"; its capability is `canNew`, so with
  // a true capability it reaches the frozen IPC like any other router command.
  const calls = [];
  const created = await queueCadCommand(async (...args) => { calls.push(args); return { accepted: true }; }, 'NEW', loaded, false);
  assert.deepEqual(calls, [['cad_command', { command: 'NEW' }]]);
  assert.deepEqual(created, { kind: 'accepted', command: 'NEW' });
  // A false capability refuses before IPC and carries the catalog reason key.
  const refusedCalls = [];
  const save = await queueCadCommand(async (...args) => { refusedCalls.push(args); return { accepted: true }; }, 'SAVE', empty, false);
  assert.deepEqual(refusedCalls, []);
  assert.deepEqual(save, { kind: 'failed', error: 'CAD_COMMAND_UNAVAILABLE', command: 'SAVE', reasonKey: 'plot.no_document' });
  for (const invoke of [async () => { throw Error('QUEUE_FULL'); }, async () => { throw Error('IPC_DISCONNECTED'); }, async () => ({ accepted: false }), async () => null]) {
    const result = await queueCadCommand(invoke, 'OPEN', loaded, false);
    assert.equal(result.kind, 'failed');
    assert.ok(result.error);
  }
});

test('loading, pending, unknown status and missing database capability disable admission', async () => {
  assert.equal(commandEnabled(empty, false, 'OPEN'), true);
  assert.equal(commandEnabled(empty, false, 'FIT'), false);
  assert.equal(commandEnabled({ ...empty, canFit: true }, false, 'FIT'), false);
  assert.equal(commandEnabled(loaded, false, 'FIT'), true);
  for (const status of [null, { ...loaded, loading: true }, { ...loaded, canOpen: false, canFit: false }]) {
    assert.equal(commandEnabled(status, false, 'OPEN'), false);
    assert.equal((await queueCadCommand(() => assert.fail('disabled command reached IPC'), 'OPEN', status, false)).kind, 'failed');
  }
  assert.equal(commandEnabled(loaded, true, 'FIT'), false);
});

test('keyboard submits once on Enter without trapping Tab, Escape or IME composition', () => {
  let submitted = 0;
  let prevented = 0;
  const event = props => ({ preventDefault() { prevented++; }, ...props });
  for (const key of ['Tab', 'Escape', 'ArrowLeft', 'ArrowRight']) handleCommandKey(event({ key }), () => submitted++);
  handleCommandKey(event({ key: 'Enter', isComposing: true }), () => submitted++);
  handleCommandKey(event({ key: 'Enter', repeat: true }), () => submitted++);
  assert.equal(submitted, 0);
  assert.equal(prevented, 0);
  handleCommandKey(event({ key: 'Enter' }), () => submitted++);
  assert.equal(submitted, 1);
  assert.equal(prevented, 1);
});

test('status polling is bounded, nonoverlapping, explicit on error and stops cleanly', async () => {
  const tasks = new Map();
  let sequence = 0;
  const clock = { setTimeout(callback, delay) { tasks.set(++sequence, { callback, delay }); return sequence; }, clearTimeout(id) { tasks.delete(id); } };
  let resolve;
  let calls = 0;
  const statuses = [];
  const errors = [];
  const stop = startCadPolling(async name => {
    assert.equal(name, 'cad_status');
    calls++;
    if (calls === 1) return new Promise(done => { resolve = done; });
    if (calls === 2) return {};
    return loaded;
  }, status => statuses.push(status), error => errors.push(error), clock);
  assert.equal(calls, 1);
  assert.equal(tasks.size, 0);
  resolve(empty); await settle();
  assert.equal(tasks.size, 1);
  assert.equal([...tasks.values()][0].delay, 250);
  const runTimer = async () => { const entry = [...tasks.entries()][0]; tasks.delete(entry[0]); await entry[1].callback(); };
  await runTimer();
  assert.equal(statuses.at(-1), null);
  assert.match(errors[0], /CAD_STATUS_INVALID/);
  assert.equal([...tasks.values()][0].delay, 1000);
  await runTimer();
  assert.equal(statuses.at(-1), loaded);
  assert.equal([...tasks.values()][0].delay, 250);
  stop();
  assert.equal(tasks.size, 0);
});

test('React command copy is narrow and only accepted input is cleared', async () => {
  const source = await readAppSource();
  assert.ok(source.includes("result.kind === 'accepted' && options.fromInput"));
  // The field's placeholder is catalog copy in both modes: the command prompt,
  // or the text step's own prompt (`text-tool.test.mjs` pins that one).
  assert.ok(source.includes("t('command.placeholder')"));
  assert.ok(source.includes("t(TEXT_PROMPT_KEY)"));
  assert.ok(!source.includes("t('command.completed')"));
  assert.ok(!source.includes("t('command.prompt')"));
  assert.ok(!source.includes("t('status.scaffold')"));
  // Focusing the command line suspends the CAD pointer lane; blurring restores it.
  assert.ok(source.includes('shell.suspendPointer(true)'));
  assert.ok(source.includes('shell.suspendPointer(false)'));
});
