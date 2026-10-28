// Contracts for the `dom-canvas` frame pump (`src/cad/frame-loop.ts`).
//
// Scope, stated honestly: a hand-written `requestAnimationFrame` stub and a
// hand-written session stub. There is no browser, no wasm module and no pixel
// evidence here. What is pinned is the *loop rule* the page carrier depends on:
// a kick always buys at least one frame, the chain stops once the session has
// caught up, and one pump can never turn into an unbounded spin.
import test from 'node:test';
import assert from 'node:assert/strict';
import { FRAME_IDLE_STEPS, FRAME_MAX_STEPS, createFrameLoop } from '../src/cad/frame-loop.ts';

/** A deterministic rAF: frames run only when `flush()` is called. */
function stubEnv() {
  const queue = new Map();
  let next = 0;
  return {
    requestAnimationFrame(callback) {
      next += 1;
      queue.set(next, callback);
      return next;
    },
    cancelAnimationFrame(handle) {
      queue.delete(handle);
    },
    /** Run every callback that is scheduled right now (one step per callback). */
    flush() {
      const due = [...queue.entries()];
      queue.clear();
      for (const [, callback] of due) callback(0);
    },
    get pending() {
      return queue.size;
    },
  };
}

/** A frame export answering `presented` for the given script, then `false`. */
function stubModule(script) {
  const seen = [];
  let index = 0;
  return {
    calls: seen,
    frame() {
      const presented = index < script.length ? script[index] : false;
      index += 1;
      seen.push(presented);
      return JSON.stringify({ ok: true, presented, frames: index, waited: 0, ticks: index, canvasFrames: index });
    },
  };
}

test('the documented bounds are the ones the loop uses', () => {
  assert.equal(FRAME_IDLE_STEPS, 2);
  assert.equal(FRAME_MAX_STEPS, 600);
  assert.throws(() => createFrameLoop({ module: null }), TypeError);
  assert.throws(() => createFrameLoop({ module: {} }), TypeError);
  assert.throws(() => createFrameLoop({ module: { frame() {} }, idleSteps: 0 }), TypeError);
  assert.throws(() => createFrameLoop({ module: { frame() {} }, idleSteps: 3, maxSteps: 2 }), TypeError);
});

test('a pump presents frames until the session has caught up, then stops', async () => {
  // Three presented frames (the tool echo, the scene rebuild, the camera), then the
  // renderer has nothing new: two no-present steps end the chain.
  const module = stubModule([true, true, true]);
  const env = stubEnv();
  const lines = [];
  const loop = createFrameLoop({ module, env, onFrame: line => lines.push(line) });

  assert.equal(loop.pump(), true, 'a kick schedules a frame');
  assert.equal(loop.running, true);
  for (let step = 0; step < 20 && loop.running; step += 1) {
    env.flush();
    // Each step schedules the next one; flush them one at a time.
    if (loop.running) assert.equal(env.pending, 1, 'exactly one frame is ever scheduled');
  }
  await loop.whenIdle();
  assert.deepEqual(module.calls, [true, true, true, false, false]);
  assert.equal(loop.running, false);
  assert.equal(env.pending, 0, 'an idle loop schedules nothing');
  // The parsed frame lines reached the caller (diagnostics without a second parse).
  assert.equal(lines.length, 5);
  assert.equal(lines[0].presented, true);
  assert.equal(lines[4].presented, false);
});

test('an idle session presents nothing and costs nothing', () => {
  const module = stubModule([]);
  const env = stubEnv();
  const loop = createFrameLoop({ module, env });
  loop.pump();
  env.flush();
  env.flush();
  assert.equal(module.calls.length, FRAME_IDLE_STEPS, 'the idle bound is exactly the documented one');
  assert.equal(loop.running, false);
  assert.equal(env.pending, 0);
  // And nothing runs afterwards: no timer, no second chain.
  env.flush();
  assert.equal(module.calls.length, FRAME_IDLE_STEPS);
});

