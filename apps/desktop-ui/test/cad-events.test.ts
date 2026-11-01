// Contracts for the typed push event source that replaced `cad_status` polling.
//
// Same conventions as the other suites: `.ts` by filename only, plain JavaScript
// bodies, behaviour imported directly from `src/`, structure read as source text.
// Every timer here is a **fake clock**: the source must never need a real one in
// steady state, and a recovery retry is driven by hand.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import {
  CAD_EVENT_BACKOFF_CAP_MS,
  cadEventBackoff,
  cadEventPumpsFrames,
  createTauriEventSource,
  createWasmEventSource,
  parseCadEventFrame,
} from '../src/cad/cad-events.ts';

const settle = () => new Promise(resolve => setImmediate(resolve));

/** A complete, valid `cad_status` payload (the fields `parseCadStatus` enforces). */
function status(overrides = {}) {
  return {
    documentLabel: null, entityCount: null, loading: false, importProgress: null,
    error: null, canNew: true, canOpen: true, canSave: false, canPlot: false, canFit: false,
    backend: null, adapter: null, camera: null, importCount: 0, frameCount: 0,
    cursorWorld: null, gridEnabled: true, snapGridEnabled: false, snapSpacing: 10,
    orthoEnabled: false,
    selection: { count: 0, empty: true, rows: [], mixedKeys: [], text: null, text_multiline: false },
    hostProfile: { surfaceMode: 'native-below', documentOwner: 'native', palettesAvailable: true, standardViews: null },
    ...overrides,
  };
}

/** A deterministic clock: nothing runs unless the test says so. */
function fakeClock() {
  let next = 0;
  const timers = new Map();
  return {
    setTimeout(callback, delay) {
      next += 1;
      timers.set(next, { callback, delay });
      return next;
    },
    clearTimeout(handle) {
      timers.delete(handle);
    },
    runNext() {
      const entry = timers.entries().next().value;
      if (!entry) throw new Error('no timer scheduled');
      timers.delete(entry[0]);
      entry[1].callback();
      return entry[1].delay;
    },
    get delays() {
      return [...timers.values()].map(timer => timer.delay);
    },
    get size() {
      return timers.size;
    },
  };
}

/** The `listen` seam: records handlers, exposes `emit`, and unlistens by count. */
function fakeListener() {
  const handlers = new Set();
  const calls = { listen: 0, unlisten: 0 };
  const listenJson = handler => {
    calls.listen += 1;
    handlers.add(handler);
    return Promise.resolve(() => {
      calls.unlisten += 1;
      handlers.delete(handler);
    });
  };
  return {
    calls,
    listenJson,
    emit(frame) {
      const json = typeof frame === 'string' ? frame : JSON.stringify(frame);
      for (const handler of [...handlers]) handler(json);
    },
  };
}

/** An `invoke` that answers `cad_status` with the given payload and counts calls. */
function fakeInvoke(answer = status()) {
  const calls = [];
  const invoke = async (command) => {
    calls.push(command);
    if (command === 'cad_status') return answer;
    throw new Error(`unexpected invoke ${command}`);
  };
  return { calls, invoke };
}

