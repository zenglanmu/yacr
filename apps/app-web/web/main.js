/*
 * Functional shell for the wasm CAD session (`surfaceMode: dom-canvas`).
 *
 * It is the browser sibling of the Linux host page: no framework, no bundler,
 * no second document. All navigation goes to the wasm exports; this file only
 * measures the canvas, forwards pointer/wheel input in CSS pixels, opens local
 * DWG files through the File API and prints the JSON status each call returns.
 *
 * The wasm module is the **only** CAD session (ADR 0024 D2); this page holds no
 * geometry, no scene and no camera state of its own.
 *
 * # Two carriers, chosen by measured capability (ADR 0024 stage ⑧)
 *
 * `boot()` asks for worker isolation first and uses the main thread only when the
 * browser cannot give it:
 *
 *   1. `canvas.transferControlToOffscreen` must exist (Chromium 141 and
 *      WebKitGTK 2.52.6 both have it, so this alone proves nothing);
 *   2. a module Worker (`./worker.js`) must report WebGL2 available on an
 *      `OffscreenCanvas` **inside the Worker** (`worker_capability()`).
 *
 * Measured 2026-10-24: Chromium 141 renders in a Worker, WebKitGTK 2.52.6 returns
 * no WebGL2 context there. On WebKitGTK this page therefore runs the main-thread
 * carrier — the already-verified path — and prints
 * `isolation=main-thread (no WebGL2 in worker)`: the fallback is never silent.
 *
 * Whichever branch wins owns the session alone. On the isolated branch this page
 * never initialises the wasm module (only the Worker does), so there is exactly one
 * CAD session and one document per canvas.
 *
 * Serve this directory over HTTP with the wasm-bindgen `web` output in `./pkg/`:
 *   cargo build -p app-web --target wasm32-unknown-unknown --release
 *   wasm-bindgen --target web --out-dir apps/app-web/web/pkg \
 *       target/wasm32-unknown-unknown/release/app_web.wasm
 *
 * The font package is **not** bundled or preloaded. It is served by the project's
 * Pages CDN at same-origin `fonts/` and fetched lazily, after a document opens,
 * only when the drawing needs text or names a face: `cad_fonts` says what the
 * open document needs, the matching catalogue faces are fetched, installed with
 * `set_font_package`, and `apply_font_package` re-runs the install on the live
 * session. The isolated Worker carrier has no `CadSession`, so it refuses font
 * loading by name instead of pretending (see `worker.js`).
 */

import init, {
  start,
  open_bytes,
  resize,
  zoom_at,
  pan,
  document as cadDocument,
  last_import_ms as lastImportMs,
  frame as presentFrame,
  set_font_package,
  apply_font_package,
  invoke,
} from './pkg/app_web.js';

const canvas = document.getElementById('cad-canvas');
const statusLine = document.getElementById('cad-status');
const fileInput = document.getElementById('cad-file');
const fileName = document.getElementById('cad-file-name');

/** One wheel notch, matching the desktop session's ZOOM_IN_FACTOR. */
const ZOOM_STEP = 1.25;
/** A DOM wheel notch is 120 px in deltaMode 0 (2 = "pixels" in practice). */
const WHEEL_NOTCH_PX = 120;
/**
 * How long the worker may take to answer the capability probe. It instantiates a
 * ~10 MB wasm module first, so this is generous; a timeout means "cannot prove
 * isolation", which falls back to the main-thread carrier with that reason.
 */
const CAPABILITY_TIMEOUT_MS = 15000;

/** Live session is required by every navigation export. */
let ready = false;
/** Active drag: pointer id plus the last canvas-local CSS position. */
let drag = null;
/** A resize request already scheduled for the next animation frame. */
let resizePending = false;
/**
 * The carrier actually in use: `{kind: 'worker', bridge}` or
 * `{kind: 'main-thread', reason}`. `null` until `boot()` has decided.
 */
let carrier = null;

