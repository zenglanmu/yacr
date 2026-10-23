/*
 * Functional shell for the wasm CAD session (`surfaceMode: dom-canvas`).
 *
 * It is the browser sibling of the Linux host page: no framework, no bundler,
 * no second document. All navigation goes to the wasm exports; this file only
 * measures the canvas, forwards pointer/wheel input in CSS pixels and prints the
 * JSON status each call returns.
 *
 * Serve this directory over HTTP with the wasm-bindgen `web` output in `./pkg/`:
 *   cargo build -p app-web --target wasm32-unknown-unknown --release
 *   wasm-bindgen --target web --out-dir apps/app-web/web/pkg \
 *       target/wasm32-unknown-unknown/release/app_web.wasm
 */

import init, { start, resize, zoom_at, pan } from './pkg/app_web.js';

const canvas = document.getElementById('cad-canvas');
const statusLine = document.getElementById('cad-status');

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
}

/** Run one wasm export, reporting either its status or its rejection. */
function run(label, action) {
  try {
    report(label, action());
  } catch (error) {
    // The wasm exports reject with the message that names the failing step.
    statusLine.textContent = `${label} failed: ${String(error)}`;
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

async function boot() {
  await init();
  const size = cssSize();
  // `start` reports its own status (including a failure with the failing step
  // and, for a missing WebGL2 context, the marker
  // `yacr: unsupported-runtime: webgl2`). A real host maps that marker to its
  // explicit refusal instead of showing a blank canvas.
  report('start', await start('cad-canvas', size.w, size.h, devicePixelRatio()));
  ready = true;
}

boot().catch((error) => {
  statusLine.textContent = `start failed: ${String(error)}`;
});
