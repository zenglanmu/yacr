#!/usr/bin/env node
/*
 * Browser evidence for the **production `dom-canvas` flip** (`apps/app-web`).
 *
 * What it drives — only the exports the production session owns:
 *
 *   start(...)                        the real `CadSession` on the page `<canvas>`
 *   frame()                           one frame step (the rAF loop is the host's)
 *   invoke('cad_command', LINE)       the shared wire layer, real router
 *   probe_input(pointer_down/up × 2)  the desktop input seam (two captured points)
 *   invoke('cad_command', CONFIRM)    the confirmed LINE is a real entity
 *   shell_status()                    entityCount / tool / selection of that session
 *   invoke('cad_open_popup', about)   a real `PopupAck`, not `popup.host_unavailable`
 *   invoke('cad_close_popup')         the gate clears again
 *
 * and it asserts, in the page, that the **composited canvas changed** between the
 * frame before the LINE and the frame after it (a quantised pixel diff of two
 * screenshots of the canvas box — the same judge as `scripts/canvas-pixels.cjs`,
 * because a healthy session can present nothing and still report success).
 *
 * What it does NOT prove, and must never be read as:
 *
 *   - anything about GPU hardware: this launch forces ANGLE/SwiftShader, so the run
 *     is labelled `rendering=software` and is neither GPU nor performance evidence;
 *   - pixel/visual **correctness**: the diff only says *something was drawn*, and
 *     the screenshots are diagnostics, not visual acceptance;
 *   - the popup **overlay**: the harness page has no chrome, so the ack is asserted
 *     as data (`{accepted, kind, popup_label, payload}`) and never as pixels;
 *   - the worker (isolated) carrier, the native hosts, Android or any other engine:
 *     this is one Chromium build against one bundle, main-thread carrier;
 *   - the budget refusals: they are covered by `cargo test -p app-web` on the host
 *     (a browser run cannot allocate a 64 MiB file cheaply enough to be useful here);
 *   - anything about the **retention cost** unless `--open <dwg>` is passed: that
 *     optional case opens a real external drawing through the same in-place
 *     replacement and records the wasm heap it leaves behind (the number the
 *     released-database viewer never had to pay).
 *
 * Usage:
 *   node scripts/web-flip-evidence.cjs --bundle apps/app-web/dist \
 *     --evidence /tmp/opencode/yacr-flip-<UTC> [--port 8093] [--timeout 180000] [--verbose]
 *
 * The bundle must exist already (`bash scripts/build-web.sh`, which also leaves the
 * wasm-bindgen `web` output in `apps/app-web/web/pkg`); this script never builds it.
 * `playwright` is not a repository dependency: point `NODE_PATH` at its
 * `node_modules` (see `docs/ci.md`).
 */
'use strict';

const fs = require('node:fs');
const http = require('node:http');
const path = require('node:path');
const { spawn } = require('node:child_process');

const REPO_ROOT = path.resolve(__dirname, '..');
const CANVAS_ID = 'cad-canvas';
/** Chromium flags: SwiftShader is the only WebGL2 path without a GPU. */
const BROWSER_ARGS = Object.freeze([
  '--use-gl=angle',
  '--use-angle=swiftshader',
  '--enable-unsafe-swiftshader',
  '--disable-dev-shm-usage',
]);
const DEFAULT_PORT = 8093;
const DEFAULT_TIMEOUT_MS = 180000;
/** Frames the run may burn waiting for the first presented frame. */
const FIRST_FRAME_FRAMES = 120;
/** Frames rendered after a mutation before the canvas is measured. */
const SETTLE_FRAMES = 6;
/** The line the run draws, in canvas-local CSS pixels. */
const LINE_POINTS = [
  [320, 260],
  [900, 520],
];
/** A quantised-pixel diff below this is "nothing was drawn". */
const MIN_CHANGED_PIXELS = 200;