test('a kick while a chain runs continues it instead of starting a second one', () => {
  const module = stubModule([true, true]);
  const env = stubEnv();
  const loop = createFrameLoop({ module, env });
  loop.pump();
  assert.equal(env.pending, 1);
  assert.equal(loop.pump(), true, 'already running is still "a frame is scheduled"');
  assert.equal(env.pending, 1, 'one chain, one scheduled frame');
  env.flush();
  // The kick reset the idle counter: the chain keeps going while frames present.
  env.flush();
  env.flush();
  assert.deepEqual(module.calls.slice(0, 3), [true, true, false]);
});

test('a kick cannot clear the spin guard of a chain that is already running', () => {
  // The page's status poll notices every presented frame, so a "kick on change"
  // arrives continuously while a runaway session presents: if that reset the step
  // budget the bound below would never be reached.
  const module = stubModule(Array.from({ length: 40 }, () => true));
  const env = stubEnv();
  const loop = createFrameLoop({ module, env, maxSteps: 4 });
  loop.pump();
  env.flush();
  loop.pump();
  env.flush();
  loop.pump();
  env.flush();
  loop.pump();
  env.flush();
  assert.equal(module.calls.length, 4, 'four steps, then the chain stops');
  assert.equal(loop.running, false);
});

test('one pump is bounded, so an always-presenting session cannot spin forever', () => {
  const module = stubModule(Array.from({ length: 50 }, () => true));
  const env = stubEnv();
  const loop = createFrameLoop({ module, env, maxSteps: 4 });
  loop.pump();
  for (let step = 0; step < 10; step += 1) env.flush();
  assert.equal(module.calls.length, 4, 'the spin guard stops the chain');
  assert.equal(loop.running, false);
  // Nothing is latched: the next kick resumes the work.
  loop.pump();
  env.flush();
  assert.equal(module.calls.length, 5);
});

test('a throwing frame step ends the chain and is remembered, not reported twice', () => {
  const env = stubEnv();
  let calls = 0;
  const loop = createFrameLoop({
    module: {
      frame() {
        calls += 1;
        if (calls >= 2) throw 'no live CAD session on this carrier';
        return '{"ok":true,"presented":true}';
      },
    },
    env,
  });
  loop.pump();
  env.flush();
  assert.equal(loop.running, true);
  env.flush();
  assert.equal(loop.running, false, 'a throw stops the chain');
  assert.equal(loop.lastError, 'no live CAD session on this carrier');
  // A new kick clears the remembered failure and asks again (nothing is latched:
  // the transport owns latching, and a dead session is reported through the
  // session's own status, not twice through this loop).
  loop.pump();
  assert.equal(loop.lastError, null);
  env.flush();
  assert.equal(calls, 3);
});

test('an unreadable frame line is a no-present step, never a presented one', async () => {
  const module = { frame: () => '<html>not a frame line</html>' };
  const env = stubEnv();
  const lines = [];
  const loop = createFrameLoop({ module, env, onFrame: line => lines.push(line) });
  loop.pump();
  for (let step = 0; step < 5; step += 1) env.flush();
  await loop.whenIdle();
  assert.equal(loop.running, false);
  assert.equal(lines.length, FRAME_IDLE_STEPS);
  assert.ok(lines.every(line => line === null), 'an unreadable line is reported as null');
});

test('dispose cancels the scheduled frame and resolves every waiter', async () => {
  const env = stubEnv();
  const module = stubModule([true, true, true]);
  const loop = createFrameLoop({ module, env });
  loop.pump();
  const idle = loop.whenIdle();
  assert.equal(env.pending, 1);
  loop.dispose();
  assert.equal(env.pending, 0, 'the pending frame is cancelled');
  await idle;
  assert.equal(loop.running, false);
  assert.equal(loop.pump(), false, 'a disposed loop never schedules again');
  assert.equal(env.pending, 0);
});

test('whenIdle resolves immediately when nothing is scheduled', async () => {
  const loop = createFrameLoop({ module: stubModule([]), env: stubEnv() });
  await loop.whenIdle();
  assert.equal(loop.running, false);
});