test('the frame union parses every modelled type and ignores an unknown one', () => {
  const base = { revision: 3 };
  assert.equal(parseCadEventFrame({ ...base, type: 'command-admitted', command: 'LINE' }).type, 'command-admitted');
  assert.deepEqual(
    parseCadEventFrame({ ...base, type: 'command-rejected', command: 'ZOOM', code: 'zoom.argument_required' }),
    { revision: 3, type: 'command-rejected', command: 'ZOOM', code: 'zoom.argument_required' },
  );
  assert.deepEqual(
    parseCadEventFrame({ ...base, type: 'selection-changed', count: 2, empty: false }),
    { revision: 3, type: 'selection-changed', count: 2, empty: false },
  );
  assert.deepEqual(
    parseCadEventFrame({ ...base, type: 'tool-step', tool: 'line', step: 2 }),
    { revision: 3, type: 'tool-step', tool: 'line', step: 2 },
  );
  assert.deepEqual(
    parseCadEventFrame({ ...base, type: 'document-opened', label: 'plan.dwg' }),
    { revision: 3, type: 'document-opened', label: 'plan.dwg' },
  );
  assert.deepEqual(parseCadEventFrame({ ...base, type: 'document-closed' }), { revision: 3, type: 'document-closed' });
  assert.deepEqual(
    parseCadEventFrame({ ...base, type: 'error-raised', error: 'host text' }),
    { revision: 3, type: 'error-raised', error: 'host text' },
  );
  assert.deepEqual(parseCadEventFrame({ ...base, type: 'popup-changed', popup: null }), { revision: 3, type: 'popup-changed', popup: null });
  assert.deepEqual(
    parseCadEventFrame({ ...base, type: 'events-dropped', count: 3 }),
    { revision: 3, type: 'events-dropped', count: 3 },
  );
  // A newer host may add a frame: the type is ignored, and this build keeps going.
  assert.equal(parseCadEventFrame({ revision: 4, type: 'layer-changed', layer: '0' }), null);
  // Malformed known frames are loud, never coerced into a plausible value.
  for (const bad of [
    null, [], 'nope', { revision: -1, type: 'document-closed' },
    { revision: 1, type: 'tool-step', tool: 'line', step: -2 },
    // The producer never emits a nameless refusal or a missing/empty code;
    // those shapes must stay loud instead of tearing down the status channel.
    { revision: 1, type: 'command-rejected', command: 'ZOOM' },
    { revision: 1, type: 'command-rejected', command: 'ZOOM', code: null },
    { revision: 1, type: 'command-rejected', command: 'ZOOM', code: '' },
    { revision: 1, type: 'command-rejected', command: '', code: 'zoom.argument_required' },
    { revision: 1, type: 'command-admitted', command: '' },
    { revision: 1, type: 'status-changed', status: {} },
  ]) {
    assert.throws(() => parseCadEventFrame(bad), /CAD_EVENT_INVALID|CAD_STATUS_INVALID/);
  }
});

test('subscribe takes exactly one initial cad_status snapshot and schedules no timer', async () => {
  const clock = fakeClock();
  const listener = fakeListener();
  const inv = fakeInvoke(status({ documentLabel: 'plan.dwg', frameCount: 42 }));
  const source = createTauriEventSource(inv.invoke, listener.listenJson, { clock });
  const frames = [];
  const errors = [];
  const subscription = source.subscribe(frame => frames.push(frame), error => errors.push(error));
  await settle();

  assert.deepEqual(inv.calls, ['cad_status'], 'one snapshot request, not a poll');
  assert.equal(frames.length, 1);
  assert.equal(frames[0].type, 'status-changed');
  assert.equal(frames[0].revision, 0);
  assert.equal(frames[0].status.frameCount, 42, 'the FPS sampler reads the snapshot');
  assert.equal(clock.size, 0, 'no steady-state timer');
  assert.deepEqual(errors, []);

  listener.emit({ revision: 1, type: 'command-admitted', command: 'LINE' });
  assert.equal(frames.at(-1).type, 'command-admitted');
  assert.equal(subscription.revision, 1);
  subscription.close();
  assert.equal(listener.calls.unlisten, 1);
  assert.equal(clock.size, 0);
});

test('an event before the initial snapshot leaves the cursor contiguous', async () => {
  const clock = fakeClock();
  const listener = fakeListener();
  const calls = [];
  let resolveStatus;
  const pending = new Promise(resolve => { resolveStatus = resolve; });
  const invoke = async command => {
    calls.push(command);
    return pending;
  };
  const source = createTauriEventSource(invoke, listener.listenJson, { clock });
  const frames = [];
  const subscription = source.subscribe(frame => frames.push(frame), () => {});
  await settle();
  assert.deepEqual(calls, ['cad_status'], 'the initial snapshot is in flight');

  // `listen` is attached before `cad_status` resolves: a live event lands first.
  listener.emit({ revision: 1, type: 'command-admitted', command: 'LINE' });
  assert.equal(frames.at(-1).revision, 1);

  resolveStatus(status({ frameCount: 11 }));
  await settle();
  assert.equal(frames.at(-1).type, 'status-changed');
  assert.equal(frames.at(-1).revision, 1, 'the snapshot seeds from the live cursor, not 0');

  listener.emit({ revision: 2, type: 'document-closed' });
  await settle();
  assert.deepEqual(calls, ['cad_status'], 'a contiguous event never asks for another snapshot');
  assert.equal(subscription.revision, 2);
  subscription.close();
});