function devicePixelRatio() {
  const ratio = window.devicePixelRatio;
  return Number.isFinite(ratio) && ratio > 0 ? ratio : 1;
}

/** The canvas CSS size; a zero-sized element is reported as 1 px, not 0. */
function cssSize() {
  const rect = canvas.getBoundingClientRect();
  return { w: Math.max(rect.width, 1), h: Math.max(rect.height, 1) };
}

/** Canvas-local CSS position of a pointer/mouse event. */
function localPoint(event) {
  const rect = canvas.getBoundingClientRect();
  return { x: event.clientX - rect.left, y: event.clientY - rect.top };
}

/** Show which file the wasm session has open (`status.label`, or `-`). */
function showLabel(label) {
  fileName.textContent = typeof label === 'string' && label !== '' ? label : 'no file open';
}

/** Whether the isolated (Worker) carrier is in use. */
function isIsolated() {
  return carrier !== null && carrier.kind === 'worker';
}

/**
 * The isolation actually used, as it appears on the status line.
 *
 * `worker` only when a Worker really renders this canvas. Every fallback keeps its
 * measured reason, so `isolation=main-thread (no WebGL2 in worker)` on WebKitGTK can
 * never be misread as an unexplained choice.
 */
function isolationText() {
  if (carrier === null) {
    return 'isolation=pending (probing worker WebGL2)';
  }
  if (carrier.kind === 'worker') {
    return 'isolation=worker';
  }
  return `isolation=main-thread (${carrier.reason})`;
}

// ---------------------------------------------------------------------------
// Worker bridge
// ---------------------------------------------------------------------------

/**
 * Request/reply channel to the module Worker that owns the isolated session.
 *
 * Keep-alive is the caller's business (`dispose()` terminates the worker, which
 * ends its session and frees its canvas).
 */
class WorkerBridge {
  constructor(worker) {
    this.worker = worker;
    this.pending = new Map();
    this.nextId = 1;
    worker.addEventListener('message', (event) => {
      const message = event.data;
      if (!message || typeof message.id !== 'number') {
        return;
      }
      const entry = this.pending.get(message.id);
      if (!entry) {
        return;
      }
      this.pending.delete(message.id);
      if (message.ok) {
        entry.resolve(message);
      } else {
        entry.reject(new Error(message.error ?? 'worker command failed'));
      }
    });
    worker.addEventListener('error', (event) => {
      // A module that fails to load (404, syntax error, CSP) fires here and never
      // answers, so every waiter must fail instead of hanging.
      this.failAll(`worker error: ${event.message ?? 'unknown'}`);
    });
  }

  /** Reject every in-flight request (worker gone or unusable). */
  failAll(reason) {
    const entries = [...this.pending.values()];
    this.pending.clear();
    for (const entry of entries) {
      entry.reject(new Error(reason));
    }
  }

  /** Send one command, optionally transferring buffers, and await its reply. */
  call(message, transfer = []) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      try {
        this.worker.postMessage({ ...message, id }, transfer);
      } catch (error) {
        this.pending.delete(id);
        reject(error);
      }
    });
  }

  /** End the worker session (its wasm instance and canvas go with it). */
  dispose() {
    this.failAll('worker session disposed');
    this.worker.terminate();
  }
}

/** Reject if `promise` does not settle within `ms` (never leaves a listener behind). */
function withTimeout(promise, ms) {
  return new Promise((resolve, reject) => {
    const timer = window.setTimeout(
      () => reject(new Error(`timed out after ${ms} ms`)),
      ms,
    );
    promise.then(
      (value) => {
        window.clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        window.clearTimeout(timer);
        reject(error);
      },
    );
  });
}

/**
 * Ask a fresh Worker whether it can render at all, and keep it if it can.
 *
 * Returns `{bridge}` on success or `{reason}` when the page must stay on the main
 * thread. The probe is the measured discriminator (`OffscreenCanvas.getContext(
 * 'webgl2')` **inside the Worker**), not a feature sniff: both engines expose
 * `OffscreenCanvas` and `transferControlToOffscreen`.
 */
