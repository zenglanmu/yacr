import assert from "node:assert/strict";
import test from "node:test";
import {
  installTouchNavigation,
  sampleTouches,
  interactionAllowsTouch,
  readInteractionAllowsTouch,
} from "../apps/app-web/web/host/touch.js";
import { CONFIG_EVENT_NAME } from "../apps/app-web/web/host/config.js";

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
function harness(canvasHitTest) {
  const listeners = new Map();
  const canvas = {
    addEventListener: (kind, handler) => listeners.set(kind, handler),
    getBoundingClientRect: () => ({ left: 0, top: 0 }),
  };
  globalThis.document = { getElementById: () => canvas };
  const calls = [];
  const picks = [];
  const cancellations = [];
  installTouchNavigation({
    canvas_hit_test: canvasHitTest,
    shell_geometry: () => [0, 50, 300, 400],
    touch_navigate: (...args) => calls.push(args),
    touch_pick: (...args) => picks.push(args),
    touch_cancel_draw: () => cancellations.push(true),
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
  return { fire, calls, picks, cancellations };
}

test("floating shell controls are excluded by the authoritative Slint hit query", (t) => {
  const old = globalThis.document;
  t.after(() => { globalThis.document = old; });
  const { fire, calls, picks } = harness((x, y) => x < 220 && y >= 50 && y < 450);
  assert.equal(fire("touchstart", [[250, 80]]), false, "floating fit remains Slint input");
  assert.equal(fire("touchstart", [[120, 200]]), true);
  fire("touchend", []);
  assert.deepEqual(picks, [[120, 150]]);
  assert.equal(calls.length, 0);
});

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
  assert.deepEqual(
    calls.pop(),
    [10, 10, 1],
    "delta is relative to the new centroid",
  );
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

test("second-finger landing and touchcancel abandon drawing capture without a pick", (t) => {
  const old = globalThis.document;
  t.after(() => {
    globalThis.document = old;
  });
  const { fire, calls, picks, cancellations } = harness();
  fire("touchstart", [[100, 200]]);
  assert.equal(cancellations.length, 0);
  fire("touchstart", [
    [100, 200],
    [200, 200],
  ]);
  assert.equal(
    cancellations.length,
    1,
    "cancel immediately, even without movement",
  );
  assert.deepEqual(calls, []);
  fire("touchend", []);
  assert.deepEqual(picks, []);
  fire("touchstart", [[100, 200]]);
  fire("touchcancel", []);
  assert.equal(cancellations.length, 2);
  assert.deepEqual(picks, []);
});

test("the interaction predicate is enabled unless touch is explicitly false", () => {
  assert.equal(
    interactionAllowsTouch(
      '{"pointer":true,"touch":false,"keyboardShortcuts":true}',
    ),
    false,
  );
  assert.equal(interactionAllowsTouch('{"touch":true}'), true);
  assert.equal(interactionAllowsTouch('{"pointer":false}'), true);
  // Absent/null/malformed input follows the Rust `touch_enabled()`
  // `unwrap_or(true)` default: enabled, so the wasm backstop still decides.
  assert.equal(interactionAllowsTouch(undefined), true);
  assert.equal(interactionAllowsTouch(""), true);
  assert.equal(interactionAllowsTouch("null"), true);
  assert.equal(interactionAllowsTouch("not json"), true);
  assert.equal(interactionAllowsTouch("[1,2]"), true);
  // A host that has not started (or lacks the export) is not treated as off.
  assert.equal(readInteractionAllowsTouch({}), true);
  assert.equal(readInteractionAllowsTouch(undefined), true);
  assert.equal(
    readInteractionAllowsTouch({
      viewer_interaction_json: () => '{"touch":false}',
    }),
    false,
  );
});

/// Install the router under a fake canvas + window so the config-changed event
/// can be emitted, and return helpers beside the captured wasm calls.
function gatedHarness(interactionJson) {
  const listeners = new Map();
  const windowListeners = new Map();
  const canvas = {
    addEventListener: (kind, handler) => listeners.set(kind, handler),
    getBoundingClientRect: () => ({ left: 0, top: 0 }),
  };
  globalThis.document = { getElementById: () => canvas };
  globalThis.window = {
    addEventListener: (kind, handler) => windowListeners.set(kind, handler),
    removeEventListener: (kind) => windowListeners.delete(kind),
  };
  const calls = [];
  const picks = [];
  let json = interactionJson;
  installTouchNavigation({
    canvas_hit_test: () => true,
    shell_geometry: () => [0, 50, 300, 400],
    viewer_interaction_json: () => json,
    touch_navigate: (...args) => calls.push(args),
    touch_pick: (...args) => picks.push(args),
    touch_cancel_draw: () => {},
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
  const firePointer = (kind, clientX, clientY) => {
    let consumed = false;
    listeners.get(kind)({
      pointerType: "touch",
      pointerId: 1,
      clientX,
      clientY,
      preventDefault: () => {
        consumed = true;
      },
      stopImmediatePropagation() {},
    });
    return consumed;
  };
  const configure = (next) => {
    json = next;
    windowListeners.get(CONFIG_EVENT_NAME)();
  };
  return { fire, firePointer, calls, picks, configure, windowListeners };
}

test("touch routing is gated by the interaction config and resumes on change", (t) => {
  const oldDocument = globalThis.document;
  const oldWindow = globalThis.window;
  t.after(() => {
    globalThis.document = oldDocument;
    globalThis.window = oldWindow;
  });
  const { fire, firePointer, calls, picks, configure, windowListeners } =
    gatedHarness('{"touch":false}');
  assert.equal(
    typeof windowListeners.get(CONFIG_EVENT_NAME),
    "function",
    "the router subscribes to config changes",
  );
  // Disabled: no pointer capture and no forwarded touch events.
  assert.equal(fire("touchstart", [[100, 200]]), false);
  assert.equal(fire("touchmove", [[110, 210]]), false);
  assert.equal(fire("touchend", []), false);
  assert.equal(firePointer("pointerdown", 100, 200), false);
  assert.equal(calls.length, 0);
  assert.equal(picks.length, 0);
  // Re-enabling on the change event takes effect without a reload.
  configure('{"touch":true}');
  assert.equal(fire("touchstart", [[100, 200]]), true);
  assert.equal(firePointer("pointerdown", 100, 200), true);
  fire("touchend", []);
  assert.deepEqual(picks, [[100, 150]]);
});

test("disabling touch abandons the in-flight baseline and restarts clean", (t) => {
  const oldDocument = globalThis.document;
  const oldWindow = globalThis.window;
  t.after(() => {
    globalThis.document = oldDocument;
    globalThis.window = oldWindow;
  });
  const { fire, calls, picks, configure } = gatedHarness('{"touch":true}');
  fire("touchstart", [[100, 200]]);
  fire("touchmove", [[110, 210]]);
  assert.deepEqual(calls.pop(), [10, 10, 1]);

  // Turning touch off drops the stale origin/baseline: no pick on the end.
  configure('{"touch":false}');
  assert.equal(fire("touchmove", [[500, 500]]), false);
  fire("touchend", []);
  assert.equal(calls.length, 0);
  assert.equal(picks.length, 0);

  // Re-enabling starts a fresh gesture, not a jump from the abandoned point.
  configure('{"touch":true}');
  fire("touchstart", [[120, 240]]);
  fire("touchend", []);
  assert.deepEqual(picks, [[120, 190]]);
});

test("a malformed interaction payload keeps touch enabled through the router", (t) => {
  const oldDocument = globalThis.document;
  const oldWindow = globalThis.window;
  t.after(() => {
    globalThis.document = oldDocument;
    globalThis.window = oldWindow;
  });
  const { fire } = gatedHarness("not json");
  assert.equal(fire("touchstart", [[100, 200]]), true);
});