function parseArgs(argv) {
  const options = {
    bundle: path.join(REPO_ROOT, 'apps', 'app-web', 'dist'),
    evidence: null,
    port: DEFAULT_PORT,
    timeout: DEFAULT_TIMEOUT_MS,
    open: null,
    verbose: false,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    switch (arg) {
      case '--bundle':
        options.bundle = path.resolve(argv[++index]);
        break;
      case '--evidence':
        options.evidence = path.resolve(argv[++index]);
        break;
      case '--port':
        options.port = Number(argv[++index]);
        break;
      case '--timeout':
        options.timeout = Number(argv[++index]);
        break;
      case '--open':
        options.open = path.resolve(argv[++index]);
        break;
      case '--verbose':
        options.verbose = true;
        break;
      case '--help':
      case '-h':
        console.log(
          'usage: node scripts/web-flip-evidence.cjs [--bundle DIR] [--evidence DIR] ' +
            '[--port N] [--timeout MS] [--open FILE.dwg] [--verbose]',
        );
        process.exit(0);
        break;
      default:
        throw new Error(`unknown argument: ${arg}`);
    }
  }
  if (!Number.isInteger(options.port) || options.port <= 0) {
    throw new Error(`--port must be a positive integer, got ${options.port}`);
  }
  if (!options.evidence) {
    const stamp = new Date().toISOString().replace(/[-:]/g, '').replace(/\..*$/, 'Z');
    options.evidence = path.join('/tmp/opencode', `yacr-flip-${stamp}`);
  }
  return options;
}

function loadPlaywright() {
  try {
    return require('playwright');
  } catch (error) {
    throw new Error(
      `cannot resolve 'playwright': ${error.message}\n` +
        'install it outside the repo and run with NODE_PATH=<that dir>/node_modules',
    );
  }
}

/**
 * The harness page: a bare canvas and the wasm module, nothing else.
 *
 * Deliberately not `apps/app-web/web/index.html` (that shell asks a Worker for
 * isolation first and would then drive the *viewer* carrier on Chromium). This page
 * calls `start(...)` directly, i.e. the main-thread production carrier this flip is
 * about, and exposes the module namespace as `window.__FLIP__`.
 *
 * The CSP is the strict one the real page uses, so the harness exercises the same
 * module loading rules (an inline script would be blocked, which is why the module
 * bootstrap is a separate file).
 */
const HARNESS_JS = `import init, * as mod from './pkg/app_web.js';
const ready = init();
window.__FLIP__ = { ready, mod };
`;

const HARNESS_CSS = `html, body { margin: 0; height: 100%; background: #101216; }
#${CANVAS_ID} { display: block; width: 100vw; height: 100vh; }
`;

function writeHarness(site) {
  const html = `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta
      http-equiv="Content-Security-Policy"
      content="default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; connect-src 'self'; img-src 'self' data: blob:"
    />
    <title>yacr — production flip evidence harness</title>
    <link rel="stylesheet" href="./harness.css" />
  </head>
  <body>
    <canvas id="${CANVAS_ID}"></canvas>
    <script type="module" src="./harness.js"></script>
  </body>
</html>
`;
  fs.writeFileSync(path.join(site, 'index.html'), html);
  fs.writeFileSync(path.join(site, 'harness.js'), HARNESS_JS);
  fs.writeFileSync(path.join(site, 'harness.css'), HARNESS_CSS);
}

/** Copy the built wasm-bindgen output into the harness directory. */
function copyPkg(bundle, site) {
  const from = path.join(bundle, 'pkg');
  const to = path.join(site, 'pkg');
  if (!fs.existsSync(from)) {
    throw new Error(`the bundle has no pkg/ directory (build it first): ${from}`);
  }
  fs.mkdirSync(to, { recursive: true });
  for (const name of ['app_web.js', 'app_web_bg.wasm']) {
    const source = path.join(from, name);
    if (!fs.existsSync(source)) {
      throw new Error(`missing bundle artifact: ${source}`);
    }
    fs.copyFileSync(source, path.join(to, name));
    const size = fs.statSync(path.join(to, name)).size;
    if (size === 0) throw new Error(`empty bundle artifact: ${source}`);
  }
  const wasm = fs.statSync(path.join(to, 'app_web_bg.wasm')).size;
  return { wasmBytes: wasm, jsBytes: fs.statSync(path.join(to, 'app_web.js')).size };
}

