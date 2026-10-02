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

test("touch routing rebases finger transitions, cancels, and leaves shell controls alone", (t) => {
  const old = globalThis.document;
  t.after(() => {
    globalThis.document = old;
  });
  const listeners = new Map();
  const canvas = {
    addEventListener: (kind, handler) => listeners.set(kind, handler),
    getBoundingClientRect: () => ({ left: 0, top: 0 }),
  };
  globalThis.document = { getElementById: () => canvas };
  const calls = [],
    picks = [];
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
