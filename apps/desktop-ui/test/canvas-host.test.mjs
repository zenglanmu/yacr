// Contracts for the `dom-canvas` transport (ADR 0024 D1-D3).
//
// Scope, stated honestly: everything here is a **pure function** test, a test
// against a hand-written element stub, or a **source-structure** assertion. There
// is no DOM mount, no browser run, no wasm module and no pixel evidence in this
// file; the mode is a data flag and the transport is a logic seam. What is pinned:
// the wheel/pan arithmetic the session receives, the degenerate-input guards, the
// one-shot failure routing, and the structural invariants that keep
// `native-below` and `<canvas>` untouched.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  CANVAS_BOOTSTRAP_FILE,
  CANVAS_EXPORTS,
  CANVAS_GLOBAL,
  CANVAS_STEP_CALL,
  CANVAS_STEP_MODULE,
  CANVAS_STEP_UNCLASSIFIED,
  CANVAS_STEP_UNREADABLE,
  CANVAS_WASM_FILE,
  ZOOM_STEP,
  bindCanvasNavigation,
  canvasBaseDir,
  canvasCssSize,
  canvasModuleUrl,
  canvasScriptUrl,
  canvasWasmUrl,
  createCanvasTransport,
  loadCanvasModule,
  panDelta,
  parseCanvasStatus,
  statusFailure,
  wheelZoomFactor,
  wheelZoomNotches,
} from '../src/canvas-host.mjs';
import { readFile } from 'node:fs/promises';
import { readAppSource, readModule } from './helpers.mjs';

/** The repo-root build entry point (source, not generated output). */
const BUILD_SCRIPT = new URL('../../../scripts/build-canvas-wasm.sh', import.meta.url);

const OK_STATUS = JSON.stringify({
  ok: true,
  backend: 'gl',
  format: 'Rgba8UnormSrgb',
  batches: 1,
  draws: 4,
  presented: 1,
  physical: [320, 200],
});

const WEBGL2_FAILURE = JSON.stringify({
  ok: false,
  step: 'webgl2',
  error: 'no webgl2 context',
  marker: 'yacr: unsupported-runtime: webgl2',
});

/** An element stub: records its own listeners, nothing else exists to record. */
function stubElement(box = { left: 40, top: 10, width: 320, height: 200 }) {
  const listeners = new Map();
  const captured = new Set();
  return {
    listeners,
    captured,
    getBoundingClientRect: () => ({ ...box }),
    addEventListener(type, handler) { listeners.set(type, handler); },
    removeEventListener(type, handler) { if (listeners.get(type) === handler) listeners.delete(type); },
    setPointerCapture(id) { captured.add(id); },
    hasPointerCapture(id) { return captured.has(id); },
    releasePointerCapture(id) { captured.delete(id); },
    dispatch(type, event) {
      const handler = listeners.get(type);
      assert.equal(typeof handler, 'function', `no ${type} listener`);
      handler(event);
    },
  };
}

function pointerEvent(overrides = {}) {
  const event = {
    pointerId: 7,
    button: 0,
    clientX: 0,
    clientY: 0,
    deltaY: 0,
    deltaMode: 0,
    prevented: false,
    preventDefault() { event.prevented = true; },
    ...overrides,
  };
  return event;
}

/** A wasm-session stub whose every export records its arguments. */
function stubModule({ start = async () => OK_STATUS, resize = () => OK_STATUS } = {}) {
  const calls = [];
  return {
    calls,
    start: (...args) => { calls.push(['start', ...args]); return start(); },
    resize: (...args) => { calls.push(['resize', ...args]); return resize(); },
    zoom_at: (...args) => { calls.push(['zoom_at', ...args]); return OK_STATUS; },
    pan: (...args) => { calls.push(['pan', ...args]); return OK_STATUS; },
  };
}

// --- wheel -> zoom ---------------------------------------------------------