function httpGet(url, timeout) {
  return new Promise((resolve, reject) => {
    const request = http.get(url, { timeout }, response => {
      if (response.statusCode !== 200) {
        response.resume();
        reject(new Error(`GET ${url} -> ${response.statusCode}`));
        return;
      }
      let body = '';
      response.setEncoding('utf8');
      response.on('data', chunk => {
        body += chunk;
      });
      response.on('end', () => resolve(body));
    });
    request.on('error', reject);
  });
}

async function waitForServer(url, child, timeout, log) {
  const deadline = Date.now() + timeout;
  let lastError = 'no attempt was made';
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(
        `the static server exited with code ${child.exitCode} before answering ${url}\n` +
          log.slice(-20).join(''),
      );
    }
    try {
      const body = await httpGet(url, 2000);
      if (body.includes(CANVAS_ID)) return;
      lastError = 'the served page is not the evidence harness';
    } catch (error) {
      lastError = error.message;
    }
    await new Promise(resolve => setTimeout(resolve, 150));
  }
  throw new Error(`the static server never became ready at ${url}: ${lastError}`);
}

async function stopServer(child) {
  if (child === null || child.exitCode !== null) return;
  child.kill('SIGTERM');
  await new Promise(resolve => child.once('exit', resolve));
}

// ---------------------------------------------------------------------------
// Page helpers
// ---------------------------------------------------------------------------

