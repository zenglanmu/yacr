#!/usr/bin/env node
/*
 * Headless browser smoke for the wasm CAD bundle (`apps/app-web/dist`).
 *
 * What it proves:
 *   - the served bundle loads (module + wasm) with no page error and no failed
 *     request for its own assets;
 *   - `start()` opens the compiled-in synthetic fixture and the status line says
 *     `start: ok` (never a silent blank canvas);
 *   - a real carrier is drawing: `target` is `html-canvas` or
 *     `offscreen-canvas`, the backing size is non-zero, and the document reports
 *     a non-zero `batches` (the prepared scene's resident batches) and `presented`
 *     (frames that reached the canvas) counter. `draws` is deliberately **not**
 *     required: the production page session's engine owns the draw submission
 *     count and does not publish it to this layer, so the field is `null` there —
 *     asserting a number would force the carrier to invent one.
 *
 * What it does NOT prove — and must never be read as:
 *   - anything about GPU hardware or driver behaviour: CI has no GPU and the
 *     Chromium here renders through SwiftShader, so the run is labelled
 *     `rendering=software`;
 *   - pixel/visual correctness: no reference image is compared. The screenshot
 *     is a debugging artefact (`--screenshot`), not visual acceptance;
 *   - performance, other browsers/engines, or a shipped web product.
 *
 * Worker isolation is a measured capability, not a requirement: Chromium can
 * render inside a Worker; an engine that cannot keeps the main-thread carrier and
 * says so (`isolation=main-thread (…reason…)`). Either carrier passes, but the
 * chosen one is printed so a silent fallback is visible.
 *
 * Usage:
 *   node scripts/web-smoke.cjs --url http://127.0.0.1:8090/
 *   node scripts/web-smoke.cjs --url … --screenshot /tmp/web-smoke.png [--timeout 90000]
 *
 * Requires `playwright` (and its Chromium) resolvable: `npm i playwright` locally,
 * or `NODE_PATH=<dir>/node_modules` in CI. The server is not started here — use
 * `python3 scripts/serve-web.py --directory apps/app-web/dist --port 8090`.
 */
'use strict';

const fs = require('node:fs');
const path = require('node:path');

const DEFAULT_URL = 'http://127.0.0.1:8090/';
const DEFAULT_TIMEOUT_MS = 90000;
const EXPECTED_LABEL = 'synthetic-four-lines.dwg';

function loadPlaywright() {
  try {
    return require('playwright');
  } catch (error) {
    const hint = process.env.PW_NODE_MODULES
      ? ` at ${path.join(process.env.PW_NODE_MODULES, 'playwright')}`
      : '';
    throw new Error(
      `cannot resolve 'playwright'${hint}: ${error.message}\n` +
        'install it (`npm install playwright`) or set NODE_PATH to a directory containing node_modules',
    );
  }
}

function parseArgs(argv) {
  const options = { url: DEFAULT_URL, timeout: DEFAULT_TIMEOUT_MS, screenshot: null };
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index];
    const value = argv[index + 1];
    if (flag === '--url' && value) {
      options.url = value;
      index += 1;
    } else if (flag === '--timeout' && value) {
      const parsed = Number(value);
      if (!Number.isFinite(parsed) || parsed <= 0) {
        throw new Error(`--timeout expects a positive number of milliseconds, got ${value}`);
      }
      options.timeout = parsed;
      index += 1;
    } else if (flag === '--screenshot' && value) {
      options.screenshot = value;
      index += 1;
    } else {
      throw new Error(`unknown argument: ${flag}`);
    }
  }
  return options;
}

/** One `key=value` token from the status line, or null. */
function token(text, key) {
  const match = new RegExp(`(?:^|\\s)${key}=([^\\s]+)`).exec(text);
  return match ? match[1] : null;
}