async function probeWorkerIsolation() {
  if (typeof canvas.transferControlToOffscreen !== 'function') {
    return { reason: 'transferControlToOffscreen unavailable' };
  }
  let worker;
  try {
    worker = new Worker(new URL('./worker.js', import.meta.url), { type: 'module' });
  } catch (error) {
    return { reason: `worker could not be created: ${String(error)}` };
  }
  const bridge = new WorkerBridge(worker);
  try {
    const reply = await withTimeout(
      bridge.call({ type: 'capability' }),
      CAPABILITY_TIMEOUT_MS,
    );
    const report = JSON.parse(reply.capability ?? 'null');
    if (report !== null && report.ok === true && report.isolation === 'worker') {
      // The worker owns the viewer session, which has no CadSession and therefore
      // no font install. The worker names that refusal; keep it visible instead of
      // letting this page imply the package was loaded there.
      if (typeof reply.fontPackage === 'string') {
        globalThis.__YACR_FONT_PACKAGE_ERROR__ = reply.fontPackage;
      }
      return { bridge };
    }
    const reason =
      report !== null && typeof report.reason === 'string'
        ? report.reason
        : 'worker reported no WebGL2 context';
    bridge.dispose();
    return { reason };
  } catch (error) {
    bridge.dispose();
    return { reason: `worker capability probe failed: ${String(error)}` };
  }
}

// ---------------------------------------------------------------------------
// Status reporting
// ---------------------------------------------------------------------------

/** Print one status object (or one rejection) on the status line. */
function report(label, json) {
  let parsed;
  try {
    parsed = JSON.parse(json);
  } catch (error) {
    statusLine.textContent = `${label}: unreadable status (${String(error)}): ${json}`;
    return;
  }
  if (parsed.label) {
    showLabel(parsed.label);
  }
  const parts = [
    `${label}: ${parsed.ok ? 'ok' : 'FAILED'}`,
    `file=${parsed.label ?? '-'}`,
    `bytes=${parsed.bytes ?? 0}`,
    `backend=${parsed.backend ?? '-'}`,
    `format=${parsed.format ?? '-'}`,
    // What the wasm session itself reports it draws into (`html-canvas` |
    // `offscreen-canvas`); `isolation` is printed once, from `isolationText()`, so
    // a session status can never restate it with a different value.
    `target=${parsed.target ?? '-'}`,
    `physical=${Array.isArray(parsed.physical) ? parsed.physical.join('x') : '-'}`,
    `batches=${parsed.batches}`,
    `draws=${parsed.draws}`,
    `presented=${parsed.presented}`,
    isolationText(),
  ];
  if (!parsed.ok) {
    parts.push(`step=${parsed.step ?? '-'}`);
    if (parsed.marker) {
      parts.push(`marker=${parsed.marker}`);
    }
    if (parsed.error) {
      parts.push(`error=${parsed.error}`);
    }
  }
  statusLine.textContent = parts.join('  ');
}

/** Run one synchronous wasm export (main-thread carrier). */
function runLocal(label, action) {
  try {
    report(label, action());
  } catch (error) {
    // The wasm exports reject with the message that names the failing step.
    statusLine.textContent = `${label} failed: ${String(error)}`;
  }
}

/** Run one command on the isolated carrier's worker. */
async function runWorker(label, message, transfer = []) {
  try {
    const reply = await carrier.bridge.call(message, transfer);
    report(label, reply.status);
  } catch (error) {
    statusLine.textContent = `${label} failed: ${String(error)}  ${isolationText()}`;
  }
}

/**
 * Route one command to whichever carrier owns the session.
 *
 * `localAction` runs the wasm export on this thread; `message` is the equivalent
 * worker command. Both reach the same Rust session implementation, so the two
 * branches cannot diverge in behaviour — only in where they run.
 */