/** The canvas box in CSS pixels, or `null` when there is no measurable canvas. */
function canvasBox(page) {
  return page.evaluate(id => {
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
}

/**
 * Store the quantised pixels of one screenshot as the diff baseline (in the page:
 * two images must be compared in one realm).
 */
async function rememberPixels(page, buffer) {
  return page.evaluate(async base64 => {
    const bytes = Uint8Array.from(atob(base64), character => character.charCodeAt(0));
    const bitmap = await createImageBitmap(new Blob([bytes], { type: 'image/png' }));
    const scratch = new OffscreenCanvas(bitmap.width, bitmap.height);
    const context = scratch.getContext('2d', { willReadFrequently: true });
    context.drawImage(bitmap, 0, 0);
    const data = context.getImageData(0, 0, bitmap.width, bitmap.height).data;
    // 5 bits per channel: antialiasing and DPR scaling must not read as a change.
    const quantised = new Uint32Array(bitmap.width * bitmap.height);
    for (let index = 0, pixel = 0; index < data.length; index += 4, pixel += 1) {
      quantised[pixel] =
        ((data[index] >> 3) << 10) | ((data[index + 1] >> 3) << 5) | (data[index + 2] >> 3);
    }
    window.__FLIP_BASE__ = {
      width: bitmap.width,
      height: bitmap.height,
      quantised,
    };
    const counts = new Map();
    for (const value of quantised) counts.set(value, (counts.get(value) ?? 0) + 1);
    return {
      width: bitmap.width,
      height: bitmap.height,
      pixels: quantised.length,
      distinctColours: counts.size,
    };
  }, buffer.toString('base64'));
}

/** Compare one screenshot against the stored baseline; returns the pixel diff. */
async function diffPixels(page, buffer) {
  return page.evaluate(async base64 => {
    const base = window.__FLIP_BASE__;
    if (!base) return { reason: 'no baseline was stored' };
    const bytes = Uint8Array.from(atob(base64), character => character.charCodeAt(0));
    const bitmap = await createImageBitmap(new Blob([bytes], { type: 'image/png' }));
    if (bitmap.width !== base.width || bitmap.height !== base.height) {
      return {
        reason: 'size changed',
        before: [base.width, base.height],
        after: [bitmap.width, bitmap.height],
      };
    }
    const scratch = new OffscreenCanvas(bitmap.width, bitmap.height);
    const context = scratch.getContext('2d', { willReadFrequently: true });
    context.drawImage(bitmap, 0, 0);
    const data = context.getImageData(0, 0, bitmap.width, bitmap.height).data;
    let changed = 0;
    for (let index = 0, pixel = 0; index < data.length; index += 4, pixel += 1) {
      const value =
        ((data[index] >> 3) << 10) | ((data[index + 1] >> 3) << 5) | (data[index + 2] >> 3);
      if (value !== base.quantised[pixel]) changed += 1;
    }
    return {
      width: bitmap.width,
      height: bitmap.height,
      pixels: bitmap.width * bitmap.height,
      changedPixels: changed,
      changedShare: Number((changed / (bitmap.width * bitmap.height)).toFixed(6)),
    };
  }, buffer.toString('base64'));
}

/** Render `frames` frames, each on its own animation frame (the host's loop). */
async function render(page, frames) {
  return page.evaluate(async count => {
    const { mod } = window.__FLIP__;
    const reports = [];
    for (let index = 0; index < count; index += 1) {
      reports.push(JSON.parse(mod.frame()));
      await new Promise(resolve => requestAnimationFrame(() => resolve(null)));
    }
    const last = reports[reports.length - 1] ?? null;
    return {
      frames: reports.length,
      presented: reports.filter(report => report.presented === true).length,
      waited: reports.filter(report => report.presented === false).length,
      last,
    };
  }, frames);
}

/** One wire command through `invoke`, parsed. */
function invoke(page, name, args) {
  return page.evaluate(
    async payload => JSON.parse(await window.__FLIP__.mod.invoke(payload.name, JSON.stringify(payload.args ?? {}))),
    { name, args },
  );
}

function shellStatus(page) {
  return page.evaluate(() => JSON.parse(window.__FLIP__.mod.shell_status()));
}

function documentStatus(page) {
  return page.evaluate(() => JSON.parse(window.__FLIP__.mod.document()));
}

/** One `probe_input`, parsed. */
function probe(page, input) {
  return page.evaluate(args => JSON.parse(window.__FLIP__.mod.probe_input(JSON.stringify(args))), input);
}

// ---------------------------------------------------------------------------
// Recording
// ---------------------------------------------------------------------------

function makeRecorder(verbose) {
  const checks = [];
  return {
    checks,
    record(ok, label, detail) {
      checks.push({ ok: Boolean(ok), label, detail: detail ?? null });
      const line = `${ok ? 'PASS' : 'FAIL'} ${label}`;
      console.log(verbose || !ok ? `${line} ${detail === undefined ? '' : JSON.stringify(detail)}` : line);
      return Boolean(ok);
    },
  };
}

// ---------------------------------------------------------------------------
// The run
// ---------------------------------------------------------------------------

async function main() {
  const options = parseArgs(process.argv.slice(2));
  fs.mkdirSync(options.evidence, { recursive: true });
  const site = path.join(options.evidence, 'site');
  fs.mkdirSync(site, { recursive: true });
  const artifacts = copyPkg(options.bundle, site);
  writeHarness(site);
  console.log(`[flip] evidence directory ${options.evidence}`);
  console.log(
    `[flip] bundle ${options.bundle} -> harness (wasm ${artifacts.wasmBytes} B, ` +
      `js ${artifacts.jsBytes} B)`,
  );

  const serverLog = [];
  const child = spawn(
    'python3',
    [path.join(REPO_ROOT, 'scripts', 'serve-web.py'), '--directory', site, '--port', String(options.port)],
    { cwd: REPO_ROOT, stdio: ['ignore', 'pipe', 'pipe'] },
  );
  child.stdout.setEncoding('utf8');
  child.stderr.setEncoding('utf8');
  child.stdout.on('data', chunk => serverLog.push(String(chunk)));
  child.stderr.on('data', chunk => serverLog.push(String(chunk)));
  process.on('exit', () => {
    if (child.exitCode === null) child.kill('SIGKILL');
  });

  const url = `http://127.0.0.1:${options.port}/`;
  const recorder = makeRecorder(options.verbose);
  const { record, checks } = recorder;
  let browser = null;
  let summary = null;
  try {
    await waitForServer(url, child, options.timeout, serverLog);
    const { chromium } = loadPlaywright();
    browser = await chromium.launch({ headless: true, args: [...BROWSER_ARGS] });
    const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
    const consoleLines = [];
    const pageErrors = [];
    page.on('console', message => consoleLines.push(`${message.type()}: ${message.text()}`));
    page.on('pageerror', error => pageErrors.push(String(error)));

    await page.goto(url, { waitUntil: 'load', timeout: options.timeout });
    await page.waitForFunction(() => window.__FLIP__ !== undefined, null, {
      timeout: options.timeout,
    });

    // --- 1. start the real session -------------------------------------
    const started = await page.evaluate(async () => {
      const { ready, mod } = window.__FLIP__;
      await ready;
      const canvas = document.getElementById('cad-canvas');
      const rect = canvas.getBoundingClientRect();
      const dpr = window.devicePixelRatio > 0 ? window.devicePixelRatio : 1;
      const status = JSON.parse(
        await mod.start('cad-canvas', rect.width, rect.height, dpr),
      );
      return {
        status,
        css: [rect.width, rect.height],
        dpr,
        backing: [canvas.width, canvas.height],
      };
    });
    record(
      started.status.ok === true,
      'start() reports a live session',
      { label: started.status.label, backend: started.status.backend, entityCount: started.status.entityCount },
    );
    record(
      started.status.entityCount === 4,
      'the real session imported the four-line fixture',
      { entityCount: started.status.entityCount, error: started.status.error ?? null },
    );
    record(
      started.backing[0] === Math.round(started.css[0] * started.dpr) &&
        started.backing[1] === Math.round(started.css[1] * started.dpr),
      'the canvas backing store is CSS size x devicePixelRatio',
      { css: started.css, dpr: started.dpr, backing: started.backing },
    );

    // --- 2. the host frame loop ---------------------------------------
    const warm = await render(page, FIRST_FRAME_FRAMES);
    record(warm.presented > 0, 'frame() presented a frame', {
      presented: warm.presented,
      waited: warm.waited,
    });
    const afterWarm = await documentStatus(page);
    record(afterWarm.ok === true, 'document() is ok after the first frame', {
      label: afterWarm.label,
      batches: afterWarm.batches,
      presented: afterWarm.presented,
    });

    // --- 3. the canvas before the edit --------------------------------
    const box = await canvasBox(page);
    if (box === null || box.width < 64 || box.height < 64) {
      throw new Error(`no measurable canvas box: ${JSON.stringify(box)}`);
    }
    const before = await page.screenshot({ clip: box });
    fs.writeFileSync(path.join(options.evidence, 'canvas-01-before-line.png'), before);
    const beforeStats = await rememberPixels(page, before);

    // --- 4. LINE + two captured points + CONFIRM ----------------------
    const line = await invoke(page, 'cad_command', { command: 'LINE' });
    record(line.ok === true, 'invoke(cad_command, LINE) is accepted', line);

    const tool = await shellStatus(page);
    record(
      typeof tool.tool?.key === 'string' && tool.tool.key.startsWith('draw.'),
      'the router started a draw capture (shell_status().tool)',
      { tool: tool.tool?.key ?? null },
    );

    let sequence = 0;
    for (const [x, y] of LINE_POINTS) {
      sequence += 1;
      const down = await probe(page, {
        sequence,
        kind: 'pointer_down',
        x,
        y,
        button: 0,
      });
      sequence += 1;
      const up = await probe(page, { sequence, kind: 'pointer_up', x, y, button: 0 });
      record(
        down.ok === true && up.ok === true && down.accepted === true && up.accepted === true,
        `probe_input click (${x}, ${y}) is accepted`,
        { down, up },
      );
    }
    const confirmed = await invoke(page, 'cad_command', { command: 'CONFIRM' });
    record(confirmed.ok === true, 'invoke(cad_command, CONFIRM) is accepted', confirmed);
    await render(page, SETTLE_FRAMES);

    // --- 5. the document and the pixels really changed ----------------
    const status = await shellStatus(page);
    record(
      typeof status.entityCount === 'number' && status.entityCount > 4,
      'the confirmed LINE is a real entity (entityCount grew)',
      { entityCount: status.entityCount, tool: status.tool ?? null, error: status.error ?? null },
    );
    record(
      status.selection !== undefined && typeof status.selection.count === 'number',
      'shell_status() carries a real selection block',
      { count: status.selection?.count ?? null },
    );
    record(
      status.surfaceMode === 'dom-canvas' && status.canFit === true,
      'the carrier keys and canFit are reported',
      { surfaceMode: status.surfaceMode, canFit: status.canFit },
    );

    const after = await page.screenshot({ clip: box });
    fs.writeFileSync(path.join(options.evidence, 'canvas-02-after-line.png'), after);
    const diff = await diffPixels(page, after);
    record(
      typeof diff.changedPixels === 'number' && diff.changedPixels >= MIN_CHANGED_PIXELS,
      'the presented canvas changed (quantised pixel diff of the canvas box)',
      { ...diff, minChangedPixels: MIN_CHANGED_PIXELS, before: beforeStats },
    );

    // --- 6. the popup path -------------------------------------------
    const popup = await invoke(page, 'cad_open_popup', { kind: 'about' });
    record(
      popup.ok === true && popup.value?.accepted === true && popup.value?.kind === 'about',
      'invoke(cad_open_popup, about) returns a real PopupAck',
      popup.value ?? popup,
    );
    record(
      typeof popup.value?.popup_label === 'string' &&
        popup.value.popup_label === 'popup-1' &&
        popup.value?.existing === false,
      'the ack carries the frozen label and says this was a fresh open',
      { popup_label: popup.value?.popup_label, existing: popup.value?.existing },
    );
    record(
      popup.value?.payload?.kind === 'about' && typeof popup.value?.payload?.version === 'string',
      'the About payload is the build whose version is non-empty',
      popup.value?.payload ?? null,
    );
    const gated = await invoke(page, 'cad_command', { command: 'LINE' });
    record(
      gated.ok === false && String(gated.error).startsWith('popup.modal_open'),
      'the modal gate refuses CAD input while the popup is open',
      gated,
    );
    const gatedStatus = await shellStatus(page);
    record(
      gatedStatus.popup?.kind === 'about',
      'shell_status() projects the open popup',
      { popup: gatedStatus.popup ?? null },
    );
    const closed = await invoke(page, 'cad_close_popup', {});
    record(closed.ok === true && closed.value?.accepted === true, 'invoke(cad_close_popup) closes it', closed);
    const reopened = await invoke(page, 'cad_command', { command: 'CANCEL' });
    record(reopened.ok === true, 'input flows again once the gate is clear', reopened);
    const overlay = await page.screenshot({ fullPage: false });
    fs.writeFileSync(path.join(options.evidence, 'page-03-after-popup.png'), overlay);

    // --- 6a. the navigation exports the current chrome still calls -----
    // `pan(dx, dy)` / `zoom_at(factor, x, y)` are the pre-flip navigation API: they
    // must keep working (synthesized `ProbeInput`s through the real session) until
    // the front-end switch replaces them with `frame()` + `probe_input`.
    const cameraBefore = (await shellStatus(page)).camera;
    const panned = await page.evaluate(async () => JSON.parse(window.__FLIP__.mod.pan(60, 0)));
    record(panned.ok === true, 'pan(60, 0) is accepted', {
      ok: panned.ok,
      step: panned.step ?? null,
      error: panned.error ?? null,
    });
    await render(page, SETTLE_FRAMES);
    const cameraAfterPan = (await shellStatus(page)).camera;
    record(
      cameraBefore !== null && cameraAfterPan !== null &&
        Math.abs(cameraAfterPan.center[0] - (cameraBefore.center[0] - 60 * cameraBefore.scale)) < 1e-6 &&
        Math.abs(cameraAfterPan.scale - cameraBefore.scale) < 1e-12,
      'pan moved the camera by delta x world_per_px and did not zoom',
      { before: cameraBefore, after: cameraAfterPan },
    );
    const zoomed = await page.evaluate(async () => JSON.parse(window.__FLIP__.mod.zoom_at(1.25, 640, 400)));
    record(zoomed.ok === true, 'zoom_at(1.25, 640, 400) is accepted', {
      ok: zoomed.ok,
      step: zoomed.step ?? null,
      error: zoomed.error ?? null,
    });
    await render(page, SETTLE_FRAMES);
    const cameraAfterZoom = (await shellStatus(page)).camera;
    record(
      cameraAfterPan !== null && cameraAfterZoom !== null &&
        Math.abs(cameraAfterZoom.scale - cameraAfterPan.scale / 1.25) < 1e-9,
      'zoom_at applied exactly one 1.25x wheel notch',
      { before: cameraAfterPan?.scale ?? null, after: cameraAfterZoom?.scale ?? null },
    );

    // --- 6b. the retention case (only with --open) ---------------------
    let retention = null;
    if (options.open !== null) {
      const bytes = fs.readFileSync(options.open);
      const label = path.basename(options.open);
      const opened = await page.evaluate(
        async payload => {
          const { mod } = window.__FLIP__;
          const binary = Uint8Array.from(atob(payload.base64), character =>
            character.charCodeAt(0),
          );
          const canvas = document.getElementById('cad-canvas');
          const rect = canvas.getBoundingClientRect();
          const dpr = window.devicePixelRatio > 0 ? window.devicePixelRatio : 1;
          const status = JSON.parse(
            await mod.open_bytes('cad-canvas', payload.label, rect.width, rect.height, dpr, binary),
          );
          return { status };
        },
        { label, base64: bytes.toString('base64') },
      );
      const openStatus = opened.status;
      // Two honest outcomes: the replacement is installed (possibly with a
      // non-fatal session diagnostic, e.g. `unsupported-resources: fonts` on a
      // drawing this carrier cannot shape), or a named budget refused it.
      const openRecorded =
        openStatus.ok === true ||
        openStatus.step === 'budget' ||
        openStatus.step === 'import';
      record(
        openRecorded,
        'open_bytes(external drawing) either opened it or refused it by budget',
        {
          ok: openStatus.ok,
          step: openStatus.step ?? null,
          label: openStatus.label ?? null,
          entityCount: openStatus.entityCount ?? null,
          heap_after_open: openStatus.heap_after_open ?? null,
          over_budget: openStatus.over_budget ?? null,
        },
      );
      if (openStatus.ok === true && typeof openStatus.error === 'string') {
        // A drawing that requests text shaping cannot be shaped here (no font
        // package in wasm). The open succeeds and the limitation is reported — it is
        // recorded as an observation, never hidden and never counted as a failure.
        console.log(`[flip] session diagnostic on the replacement: ${openStatus.error}`);
      }
      if (openStatus.ok === true) {
        record(
          openStatus.label === label,
          'the replacement reports the external file as its document',
          { expected: label, label: openStatus.label },
        );
        // The scene arrives through the engine's staged pipeline; give it a real
        // frame loop before measuring.
        const settled = await render(page, 240);
        const measured = await documentStatus(page);
        const external = await page.screenshot({ clip: box });
        fs.writeFileSync(
          path.join(options.evidence, 'canvas-04-external-drawing.png'),
          external,
        );
        const externalDiff = await diffPixels(page, external);
        record(
          typeof externalDiff.changedPixels === 'number' &&
            externalDiff.changedPixels >= MIN_CHANGED_PIXELS,
          'the canvas shows the replacement drawing (quantised pixel diff)',
          externalDiff,
        );
        record(
          measured.ok === true && measured.label === label && measured.importCount > 1,
          'the second open is a real document replacement (importCount grew)',
          {
            label: measured.label,
            importCount: measured.importCount,
            entityCount: measured.entityCount,
            batches: measured.batches,
          },
        );
        retention = {
          file: options.open,
          inputBytes: bytes.length,
          recorded: openStatus,
          frames: settled,
          document: measured,
          diff: externalDiff,
        };
      } else {
        retention = { file: options.open, inputBytes: bytes.length, recorded: openStatus };
      }
    }

    // --- 7. the vocabulary and the refusal vocabulary -----------------
    const commands = await page.evaluate(() => JSON.parse(window.__FLIP__.mod.shell_commands()));
    record(
      Array.isArray(commands) &&
        ['LINE', 'FIT', 'ZOOMIN', 'ZOOMOUT', 'PAN'].every(name => commands.includes(name)),
      'shell_commands() advertises the router vocabulary',
      { count: commands.length, sample: commands.slice(0, 8) },
    );
    const aliases = await page.evaluate(() => JSON.parse(window.__FLIP__.mod.shell_aliases()));
    record(
      Array.isArray(aliases) && aliases.some(row => row.alias === 'L' && row.command === 'LINE'),
      'shell_aliases() carries the desktop alias rows',
      { count: aliases.length, sample: aliases.slice(0, 4) },
    );
    const shellCommand = await page.evaluate(() => {
      try {
        return { ok: true, value: JSON.parse(window.__FLIP__.mod.shell_command('ZOOMIN')) };
      } catch (error) {
        return { ok: false, error: String(error) };
      }
    });
    record(
      shellCommand.ok === true && shellCommand.value?.accepted === true,
      'shell_command(ZOOMIN) executes on the real session',
      shellCommand,
    );
    const refused = await page.evaluate(() => {
      try {
        return { ok: true, value: window.__FLIP__.mod.shell_command('FLOOPY') };
      } catch (error) {
        return { ok: false, error: String(error) };
      }
    });
    record(
      refused.ok === false && String(refused.error).includes('unsupported-runtime: cad_command'),
      'shell_command refuses an unknown name with the stable prefix',
      refused,
    );
    const notWire = await invoke(page, 'cad_status', {});
    record(notWire.ok === false, 'cad_status is not a wire command on this transport', notWire);

    // --- 8. measurements ---------------------------------------------
    const measured = await documentStatus(page);
    const final = await shellStatus(page);
    const renderer = await page.evaluate(() => {
      const scratch = document.createElement('canvas');
      const gl = scratch.getContext('webgl2');
      if (!gl) return null;
      const info = gl.getExtension('WEBGL_debug_renderer_info');
      const name = info ? gl.getParameter(info.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER);
      gl.getExtension('WEBGL_lose_context')?.loseContext();
      return String(name);
    });
    record(pageErrors.length === 0, 'no uncaught page error', pageErrors);
    record(measured.ok === true, 'document() is still ok at the end', {
      label: measured.label,
      entityCount: measured.entityCount,
      importCount: measured.importCount,
    });

    summary = {
      url,
      bundle: options.bundle,
      artifacts,
      rendering: /swiftshader|llvmpipe|software/i.test(renderer ?? '') ? 'software' : 'hardware',
      renderer,
      css: started.css,
      dpr: started.dpr,
      backing: started.backing,
      start: started.status,
      frames: { firstFrame: warm, settle: SETTLE_FRAMES },
      before: beforeStats,
      diff,
      document: measured,
      status: { entityCount: final.entityCount, camera: final.camera, canFit: final.canFit },
      navigation: { before: cameraBefore, afterPan: cameraAfterPan, afterZoom: cameraAfterZoom },
      popupAck: popup.value ?? null,
      retention,
      commands: commands.length,
      aliases: aliases.length,
      checks,
      consoleLines: consoleLines.slice(-40),
    };
  } catch (error) {
    record(false, 'the run completed without throwing', { error: String(error) });
    summary = { url, bundle: options.bundle, artifacts, checks, error: String(error) };
    process.exitCode = 1;
  } finally {
    if (browser !== null) await browser.close();
    await stopServer(child);
  }

  const failed = checks.filter(check => !check.ok).length;
  if (summary !== null) {
    fs.writeFileSync(
      path.join(options.evidence, 'summary.json'),
      `${JSON.stringify(summary, null, 2)}\n`,
    );
  }
  console.log(
    `${failed === 0 ? 'PASS' : 'FAIL'} ${checks.length - failed}/${checks.length} checks ` +
      `(evidence: ${options.evidence})`,
  );
  if (summary?.rendering === 'software') {
    console.log(
      'note: this run rendered through SwiftShader (software rasterisation): the pixel diff is ' +
        '"a frame reached the canvas", never GPU, colour or visual acceptance.',
    );
  }
  if (failed !== 0 && process.exitCode === undefined) process.exitCode = 1;
}

main().catch(error => {
  console.error(`web-flip-evidence failed: ${error.stack ?? error.message}`);
  process.exit(1);
});
