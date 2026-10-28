// Contracts for the `dom-canvas` transport (ADR 0024 D1-D3).
//
// Scope, stated honestly: everything here is a **pure function** test, a test
// against a hand-written element stub, or a **source-structure** assertion. There
// is no DOM mount, no browser run, no wasm module and no pixel evidence in this
// file; the mode is a data flag and the transport is a logic seam. What is pinned:
// the size/DPR the session is configured with, the degenerate-input guards, the
// one-shot failure routing, and the structural invariants that keep `native-below`
// and `<canvas>` untouched — plus the fact that the camera is **not** here at all
// any more (pan/zoom are the shared input lane's, `test/input.test.ts` and
// `test/frame-loop.test.ts` cover the rest).
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
  canvasBaseDir,
  canvasCssSize,
  canvasModuleUrl,
  canvasScriptUrl,
  canvasWasmUrl,
  createCanvasTransport,
  loadCanvasModule,
  parseCanvasStatus,
  statusFailure,
  watchCanvasRect,
} from '../src/cad/canvas-host.ts';
import { readFile } from 'node:fs/promises';
import { readAppSource, readModule } from './helpers.ts';

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

/** A wasm-session stub whose every export records its arguments. */
function stubModule({ start = async () => OK_STATUS, resize = () => OK_STATUS, frame = () => '{"ok":true,"presented":false}' } = {}) {
  const calls = [];
  return {
    calls,
    start: (...args) => { calls.push(['start', ...args]); return start(); },
    resize: (...args) => { calls.push(['resize', ...args]); return resize(); },
    frame: (...args) => { calls.push(['frame', ...args]); return frame(); },
  };
}

// --- no camera here --------------------------------------------------------

