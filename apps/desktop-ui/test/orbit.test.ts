// Contract tests for the orbit gesture: the host-mirroring delta, the admission
// rules (`hostProfile.orbit` + a 3D drawing), the typed `cad_orbit` request, and
// the per-frame coalescer with its backpressure and stop-on-refusal rules.
//
// `{accepted:true}` is admission only. Nothing here asserts that a camera moved:
// the tests assert exactly which requests were sent, and that a refusal stops the
// gesture instead of retrying once per frame.
import test from 'node:test';
import assert from 'node:assert/strict';
import { NO_DOCUMENT_KEY } from '../src/commands/commands.ts';
import {
  CAD_ORBIT_NOT_ACCEPTED,
  CAD_ORBIT_NO_RESULT,
  CAD_ORBIT_NOT_FINITE,
  ORBIT_NEEDS_3D_KEY,
  ORBIT_RADIANS_PER_PIXEL,
  ORBIT_UNAVAILABLE_KEY,
  OrbitDrag,
  orbitAvailable,
  orbitFailure,
  orbitDelta,
  orbitUnavailableReasonKey,
  requestOrbit,
  submitOrbit,
} from '../src/cad/orbit.ts';
import { createTranslator } from '../src/i18n.ts';
import { parseOrbitCapability } from '../src/cad/host-profile.ts';
import { readCatalog, readModule } from './helpers.ts';

const t = createTranslator({ 'zh-CN': await readCatalog('zh-CN'), en: await readCatalog('en') }, 'en');
const loaded = { loading: false, view: { mode: '3d' }, hostProfile: {} };
const withView = view => ({ ...loaded, view });
const withProfile = profile => ({ ...loaded, hostProfile: profile });

/** A manual animation-frame queue: tests decide when a frame runs. */
function manualFrames() {
  let next = 1;
  const tasks = new Map();
  return {
    schedule(callback) {
      const id = next;
      next += 1;
      tasks.set(id, callback);
      return id;
    },
    cancel(id) {
      tasks.delete(id);
    },
    runFrame() {
      const due = [...tasks.values()];
      tasks.clear();
      for (const callback of due) callback();
    },
    get pending() {
      return tasks.size;
    },
  };
}

/** Let the coalescer's promise callbacks run. */
const settle = () => new Promise(resolve => setImmediate(resolve));

test('the drag delta mirrors cad-host: right yaws negative, down pitches positive', () => {
  assert.equal(ORBIT_RADIANS_PER_PIXEL, 0.008);
  const delta = orbitDelta([10, 10], [20, 22]);
  assert.equal(delta.yaw, -10 * ORBIT_RADIANS_PER_PIXEL);
  assert.equal(delta.pitch, 12 * ORBIT_RADIANS_PER_PIXEL);
  // A pure horizontal drag never pitches, and a pure vertical drag never yaws.
  assert.deepEqual(orbitDelta([5, 5], [15, 5]), { yaw: -0.08, pitch: 0 });
  assert.deepEqual(orbitDelta([5, 5], [5, 15]), { yaw: 0, pitch: 0.08 });
  // Non-finite input poisons nothing.
  assert.deepEqual(orbitDelta([0, 0], [Number.NaN, 10]), { yaw: 0, pitch: 0 });
  assert.deepEqual(orbitDelta([0, 0], [10, Infinity]), { yaw: 0, pitch: 0 });
  assert.deepEqual(orbitDelta([0, 0], [10, 10]), orbitDelta([100, 100], [110, 110]), 'only the difference matters');
});

test('orbit needs a loaded 3D drawing and a carrier that did not refuse it', () => {
  assert.equal(orbitAvailable({ status: loaded, documentOpen: true }), true);
  // Before the first status, during an import, and with no drawing: no gesture.
  assert.equal(orbitAvailable({ status: null, documentOpen: true }), false);
  assert.equal(orbitAvailable({ status: { ...loaded, loading: true }, documentOpen: true }), false);
  assert.equal(orbitAvailable({ status: loaded, documentOpen: false }), false);
  // A 2D view cannot orbit, and the carrier's own `false` is authoritative.
  assert.equal(orbitAvailable({ status: withView({ mode: '2d' }), documentOpen: true }), false);
  assert.equal(orbitAvailable({ status: withProfile({ orbit: false }), documentOpen: true }), false);
  // `true` is an explicit yes and `null` is "undeclared" (the desktop rule).
  assert.equal(orbitAvailable({ status: withProfile({ orbit: true }), documentOpen: true }), true);
  assert.equal(orbitAvailable({ status: withProfile({ orbit: null }), documentOpen: true }), true);
  // The live view wins over the last full status snapshot (a 2D/3D switch is
  // published through the volatile store before the next status arrives).
  assert.equal(orbitAvailable({ status: loaded, documentOpen: true, view: { mode: '2d' } }), false);
  assert.equal(orbitAvailable({ status: withView({ mode: '2d' }), documentOpen: true, view: { mode: '3d' } }), true);
});

