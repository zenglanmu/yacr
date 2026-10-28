#!/usr/bin/env node
/*
 * The **command** acceptance case of `scripts/web-ui-smoke.cjs`: the four commands
 * the browser/Android carrier really executes, driven through the real chrome.
 *
 * Split into its own file because `scripts/check-file-size.py` bounds every code
 * file at 1000 lines; this module owns one case and nothing else. It is not a
 * second harness: `web-ui-smoke.cjs` passes its page helpers in, so the browser
 * launch, the server, the reporting and every other assertion stay where they were.
 *
 * What the case proves (and only this):
 *   - the session advertises `FIT`/`ZOOMIN`/`ZOOMOUT` and reports `canFit: true`
 *     with a document;
 *   - `ZOOMIN` typed into `#cad-command` + Enter changes the live camera scale by
 *     exactly one step (`1/1.25`), with no error banner;
 *   - `FIT` returns the scale to the framing the zoom moved away from (the same fit
 *     rule the open used, run again from the retained bounds), with no error banner;
 *   - `ZOOMOUT` applies the reciprocal step;
 *   - `OPEN` opens the host file picker (Playwright's `filechooser`), the chosen
 *     DWG's bytes reach the session's `open_bytes`, and the session then reports that
 *     file as its `documentLabel` with a larger `importCount` — the document really
 *     was replaced.
 *
 * What it does NOT prove: pixels, GPU behaviour, performance or a shipped product.
 * The run is SwiftShader (software rasterisation), exactly like the rest of the
 * smoke suite.
 */
'use strict';

const path = require('node:path');

const { measureCanvasPixels } = require('./canvas-pixels.cjs');

const REPO_ROOT = path.resolve(__dirname, '..');

/** The desktop viewport this case runs at (the command line exists there). */
const COMMAND_VIEWPORT = Object.freeze({ width: 1280, height: 800 });

/**
 * The zoom step the `ZOOMIN` selector applies
 * (`cad_ui_core::session::ZOOM_IN_FACTOR`), mirrored here only to predict the
 * measured scale.
 */
const ZOOM_IN_FACTOR = 1.25;

/**
 * The drawing the `OPEN` case picks, as an **absolute** path (Playwright's
 * `setInputFiles` never resolves relative paths).
 *
 * `fixtures/dwg/synthetic-four-lines.dwg` is the only DWG in the repository
 * (20 755 bytes, four LINE entities) and the same synthetic fixture the wasm
 * `start` loads. That is deliberate: the case then proves the *command* path
 * (picker → bytes → `open_bytes` → replaced document) without depending on an
 * external sample, and `importCount` is what distinguishes the re-open of the same
 * bytes from no open at all.
 */
const OPEN_DWG = path.join(REPO_ROOT, 'fixtures', 'dwg', 'synthetic-four-lines.dwg');

/** The label the session reports after opening {@link OPEN_DWG}. */
const OPEN_DWG_LABEL = path.basename(OPEN_DWG);

const FIELD_ID = 'cad-command';

/** The live session's own `shell_status()` snapshot, parsed (or `null`). */
function readShellStatus(page) {
  return page.evaluate(() => {
    const module = window.__YACR_CANVAS_MODULE__;
    if (!module || typeof module.shell_status !== 'function') return null;
    try {
      return JSON.parse(module.shell_status());
    } catch (error) {
      return { unreadable: String(error) };
    }
  });
}

/** The camera scale the live session reports, or `null` when it reports none. */
async function readCameraScale(page) {
  const status = await readShellStatus(page);
  const scale = status?.camera?.scale;
  return typeof scale === 'number' && Number.isFinite(scale) ? scale : null;
}

/**
 * Wait until the session's camera scale is `target` ± `tolerance`.
 *
 * `target === null` accepts the first finite scale (used to read the settled
 * value). The predicate runs inside the page against the session's own
 * `shell_status()`; nothing in it is evaluated from a string, because the page CSP
 * has no `'unsafe-eval'`.
 */
async function waitForScale(page, target, tolerance, timeout) {
  const selectors = { target, tolerance };
  try {
    await page.waitForFunction(args => {
      const module = window.__YACR_CANVAS_MODULE__;
      if (!module || typeof module.shell_status !== 'function') return false;
      let status;
      try {
        status = JSON.parse(module.shell_status());
      } catch (error) {
        return false;
      }
      const scale = status?.camera?.scale;
      if (typeof scale !== 'number' || !Number.isFinite(scale)) return false;
      return args.target === null || Math.abs(scale - args.target) <= args.tolerance;
    }, selectors, { timeout });
  } catch (error) {
    return null;
  }
  return readCameraScale(page);
}

