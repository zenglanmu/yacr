import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { bindProbeInput, createInputLane } from '../src/input.mjs';

const settle = () => new Promise(resolve => setImmediate(resolve));

test('disconnected IPC reports each failure and does not swallow terminal input', async () => {
  const attempted = [];
  const errors = [];
  const lane = createInputLane(async input => {
    attempted.push(input.kind);
    throw Error(`IPC_DISCONNECTED:${input.kind}`);
  }, error => errors.push(error));
  lane.push({ kind: 'pointer_down' });
  lane.push({ kind: 'pointer_move', x: 5 });
  lane.push({ kind: 'pointer_up' });
  lane.push({ kind: 'blur' });
  await settle();
  assert.deepEqual(attempted, ['pointer_down', 'pointer_move', 'pointer_up', 'blur']);
  assert.equal(errors.length, 4);
  assert.ok(errors.every(error => error.includes('IPC_DISCONNECTED')));
  assert.equal(lane.pending(), 0);
});

test('ordered async lane coalesces moves without crossing down/up boundaries', async () => {
  const sent = [];
  let unblock;
  let inflight = 0;
  let maxInflight = 0;
  const lane = createInputLane(async input => {
    inflight++;
    maxInflight = Math.max(maxInflight, inflight);
    sent.push(input);
    if (sent.length === 1) await new Promise(resolve => { unblock = resolve; });
    inflight--;
  }, assert.fail);
  lane.push({ kind: 'pointer_down' });
  for (let x = 0; x < 1000; x++) lane.push({ kind: 'pointer_move', x });
  lane.push({ kind: 'pointer_up' });
  lane.push({ kind: 'pointer_down' });
  lane.push({ kind: 'pointer_cancel' });
  assert.equal(lane.pending(), 5);
  unblock();
  await settle();
  assert.deepEqual(sent.map(input => input.kind), ['pointer_down', 'pointer_move', 'pointer_up', 'pointer_down', 'pointer_cancel']);
  assert.equal(sent[1].x, 999);
  assert.equal(maxInflight, 1);
  assert.ok(sent.every((input, i) => !i || input.sequence > sent[i - 1].sequence));
});

test('bounded queue reserves room for termination and reports failures honestly', async () => {
  const errors = [];
  const sent = [];
  let unblock;
  const lane = createInputLane(async input => {
    sent.push(input);
    if (sent.length === 1) await new Promise(resolve => { unblock = resolve; });
    if (input.kind === 'blur') throw Error('native unavailable');
  }, error => errors.push(error));
  lane.push({ kind: 'resize' });
  while (lane.canStart()) {
    lane.push({ kind: 'pointer_down' });
    lane.push({ kind: 'pointer_up' });
  }
  lane.push({ kind: 'pointer_cancel' });
  lane.push({ kind: 'blur' });
  assert.ok(lane.pending() <= 65);
  unblock();
  await settle();
  assert.equal(sent.at(-2).kind, 'pointer_cancel');
  assert.equal(sent.at(-1).kind, 'blur');
  assert.match(errors[0], /native unavailable/);
});

test('adjacent wheel events preserve cumulative deltas', async () => {
  const sent = [];
  let unblock;
  const lane = createInputLane(async input => {
    sent.push(input);
    if (sent.length === 1) await new Promise(resolve => { unblock = resolve; });
  }, assert.fail);
  lane.push({ kind: 'resize' });
  lane.push({ kind: 'wheel', dx: 2, dy: 4 });
  lane.push({ kind: 'wheel', dx: 3, dy: -1 });
  unblock();
  await settle();
  assert.equal(sent[1].dx, 5);
  assert.equal(sent[1].dy, 3);
});

test('alternating continuous events cannot consume terminal event reserve', async () => {
  const sent = [];
  let unblock;
  const lane = createInputLane(async input => {
    sent.push(input);
    if (sent.length === 1) await new Promise(resolve => { unblock = resolve; });
  }, assert.fail);
  lane.push({ kind: 'pointer_down' });
  for (let i = 0; i < 10000; i++) lane.push({ kind: i % 2 ? 'wheel' : 'pointer_move', x: i, dy: 1 });
  assert.ok(lane.pending() <= 57);
  assert.equal(lane.push({ kind: 'pointer_up' }), true);
  assert.equal(lane.push({ kind: 'pointer_cancel' }), true);
  assert.equal(lane.push({ kind: 'blur' }), true);
  unblock();
  await settle();
  assert.deepEqual(sent.slice(-3).map(input => input.kind), ['pointer_up', 'pointer_cancel', 'blur']);
});

