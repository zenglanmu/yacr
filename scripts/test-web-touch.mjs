import assert from "node:assert/strict";
import test from "node:test";
import {
  installTouchNavigation,
  sampleTouches,
} from "../apps/app-web/web/host/touch.js";

test("pinch centroid and distance use CSS pixels", () => {
  assert.deepEqual(
    sampleTouches([
      { clientX: 10, clientY: 20 },
      { clientX: 30, clientY: 20 },
    ]),
    { x: 20, y: 20, distance: 20, count: 2 },
  );
  assert.equal(sampleTouches([]), null);
});

/// Install the touch router over a fake canvas and return a `fire` helper plus
/// the captured navigation/pick calls. Each `fire` returns whether the event was
/// consumed by the CAD router.
function harness() {
  const listeners = new Map();
  const canvas = {
    addEventListener: (kind, handler) => listeners.set(kind, handler),
    getBoundingClientRect: () => ({ left: 0, top: 0 }),
  };
  globalThis.document = { getElementById: () => canvas };
  const calls = [];
  const picks = [];
  installTouchNavigation({
    shell_geometry: () => [0, 50, 300, 400],
    touch_navigate: (...args) => calls.push(args),
    touch_pick: (...args) => picks.push(args),
  });
  const fire = (kind, points) => {
    let consumed = false;
    listeners.get(kind)({
      touches: points.map(([clientX, clientY]) => ({ clientX, clientY })),
      preventDefault: () => {
        consumed = true;
      },
      stopImmediatePropagation() {},
    });
    return consumed;
  };
  return { fire, calls, picks };
}

test("touch routing rebases finger transitions, cancels, and leaves shell controls alone", (t) => {
  const old = globalThis.document;
  t.after(() => {
    globalThis.document = old;
  });
  const { fire, calls, picks } = harness();
  assert.equal(
    fire("touchstart", [[20, 20]]),
    false,
    "ribbon input belongs to Slint",
  );
  fire("touchstart", [
    [100, 200],
    [200, 200],
  ]);
  fire("touchmove", [
    [50, 200],
    [250, 200],
  ]);
  assert.deepEqual(calls.pop(), [0, 0, 2]);
  fire("touchend", [[50, 200]]);
  fire("touchmove", [[60, 210]]);
  assert.deepEqual(calls.pop(), [10, 10, 1]);
  fire("touchcancel", []);
  fire("touchmove", [[150, 250]]);
  assert.equal(calls.length, 0);
  assert.equal(picks.length, 0, "pinch/drag/cancel never produce a pick");
  fire("touchstart", [[100, 200]]);
  fire("touchend", []);
  assert.deepEqual(picks, [[100, 150]], "tap uses canvas-local coordinates");
});

test("a second finger landing does not emit the centroid jump", (t) => {
  const old = globalThis.document;
  t.after(() => {
    globalThis.document = old;
  });
  const { fire, calls } = harness();
  // One-finger pan.
  fire("touchstart", [[100, 200]]);
  fire("touchmove", [[110, 210]]);
  assert.deepEqual(calls.pop(), [10, 10, 1]);

  // The second finger lands far to the right: the two-finger centroid is at
  // (305, 210), a jump of ~195px from the one-finger sample. The touchstart
  // must re-establish the baseline and emit nothing.
  fire("touchstart", [
    [110, 210],
    [500, 210],
  ]);
  assert.equal(calls.length, 0, "landing the second finger emits no delta");

  // The next small move is measured from the *new* two-finger baseline, so the
  // emitted delta is the move, not the 195px centroid jump. Moving both fingers
  // together keeps the pinch distance constant (zoom 1).
  fire("touchmove", [
    [120, 220],
    [510, 220],
  ]);
  assert.deepEqual(calls.pop(), [10, 10, 1], "delta is relative to the new centroid");
});

test("finger-count change then move is relative to the new baseline", (t) => {
  const old = globalThis.document;
  t.after(() => {
    globalThis.document = old;
  });
  const { fire, calls } = harness();
  // Two-finger pinch: both fingers spread symmetrically so the centroid holds.
  fire("touchstart", [
    [100, 200],
    [200, 200],
  ]);
  fire("touchmove", [
    [50, 200],
    [250, 200],
  ]);
  assert.deepEqual(calls.pop(), [0, 0, 2], "fingers 100px -> 200px apart");

  // One finger lifts: the baseline becomes the remaining finger at (50,200).
  fire("touchend", [[50, 200]]);
  assert.equal(calls.length, 0, "lifting a finger emits no delta");
  // The move after the count change is relative to that new one-finger
  // baseline (no zoom while a single finger is down).
  fire("touchmove", [[60, 220]]);
  assert.deepEqual(calls.pop(), [10, 20, 1]);
});

test("a third finger does not move the established baseline", (t) => {
  const old = globalThis.document;
  t.after(() => {
    globalThis.document = old;
  });
  const { fire, calls } = harness();
  fire("touchstart", [
    [100, 200],
    [200, 200],
  ]);
  fire("touchmove", [
    [100, 200],
    [300, 200],
  ]);
  calls.length = 0;

  // A third finger lands; `sampleTouches` caps at two, so the finger count is
  // unchanged and the baseline must not be re-taken.
  fire("touchstart", [
    [100, 200],
    [300, 200],
    [900, 900],
  ]);
  assert.equal(calls.length, 0, "an unchanged finger count never navigates");
  // The first two fingers move together by 10px: the delta follows them.
  fire("touchmove", [
    [110, 200],
    [310, 200],
    [900, 900],
  ]);
  assert.deepEqual(calls.pop(), [10, 0, 1]);
});

test("a single-finger tap picks, a drag does not", (t) => {
  const old = globalThis.document;
  t.after(() => {
    globalThis.document = old;
  });
  const { fire, picks } = harness();
  // A drag beyond the movement threshold is navigation, never a pick.
  fire("touchstart", [[100, 200]]);
  fire("touchmove", [[160, 200]]);
  fire("touchend", []);
  assert.equal(picks.length, 0, "a drag never picks");

  // A tap in place still reaches the existing pick channel.
  fire("touchstart", [[120, 240]]);
  fire("touchend", []);
  assert.deepEqual(picks, [[120, 190]]);
});