test('a revision gap asks for one status snapshot and then continues', async () => {
  const clock = fakeClock();
  const listener = fakeListener();
  const inv = fakeInvoke();
  const source = createTauriEventSource(inv.invoke, listener.listenJson, { clock });
  const frames = [];
  source.subscribe(frame => frames.push(frame), () => {});
  await settle();
  assert.equal(inv.calls.length, 1);

  listener.emit({ revision: 5, type: 'selection-changed', count: 2, empty: false });
  await settle();
  assert.equal(inv.calls.length, 2, 'exactly one recovery request');
  assert.equal(frames.filter(frame => frame.type === 'status-changed').length, 2);
  // The gap event itself is still delivered; recovery never swallows events.
  assert.equal(frames.at(-1).type, 'status-changed', 'the recovery snapshot follows the gap event');
});

test('a burst of gaps coalesces into one requestStatus', async () => {
  const clock = fakeClock();
  const listener = fakeListener();
  const inv = fakeInvoke();
  const source = createTauriEventSource(inv.invoke, listener.listenJson, { clock });
  source.subscribe(() => {}, () => {});
  await settle();
  assert.equal(inv.calls.length, 1);

  listener.emit({ revision: 5, type: 'document-closed' });
  listener.emit({ revision: 9, type: 'document-closed' });
  listener.emit({ revision: 12, type: 'document-closed' });
  await settle();
  assert.equal(inv.calls.length, 2, 'one request while the first is still in flight');
});

test('an unmodelled event advances the revision without a gap request or a frame', async () => {
  const clock = fakeClock();
  const listener = fakeListener();
  const inv = fakeInvoke();
  const source = createTauriEventSource(inv.invoke, listener.listenJson, { clock });
  const frames = [];
  const subscription = source.subscribe(frame => frames.push(frame), () => {});
  await settle();
  assert.equal(inv.calls.length, 1);

  listener.emit({ revision: 1, type: 'layer-changed', layer: '0' });
  assert.equal(subscription.revision, 1);
  listener.emit({ revision: 2, type: 'tool-step', tool: 'line', step: 1 });
  assert.equal(inv.calls.length, 1, 'contiguous after the ignored frame: no gap');
  assert.equal(frames.at(-1).type, 'tool-step');
});

test('a transport failure reports once and resubscribes with the documented backoff', async () => {
  const clock = fakeClock();
  const inv = fakeInvoke();
  let attempts = 0;
  const listenJson = () => {
    attempts += 1;
    if (attempts === 1) return Promise.reject(new Error('ipc down'));
    if (attempts === 2) return Promise.reject(new Error('still down'));
    return Promise.resolve(() => {});
  };
  const source = createTauriEventSource(inv.invoke, listenJson, { clock });
  const errors = [];
  source.subscribe(() => {}, error => errors.push(error));
  await settle();

  assert.equal(errors.length, 1);
  assert.match(errors[0], /ipc down/);
  assert.deepEqual(clock.delays, [1000], '500 ms * 2^1');

  clock.runNext();
  await settle();
  assert.equal(attempts, 2);
  assert.deepEqual(clock.delays, [2000], '500 ms * 2^2');

  clock.runNext();
  await settle();
  assert.equal(attempts, 3);
  assert.equal(clock.size, 0, 'a successful resubscribe clears the retry timer');
  assert.equal(errors.length, 2, 'each failure is reported, never swallowed');

  // The cap is the documented one, not an ever-growing delay.
  assert.equal(cadEventBackoff(9), CAD_EVENT_BACKOFF_CAP_MS);
  assert.equal(CAD_EVENT_BACKOFF_CAP_MS, 4000);
});

