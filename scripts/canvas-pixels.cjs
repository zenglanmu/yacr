#!/usr/bin/env node
/*
 * The **presented** CAD pixels of a `dom-canvas` host, measured from a screenshot.
 *
 * Why this exists: a wasm CAD session can be completely healthy — `status().ok`
 * true, `presented` growing, the canvas backing store exactly `CSS x DPR`, no
 * error anywhere — while **nothing is on screen**. A present that draws no
 * fragments still reports success, so no session API answer can tell you whether
 * a frame is visible. This is the measured Android WebView defect
 * (`docs/validation-android-present-2026-10-28.md`): wgpu-hal's sRGB present
 * shader draws its fullscreen triangle from an array indexed by `gl_VertexID`,
 * and on that GLES implementation every element of a dynamically indexed array
 * reads as zero, so the triangle is degenerate and the canvas stays blank.
 *
 * Therefore the judge is the composited pixels: this takes a `page.screenshot`
 * of the canvas' box (what a user actually sees) and decodes it back in the page.
 * `readPixels` on the default framebuffer is deliberately **not** used — with
 * `preserveDrawingBuffer: false` it reads an already-cleared buffer and would
 * report "blank" for a healthy canvas.
 *
 * The verdict is "not uniform": a real frame is a clear colour plus geometry.
 * Colours are quantised to 5 bits per channel so that antialiasing and DPR
 * scaling cannot be mistaken for content, and the threshold is tiny (0.05 % of
 * the box) because a few thin CAD lines are few pixels. This asserts that
 * *something* was presented — never that what was presented is correct (colour
 * space, camera, geometry and antialiasing are not judged here).
 *
 * Usage (from `web-ui-smoke.cjs`):
 *   const { measureCanvasPixels } = require('./canvas-pixels.cjs');
 *   const measured = await measureCanvasPixels(page, evidenceDir, 'name');
 *   record(checks, measured.ok, 'label', measured.detail);
 */

'use strict';

const fs = require('node:fs');
const path = require('node:path');

/** Storage key of the canvas the wasm session presents on (`canvas-host.ts`). */
const CANVAS_ID = 'cad-canvas';

/** Below this share of non-modal pixels the band counts as uniform (blank). */
const MIN_NON_MODAL_SHARE = 0.0005;

/**
 * Measure the composited CAD canvas of `page`.
 *
 * `evidence` is a directory or `null`; when set, the exact screenshot that was
 * measured is written there as `canvas-<name>.png` so a failure can be looked at.
 *
 * Resolves to `{ ok, detail }` and never throws for "no canvas": a missing or
 * unmeasurable canvas is a failed measurement (`ok: false`) with the reason in
 * `detail`, which is what the caller records.
 */
async function measureCanvasPixels(page, evidence, name) {
  const rect = await page.evaluate(id => {
    const canvas = document.getElementById(id);
    if (!canvas) return null;
    const box = canvas.getBoundingClientRect();
    return {
      x: Math.round(box.x + window.scrollX),
      y: Math.round(box.y + window.scrollY),
      width: Math.round(box.width),
      height: Math.round(box.height),
    };
  }, CANVAS_ID);
  if (rect === null || rect.width < 4 || rect.height < 4) {
    return { ok: false, detail: { reason: 'no measurable canvas box', rect } };
  }
  const shot = await page.screenshot({ clip: rect });
  if (evidence) fs.writeFileSync(path.join(evidence, `canvas-${name}.png`), shot);
  const stats = await page.evaluate(async base64 => {
    const bytes = Uint8Array.from(atob(base64), character => character.charCodeAt(0));
    const bitmap = await createImageBitmap(new Blob([bytes], { type: 'image/png' }));
    const scratch = new OffscreenCanvas(bitmap.width, bitmap.height);
    const context = scratch.getContext('2d', { willReadFrequently: true });
    context.drawImage(bitmap, 0, 0);
    const data = context.getImageData(0, 0, bitmap.width, bitmap.height).data;
    const counts = new Map();
    let total = 0;
    for (let index = 0; index < data.length; index += 4) {
      // 5 bits per channel: antialiasing and DPR scaling must not read as
      // "lots of colours"; the question is only whether a picture is there.
      const key = ((data[index] >> 3) << 10) | ((data[index + 1] >> 3) << 5) | (data[index + 2] >> 3);
      counts.set(key, (counts.get(key) ?? 0) + 1);
      total += 1;
    }
    let modal = 0;
    let modalKey = 0;
    for (const [key, count] of counts) {
      if (count > modal) {
        modal = count;
        modalKey = key;
      }
    }
    return {
      width: bitmap.width,
      height: bitmap.height,
      pixels: total,
      distinctColours: counts.size,
      modalColours: {
        r: ((modalKey >> 10) & 31) << 3,
        g: ((modalKey >> 5) & 31) << 3,
        b: (modalKey & 31) << 3,
      },
      modalShare: Number((modal / total).toFixed(4)),
      nonModalPixels: total - modal,
    };
  }, shot.toString('base64'));
  const nonModalShare = stats.pixels === 0 ? 0 : stats.nonModalPixels / stats.pixels;
  const uniform = stats.distinctColours <= 1 || nonModalShare < MIN_NON_MODAL_SHARE;
  return {
    ok: !uniform,
    detail: {
      ...stats,
      nonModalShare: Number(nonModalShare.toFixed(4)),
      minNonModalShare: MIN_NON_MODAL_SHARE,
      uniform,
    },
  };
}

module.exports = { measureCanvasPixels, MIN_NON_MODAL_SHARE, CANVAS_ID };