test('each refusal has its own sentence, and both catalogs word it', async () => {
  assert.equal(orbitUnavailableReasonKey({ status: null }), 'command.unsupported');
  assert.equal(orbitUnavailableReasonKey({ status: loaded, documentOpen: false }), NO_DOCUMENT_KEY);
  assert.equal(
    orbitUnavailableReasonKey({ status: withProfile({ orbit: false }), documentOpen: true }),
    ORBIT_UNAVAILABLE_KEY,
    'a carrier limitation is not "no drawing is open"',
  );
  assert.equal(
    orbitUnavailableReasonKey({ status: withView({ mode: '2d' }), documentOpen: true }),
    ORBIT_NEEDS_3D_KEY,
  );
  assert.equal(orbitUnavailableReasonKey({ status: loaded, documentOpen: true }), 'command.unsupported');
  // The carrier gate is consulted before the document state, exactly like the
  // standard-view helper: a `dom-canvas` carrier is refused for its own reason
  // even on the start page.
  assert.equal(
    orbitUnavailableReasonKey({ status: withProfile({ orbit: false }), documentOpen: false }),
    ORBIT_UNAVAILABLE_KEY,
  );
  assert.notEqual(ORBIT_UNAVAILABLE_KEY, ORBIT_NEEDS_3D_KEY);
  assert.notEqual(ORBIT_UNAVAILABLE_KEY, NO_DOCUMENT_KEY);
  for (const locale of ['zh-CN', 'en']) {
    const catalog = await readCatalog(locale);
    for (const key of [ORBIT_UNAVAILABLE_KEY, ORBIT_NEEDS_3D_KEY]) {
      assert.equal(typeof catalog[key], 'string', `${locale} is missing ${key}`);
    }
  }
});

test('one cad_orbit request reports admission only', async () => {
  const calls = [];
  const accepted = await requestOrbit(async (command, args) => {
    calls.push([command, args]);
    return { accepted: true };
  }, 0.25, -0.5);
  assert.deepEqual(accepted, { kind: 'accepted' });
  assert.deepEqual(calls, [['cad_orbit', { yaw: 0.25, pitch: -0.5 }]], 'the typed channel and its radians');

  // A non-finite angle never reaches the transport.
  let invoked = false;
  const refused = await requestOrbit(async () => { invoked = true; return { accepted: true }; }, Number.NaN, 0);
  assert.equal(refused.kind, 'refused');
  assert.equal(refused.error, CAD_ORBIT_NOT_FINITE);
  assert.equal(invoked, false, 'a non-finite delta is a local programming error, not an IPC');

  // A host that answers without admitting, and a host that rejects, both surface
  // a named refusal (the banner shows it through `command.failed`).
  const notAccepted = await requestOrbit(async () => ({ accepted: false }), 0.1, 0.1);
  assert.equal(notAccepted.kind, 'refused');
  assert.ok(notAccepted.error.includes(CAD_ORBIT_NOT_ACCEPTED), notAccepted.error);
  const thrown = await requestOrbit(async () => { throw Error('view.orbit_2d: not in 3D'); }, 0.1, 0.1);
  assert.equal(thrown.kind, 'refused');
  assert.ok(thrown.error.includes('view.orbit_2d'), thrown.error);
});

test('the coalescer sends one summed request per frame', async () => {
  const frames = manualFrames();
  const sent = [];
  const drag = new OrbitDrag({
    send: async (yaw, pitch) => { sent.push([yaw, pitch]); return { kind: 'accepted' }; },
    schedule: frames.schedule,
    cancel: frames.cancel,
  });
  drag.push(0.01, 0);
  drag.push(0.02, 0.5);
  assert.deepEqual(sent, [], 'nothing is sent before a frame runs');
  assert.equal(frames.pending, 1, 'a burst schedules exactly one frame');
  frames.runFrame();
  assert.deepEqual(sent, [[0.03, 0.5]], 'the frame sends the summed delta once');
  await settle();
  assert.equal(frames.pending, 0, 'an idle coalescer schedules nothing');

  // Zero and non-finite deltas are dropped, not sent.
  drag.push(0, 0);
  drag.push(Number.NaN, 0.2);
  drag.push(0.2, Infinity);
  assert.equal(frames.pending, 0);
  drag.flush();
  assert.equal(sent.length, 1);
});

test('at most one request is in flight; the backlog waits for its answer', async () => {
  const frames = manualFrames();
  const sent = [];
  let resolveSend = () => {};
  const drag = new OrbitDrag({
    send: (yaw, pitch) => {
      sent.push([yaw, pitch]);
      return new Promise(resolve => { resolveSend = resolve; });
    },
    schedule: frames.schedule,
    cancel: frames.cancel,
  });
  drag.push(0.1, 0);
  frames.runFrame();
  assert.equal(sent.length, 1);
  drag.push(0.2, 0);
  drag.push(0.3, 0);
  assert.equal(sent.length, 1, 'a push during a request never queues a second one');
  assert.equal(frames.pending, 0, 'and does not schedule a frame either');
  assert.deepEqual(drag.pending.map(value => Number(value.toFixed(3))), [0.5, 0], 'the backlog is kept');
  resolveSend({ kind: 'accepted' });
  await settle();
  assert.equal(frames.pending, 1, 'the answer schedules the backlog frame');
  frames.runFrame();
  assert.equal(sent.length, 2);
  assert.equal(Number(sent[1][0].toFixed(3)), 0.5, 'the backlog is one summed request');
  resolveSend({ kind: 'accepted' });
  await settle();
  assert.equal(frames.pending, 0);
});