test('one wheel notch zooms by exactly one ZOOM_STEP, in the desktop direction', () => {
  assert.equal(ZOOM_STEP, 1.25);
  // Wheel up (negative deltaY) zooms in: factor > 1, anchored by the session.
  assert.equal(wheelZoomFactor(-120, 0), 1.25);
  // Wheel down zooms out.
  assert.equal(wheelZoomFactor(120, 0), 1 / ZOOM_STEP);
  // Two notches compose multiplicatively, not additively.
  assert.equal(wheelZoomFactor(-240, 0), ZOOM_STEP ** 2);
  assert.equal(wheelZoomFactor(240, 0), 1 / (ZOOM_STEP ** 2));
  // A fraction of a notch is a fraction of the zoom: `1.25 ** (60 / 120)`.
  assert.equal(wheelZoomFactor(-60, 0), Math.sqrt(1.25));
});

test('line and page delta modes normalise to the same notch', () => {
  // deltaMode 1 counts lines: three lines are one notch.
  assert.equal(wheelZoomNotches(-3, 1), -1);
  assert.equal(wheelZoomFactor(-3, 1), 1.25);
  // deltaMode 2 counts pages: one page is one notch, whatever its height.
  assert.equal(wheelZoomNotches(-1, 2), -1);
  assert.equal(wheelZoomFactor(-1, 2), 1.25);
  // Pixels (mode 0 and any unknown mode) use the 120 px notch.
  assert.equal(wheelZoomNotches(-120, 0), -1);
  assert.equal(wheelZoomNotches(-120), -1);
  assert.equal(wheelZoomNotches(-120, 9), -1);
});

test('a wheel event with nothing to forward yields no zoom', () => {
  for (const deltaY of [0, -0, NaN, Infinity, -Infinity]) {
    assert.equal(wheelZoomNotches(deltaY, 0), null, `deltaY ${deltaY}`);
    assert.equal(wheelZoomFactor(deltaY, 0), null, `deltaY ${deltaY}`);
  }
  assert.equal(wheelZoomFactor(undefined, 0), null);
  assert.equal(wheelZoomFactor('120', 0), null);
});

// --- drag -> pan -----------------------------------------------------------

test('a drag reports the pointer delta with the pointer sign', () => {
  // Down-right is a positive screen delta (x right, y down).
  assert.deepEqual(panDelta({ x: 10, y: 20 }, { x: 15, y: 12 }), { dx: 5, dy: -8 });
  assert.deepEqual(panDelta({ x: 100, y: 100 }, { x: 90, y: 130 }), { dx: -10, dy: 30 });
  // One CSS pixel of motion is one CSS pixel of delta: no scaling happens here.
  assert.deepEqual(panDelta({ x: 0.5, y: 0.5 }, { x: 1.5, y: 0.5 }), { dx: 1, dy: 0 });
});

test('a degenerate or no-op drag reports nothing', () => {
  assert.equal(panDelta({ x: 10, y: 10 }, { x: 10, y: 10 }), null);
  assert.equal(panDelta(null, { x: 1, y: 1 }), null);
  assert.equal(panDelta({ x: 1, y: 1 }, null), null);
  assert.equal(panDelta({ x: 1, y: 1 }, { x: NaN, y: 1 }), null);
  assert.equal(panDelta({ x: 1, y: 1 }, { x: 1, y: Infinity }), null);
});

// --- rect -> session size --------------------------------------------------

test('a rect becomes the CSS size and DPR the session is configured with', () => {
  assert.deepEqual(canvasCssSize({ width: 320, height: 200, dpr: 2 }), { width: 320, height: 200, dpr: 2 });
  assert.deepEqual(canvasCssSize({ width: 800.5, height: 600.25, dpr: 1.5 }), { width: 800.5, height: 600.25, dpr: 1.5 });
});

