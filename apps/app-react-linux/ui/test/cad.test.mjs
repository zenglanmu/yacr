import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { commandEnabled, handleCommandKey, parseCadStatus, queueCadCommand, startCadPolling } from '../src/cad.mjs';

const empty = { documentLabel: null, entityCount: null, loading: false, error: null, canOpen: true, canFit: false, backend: null, adapter: null, camera: null, importCount: 0, frameCount: 0 };
const loaded = { ...empty, documentLabel: 'fixture.dwg', entityCount: 3, canFit: true, backend: 'Vulkan', adapter: 'hardware', camera: { center: [10, 20], scale: 2 }, importCount: 1, frameCount: 10 };
const settle = () => new Promise(resolve => setImmediate(resolve));

test('status validates explicit empty document and rejects unknown or malformed JSON', () => {
  assert.equal(parseCadStatus(empty), empty);
  assert.equal(parseCadStatus(loaded), loaded);
  for (const value of [null, {}, [], 'unknown', { ...empty, entityCount: undefined }, { ...empty, loading: 'false' }, { ...empty, camera: { center: [0], scale: 1 } }, { ...empty, camera: { center: [0, 0], scale: 0 } }, { ...empty, frameCount: -1 }]) {
    assert.throws(() => parseCadStatus(value), /CAD_STATUS_INVALID/);
  }
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
  const source = await readFile(new URL('../src/main.jsx', import.meta.url), 'utf8');
  assert.ok(source.includes("result.kind === 'accepted' && fromInput"));
  assert.ok(source.includes('placeholder="OPEN / FIT"'));
  assert.ok(!source.includes("t('command.completed')"));
  assert.ok(!source.includes("t('command.prompt')"));
  assert.ok(!source.includes("t('status.scaffold')"));
  assert.ok(source.includes('onFocus={() => bridge.current?.suspend(true)}'));
  assert.ok(source.includes('onBlur={() => bridge.current?.suspend(false)}'));
});