test('a refusal stops the gesture until it ends', async () => {
  const frames = manualFrames();
  const sent = [];
  const drag = new OrbitDrag({
    send: async (yaw, pitch) => { sent.push([yaw, pitch]); return { kind: 'refused', error: 'view.orbit_2d: no' }; },
    schedule: frames.schedule,
    cancel: frames.cancel,
  });
  drag.push(0.1, 0);
  frames.runFrame();
  await settle();
  assert.equal(drag.stopped, true);
  drag.push(0.2, 0);
  assert.equal(frames.pending, 0, 'a refused gesture never retries');
  frames.runFrame();
  drag.flush();
  assert.equal(sent.length, 1, 'the refusal produced exactly one request');
  // A throwing send is the same verdict, not an unhandled rejection.
  const throwing = new OrbitDrag({
    send: () => { throw Error('no bridge'); },
    schedule: frames.schedule,
    cancel: frames.cancel,
  });
  throwing.push(0.1, 0);
  throwing.flush();
  await settle();
  assert.equal(throwing.stopped, true);
});

test('end() cancels the scheduled frame and flushes what is owed', async () => {
  const frames = manualFrames();
  const sent = [];
  const drag = new OrbitDrag({
    send: async (yaw, pitch) => { sent.push([yaw, pitch]); return { kind: 'accepted' }; },
    schedule: frames.schedule,
    cancel: frames.cancel,
  });
  drag.push(0.4, 0);
  assert.equal(frames.pending, 1);
  // `pointerup` flushes the last real movement, then ends the gesture.
  drag.flush();
  assert.equal(Number(sent[0][0].toFixed(1)), 0.4);
  drag.end();
  assert.equal(frames.pending, 0, 'end cancels a still-scheduled frame');
  drag.push(0.5, 0);
  drag.flush();
  assert.equal(sent.length, 1, 'nothing is sent after end()');

  // And `end()` alone cancels without sending.
  const silent = [];
  const other = new OrbitDrag({
    send: async (yaw, pitch) => { silent.push([yaw, pitch]); return { kind: 'accepted' }; },
    schedule: frames.schedule,
    cancel: frames.cancel,
  });
  other.push(0.4, 0);
  other.end();
  assert.equal(frames.pending, 0);
  frames.runFrame();
  assert.deepEqual(silent, []);
});

test('a submission reports only a refusal, never a claim that the camera moved', async () => {
  const reported = [];
  const accepted = await submitOrbit({
    invoke: async () => ({ accepted: true }),
    yaw: 0.1,
    pitch: 0.2,
    t,
    report: message => reported.push(message),
  });
  assert.equal(accepted.kind, 'accepted');
  assert.deepEqual(reported, [], 'admission is not completion: nothing is reported');
  const refused = await submitOrbit({
    invoke: async () => { throw Error('view.orbit_2d: no 3D camera'); },
    yaw: 0.1,
    pitch: 0.2,
    t,
    report: message => reported.push(message),
  });
  assert.equal(refused.kind, 'refused');
  assert.equal(reported.length, 1);
  assert.ok(reported[0].includes('view.orbit_2d'), reported[0]);
  assert.notEqual(reported[0], 'view.orbit_2d: no 3D camera', 'the banner shows catalog copy, not the bare code');
  // A dropped answer is a named refusal, not a silent success.
  assert.equal(orbitFailure({ kind: 'accepted' }, t), '');
  assert.ok(orbitFailure(null, t).includes(CAD_ORBIT_NO_RESULT));
});

test('the hostProfile orbit capability is validated strictly by its own reader', async () => {
  // `null`/absent is "undeclared"; a real boolean is the carrier's answer.
  for (const value of [undefined, null, true, false]) {
    assert.doesNotThrow(() => parseOrbitCapability(value), String(value));
  }
  // A truthy non-boolean would enable a gesture the carrier refused, so it is a
  // contract violation, never a coercion.
  for (const value of ['false', 'true', 0, 1, {}, []]) {
    assert.throws(() => parseOrbitCapability(value), /CAD_STATUS_INVALID/, JSON.stringify(value));
  }
  // The status parser delegates to that reader instead of carrying a second copy.
  const status = await readModule('cad/cad-status.ts');
  assert.ok(status.includes('parseOrbitCapability(profile.orbit)'), 'the parser calls the extracted rule');
  assert.equal(status.includes("typeof profile.orbit !== 'boolean'"), false, 'no duplicate orbit rule in the parser');
});