test('a degenerate rect is either clamped honestly or refused, never invented', () => {
  // A present but zero-sized element is 1 px, not 0: a surface cannot be zero.
  assert.deepEqual(canvasCssSize({ width: 0, height: 0, dpr: 1 }), { width: 1, height: 1, dpr: 1 });
  assert.deepEqual(canvasCssSize({ width: -5, height: 10, dpr: 1 }), { width: 1, height: 10, dpr: 1 });
  // An absent/non-finite/non-positive DPR falls back to 1 (the reference shell's rule).
  assert.deepEqual(canvasCssSize({ width: 10, height: 10 }), { width: 10, height: 10, dpr: 1 });
  assert.deepEqual(canvasCssSize({ width: 10, height: 10, dpr: 0 }), { width: 10, height: 10, dpr: 1 });
  assert.deepEqual(canvasCssSize({ width: 10, height: 10, dpr: NaN }), { width: 10, height: 10, dpr: 1 });
  // No rect, or an unusable axis: no size at all (and therefore no call).
  assert.equal(canvasCssSize(null), null);
  assert.equal(canvasCssSize(undefined), null);
  assert.equal(canvasCssSize({ width: NaN, height: 10, dpr: 1 }), null);
  assert.equal(canvasCssSize({ width: 10, height: undefined, dpr: 1 }), null);
});

// --- status routing --------------------------------------------------------

test('status parsing keeps only objects and reports unreadable text', () => {
  assert.deepEqual(parseCanvasStatus(OK_STATUS).physical, [320, 200]);
  assert.equal(parseCanvasStatus('not json'), null);
  assert.equal(parseCanvasStatus('[1,2]'), null);
  assert.equal(parseCanvasStatus('null'), null);
  assert.equal(parseCanvasStatus(undefined), null);
});

test('a webgl2 refusal carries its step, detail and stable marker', () => {
  const failure = statusFailure(parseCanvasStatus(WEBGL2_FAILURE), WEBGL2_FAILURE);
  assert.equal(failure.step, 'webgl2');
  assert.equal(failure.marker, 'yacr: unsupported-runtime: webgl2');
  assert.equal(failure.detail, 'no webgl2 context');
  // A success is not a failure.
  assert.equal(statusFailure(parseCanvasStatus(OK_STATUS), OK_STATUS), null);
});

test('a failure status without a step or error is not silently a success', () => {
  const failure = statusFailure(parseCanvasStatus('{"ok":false}'), '{"ok":false}');
  assert.equal(failure.step, CANVAS_STEP_UNCLASSIFIED);
  assert.equal(failure.marker, null);
  assert.equal(failure.detail, '{"ok":false}');
  // Unreadable JSON is its own step, with the raw text as the detail.
  const unreadable = statusFailure(null, '<html>');
  assert.equal(unreadable.step, CANVAS_STEP_UNREADABLE);
  assert.equal(unreadable.detail, '<html>');
});

// --- module URL ------------------------------------------------------------

test('the loader targets the public bootstrap module and one shared wasm binary', () => {
  assert.equal(CANVAS_BOOTSTRAP_FILE, 'bootstrap.js');
  assert.equal(CANVAS_WASM_FILE, 'app_web_bg.wasm');
  assert.equal(CANVAS_GLOBAL, '__YACR_CANVAS_MODULE__');
  // The injected artifact is the bootstrap module, which itself imports app_web.js.
  assert.equal(canvasScriptUrl(), '/canvas/bootstrap.js');
  assert.equal(canvasWasmUrl(), '/canvas/app_web_bg.wasm');
  assert.equal(canvasModuleUrl(), '/canvas/bootstrap.js');
  // Directory, bare directory and file URL all resolve to the same pair.
  for (const base of ['/canvas/', '/canvas', '/canvas/bootstrap.js']) {
    assert.equal(canvasScriptUrl(base), '/canvas/bootstrap.js', base);
    assert.equal(canvasWasmUrl(base), '/canvas/app_web_bg.wasm', base);
  }
  assert.equal(canvasBaseDir('/canvas'), '/canvas/');
  assert.equal(canvasBaseDir('https://host.test/artifacts/'), 'https://host.test/artifacts/');
  assert.equal(canvasScriptUrl('https://host.test/artifacts'), 'https://host.test/artifacts/bootstrap.js');
  // A non-string is not a base: the default is used rather than a broken specifier.
  assert.equal(canvasScriptUrl(''), '/canvas/bootstrap.js');
  assert.equal(canvasScriptUrl(null), '/canvas/bootstrap.js');
});

