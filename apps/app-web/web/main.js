/*
 * Functional shell for the wasm CAD session (`surfaceMode: dom-canvas`).
 *
 * It is the browser sibling of the Linux host page: no framework, no bundler,
 * no second document. All navigation goes to the wasm exports; this file only
 * measures the canvas, forwards pointer/wheel input in CSS pixels, opens local
 * DWG files through the File API and prints the JSON status each call returns.
 *
 * The wasm module is the **only** CAD session (ADR 0024 D2): this page holds no
 * geometry, no scene and no camera state of its own.
 *
 * Serve this directory over HTTP with the wasm-bindgen `web` output in `./pkg/`:
 *   cargo build -p app-web --target wasm32-unknown-unknown --release
 *   wasm-bindgen --target web --out-dir apps/app-web/web/pkg \
 *       target/wasm32-unknown-unknown/release/app_web.wasm
 */

import init, {
  start,
  open_bytes,
  resize,
  zoom_at,
  pan,
  document as cadDocument,
  last_import_ms as lastImportMs,
} from './pkg/app_web.js';

const canvas = document.getElementById('cad-canvas');
const statusLine = document.getElementById('cad-status');
const fileInput = document.getElementById('cad-file');
const fileName = document.getElementById('cad-file-name');

/** One wheel notch, matching the desktop session's ZOOM_IN_FACTOR. */
const ZOOM_STEP = 1.25;
/** A DOM wheel notch is 120 px in deltaMode 0 (2 = "pixels" in practice). */
const WHEEL_NOTCH_PX = 120;

/** Live session is required by every navigation export. */
let ready = false;
/** Active drag: pointer id plus the last canvas-local CSS position. */
let drag = null;
/** A resize request already scheduled for the next animation frame. */
let resizePending = false;

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

/** Print one status object (or one rejection) on the status line. */
function report(label, json) {
  let parsed;
  try {
    parsed = JSON.parse(json);
  } catch (error) {
    statusLine.textContent = `${label}: unreadable status (${String(error)}): ${json}`;
    return;
  }
  const parts = [
    `${label}: ${parsed.ok ? 'ok' : 'FAILED'}`,
    `file=${parsed.label ?? '-'}`,
    `bytes=${parsed.bytes ?? 0}`,
    `backend=${parsed.backend ?? '-'}`,
    `format=${parsed.format ?? '-'}`,
    `physical=${Array.isArray(parsed.physical) ? parsed.physical.join('x') : '-'}`,
    `batches=${parsed.batches}`,
    `draws=${parsed.draws}`,
    `presented=${parsed.presented}`,
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
  if (parsed.label) {
    showLabel(parsed.label);
  }
}

/**
 * The evidence line for one document: the file the session actually holds, its
 * source size, and the measured import duration / wasm heap.
 *
 * `wasm_heap_bytes` is the module's own linear memory, **not** process RSS, and
 * `import_ms` is one measurement of the last open — neither is a performance
 * claim.
 */
function documentSummary() {
  try {
    const doc = JSON.parse(cadDocument());
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
    ].join('  ');
  } catch (error) {
    return `document: unreadable (${String(error)})`;
  }
}

/** Run one synchronous wasm export, reporting either its status or its rejection. */
function run(label, action) {
  try {
    report(label, action());
  } catch (error) {
    // The wasm exports reject with the message that names the failing step.
    statusLine.textContent = `${label} failed: ${String(error)}`;
  }
}

/** Whether the wasm session currently holds a file (`document().label`). */
function sessionIsOpen() {
  try {
    return JSON.parse(cadDocument()).label !== null;
  } catch (error) {
    return false;
  }
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
    run('resize', () => resize(size.w, size.h, devicePixelRatio()));
  });
}

/**
 * Open one local DWG in the wasm session.
 *
 * `arrayBuffer()` is the File API read; the bytes cross into wasm through
 * `open_bytes` and nothing is kept on this side. A rejected file (`step` `import`
 * or `empty-scene`) leaves the previous session open — the status line reports
 * the failure and the document line still names the file on screen.
 */
async function openFile(file) {
  const size = cssSize();
  let line;
  fileInput.disabled = true;
  try {
    const buffer = await file.arrayBuffer();
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
  } catch (error) {
    line = `open ${file.name} failed: ${String(error)}`;
    console.error(line);
    // A rejected file leaves the previous session open; keep navigating it.
    ready = sessionIsOpen();
  } finally {
    fileInput.disabled = false;
  }
  // Measurement surfaces for the evidence run: the raw values, not a rendering
  // of them. `lastImportMs()` is negative until one open completed.
  console.log(documentSummary());
  console.log(`last_import_ms=${lastImportMs()}`);
  statusLine.textContent = `${line}\n${documentSummary()}`;
}

fileInput.addEventListener('change', () => {
  const file = fileInput.files && fileInput.files[0];
  if (!file) {
    return;
  }
  // The `File` stays readable after this, and clearing the input lets a second
  // selection of the *same* file fire `change` again (reopening for evidence).
  fileInput.value = '';
  openFile(file);
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
    run('pan', () => pan(dx, dy));
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
    run('zoom', () => zoom_at(factor, point.x, point.y));
  },
  { passive: false },
);

window.addEventListener('resize', scheduleResize);

/**
 * Measurement handle for the evidence run (console access): the two measurement
 * exports plus the document summary this page already prints. Not a product API;
 * it exposes no geometry and no second session.
 */
window.__YACR_CANVAS__ = {
  document: cadDocument,
  lastImportMs,
  summary: documentSummary,
};

async function boot() {
  await init();
  const size = cssSize();
  // `start` loads the compiled-in synthetic fixture and reports its own status
  // (including a failure with the failing step and, for a missing WebGL2
  // context, the marker `yacr: unsupported-runtime: webgl2`). A real host maps
  // that marker to its explicit refusal instead of showing a blank canvas.
  // Opening a local file replaces the session through `open_bytes`.
  try {
    const status = await start('cad-canvas', size.w, size.h, devicePixelRatio());
    report('start', status);
    ready = sessionIsOpen();
  } catch (error) {
    statusLine.textContent = `start failed: ${String(error)}`;
    ready = false;
  }
  statusLine.textContent = `${statusLine.textContent}\n${documentSummary()}`;
}

boot().catch((error) => {
  statusLine.textContent = `start failed: ${String(error)}`;
});