function mockEnvironment() {
  class Target {
    handlers = new Map();
    addEventListener(type, callback) { this.handlers.set(type, callback); }
    removeEventListener(type) { this.handlers.delete(type); }
    emit(type, event = {}) { this.handlers.get(type)?.(event); }
  }
  const env = new Target();
  const element = new Target();
  element.rect = { left: 0, top: 0, width: 800, height: 600 };
  element.getBoundingClientRect = () => element.rect;
  let captured = null;
  element.setPointerCapture = id => { captured = id; };
  element.hasPointerCapture = id => captured === id;
  element.releasePointerCapture = id => { captured = null; element.emit('lostpointercapture', { pointerId: id }); };
  Object.assign(env, { innerWidth: 800, innerHeight: 600, devicePixelRatio: 1, document: { documentElement: {} } });
  const frames = new Map();
  let counter = 0;
  env.requestAnimationFrame = callback => { frames.set(++counter, callback); return counter; };
  env.cancelAnimationFrame = id => frames.delete(id);
  env.tick = () => { const tasks = [...frames.values()]; frames.clear(); tasks.forEach(callback => callback()); };
  const media = [];
  env.matchMedia = () => { const target = new Target(); media.push(target); return target; };
  env.ResizeObserver = class {
    constructor(callback) { env.observerNotify = callback; }
    observe(target) { env.observedTarget = target; }
    disconnect() { env.observerDisconnected = true; }
  };
  return { env, element, frames, media, captured: () => captured };
}

test('pointer capture, RAF flush, cancel, blur, overlay exclusion, wheel and DPR', () => {
  const { env, element, frames, media, captured } = mockEnvironment();
  const inputs = [];
  let allowed = true;
  const lane = { canStart: () => allowed, push: input => inputs.push(input) };
  const binding = bindProbeInput(element, lane, env);
  assert.deepEqual(inputs[0], { kind: 'resize', x: 0, y: 0, width: 800, height: 600, dpr: 1 });
  const point = (x, pointerId = 7) => ({ pointerId, clientX: x, clientY: 10 });
  element.emit('pointerdown', point(1));
  assert.equal(captured(), 7);
  for (let x = 2; x <= 500; x++) element.emit('pointermove', point(x));
  assert.equal(frames.size, 1);
  element.emit('pointerup', point(501));
  assert.equal(captured(), null);
  assert.equal(frames.size, 0);
  assert.deepEqual(inputs.slice(1).map(input => input.kind), ['pointer_down', 'pointer_move', 'pointer_up']);
  assert.equal(inputs[2].x, 500);
  element.emit('pointerdown', point(2));
  element.emit('pointercancel', point(3));
  assert.equal(inputs.at(-1).kind, 'pointer_cancel');
  element.emit('pointerdown', point(2));
  env.emit('blur');
  assert.deepEqual(inputs.slice(-2).map(input => input.kind), ['pointer_cancel', 'blur']);
  // Refocusing must not leave a captured pointer or replay stale RAF work.
  assert.equal(captured(), null);
  assert.equal(frames.size, 0);
  env.emit('focus');
  element.emit('pointerdown', point(4));
  element.emit('pointerup', point(5));
  assert.deepEqual(inputs.slice(-2).map(input => input.kind), ['pointer_down', 'pointer_up']);
  binding.suspend(true);
  const length = inputs.length;
  element.emit('pointerdown', point(2));
  element.emit('pointermove', point(3));
  let prevented = false;
  element.emit('wheel', { preventDefault() { prevented = true; }, deltaX: 1, deltaY: 2 });
  assert.equal(prevented, true);
  assert.equal(inputs.length, length);
  binding.suspend(false);
  allowed = false;
  element.emit('pointerdown', point(2));
  assert.equal(inputs.length, length);
  allowed = true;
  element.emit('wheel', { ...point(2), preventDefault() {}, deltaX: 1, deltaY: 2, deltaMode: 1 });
  assert.equal(inputs.at(-1).dy, 32);
  env.devicePixelRatio = 2;
  media.at(-1).emit('change');
  env.tick();
  assert.equal(inputs.at(-1).dpr, 2);
  env.emit('resize');
  env.tick();
  assert.equal(inputs.at(-1).dpr, 2);
  binding.dispose();
  assert.equal(element.handlers.size, 0);
  assert.equal(env.handlers.size, 0);
  assert.equal(env.observerDisconnected, true);
});