// --- transport -------------------------------------------------------------

test('the transport starts once and resizes only on a real size change', async () => {
  const module = stubModule();
  const statuses = [];
  const transport = createCanvasTransport({
    module,
    element: { id: 'native-input' },
    rect: { x: 0, y: 0, width: 320, height: 200, dpr: 2 },
    onStatus: next => statuses.push(next),
  });
  await transport.whenIdle();
  assert.deepEqual(module.calls, [['start', 'cad-canvas', 320, 200, 2]]);
  assert.equal(statuses.length, 1);
  assert.equal(statuses[0].ok, true);

  // A move within the same viewport is not a resize.
  assert.equal(transport.update({ x: 12, y: 8, width: 320, height: 200, dpr: 2 }), false);
  // A new size is.
  assert.equal(transport.update({ x: 0, y: 0, width: 640, height: 480, dpr: 2 }), true);
  assert.equal(transport.update({ x: 0, y: 0, width: 640, height: 480, dpr: 1 }), true);
  await transport.whenIdle();
  assert.deepEqual(module.calls.slice(1), [
    ['resize', 640, 480, 2],
    ['resize', 640, 480, 1],
  ]);
});

test('a degenerate rect makes no call at all', async () => {
  const module = stubModule();
  const failures = [];
  const transport = createCanvasTransport({
    module,
    rect: { width: NaN, height: 10, dpr: 1 },
    onFailure: failure => failures.push(failure),
  });
  assert.equal(transport.update({}), false);
  await transport.whenIdle();
  assert.deepEqual(module.calls, []);
  // "No size" is a caller-side fact, not a session failure.
  assert.deepEqual(failures, []);
  assert.equal(transport.failed, false);
});

test('pan and zoom forward their arguments and drop no-ops', async () => {
  const module = stubModule();
  const transport = createCanvasTransport({ module, rect: { width: 320, height: 200, dpr: 1 } });
  assert.equal(transport.pan(0, 0), false);
  assert.equal(transport.pan(10, -4), true);
  assert.equal(transport.zoom(1.25, 100.5, 60.25), true);
  assert.equal(transport.zoom(0, 10, 10), false);
  assert.equal(transport.zoom(1.25, NaN, 10), false);
  await transport.whenIdle();
  assert.deepEqual(module.calls.slice(1), [
    ['pan', 10, -4],
    ['zoom_at', 1.25, 100.5, 60.25],
  ]);
});

test('a webgl2 start failure is reported once, with the marker, and latches the transport', async () => {
  const module = stubModule({ start: async () => WEBGL2_FAILURE });
  const failures = [];
  const statuses = [];
  const transport = createCanvasTransport({
    module,
    rect: { width: 320, height: 200, dpr: 1 },
    onFailure: failure => failures.push(failure),
    onStatus: next => statuses.push(next),
  });
  await transport.whenIdle();
  assert.deepEqual(failures, [{
    step: 'webgl2',
    detail: 'no webgl2 context',
    marker: 'yacr: unsupported-runtime: webgl2',
  }]);
  // No success is ever reported for a refusal.
  assert.deepEqual(statuses, []);
  assert.equal(transport.failed, true);

  // No retry and no further calls, in either direction.
  assert.equal(transport.update({ width: 400, height: 300, dpr: 1 }), false);
  assert.equal(transport.pan(5, 5), false);
  assert.equal(transport.zoom(1.25, 1, 1), false);
  await transport.whenIdle();
  await new Promise(resolve => setTimeout(resolve, 5));
  assert.equal(module.calls.length, 1);
  assert.equal(failures.length, 1);
});