test('the transport module owns no camera: pan/zoom are the input lane\'s', async () => {
  const transport = await readModule('cad/canvas-host.ts');
  // The one camera owner on this carrier is the session itself, reached through
  // the shared `probe_input` lane. A second "zoom the camera from the transport"
  // path could only disagree with it, so it is gone: no wheel arithmetic, no drag
  // delta, no navigation binding, and no export that moves the camera.
  for (const name of ['bindCanvasNavigation', 'wheelZoomFactor', 'wheelZoomNotches', 'panDelta', 'ZOOM_STEP']) {
    assert.ok(!transport.includes(name), `${name} must not be in the transport module`);
  }
  assert.ok(!/\bpan\s*\(/.test(transport), 'no pan export');
  assert.ok(!/\bzoom_at\b/.test(transport), 'no zoom_at call');
  // The exports it does drive: one start, one resize per real size change, and the
  // frame step the pump calls.
  assert.deepEqual([...CANVAS_EXPORTS], ['start', 'resize', 'frame']);
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

test('the transport exposes no camera surface at all', async () => {
  const module = stubModule();
  const transport = createCanvasTransport({ module, rect: { width: 320, height: 200, dpr: 1 } });
  // `native-below` keeps its pan/zoom; on the page carrier the camera belongs to
  // the session and is driven through `probe_input`, so a `pan`/`zoom` here would
  // be a second owner. The type is the contract: nothing to call.
  assert.equal('pan' in transport, false);
  assert.equal('zoom' in transport, false);
  await transport.whenIdle();
  assert.deepEqual(module.calls, [['start', 'cad-canvas', 320, 200, 1]]);
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

  // No retry and no further calls.
  assert.equal(transport.update({ width: 400, height: 300, dpr: 1 }), false);
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
  // `frame` is part of the contract too: a session whose frame step is missing can
  // never repaint, so it is refused at load time (see `CANVAS_EXPORTS`).
  assert.ok(CANVAS_EXPORTS.includes('frame'));
});

// --- source structure: the two paths stay separate -------------------------

/** Drop whole-line comments: prose may describe code the tree must not contain. */
function codeLines(source) {
  return source.split('\n').filter(line => !/^\s*(\/\/|\*|\/\*)/.test(line)).join('\n');
}

test('native-below never imports or loads the wasm module', async () => {
  const app = await readModule('App.tsx');
  const transport = await readModule('cad/canvas-host.ts');
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
  assert.match(transport, /const bootstrapByEnv = new WeakMap[<(]/);
  assert.match(transport, /if \(pending === undefined\)/);
  assert.match(transport, /const (?:loaded|module) = env\[CANVAS_GLOBAL\]/);
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
  assert.ok(app.indexOf('createFrameLoop({ module })') > guard);
  assert.ok(app.indexOf('step: CANVAS_STEP_MODULE') > guard);
});

test('one input lane serves both carriers, and the transport builds no second one', async () => {
  const app = await readModule('App.tsx');
  // One lane, one transport, one probe_input call site: none can exist twice, and
  // `dom-canvas` is no exception any more - it forwards the same events to the
  // session's own `probe_input` through the adapter.
  assert.equal([...app.matchAll(/createInputLane\(/g)].length, 1);
  assert.equal([...app.matchAll(/invoke\('probe_input'/g)].length, 1);
  assert.equal([...app.matchAll(/createCanvasTransport\(\{/g)].length, 1);
  assert.equal([...app.matchAll(/createFrameLoop\(/g)].length, 1);

  // The lane waits for a **known** carrier and, on the page carrier, for a live
  // session: before that the wasm carrier's own "no live CAD session" refusal would
  // land in the error banner, and on the unknown mount the pre-answer default is a
  // compatibility guess rather than a fact.
  assert.ok(app.includes('const carrierKnown = status !== null;'));
  assert.ok(app.includes('const inputLaneReady = carrierKnown && (surfaceMode !== SURFACE_MODE_DOM_CANVAS || canvasLive);'));
  assert.ok(app.includes('if (!inputLaneReady) return undefined;'));
  assert.ok(app.includes('setCanvasLive(true);'));
  assert.match(app, /\}, \[inputLaneReady\]\);/);
  // The partition binding re-runs with the lane: the lane is created a commit after
  // the partition is measured, so without this dependency there would be a lane and
  // nothing bound to it.
  assert.match(app, /\}, \[surfaceNode, inputLaneReady\]\);/);

  // The suspension gate is untouched: still the two independent reasons, still one
  // gate applied to the one binding.
  assert.equal([...app.matchAll(/createSuspendGate\(/g)].length, 1);
  assert.ok(app.includes("suspendGate.current!.set('popupOpen', popupOpen);"));
  assert.ok(app.includes("suspendPointer: value => suspendGate.current!.set('textFocus', value),"));

  // The transport module never speaks the host IPC and never builds a lane.
  const transport = await readModule('cad/canvas-host.ts');
  // Prose may *describe* the lane (the header explains why it is the only input
  // path); the code must not reach it.
  const transportCode = codeLines(transport);
  assert.ok(!transportCode.includes('probe_input'));
  assert.ok(!transportCode.includes('createInputLane'));
  assert.ok(!transportCode.includes('invoke('));
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
  const app = await readModule('App.tsx');
  const transport = await readModule('cad/canvas-host.ts');
  const workspace = await readModule('components/Workspace.tsx');

  // Every pointer listener in the app targets the CAD partition element. The
  // app may also add **none** at all: the local pixel readout was removed when
  // the status bar started showing the host's world coordinates, so there is no
  // app-level pointer listener left — and, above all, none on another element.
  const appTargets = [...app.matchAll(/(\w+)\.addEventListener\('pointer/g)].map(match => match[1]);
  assert.ok(appTargets.every(target => target === 'surfaceNode'), appTargets.join(','));

  // The transport hands no pointer listener out at all any more: navigation is the
  // shared lane's, which binds `.native-input` (and only it). The rect watcher is
  // the one DOM binding left here, and its only listener target is the window.
  assert.ok(!transport.includes("addEventListener('pointer"));
  assert.ok(!transport.includes("addEventListener('wheel"));
  const watchBody = transport.slice(transport.indexOf('export function watchCanvasRect'));
  const watchTargets = [...watchBody.matchAll(/(\w+)\.addEventListener\(/g)].map(match => match[1]);
  // `env` is the window; `query` is the DPR `MediaQueryList` (a listener target of
  // its own), never an element.
  assert.deepEqual([...new Set(watchTargets)], ['query', 'env']);
  // No second surface is ever looked up: the caller hands the element in.
  assert.ok(!/querySelector\(\s*['"][^'"]*cad-canvas/.test(transport));
  assert.ok(!transport.includes('getElementById'));

  // The canvas element carries no handler prop and stays out of the a11y tree.
  const canvasTag = workspace.match(/<canvas\s[^>]*>/)[0];
  assert.ok(!/on[A-Z]/.test(canvasTag));
  assert.ok(canvasTag.includes('aria-hidden="true"'));
});