test('ordinary UI strings use existing bilingual dictionary keys', async () => {
  const source = await readFile(new URL('../src/main.jsx', import.meta.url), 'utf8');
  const zh = JSON.parse(await readFile(new URL('../../../../crates/cad-ui-slint/i18n/zh-CN.json', import.meta.url), 'utf8'));
  const en = JSON.parse(await readFile(new URL('../../../../crates/cad-ui-slint/i18n/en.json', import.meta.url), 'utf8'));
  for (const match of source.matchAll(/\bt\('([^']+)'\)/g)) {
    assert.equal(typeof zh[match[1]], 'string', match[1]);
    assert.equal(typeof en[match[1]], 'string', match[1]);
  }
  assert.ok(!source.includes('<canvas'));
});

test('CAD rect reports ribbon offset, dynamic footer height and local pointer CSS coordinates', () => {
  const { env, element, frames, media } = mockEnvironment();
  element.rect = { left: 8, top: 101, width: 784, height: 439 };
  const inputs = [];
  const binding = bindProbeInput(element, { canStart: () => true, push: input => { inputs.push(input); return true; } }, env);
  assert.equal(env.observedTarget, element);
  assert.deepEqual(inputs[0], { kind: 'resize', x: 8, y: 101, width: 784, height: 439, dpr: 1 });
  element.emit('pointerdown', { pointerId: 1, clientX: 28, clientY: 131 });
  assert.deepEqual(inputs.at(-1), { kind: 'pointer_down', x: 20, y: 30 });
  element.emit('pointermove', { pointerId: 1, clientX: 38, clientY: 141 });
  element.rect = { left: 8, top: 101, width: 784, height: 409 };
  for (let i = 0; i < 1000; i++) env.observerNotify();
  assert.equal(frames.size, 2); // One move RAF and one geometry RAF, regardless of notify count.
  env.tick();
  assert.deepEqual(inputs.slice(-2), [
    { kind: 'pointer_move', x: 30, y: 40 },
    { kind: 'resize', x: 8, y: 101, width: 784, height: 409, dpr: 1 },
  ]);
  const count = inputs.length;
  env.observerNotify(); env.emit('resize'); env.tick();
  assert.equal(inputs.length, count);
  env.devicePixelRatio = 1.5;
  media.at(-1).emit('change'); env.tick();
  assert.deepEqual(inputs.at(-1), { kind: 'resize', x: 8, y: 101, width: 784, height: 409, dpr: 1.5 });
  element.rect = { left: 8, top: 201, width: 784, height: 309 };
  element.emit('pointerup', { pointerId: 1, clientX: 48, clientY: 251 });
  assert.deepEqual(inputs.slice(-2), [
    { kind: 'resize', x: 8, y: 201, width: 784, height: 309, dpr: 1.5 },
    { kind: 'pointer_up', x: 40, y: 50 },
  ]);
  binding.dispose();
});

test('resize is an ordered barrier that move coalescing cannot erase', async () => {
  const sent = [];
  let unblock;
  const lane = createInputLane(async input => {
    sent.push(input);
    if (sent.length === 1) await new Promise(resolve => { unblock = resolve; });
  }, assert.fail);
  lane.push({ kind: 'pointer_down' });
  lane.push({ kind: 'pointer_move', x: 10 });
  lane.push({ kind: 'resize', x: 0, y: 101, width: 800, height: 420, dpr: 2 });
  for (let i = 0; i < 1000; i++) lane.push({ kind: 'pointer_move', x: i });
  lane.push({ kind: 'pointer_up' });
  unblock(); await settle();
  assert.deepEqual(sent.map(input => input.kind), ['pointer_down', 'pointer_move', 'resize', 'pointer_move', 'pointer_up']);
  assert.equal(sent[2].y, 101);
  assert.equal(sent[3].x, 999);
  assert.ok(sent.every((input, i) => !i || input.sequence > sent[i - 1].sequence));
});