function fail(message, details) {
  const suffix = details && details.length ? `\n${details.join('\n')}` : '';
  throw new Error(`${message}${suffix}`);
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const { chromium } = loadPlaywright();

  const browser = await chromium.launch({
    headless: true,
    // CI has no GPU. SwiftShader is the only way to get WebGL2 there; Chrome 137+
    // requires the explicit opt-in flag instead of an implicit fallback.
    args: [
      '--use-gl=angle',
      '--use-angle=swiftshader',
      '--enable-unsafe-swiftshader',
      '--disable-dev-shm-usage',
    ],
  });

  const consoleLines = [];
  const pageErrors = [];
  const failedRequests = [];
  let page;

  try {
    const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    page = await context.newPage();
    page.on('console', (message) => consoleLines.push(`${message.type()}: ${message.text()}`));
    page.on('pageerror', (error) => pageErrors.push(String(error)));
    page.on('requestfailed', (request) => {
      // A favicon 404 is not an asset the bundle needs; everything else is.
      if (!request.url().endsWith('/favicon.ico')) {
        failedRequests.push(`${request.url()} (${request.failure()?.errorText ?? 'failed'})`);
      }
    });

    const response = await page.goto(options.url, {
      waitUntil: 'load',
      timeout: options.timeout,
    });
    if (response === null || !response.ok()) {
      fail(`the bundle URL did not load: ${options.url} -> ${response?.status() ?? 'no response'}`);
    }

    // `boot()` decides the carrier first (worker probe), then starts the session.
    await page.waitForFunction(
      () => {
        const line = document.getElementById('cad-status');
        return !!line && /^start: (ok|failed)/.test(line.textContent.trim());
      },
      undefined,
      { timeout: options.timeout },
    );

    const statusText = (await page.locator('#cad-status').textContent()).trim();
    if (!statusText.startsWith('start: ok')) {
      fail('the wasm session did not start', [`status line: ${statusText}`]);
    }
    if (pageErrors.length > 0) {
      fail('the page raised an uncaught error', pageErrors);
    }
    if (failedRequests.length > 0) {
      fail('the bundle failed to fetch one of its own assets', failedRequests);
    }

    const label = token(statusText, 'file');
    const target = token(statusText, 'target');
    const physical = token(statusText, 'physical');
    const isolation = /(?:^|\s)isolation=([^\n]+)/.exec(statusText)?.[1] ?? null;

    if (label !== EXPECTED_LABEL) {
      fail(`the synthetic fixture is not open (expected file=${EXPECTED_LABEL})`, [
        `status line: ${statusText}`,
      ]);
    }
    if (target !== 'html-canvas' && target !== 'offscreen-canvas') {
      fail(`no canvas carrier was reported (target=${target})`, [`status line: ${statusText}`]);
    }
    if (physical === null || !/^[1-9]\d*x[1-9]\d*$/.test(physical)) {
      fail(`the drawing surface has no backing size (physical=${physical})`, [
        `status line: ${statusText}`,
      ]);
    }
    if (isolation === null) {
      fail('the status line does not report which carrier/isolation is in use', [
        `status line: ${statusText}`,
      ]);
    }

    const measured = await page.evaluate(async () => {
      const docStatus = await window.__YACR_CANVAS__.document();
      const canvas = window.document.getElementById('cad-canvas');
      // A raw WebGL2 context on a scratch canvas is not a CAD session; it only
      // names the renderer so this run can be labelled software vs hardware.
      let renderer = null;
      const scratch = window.document.createElement('canvas');
      const gl = scratch.getContext('webgl2');
      if (gl) {
        const info = gl.getExtension('WEBGL_debug_renderer_info');
        renderer = info ? gl.getParameter(info.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER);
        gl.getExtension('WEBGL_lose_context')?.loseContext();
      }
      return {
        document: docStatus,
        canvas: { width: canvas.width, height: canvas.height },
        webgl2: gl !== null,
        renderer: renderer === null ? null : String(renderer),
      };
    });

    const doc = measured.document ?? {};
    if (doc.ok !== true) {
      fail('the document status is not ok', [`document: ${JSON.stringify(doc)}`]);
    }
    if (doc.label !== EXPECTED_LABEL) {
      fail(`the wasm document reports ${JSON.stringify(doc.label)} instead of ${EXPECTED_LABEL}`);
    }
    for (const field of ['batches', 'presented']) {
      if (!Number.isFinite(doc[field]) || doc[field] < 1) {
        fail(`the session reports no ${field} (${JSON.stringify(doc[field])})`, [
          `document: ${JSON.stringify(doc)}`,
        ]);
      }
    }
    if (!(measured.canvas.width > 0) || !(measured.canvas.height > 0)) {
      fail(`the canvas has no backing store (${measured.canvas.width}x${measured.canvas.height})`);
    }
    if (!measured.webgl2) {
      fail('a fresh canvas cannot get a WebGL2 context; the run does not represent the CAD path');
    }

    const [physicalWidth, physicalHeight] = physical.split('x').map(Number);
    if (measured.canvas.width !== physicalWidth || measured.canvas.height !== physicalHeight) {
      fail('the canvas backing store disagrees with the reported physical size', [
        `canvas=${measured.canvas.width}x${measured.canvas.height} physical=${physical}`,
      ]);
    }

    const software = /swiftshader|llvmpipe|softpipe|software/i.test(measured.renderer ?? '');
    const rendering = software ? 'software (SwiftShader/llvmpipe)' : `hardware (${measured.renderer})`;

    if (options.screenshot) {
      fs.mkdirSync(path.dirname(path.resolve(options.screenshot)), { recursive: true });
      await page.screenshot({ path: options.screenshot, fullPage: false });
    }

    console.log(
      '[probe] web-smoke ok ' +
        `file=${label} target=${target} isolation=${isolation} physical=${physical} ` +
        `batches=${doc.batches} presented=${doc.presented} ` +
        `draws=${doc.draws === undefined ? '-' : JSON.stringify(doc.draws)} ` +
        `gl_renderer=${JSON.stringify(measured.renderer ?? 'unknown')} rendering=${rendering}`,
    );
    if (software) {
      console.log(
        'note: this run used software rasterisation (no GPU on the CI runner); ' +
          'it is neither GPU nor pixel evidence.',
      );
    }
    if (options.screenshot) {
      console.log(`note: ${options.screenshot} is a diagnostic screenshot, not visual acceptance.`);
    }
    return 0;
  } catch (error) {
    if (page && options.screenshot) {
      try {
        await page.screenshot({ path: options.screenshot });
        console.error(`diagnostic screenshot: ${options.screenshot}`);
      } catch (screenshotError) {
        console.error(`diagnostic screenshot failed: ${screenshotError.message}`);
      }
    }
    console.error(`web-smoke FAILED: ${error.message}`);
    if (consoleLines.length > 0) {
      console.error('--- console ---');
      for (const line of consoleLines.slice(-40)) {
        console.error(line);
      }
    }
    if (pageErrors.length > 0) {
      console.error('--- page errors ---');
      for (const line of pageErrors) {
        console.error(line);
      }
    }
    return 1;
  } finally {
    await browser.close();
  }
}

main()
  .then((code) => {
    process.exitCode = code;
  })
  .catch((error) => {
    console.error(`web-smoke FAILED before the browser started: ${error.message}`);
    process.exitCode = 1;
  });
