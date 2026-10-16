import test from 'node:test';
import assert from 'node:assert/strict';
import { commandEnabled, handleCommandKey, importProgressView, parseCadStatus, queueCadCommand, requestImportCancel, startCadPolling } from '../src/cad.mjs';
import { readAppSource, readComponentsSource } from './helpers.mjs';

const empty = { documentLabel: null, entityCount: null, loading: false, importProgress: null, error: null, canOpen: true, canFit: false, backend: null, adapter: null, camera: null, importCount: 0, frameCount: 0 };
const loaded = { ...empty, documentLabel: 'fixture.dwg', entityCount: 3, canFit: true, backend: 'Vulkan', adapter: 'hardware', camera: { center: [10, 20], scale: 2 }, importCount: 1, frameCount: 10 };
const importing = { ...empty, loading: true, canOpen: false, importProgress: { phase: 'entities', entitiesDone: 40, entitiesTotal: 200, bytes: 4096, cancellable: true } };
const settle = () => new Promise(resolve => setImmediate(resolve));

test('status validates explicit empty document and rejects unknown or malformed JSON', () => {
  assert.equal(parseCadStatus(empty), empty);
  assert.equal(parseCadStatus(loaded), loaded);
  assert.equal(parseCadStatus(importing), importing);
  for (const value of [null, {}, [], 'unknown', { ...empty, entityCount: undefined }, { ...empty, loading: 'false' }, { ...empty, camera: { center: [0], scale: 1 } }, { ...empty, camera: { center: [0, 0], scale: 0 } }, { ...empty, frameCount: -1 }]) {
    assert.throws(() => parseCadStatus(value), /CAD_STATUS_INVALID/);
  }
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
});

test('unsupported, full queue, IPC failure and malformed acceptance do not clear or pretend success', async () => {
  assert.deepEqual(await queueCadCommand(() => assert.fail('unsupported command reached IPC'), 'LINE', loaded, false), { kind: 'unsupported', command: 'LINE' });
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
  assert.ok(source.includes("placeholder={t('command.placeholder')}"));
  assert.ok(!source.includes("t('command.completed')"));
  assert.ok(!source.includes("t('command.prompt')"));
  assert.ok(!source.includes("t('status.scaffold')"));
  // Focusing the command line suspends the CAD pointer lane; blurring restores it.
  assert.ok(source.includes('shell.suspendPointer(true)'));
  assert.ok(source.includes('shell.suspendPointer(false)'));
});