function command(label, localAction, message) {
  if (isIsolated()) {
    return runWorker(label, message);
  }
  runLocal(label, localAction);
  return Promise.resolve();
}

/** The `document()` object from whichever carrier owns the session. */
async function documentJson() {
  if (isIsolated()) {
    const reply = await carrier.bridge.call({ type: 'document' });
    return JSON.parse(reply.document);
  }
  return JSON.parse(cadDocument());
}

/** The measured import duration from whichever carrier owns the session. */
async function lastImportMsFromCarrier() {
  if (isIsolated()) {
    const reply = await carrier.bridge.call({ type: 'lastImportMs' });
    return reply.importMs;
  }
  return lastImportMs();
}

/**
 * The evidence line for one document: the file the session actually holds, its
 * source size, and the measured import duration / wasm heap.
 *
 * `wasm_heap_bytes` is the module's own linear memory, **not** process RSS, and
 * `import_ms` is one measurement of the last open — neither is a performance
 * claim. On the isolated carrier it is the Worker's heap and the Worker's
 * measurement, because the Worker owns the session.
 */
async function documentSummary() {
  try {
    const doc = await documentJson();
    const heap =
      typeof doc.wasm_heap_bytes === 'number' ? `${doc.wasm_heap_bytes} B` : 'unavailable';
    const importMs =
      typeof doc.import_ms === 'number' && doc.import_ms >= 0 ? `${doc.import_ms} ms` : 'not measured';
    return [
      `document=${doc.ok ? 'ok' : 'none'}`,
      `label=${doc.label ?? '-'}`,
      `bytes=${doc.bytes}`,
      `batches=${doc.batches}`,
      `wasm_heap=${heap}`,
      `import_ms=${importMs}`,
      isolationText(),
    ].join('  ');
  } catch (error) {
    return `document: unreadable (${String(error)})`;
  }
}

/** Whether the session currently holds a file (`document().label`). */
async function sessionIsOpen() {
  try {
    return (await documentJson()).label !== null;
  } catch (error) {
    return false;
  }
}

/** Print the status line plus the document line (one evidence block per action). */
async function reportWithDocument(line) {
  statusLine.textContent = `${line}\n${await documentSummary()}`;
}

function scheduleResize() {
  if (resizePending) {
    return;
  }
  resizePending = true;
  window.requestAnimationFrame(() => {
    resizePending = false;
    if (!ready) {
      return;
    }
    const size = cssSize();
    void command(
      'resize',
      () => resize(size.w, size.h, devicePixelRatio()),
      { type: 'resize', cssW: size.w, cssH: size.h, dpr: devicePixelRatio() },
    );
  });
}

/**
 * Open one local DWG in the session (whichever carrier owns it).
 *
 * `arrayBuffer()` is the File API read; on the main-thread carrier the bytes cross
 * into wasm through `open_bytes` and nothing is kept on this side, on the isolated
 * carrier they are transferred to the worker's `open_bytes_offscreen`. A rejected
 * file (`step` `import` or `empty-scene`) leaves the previous session open — the
 * status line reports the failure and the document line still names the file on
 * screen.
 */