test('an export that throws or an unreadable status is reported, not swallowed', async () => {
  const throwing = stubModule({ start: async () => { throw 'canvas not initialized'; } });
  const thrown = [];
  const throwingTransport = createCanvasTransport({
    module: throwing,
    rect: { width: 10, height: 10, dpr: 1 },
    onFailure: failure => thrown.push(failure),
  });
  await throwingTransport.whenIdle();
  assert.equal(thrown.length, 1);
  assert.equal(thrown[0].step, CANVAS_STEP_CALL);
  assert.equal(thrown[0].detail, 'canvas not initialized');

  const unreadable = stubModule({ start: async () => '<html>not a status</html>' });
  const unreadableFailures = [];
  const unreadableTransport = createCanvasTransport({
    module: unreadable,
    rect: { width: 10, height: 10, dpr: 1 },
    onFailure: failure => unreadableFailures.push(failure),
  });
  await unreadableTransport.whenIdle();
  assert.equal(unreadableFailures.length, 1);
  assert.equal(unreadableFailures[0].step, CANVAS_STEP_UNREADABLE);
  assert.equal(unreadableFailures[0].detail, '<html>not a status</html>');
});

test('a module without the export contract is refused up front', () => {
  assert.throws(() => createCanvasTransport({ module: null, rect: { width: 1, height: 1, dpr: 1 } }), TypeError);
  assert.throws(() => createCanvasTransport({ module: {}, rect: { width: 1, height: 1, dpr: 1 } }), TypeError);
  assert.throws(() => bindCanvasNavigation({}, {}), TypeError);
});

// --- navigation binding (element stub, not a DOM mount) --------------------

test('drag pan uses canvas-local CSS px and captures exactly one pointer', () => {
  const element = stubElement({ left: 40, top: 10, width: 320, height: 200 });
  const pans = [];
  const unbind = bindCanvasNavigation(element, { pan: (dx, dy) => pans.push([dx, dy]), zoom: () => {} });

  // A non-primary press never starts a drag.
  element.dispatch('pointerdown', pointerEvent({ button: 2, clientX: 140, clientY: 60 }));
  assert.equal(element.captured.size, 0);
  element.dispatch('pointermove', pointerEvent({ clientX: 150, clientY: 70 }));
  assert.deepEqual(pans, []);

  element.dispatch('pointerdown', pointerEvent({ clientX: 140, clientY: 60 }));
  assert.deepEqual([...element.captured], [7]);
  element.dispatch('pointermove', pointerEvent({ clientX: 150, clientY: 70 }));
  assert.deepEqual(pans, [[10, 10]]);
  // A second move to the same place is a no-op, not pan(0, 0).
  element.dispatch('pointermove', pointerEvent({ clientX: 150, clientY: 70 }));
  element.dispatch('pointermove', pointerEvent({ clientX: 155, clientY: 66 }));
  assert.deepEqual(pans, [[10, 10], [5, -4]]);

  element.dispatch('pointercancel', pointerEvent({ clientX: 155, clientY: 66 }));
  assert.equal(element.captured.size, 0);
  // The cancelled gesture is over: later motion is not a pan.
  element.dispatch('pointermove', pointerEvent({ clientX: 200, clientY: 100 }));
  assert.deepEqual(pans, [[10, 10], [5, -4]]);
  unbind();
});

test('wheel zoom is cursor-anchored in canvas-local CSS px and swallows the scroll', () => {
  const element = stubElement({ left: 40, top: 10, width: 320, height: 200 });
  const zooms = [];
  const unbind = bindCanvasNavigation(element, { pan: () => {}, zoom: (...args) => zooms.push(args) });

  const event = pointerEvent({ deltaY: -120, deltaMode: 0, clientX: 140, clientY: 60 });
  element.dispatch('wheel', event);
  assert.equal(event.prevented, true);
  assert.deepEqual(zooms, [[1.25, 100, 50]]);

  // A page-mode notch is one notch; a zero delta forwards nothing.
  element.dispatch('wheel', pointerEvent({ deltaY: -1, deltaMode: 2, clientX: 40, clientY: 10 }));
  element.dispatch('wheel', pointerEvent({ deltaY: 0, deltaMode: 0, clientX: 40, clientY: 10 }));
  assert.deepEqual(zooms, [[1.25, 100, 50], [1.25, 0, 0]]);
  unbind();
});