test('a malformed event is a transport error (close + backoff), never a silent skip', async () => {
  const clock = fakeClock();
  const listener = fakeListener();
  const inv = fakeInvoke();
  const source = createTauriEventSource(inv.invoke, listener.listenJson, { clock });
  const errors = [];
  const frames = [];
  source.subscribe(frame => frames.push(frame), error => errors.push(error));
  await settle();
  assert.equal(errors.length, 0);

  listener.emit('{"revision":2,"type":"status-changed","status":');
  assert.equal(errors.length, 1);
  assert.match(errors[0], /CAD_EVENT_INVALID/);
  assert.equal(listener.calls.unlisten, 1, 'the broken listener is torn down');
  assert.deepEqual(clock.delays, [1000]);
  assert.equal(frames.length, 1, 'no fabricated frame from the broken payload');
});

test('status frames never pump; every semantic frame does', () => {
  assert.equal(cadEventPumpsFrames('status-changed'), false);
  assert.equal(cadEventPumpsFrames('events-dropped'), false);
  for (const type of [
    'command-admitted', 'command-rejected', 'selection-changed', 'tool-step',
    'document-opened', 'document-closed', 'error-raised', 'popup-changed',
  ]) {
    assert.equal(cadEventPumpsFrames(type), true, `${type} must pump`);
  }
});

test('the wasm source gets its initial frame from cad_subscribe and status from shell_status', async () => {
  const clock = fakeClock();
  const frames = [];
  let unsubscribe = 0;
  let callback = null;
  const module = {
    cad_subscribe(handler) {
      callback = handler;
      return JSON.stringify({ revision: 4, type: 'status-changed', status: status({ frameCount: 7 }) });
    },
    cad_unsubscribe() { unsubscribe += 1; },
    shell_status() { return JSON.stringify(status({ frameCount: 9 })); },
  };
  const source = createWasmEventSource(module, { clock });
  const subscription = source.subscribe(frame => frames.push(frame), () => {});
  await settle();

  assert.equal(frames.length, 1);
  assert.equal(frames[0].revision, 4, 'the initial frame owns the baseline revision');
  assert.equal(frames[0].status.frameCount, 7);
  assert.equal(clock.size, 0);

  callback(JSON.stringify({ revision: 5, type: 'tool-step', tool: 'line', step: 2 }));
  assert.equal(frames.at(-1).type, 'tool-step');

  await subscription.requestStatus();
  assert.equal(frames.at(-1).type, 'status-changed');
  assert.equal(frames.at(-1).status.frameCount, 9);
  assert.equal(frames.at(-1).revision, 5, 'a recovery snapshot does not invent a revision');

  subscription.close();
  assert.equal(unsubscribe, 1);
});

test('the wasm transport lifts canOpen on the initial frame, every pushed frame and the recovery read', async () => {
  const clock = fakeClock();
  const frames = [];
  let callback = null;
  let unsubscribe = 0;
  // The session's honest answer: it has no DOM, no picker and no file bytes.
  const session = (overrides = {}) => status({ canOpen: false, canNew: false, canSave: false, ...overrides });
  const module = {
    cad_subscribe(handler) {
      callback = handler;
      return JSON.stringify({ revision: 2, type: 'status-changed', status: session() });
    },
    cad_unsubscribe() { unsubscribe += 1; },
    shell_status() { return JSON.stringify(session({ frameCount: 11 })); },
  };
  const source = createWasmEventSource(module, { clock });
  const subscription = source.subscribe(frame => frames.push(frame), () => {});
  await settle();

  // The initial frame the session hands back through `cad_subscribe`.
  assert.equal(frames.length, 1);
  assert.equal(frames[0].type, 'status-changed');
  assert.equal(frames[0].revision, 2);
  assert.equal(frames[0].status.canOpen, true, 'the page owns the DOM file picker');
  assert.equal(frames[0].status.canSave, false, 'canSave stays the session\'s answer');
  assert.equal(frames[0].status.canNew, false, 'canNew stays the session\'s answer');
  assert.equal(frames[0].status.canPlot, false);
  assert.equal(frames[0].status.canFit, false);

  // A live pushed frame is a fresh session status; it must not reset the lift.
  callback(JSON.stringify({ revision: 3, type: 'status-changed', status: session({ frameCount: 5 }) }));
  assert.equal(frames.at(-1).status.frameCount, 5);
  assert.equal(frames.at(-1).status.canOpen, true, 'a pushed frame keeps the transport capability');
  assert.equal(frames.at(-1).status.canSave, false);
  assert.equal(frames.at(-1).status.canNew, false);

  // The `shell_status` recovery/refresh read goes through the same overlay.
  await subscription.requestStatus();
  assert.equal(frames.at(-1).type, 'status-changed');
  assert.equal(frames.at(-1).status.frameCount, 11);
  assert.equal(frames.at(-1).status.canOpen, true, 'the recovery read carries the overlay too');
  assert.equal(frames.at(-1).status.canSave, false);
  assert.equal(frames.at(-1).status.canNew, false);

  subscription.close();
  assert.equal(unsubscribe, 1);
});