async function openFile(file) {
  const size = cssSize();
  let line;
  fileInput.disabled = true;
  try {
    const buffer = await file.arrayBuffer();
    if (isIsolated()) {
      const reply = await carrier.bridge.call(
        {
          type: 'open',
          label: file.name,
          bytes: buffer,
          cssW: size.w,
          cssH: size.h,
          dpr: devicePixelRatio(),
        },
        [buffer],
      );
      ready = true;
      report(`open ${file.name}`, reply.status);
      line = statusLine.textContent;
    } else {
      const status = await open_bytes(
        'cad-canvas',
        file.name,
        size.w,
        size.h,
        devicePixelRatio(),
        new Uint8Array(buffer),
      );
      ready = true;
      report(`open ${file.name}`, status);
      line = statusLine.textContent;
      // Lazy fonts: the document is open, so now (and only now) fetch the faces
      // it actually needs and re-run the install on the live session. A failure
      // keeps the open document and the status line; it is published on
      // `__YACR_FONT_PACKAGE_ERROR__` rather than shown as a successful load.
      try {
        await ensureFontPackage();
      } catch (error) {
        console.error(`yacr: fonts not installed for ${file.name}:`, error);
      }
    }
  } catch (error) {
    line = `open ${file.name} failed: ${String(error)}`;
    console.error(line);
    // A rejected file leaves the previous session open; keep navigating it.
    ready = await sessionIsOpen();
  } finally {
    fileInput.disabled = false;
  }
  // Measurement surfaces for the evidence run: the raw values, not a rendering
  // of them. `lastImportMs()` is negative until one open completed.
  console.log(await documentSummary());
  console.log(`last_import_ms=${await lastImportMsFromCarrier()}`);
  await reportWithDocument(line);
}

fileInput.addEventListener('change', () => {
  const file = fileInput.files && fileInput.files[0];
  if (!file) {
    return;
  }
  // The `File` stays readable after this, and clearing the input lets a second
  // selection of the *same* file fire `change` again (reopening for evidence).
  fileInput.value = '';
  void openFile(file);
});

canvas.addEventListener('pointerdown', (event) => {
  if (!ready || event.button !== 0) {
    return;
  }
  const point = localPoint(event);
  drag = { id: event.pointerId, x: point.x, y: point.y };
  canvas.setPointerCapture(event.pointerId);
});

canvas.addEventListener('pointermove', (event) => {
  if (!ready || drag === null || event.pointerId !== drag.id) {
    return;
  }
  const point = localPoint(event);
  const dx = point.x - drag.x;
  const dy = point.y - drag.y;
  drag.x = point.x;
  drag.y = point.y;
  if (dx !== 0 || dy !== 0) {
    void command('pan', () => pan(dx, dy), { type: 'pan', dx, dy });
  }
});

function endDrag(event) {
  if (drag !== null && event.pointerId === drag.id) {
    drag = null;
    if (canvas.hasPointerCapture(event.pointerId)) {
      canvas.releasePointerCapture(event.pointerId);
    }
  }
}

canvas.addEventListener('pointerup', endDrag);
canvas.addEventListener('pointercancel', endDrag);

canvas.addEventListener(
  'wheel',
  (event) => {
    if (!ready) {
      return;
    }
    event.preventDefault();
    // deltaMode 1 counts lines, 2 counts pages: normalize both to "pixels".
    const scale =
      event.deltaMode === 1 ? 1 / 3 : event.deltaMode === 2 ? 1 : 1 / WHEEL_NOTCH_PX;
    const notches = event.deltaY * scale;
    if (!Number.isFinite(notches) || notches === 0) {
      return;
    }
    // Wheel up (negative deltaY) zooms in, matching the desktop convention.
    const factor = Math.pow(ZOOM_STEP, -notches);
    const point = localPoint(event);
    void command(
      'zoom',
      () => zoom_at(factor, point.x, point.y),
      { type: 'zoom', factor, x: point.x, y: point.y },
    );
  },
  { passive: false },
);

window.addEventListener('resize', scheduleResize);

/**
 * Measurement handle for the evidence run (console access): the two measurement
 * exports plus the document summary this page already prints. Not a product API;
 * it exposes no geometry and no second session. `document` and `summary` are
 * asynchronous on the isolated carrier (the call crosses to the Worker), so they
 * return promises on both carriers rather than two different return types.
 */
window.__YACR_CANVAS__ = {
  document: documentJson,
  lastImportMs: lastImportMsFromCarrier,
  summary: documentSummary,
  isolation: isolationText,
};