test('unbinding removes every listener it added and releases a captured pointer', () => {
  const element = stubElement();
  const unbind = bindCanvasNavigation(element, { pan: () => {}, zoom: () => {} });
  assert.deepEqual([...element.listeners.keys()].sort(), [
    'lostpointercapture', 'pointercancel', 'pointerdown', 'pointermove', 'pointerup', 'wheel',
  ]);
  element.dispatch('pointerdown', pointerEvent({ clientX: 50, clientY: 20 }));
  assert.equal(element.captured.size, 1);
  unbind();
  assert.deepEqual([...element.listeners.keys()], []);
  assert.equal(element.captured.size, 0);
});

// --- source structure: the two paths stay separate -------------------------

/** Drop whole-line comments: prose may describe code the tree must not contain. */
function codeLines(source) {
  return source.split('\n').filter(line => !/^\s*(\/\/|\*|\/\*)/.test(line)).join('\n');
}

test('native-below never imports or loads the wasm module', async () => {
  const app = await readModule('App.jsx');
  const transport = await readModule('canvas-host.mjs');
  const shipped = await readAppSource();

  // No static import of the artifact anywhere in the shipped tree.
  for (const source of [app, transport, shipped]) {
    assert.ok(!/from\s+['"][^'"]*app_web[^'"]*['"]/.test(source), 'no static import of the wasm module');
  }
  // No dynamic import in code: Vite's dev transform rewrites `import(url)` into
  // `import(__vite__injectQuery(url, 'import'))`, which the dev server answers with
  // HTTP 500 for a public-dir file. The public bootstrap module replaced it.
  // (Whole-line comments are skipped, because the header *describes* that rewrite.)
  assert.ok(!/\bimport\s*\(/.test(codeLines(transport)), 'the loader must not use a dynamic import');
  assert.ok(!/import\s*\(/.test(codeLines(shipped)), 'no dynamic import of the canvas artifact anywhere');
  // The module loader: one injected tag, one cached promise, an awaited `ready`.
  assert.match(transport, /export function loadCanvasModule/);
  assert.equal([...transport.matchAll(/createElement\('script'\)/g)].length, 1);
  assert.match(transport, /script\.type = 'module'/);
  assert.match(transport, /script\.src = url/);
  assert.match(transport, /const bootstrapByEnv = new WeakMap\(\)/);
  assert.match(transport, /if \(pending === undefined\)/);
  assert.match(transport, /const module = env\[CANVAS_GLOBAL\]/);
  assert.match(transport, /await module\.ready/);
  // The no-modules path is gone: WebKitGTK leaves `document.currentScript`
  // `undefined`, which makes that glue's top-level `let` throw in its TDZ.
  assert.ok(!/document\.currentScript/.test(codeLines(shipped)), 'no document.currentScript reliance');
  assert.ok(!/\bwasm_bindgen\b/.test(codeLines(shipped)), 'no bare wasm_bindgen global');
  // CSP has no `'unsafe-eval'`: neither escape hatch may appear in code.
  assert.ok(!/\beval\s*\(/.test(codeLines(shipped)), 'no eval');
  assert.ok(!/new Function/.test(codeLines(shipped)), 'no new Function');

  // The app reaches it only inside the dom-canvas branch.
  const guard = app.indexOf('surfaceMode !== SURFACE_MODE_DOM_CANVAS');
  assert.ok(guard > 0);
  assert.ok(app.indexOf('loadCanvasModule(canvasModuleUrl())') > guard);
  assert.ok(app.indexOf('createCanvasTransport({') > guard);
  assert.ok(app.indexOf('bindCanvasNavigation(') > guard);
  assert.ok(app.indexOf('step: CANVAS_STEP_MODULE') > guard);
});

test('dom-canvas does not create the IPC input lane', async () => {
  const app = await readModule('App.jsx');
  // One lane, one transport: neither can be instantiated twice.
  assert.equal([...app.matchAll(/createInputLane\(/g)].length, 1);
  assert.equal([...app.matchAll(/invoke\('probe_input'/g)].length, 1);
  assert.equal([...app.matchAll(/createCanvasTransport\(\{/g)].length, 1);

  // The lane effect returns before creating anything when the host reports the
  // DOM carrier, and re-runs when the reported mode changes.
  const laneGuard = 'if (surfaceMode === SURFACE_MODE_DOM_CANVAS) return undefined;';
  const laneEffect = app.slice(app.indexOf(laneGuard), app.indexOf('createInputLane('));
  assert.ok(app.includes(laneGuard));
  assert.ok(laneEffect.length > 0);
  assert.match(app, /\}, \[surfaceMode\]\);/);

  // The transport module never speaks the host IPC and never builds a lane.
  const transport = await readModule('canvas-host.mjs');
  assert.ok(!transport.includes('probe_input'));
  assert.ok(!transport.includes('createInputLane'));
  assert.ok(!transport.includes('invoke('));
});

// --- public bootstrap module loader (stubbed page, not a browser) ----------

/**
 * A page stub: one document, one head, one recording appendChild, and a manually
 * controlled `ready` so the "awaits init" ordering is observable. The real
 * bootstrap sets `globalThis.__YACR_CANVAS_MODULE__` when it evaluates; `publish()`
 * is that step.
 */
function stubPage() {
  const appended = [];
  const document = {
    createElement(tag) {
      const element = {
        tagName: tag,
        type: '',
        src: '',
        removed: false,
        listeners: {},
        addEventListener(type, handler) { element.listeners[type] = handler; },
        removeEventListener(type) { delete element.listeners[type]; },
        remove() { element.removed = true; },
      };
      return element;
    },
    head: { appendChild(element) { appended.push(element); } },
  };
  let release;
  let fail;
  const ready = new Promise((resolve, reject) => { release = resolve; fail = reject; });
  const module = { ready };
  for (const name of CANVAS_EXPORTS) module[name] = () => OK_STATUS;
  const env = {
    document,
    location: { origin: 'http://localhost:1420', href: 'http://localhost:1420/' },
    fetch: async () => ({ status: 200, headers: { get: () => 'text/javascript' } }),
  };
  return { env, appended, module, publish() { env[CANVAS_GLOBAL] = module; }, release, fail };
}

const tick = () => new Promise(resolve => setTimeout(resolve, 0));

test('the loader injects exactly one module script and reuses the in-flight promise', async () => {
  const page = stubPage();
  const first = loadCanvasModule('/canvas/', page.env);
  const second = loadCanvasModule('/canvas/', page.env);
  // Idempotent: a second caller waits on the first attempt, no second tag.
  assert.equal(page.appended.length, 1);
  assert.equal(page.appended[0].tagName, 'script');
  assert.equal(page.appended[0].type, 'module');
  assert.equal(page.appended[0].src, '/canvas/bootstrap.js');

  // `load` means the bootstrap's top-level code has run and published the global.
  page.publish();
  page.appended[0].listeners.load();
  page.release();
  const [a, b] = await Promise.all([first, second]);
  assert.equal(a, b);
  assert.equal(a, page.module);
  assert.equal(a.start, page.module.start);

  // A third call after success is still the same attempt and appends nothing.
  assert.equal(await loadCanvasModule('/canvas/', page.env), a);
  assert.equal(page.appended.length, 1);
});

test('the loader waits for the wasm init before resolving', async () => {
  const page = stubPage();
  let settled = false;
  const pending = loadCanvasModule('/canvas/', page.env).then(module => {
    settled = true;
    return module;
  });
  page.publish();
  page.appended[0].listeners.load();
  await tick();
  await tick();
  // The script has loaded, but its wasm is still initialising behind `ready`.
  assert.equal(settled, false, 'must not resolve before the bootstrap ready promise settles');
  page.release();
  assert.equal(await pending, page.module);
  assert.equal(settled, true);
});

test('a module script that fails to load, or never publishes, is reported with the probe', async () => {
  const scriptFailure = stubPage();
  const failing = loadCanvasModule('/canvas/', scriptFailure.env);
  scriptFailure.appended[0].listeners.error();
  await assert.rejects(failing, error => {
    assert.match(error.message, /^script \/canvas\/bootstrap\.js failed: error event/);
    // The origin/fetch probe that made the `?import` bug diagnosable is kept.
    assert.match(error.message, /\[origin=http:\/\/localhost:1420 href=http:\/\/localhost:1420\/ fetch=200 text\/javascript\]/);
    return true;
  });
  assert.equal(scriptFailure.appended[0].removed, true);

  // A script that "loads" but publishes nothing is refused, not assumed.
  const noGlobal = stubPage();
  const missing = loadCanvasModule('/canvas/', noGlobal.env);
  noGlobal.appended[0].listeners.load();
  await assert.rejects(missing, /script \/canvas\/bootstrap\.js failed: no __YACR_CANVAS_MODULE__ after load/);

  // A published object without a `ready` promise is incomplete, not a success.
  const noReady = stubPage();
  const incomplete = loadCanvasModule('/canvas/', noReady.env);
  noReady.env[CANVAS_GLOBAL] = { start: () => OK_STATUS };
  noReady.appended[0].listeners.load();
  await assert.rejects(incomplete, /__YACR_CANVAS_MODULE__\.ready is missing/);
});

test('an init rejection surfaces as init <wasm> failed with the probe', async () => {
  const page = stubPage();
  const pending = loadCanvasModule('/canvas/', page.env);
  page.publish();
  page.appended[0].listeners.load();
  page.fail(new TypeError('WebAssembly.instantiateStreaming failed'));
  await assert.rejects(pending, error => {
    assert.match(error.message, /^init \/canvas\/app_web_bg\.wasm failed: TypeError: /);
    assert.match(error.message, /fetch=200/);
    return true;
  });
});

test('the build script generates the bootstrap module with the explicit wasm URL', async () => {
  const script = await readFile(BUILD_SCRIPT, 'utf8');
  const code = script.split('\n').filter(line => !/^\s*#/.test(line)).join('\n');
  assert.match(script, /BOOTSTRAP_JS="bootstrap\.js"/);
  // The exact three-line content the loader depends on.
  assert.ok(script.includes("import init, * as mod from './app_web.js';"));
  assert.ok(script.includes("new URL('./app_web_bg.wasm', import.meta.url).href"));
  assert.ok(script.includes('globalThis.__YACR_CANVAS_MODULE__ = { ready, ...mod };'));
  // One binding only: the no-modules target is not emitted any more (WebKitGTK
  // leaves `document.currentScript` undefined, which breaks that glue's TDZ).
  assert.ok(!/no-modules/.test(code), 'no no-modules emission');
  assert.ok(!/app_web_nomodules/.test(code), 'no no-modules artifact');
});

test('.native-input is the only element with pointer listeners', async () => {
  const app = await readModule('App.jsx');
  const transport = await readModule('canvas-host.mjs');
  const workspace = await readModule('components/Workspace.jsx');

  // Every pointer listener in the app targets the CAD partition element.
  const appTargets = [...app.matchAll(/(\w+)\.addEventListener\('pointer/g)].map(match => match[1]);
  assert.deepEqual([...new Set(appTargets)], ['surfaceNode']);

  // Inside the navigation binding, the only listener target is the element it was
  // handed - no query, no id lookup, no second surface.
  const bindBody = transport.slice(
    transport.indexOf('export function bindCanvasNavigation'),
    transport.indexOf('export function watchCanvasRect'),
  );
  const bindTargets = [...bindBody.matchAll(/(\w+)\.addEventListener\(/g)].map(match => match[1]);
  assert.deepEqual([...new Set(bindTargets)], ['element']);
  assert.ok(!/querySelector\(\s*['"][^'"]*cad-canvas/.test(transport));
  assert.ok(!transport.includes('getElementById'));

  // The canvas element carries no handler prop and stays out of the a11y tree.
  const canvasTag = workspace.match(/<canvas\s[^>]*>/)[0];
  assert.ok(!/on[A-Z]/.test(canvasTag));
  assert.ok(canvasTag.includes('aria-hidden="true"'));
});
