/*
 * Module Worker entry for the wasm CAD session (`surfaceMode: dom-canvas`,
 * ADR 0024 stage ⑧).
 *
 * This Worker **owns the whole CAD session**: it instantiates its own wasm module,
 * imports the DWG, builds the display scene, renders into the `OffscreenCanvas` the
 * page transferred with `transferControlToOffscreen()`, and answers input commands
 * from `main.js` with the same JSON status the main-thread path returns. The page
 * holds no geometry, no camera and no second session — `main.js` does not even
 * instantiate the wasm module when it takes this path (`apps/app-web/src/lib.rs`
 * "Single owner, enforced").
 *
 * # Measured engine split (2026-10-24)
 *
 *   Chromium 141      WebGL2 on a worker-side `OffscreenCanvas` works (verified:
 *                     ANGLE/Intel, clear + readPixels correct) -> this path runs.
 *   WebKitGTK 2.52.6  `OffscreenCanvas.getContext('webgl2')` returns **null** in a
 *                     Worker (verified) -> worker rendering is impossible, the
 *                     capability probe below answers `isolation: main-thread` with
 *                     reason `no WebGL2 in worker`, and `main.js` falls back to the
 *                     verified main-thread path **and says so in the status line**.
 *
 * Nothing here guesses: the probe asks the engine the exact question the
 * measurement used (`worker_capability()` creates a 1x1 `OffscreenCanvas` and calls
 * `getContext('webgl2')` inside this Worker), so a WebGL2-less worker is reported
 * as such instead of painting a blank canvas.
 *
 * # Message contract (page -> worker)
 *
 *   {id, type: 'capability'}                              -> {capability: JSON string}
 *   {id, type: 'init', canvas, cssW, cssH, dpr}            -> {status: JSON string}
 *   {id, type: 'open', label, bytes, cssW, cssH, dpr}      -> {status: JSON string}
 *   {id, type: 'resize', cssW, cssH, dpr}                  -> {status: JSON string}
 *   {id, type: 'zoom', factor, x, y}                       -> {status: JSON string}
 *   {id, type: 'pan', dx, dy}                              -> {status: JSON string}
 *   {id, type: 'status'}                                   -> {status: JSON string}
 *   {id, type: 'document'}                                 -> {document: JSON string}
 *   {id, type: 'lastImportMs'}                             -> {importMs: number}
 *
 * Every reply echoes `id` and is either `{id, ok: true, ...}` or
 * `{id, ok: false, error}`. `canvas` must be transferred in the `init` message's
 * transfer list; `bytes` may be transferred in `open`'s.
 */

import init, {
  open_bytes_offscreen,
  start_offscreen,
  resize,
  zoom_at,
  pan,
  status,
  document as cadDocument,
  last_import_ms as lastImportMs,
  worker_capability,
} from './pkg/app_web.js';

/** The transferred canvas. Set once by `init`; the page must not use it again. */
let offscreen = null;

/**
 * The module's own initialisation. Kept as one settled promise so a failed start is
 * reported once per message instead of becoming an unhandled rejection.
 */
let bootError = null;
const booted = init().catch((error) => {
  bootError = error;
});

/**
 * Resolve after the wasm module is usable, or throw the initialisation failure.
 *
 * A worker whose module cannot load (a 404, a syntax error, a CSP refusal) must
 * never be mistaken for "no WebGL2": the caller reports the actual error.
 */
async function moduleReady() {
  await booted;
  if (bootError !== null) {
    throw new Error(`wasm module failed to initialise in the worker: ${String(bootError)}`);
  }
}

/** Reply to one request; `payload` is `{status}`/`{document}`/`{capability}`/... */
function reply(id, payload) {
  self.postMessage({ id, ok: true, ...payload });
}

/** Report one failed request. The message names the step, never a bare "failed". */
function replyError(id, error) {
  self.postMessage({ id, ok: false, error: String(error) });
}

/**
 * Run one command. Each branch is a thin call into the wasm exports — the session,
 * budgets, chunked upload and frame path live in Rust and are shared with the
 * main-thread carrier.
 */
async function handle(message) {
  await moduleReady();
  switch (message.type) {
    case 'capability':
      // Answered in this Worker: the WebGL2 question only means something here.
      reply(message.id, { capability: worker_capability() });
      return;
    case 'init': {
      const { canvas, cssW, cssH, dpr } = message;
      if (typeof OffscreenCanvas === 'undefined' || !(canvas instanceof OffscreenCanvas)) {
        throw new Error('init needs an OffscreenCanvas transferred from the page');
      }
      offscreen = canvas;
      // `start_offscreen` is the isolated sibling of `start`: same fixture, same
      // budgets, same chunked upload. It refuses (step "webgl2") when this engine
      // has no worker-side WebGL2, which is the WebKitGTK case.
      reply(message.id, {
        status: await start_offscreen(offscreen, cssW, cssH, dpr),
        isolation: 'worker',
      });
      return;
    }
    case 'open': {
      if (offscreen === null) {
        throw new Error('open before init: the worker has no OffscreenCanvas yet');
      }
      const bytes = new Uint8Array(message.bytes);
      reply(message.id, {
        status: await open_bytes_offscreen(
          offscreen,
          message.label,
          message.cssW,
          message.cssH,
          message.dpr,
          bytes,
        ),
      });
      return;
    }
    // The remaining commands never touch a canvas handle, so the worker calls the
    // very same exports the page would (one session implementation, two carriers).
    case 'resize':
      reply(message.id, { status: resize(message.cssW, message.cssH, message.dpr) });
      return;
    case 'zoom':
      reply(message.id, { status: zoom_at(message.factor, message.x, message.y) });
      return;
    case 'pan':
      reply(message.id, { status: pan(message.dx, message.dy) });
      return;
    case 'status':
      reply(message.id, { status: status() });
      return;
    case 'document':
      reply(message.id, { document: cadDocument() });
      return;
    case 'lastImportMs':
      reply(message.id, { importMs: lastImportMs() });
      return;
    default:
      throw new Error(`unknown worker command: ${String(message.type)}`);
  }
}

self.addEventListener('message', (event) => {
  const message = event.data;
  if (!message || typeof message.type !== 'string' || typeof message.id !== 'number') {
    return;
  }
  handle(message).catch((error) => replyError(message.id, error));
});