/**
 * Lazily install the font package the open document needs.
 *
 * Nothing is fetched before a document opens, and nothing is fetched for a
 * drawing with no text and no named font. The catalogue and faces live on the
 * project's Pages CDN (`fonts/` next to the page) unless a host overrides
 * `__YACR_FONT_BASE_URL__`.
 *
 * `set_font_package` is synchronous and copies the bytes into the wasm heap;
 * `apply_font_package` then re-runs the host resource install on the live session
 * and presents one frame. A failure is published on
 * `__YACR_FONT_PACKAGE_ERROR__` and is never turned into a success: the session
 * keeps its explicit `unsupported-resources: fonts` diagnostic.
 */
const DEFAULT_FONT_FALLBACK_NAMES = ['noto sans cjk sc', 'osifont', 'arial', 'simplex'];

function fontBaseUrl() {
  const configured = globalThis.__YACR_FONT_BASE_URL__;
  if (typeof configured === 'string' && configured.trim() !== '') {
    return configured.endsWith('/') ? configured : `${configured}/`;
  }
  return new URL('fonts/', document.baseURI).href;
}

/** The `cad_fonts` projection of the live session, normalised. */
async function readFontState() {
  const raw = await invoke('cad_fonts', '{}');
  let envelope;
  try {
    envelope = JSON.parse(raw);
  } catch (error) {
    throw new Error(`cad_fonts: unreadable answer (${String(error)})`);
  }
  if (envelope === null || typeof envelope !== 'object') {
    throw new Error('cad_fonts: answer is not an object');
  }
  if (envelope.ok !== true) {
    throw new Error(`cad_fonts: ${envelope.error ?? 'refused without a reason'}`);
  }
  const value = envelope.value;
  if (value === null || typeof value !== 'object') {
    throw new Error('cad_fonts: value is not an object');
  }
  return {
    faces: Array.isArray(value.faces) ? value.faces : [],
    catalogEntries: Number(value.catalogEntries ?? 0),
    requested: Array.isArray(value.requested)
      ? value.requested.filter((key) => typeof key === 'string' && key !== '')
      : [],
    textNeeded: value.textNeeded === true,
    locale: typeof value.locale === 'string' ? value.locale.toLowerCase() : '',
  };
}

/** Every catalogue key one entry can be matched by (`file`, stem, `name[]`). */
function fontEntryKeys(entry) {
  const keys = [];
  if (entry === null || typeof entry !== 'object') return keys;
  if (typeof entry.file === 'string' && entry.file !== '') {
    keys.push(entry.file);
    const dot = entry.file.lastIndexOf('.');
    if (dot > 0) keys.push(entry.file.slice(0, dot));
  }
  if (Array.isArray(entry.name)) {
    for (const name of entry.name) {
      if (typeof name === 'string' && name !== '') keys.push(name);
    }
  }
  return keys.map((key) => key.toLowerCase());
}

/** The catalogue entries this drawing can use (its names + default/locale ones). */
function selectFontEntries(entries, requested) {
  const wanted = new Set(DEFAULT_FONT_FALLBACK_NAMES);
  for (const key of requested) wanted.add(key.toLowerCase());
  return entries.filter((entry) => {
    if (entry === null || typeof entry !== 'object') return false;
    if (typeof entry.file !== 'string' || entry.file === '') return false;
    return fontEntryKeys(entry).some((key) => wanted.has(key));
  });
}