test('the native transport relays canOpen exactly as the host reports it', async () => {
  const clock = fakeClock();
  const listener = fakeListener();
  const inv = fakeInvoke(status({ canOpen: false, canNew: false, canSave: false }));
  const source = createTauriEventSource(inv.invoke, listener.listenJson, { clock });
  const frames = [];
  source.subscribe(frame => frames.push(frame), () => {});
  await settle();

  // A native host owns its own open capability: nothing is lifted on this route.
  assert.equal(frames.length, 1);
  assert.equal(frames[0].status.canOpen, false);
  assert.equal(frames[0].status.canSave, false);

  // A pushed native frame is relayed verbatim as well.
  listener.emit(JSON.stringify({ revision: 1, type: 'status-changed', status: status({ canOpen: false, canSave: true }) }));
  assert.equal(frames.at(-1).status.canOpen, false);
  assert.equal(frames.at(-1).status.canSave, true);

  assert.equal(clock.size, 0, 'a healthy native transport schedules no timer');
});

test('a wasm module without cad_subscribe fails loudly instead of faking events', async () => {
  const clock = fakeClock();
  const source = createWasmEventSource({ shell_status() { return JSON.stringify(status()); } }, { clock });
  const errors = [];
  source.subscribe(() => {}, error => errors.push(error));
  await settle();
  assert.equal(errors.length, 1);
  assert.match(errors[0], /cad_subscribe/);
  assert.deepEqual(clock.delays, [1000]);
});

test('the carrier flip closes the native subscription and re-subscribes through the wasm adapter', async () => {
  // React hooks cannot be driven from Node, so this half is a source contract:
  // the effect observes the carrier from the status frame, then reconnects
  // through the one routing seam (`host/invoke.ts`).
  const hook = await readFile(new URL('../src/cad/use-cad-events.ts', import.meta.url), 'utf8');
  const seam = await readFile(new URL('../src/host/invoke.ts', import.meta.url), 'utf8');

  assert.match(hook, /observeCarrier\(reportedSurfaceMode\(\{ status: frame\.status \}\)\)/);
  const observed = hook.indexOf('observeCarrier(reportedSurfaceMode');
  const connected = hook.indexOf('connect();', observed);
  assert.ok(connected > observed, 'the route is re-read after the carrier observation');
  assert.ok(hook.includes('subscription?.close();'), 'a route change closes the old transport');
  assert.ok(hook.includes('cadEventSource(route).subscribe(onEvent, onError)'), 'the new transport is the seam\'s choice');
  assert.ok(hook.includes('if (route === transport && subscription !== null) return;'), 'an unchanged route never churns');

  // The seam maps a non-Tauri route to the wasm adapter, and a Tauri route to
  // the event listener - the same table, not a second one.
  assert.match(seam, /return invokeRoute\(RUNTIME, 'cad_status', isTauri\(\), carrierNow\(\)\);/);
  assert.match(seam, /route === 'tauri'[\s\S]*createTauriEventSource\(invoke, listenCadEvents\)[\s\S]*createWasmEventSource\(\(\) => loadWebShellModule\(\)\)/);
});