/** Type `text` into the command line and press Enter; returns the echoed draft. */
async function typeCommand(page, options, text) {
  const field = page.locator(`#${FIELD_ID}`);
  await field.waitFor({ state: 'visible', timeout: options.timeout });
  // A submit is in flight while the field is disabled, and the shell drops an
  // Enter that arrives then (its in-flight guard), so wait for the previous one.
  await page.waitForFunction(id => {
    const input = document.getElementById(id);
    return !!input && !input.disabled;
  }, FIELD_ID, { timeout: options.timeout });
  await field.fill(text);
  const echo = await field.inputValue();
  await field.press('Enter');
  return echo;
}

/** `|a - b| / max(|a|, |b|, 1)` — a relative difference that tolerates tiny doubles. */
function relativeDifference(a, b) {
  return Math.abs(a - b) / Math.max(Math.abs(a), Math.abs(b), 1);
}

/**
 * One labelled case: the four commands, through the chrome.
 *
 * `helpers` are the smoke's own page utilities, so this module owns no second
 * browser harness (see the header).
 */
async function runCommandCase(browser, options, helpers) {
  const { record, newPage, waitForSession, readErrorAlerts, checkNoErrorBanner, takeScreenshot } = helpers;
  const checks = [];
  const bucket = { pageErrors: [], failedRequests: [], consoleLines: [] };
  const name = 'commands ZOOMIN / FIT / ZOOMOUT / OPEN';
  const screenshots = [];
  const observations = [];
  const container = await browser.newContext({ viewport: { ...COMMAND_VIEWPORT }, deviceScaleFactor: 1 });
  const page = await newPage(container, bucket);
  let renderer = { webgl2: false, renderer: null };
  try {
    const response = await page.goto(options.url, { waitUntil: 'load', timeout: options.timeout });
    if (response === null || !response.ok()) {
      throw new Error(`the bundle URL did not load: ${options.url} -> ${response?.status() ?? 'no response'}`);
    }
    await waitForSession(page, options.timeout);

    const advertisement = await page.evaluate(() => {
      const module = window.__YACR_CANVAS_MODULE__;
      return module && typeof module.shell_commands === 'function' ? module.shell_commands() : null;
    });
    const commands = advertisement === null ? null : JSON.parse(advertisement);
    record(
      checks,
      Array.isArray(commands) && ['FIT', 'ZOOMIN', 'ZOOMOUT'].every(selector => commands.includes(selector)),
      'the session advertises the three camera selectors it executes',
      commands,
    );
    const before = await readShellStatus(page);
    record(
      checks,
      before !== null && before.canFit === true && before.documentLabel !== null,
      'the live session reports canFit and a document (the gates FIT needs)',
      { canFit: before?.canFit, documentLabel: before?.documentLabel, entityCount: before?.entityCount },
    );

    // `FIT` first: it establishes the framing the current viewport really produces
    // (the start dock collapses after the first status, which resizes the canvas
    // without refitting), so the later comparison is like with like.
    const fitEcho = await typeCommand(page, options, 'FIT');
    record(checks, fitEcho === 'FIT', 'the command field echoes the typed selector', { typed: 'FIT', echoed: fitEcho });
    const fitted = await waitForScale(page, null, 0, options.timeout);
    record(checks, fitted !== null, 'FIT leaves the session with a usable camera', { scale: fitted });

    // ZOOMIN: one 1.25x step in, about the viewport centre.
    await typeCommand(page, options, 'ZOOMIN');
    const expectedIn = fitted === null ? null : fitted / ZOOM_IN_FACTOR;
    const zoomed = expectedIn === null
      ? null
      : await waitForScale(page, expectedIn, Math.abs(expectedIn) * 1e-9, options.timeout);
    record(
      checks,
      zoomed !== null && relativeDifference(zoomed, expectedIn) <= 1e-9,
      'ZOOMIN changes the live camera scale by exactly one step',
      { before: fitted, after: zoomed, expected: expectedIn },
    );
    const alertsAfterZoom = await readErrorAlerts(page);
    checkNoErrorBanner(checks, alertsAfterZoom, 'ZOOMIN');

    // FIT again: a refit must undo the zoom step (same rule, same retained bounds).
    await typeCommand(page, options, 'FIT');
    const refitted = fitted === null
      ? null
      : await waitForScale(page, fitted, Math.abs(fitted) * 1e-9, options.timeout);
    record(
      checks,
      refitted !== null && relativeDifference(refitted, fitted) <= 1e-9,
      'FIT returns the camera to the framing the zoom moved away from',
      { zoomed, refitted, fitted },
    );
    const alertsAfterFit = await readErrorAlerts(page);
    checkNoErrorBanner(checks, alertsAfterFit, 'FIT');

    // ZOOMOUT: the reciprocal step of ZOOMIN.
    await typeCommand(page, options, 'ZOOMOUT');
    const expectedOut = refitted === null ? null : refitted * ZOOM_IN_FACTOR;
    const zoomedOut = expectedOut === null
      ? null
      : await waitForScale(page, expectedOut, Math.abs(expectedOut) * 1e-9, options.timeout);
    record(
      checks,
      zoomedOut !== null && relativeDifference(zoomedOut, expectedOut) <= 1e-9,
      'ZOOMOUT applies the reciprocal step',
      { before: refitted, after: zoomedOut, expected: expectedOut },
    );
    const alertsAfterZoomOut = await readErrorAlerts(page);
    checkNoErrorBanner(checks, alertsAfterZoomOut, 'ZOOMOUT');

    // OPEN: the host's file picker, then the session's own `open_bytes`.
    const importCountBefore = (await readShellStatus(page))?.importCount ?? null;
    const chooserPromise = page.waitForEvent('filechooser', { timeout: options.timeout });
    await typeCommand(page, options, 'OPEN');
    let chooser = null;
    try {
      chooser = await chooserPromise;
    } catch (error) {
      observations.push(`no file chooser: ${error.message}`);
    }
    record(checks, chooser !== null, 'OPEN opens the host file picker', chooser === null ? null : OPEN_DWG);
    if (chooser !== null) {
      await chooser.setFiles(OPEN_DWG);
      try {
        await page.waitForFunction(label => {
          const module = window.__YACR_CANVAS_MODULE__;
          if (!module || typeof module.shell_status !== 'function') return false;
          try {
            return JSON.parse(module.shell_status()).documentLabel === label;
          } catch (error) {
            return false;
          }
        }, OPEN_DWG_LABEL, { timeout: options.timeout });
      } catch (error) {
        observations.push(`documentLabel did not settle: ${error.message}`);
      }
      const opened = await readShellStatus(page);
      record(
        checks,
        opened !== null && opened.documentLabel === OPEN_DWG_LABEL,
        'the session reports the opened file as its document label',
        { expected: OPEN_DWG_LABEL, label: opened?.documentLabel },
      );
      record(
        checks,
        typeof opened?.importCount === 'number' && typeof importCountBefore === 'number'
          && opened.importCount > importCountBefore,
        'the open installed another document (importCount grew)',
        { before: importCountBefore, after: opened?.importCount },
      );
    }
    const alertsAfterOpen = await readErrorAlerts(page);
    checkNoErrorBanner(checks, alertsAfterOpen, 'OPEN');
    for (const text of [...alertsAfterZoom, ...alertsAfterFit, ...alertsAfterZoomOut, ...alertsAfterOpen]) {
      observations.push(`visible alert: ${text}`);
    }

    record(checks, bucket.pageErrors.length === 0, 'no uncaught page error during the commands', bucket.pageErrors);
    renderer = await page.evaluate(() => {
      const scratch = document.createElement('canvas');
      const gl = scratch.getContext('webgl2');
      if (!gl) return { webgl2: false, renderer: null };
      const info = gl.getExtension('WEBGL_debug_renderer_info');
      const reported = info ? gl.getParameter(info.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER);
      gl.getExtension('WEBGL_lose_context')?.loseContext();
      return { webgl2: true, renderer: reported === null ? null : String(reported) };
    });
    const pixels = await measureCanvasPixels(page, options.evidence, 'presented-commands');
    record(checks, pixels.ok, 'the CAD canvas still shows a frame after the commands', pixels.detail);
    if (options.evidence) {
      const shot = await takeScreenshot(page, options.evidence, 'commands-open.png');
      screenshots.push(shot);
      record(checks, shot.bytes > 0, 'screenshot commands-open.png written', shot.file);
    }
  } catch (error) {
    record(checks, false, 'command case ran to completion', error.message);
  } finally {
    await container.close();
  }
  return { name, checks, bucket, renderer, screenshots, observations };
}

module.exports = { runCommandCase, OPEN_DWG, OPEN_DWG_LABEL, ZOOM_IN_FACTOR, COMMAND_VIEWPORT };