/** Fetch the needed faces, install them, and re-run the install on the session. */
async function installFontPackage() {
  const state = await readFontState();
  if (state.catalogEntries > 0 || state.faces.some((face) => face && face.status === 'loaded')) {
    return globalThis.__YACR_FONT_PACKAGE__ ?? null;
  }
  if (!state.textNeeded && state.requested.length === 0) return null;

  const base = fontBaseUrl();
  const catalogUrl = new URL('fonts.json', base);
  const catalogResponse = await fetch(catalogUrl);
  if (!catalogResponse.ok) {
    throw new Error(`${catalogUrl.href}: HTTP ${catalogResponse.status}`);
  }
  const catalogText = await catalogResponse.text();
  const entries = JSON.parse(catalogText);
  if (!Array.isArray(entries)) {
    throw new Error('fonts.json must be a JSON array');
  }
  const selected = selectFontEntries(entries, state.requested);
  if (selected.length === 0) {
    throw new Error("fonts.json carries no entry for this drawing's font needs");
  }
  const files = {};
  for (const entry of selected) {
    const name = entry.file;
    const url = new URL(encodeURIComponent(name), base);
    const response = await fetch(url);
    if (!response.ok) {
      // A catalogue face the package promises but cannot be fetched is a loud
      // failure, never dropped so the drawing silently shapes with less.
      throw new Error(`${url.href}: HTTP ${response.status}`);
    }
    files[name] = new Uint8Array(await response.arrayBuffer());
  }
  const summary = JSON.parse(set_font_package(catalogText, files));
  const sessionStatus = JSON.parse(apply_font_package());
  presentFrame();
  const result = { ...summary, sessionStatus };
  globalThis.__YACR_FONT_PACKAGE__ = result;
  delete globalThis.__YACR_FONT_PACKAGE_ERROR__;
  return result;
}

/** One in-flight/successful load; a failure is not memoised so the next open retries. */
let fontPackageLoad = null;

function ensureFontPackage() {
  if (fontPackageLoad === null) {
    fontPackageLoad = installFontPackage().catch((error) => {
      fontPackageLoad = null;
      globalThis.__YACR_FONT_PACKAGE_ERROR__ = String(error);
      console.error('yacr: font package failed to load:', error);
      throw error;
    });
  }
  return fontPackageLoad;
}

async function startMainThread() {
  const size = cssSize();
  await init();
  // No font preload: `ready` (and `start`) never wait on the package. It is
  // fetched lazily after a document opens (see `ensureFontPackage`).
  // `start` loads the compiled-in synthetic fixture and reports its own status
  // (including a failure with the failing step and, for a missing WebGL2
  // context, the marker `yacr: unsupported-runtime: webgl2`). A real host maps
  // that marker to its explicit refusal instead of showing a blank canvas.
  // Opening a local file replaces the session through `open_bytes`.
  try {
    const status = await start('cad-canvas', size.w, size.h, devicePixelRatio());
    report('start', status);
    ready = await sessionIsOpen();
  } catch (error) {
    statusLine.textContent = `start failed: ${String(error)}  ${isolationText()}`;
    ready = false;
  }
  await reportWithDocument(statusLine.textContent);
}

/**
 * Start the session on the isolated carrier: transfer the canvas into the Worker
 * that already proved it has WebGL2 and let it open the fixture.
 *
 * This page instantiates no wasm module here — the Worker owns the only session.
 */
async function startIsolated() {
  const size = cssSize();
  const offscreen = canvas.transferControlToOffscreen();
  try {
    const reply = await carrier.bridge.call(
      {
        type: 'init',
        canvas: offscreen,
        cssW: size.w,
        cssH: size.h,
        dpr: devicePixelRatio(),
      },
      [offscreen],
    );
    report('start', reply.status);
    ready = await sessionIsOpen();
  } catch (error) {
    // The Worker reported a real failure (on WebKitGTK this is the `webgl2`
    // refusal). It is not papered over: the line keeps the step and the marker,
    // and the carrier is already gone (the canvas was transferred, so there is
    // nothing to fall back to without reloading the page).
    statusLine.textContent = `start failed: ${String(error)}  ${isolationText()}`;
    ready = false;
  }
  await reportWithDocument(statusLine.textContent);
}

async function boot() {
  const probe = await probeWorkerIsolation();
  if (probe.bridge) {
    carrier = { kind: 'worker', bridge: probe.bridge };
    await startIsolated();
    return;
  }
  carrier = { kind: 'main-thread', reason: probe.reason };
  console.log(`yacr: canvas isolation: ${isolationText()}`);
  await startMainThread();
}

boot().catch((error) => {
  statusLine.textContent = `start failed: ${String(error)}  ${isolationText()}`;
});
