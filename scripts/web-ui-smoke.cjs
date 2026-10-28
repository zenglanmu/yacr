#!/usr/bin/env node
/*
 * Responsive-chrome acceptance for the browser bundle (`apps/app-web/dist-ui`).
 *
 * What it proves (and only this):
 *   - the static server here serves the built chrome bundle, and the page loads
 *     with no uncaught error and no failed (non-favicon) request;
 *   - the layout switch (`MOBILE_MAX_WIDTH = 1024`, `matchMedia`, `data-layout`)
 *     resolves to `mobile` at 390, 914 and 1024 CSS px (phone portrait, phone
 *     landscape, 4:3 tablet) and to `desktop` at the first width past the
 *     threshold, 1025, and at 1280;
 *   - on `mobile` only the compact chrome is rendered (`.mobile-topbar`,
 *     `.mobile-commandbar`; no `.ribbon`, no `.statusbar`) and the CAD partition
 *     (`.native-input`) still gets >= 70 % of the viewport area at full width;
 *   - on `desktop` the band and the status bar are back and no mobile bar exists;
 *   - the `dom-canvas` carrier's backing store is non-zero and equals its CSS
 *     size x devicePixelRatio (the session is sized from one measurement);
 *   - **the CAD canvas is not a uniform colour**: a screenshot of its box is
 *     decoded and must contain more than one colour, so a present that draws
 *     nothing fails loudly even though the session reports success. This is the
 *     regression guard for the Android WebView present defect
 *     (`docs/validation-android-present-2026-10-28.md`); it is checked at every
 *     viewport, after the live resize and after the resize back;
 *   - the mobile drawer opens from the top-bar button (`aria-expanded` flips),
 *     a tab chip switches the panel, Escape closes it, and typed text reaches the
 *     shared command draft (the field echoes it and the completion sheet is
 *     filtered by it);
 *   - a **live** breakpoint crossing (no reload) flips the layout in both
 *     directions: a `window.__yacrProbe` marker set before the resize survives;
 *   - no error banner is painted: after load/settle (and after both crossings of
 *     the live resize) no visible `[role="alert"]` carries text. The browser
 *     carrier serves no native input, so `probe_input` must never be forwarded
 *     there and no `unsupported-runtime: …` refusal may reach `setError`; the
 *     banner is asserted absent, never suppressed.
 *
 * What it does NOT prove - and must never be read as:
 *   - anything about GPU hardware: this launch forces ANGLE/SwiftShader, so the
 *     run is labelled `rendering=software` and is neither GPU nor performance
 *     evidence;
 *   - pixel/visual **correctness**: the non-uniform-colour check only proves that
 *     *some* frame reached the canvas (colour space, geometry, camera and
 *     antialiasing are not asserted here), and the screenshots are debugging
 *     artefacts, not visual acceptance;
 *   - the native hosts: the Tauri/React desktop windows, the Wayland subsurface
 *     path and the Win32 child HWND are not exercised here at all;
 *   - a shipped web product: this is one Chromium build against one bundle.
 *
 * Usage:
 *   node scripts/web-ui-smoke.cjs
 *   node scripts/web-ui-smoke.cjs --url http://127.0.0.1:8090/ \
 *     --evidence /tmp/opencode/yacr-mobile-web-20261028T000000Z [--timeout 120000]
 *   node scripts/web-ui-smoke.cjs --port 8091 --evidence /tmp/opencode/evidence
 *   node scripts/web-ui-smoke.cjs --verbose   # print every passing check too
 *
 * The script starts and stops `scripts/serve-web.py` itself. `playwright` is not
 * a repository dependency: install it outside the repo and point `NODE_PATH` at
 * its `node_modules` (see `docs/ci.md` / the handoff entry this script came from).
 *
 * Requires the bundle to exist already (`bash scripts/build-web-ui.sh`); this
 * script never builds it, and a missing bundle is a loud non-zero exit.
 */
'use strict';

const fs = require('node:fs');
const http = require('node:http');
const path = require('node:path');
const { spawn } = require('node:child_process');

const { measureCanvasPixels } = require('./canvas-pixels.cjs');

const REPO_ROOT = path.resolve(__dirname, '..');
const SERVER_SCRIPT = path.join(REPO_ROOT, 'scripts', 'serve-web.py');
const BUNDLE_DIR = path.join(REPO_ROOT, 'apps', 'app-web', 'dist-ui');

/** The layout breakpoint, mirrored from `layout-mode.ts` (inclusive). */
const MOBILE_MAX_WIDTH = 1024;
/**
 * The widths this acceptance pins.
 *
 * 390 = phone portrait, 914x412 = the same phone in landscape (the width that used
 * to fall to the desktop band and leave the drawing 37 % of a 412px-tall window),
 * 1024 = the last `mobile` width (a 4:3 tablet in portrait), 1025 = the first
 * `desktop` one, 1280 = a desktop window. 768 is kept because it was the old
 * threshold and is still a `mobile` width.
 */
const VIEWPORT_CASES = Object.freeze([
  { width: 390, height: 844 },
  { width: 768, height: 1024 },
  { width: 914, height: 412 },
  { width: 1024, height: 900 },
  { width: 1025, height: 900 },
  { width: 1280, height: 800 },
]);
/** The resize case starts (and ends) here. */
const DESKTOP_VIEWPORT = Object.freeze({ width: 1280, height: 800 });
const MOBILE_VIEWPORT = Object.freeze({ width: 390, height: 844 });
/** The area the CAD partition must keep on the compact layout. */
const MIN_SURFACE_AREA_RATIO = 0.7;
/** Chromium flags: SwiftShader is the only WebGL2 path without a GPU. */
const BROWSER_ARGS = Object.freeze([
  '--use-gl=angle',
  '--use-angle=swiftshader',
  '--enable-unsafe-swiftshader',
  '--disable-dev-shm-usage',
]);
const DEFAULT_PORT = 8090;
const DEFAULT_TIMEOUT_MS = 120000;
/** The text typed into the mobile command bar (a catalog prefix: `LINE`…). */
const TYPED_COMMAND = 'LI';

function loadPlaywright() {
  try {
    return require('playwright');
  } catch (error) {
    throw new Error(
      `cannot resolve 'playwright': ${error.message}\n` +
        'install it outside the repo (`npm i playwright@1.64.1` in a scratch dir) and run with ' +
        'NODE_PATH=<that dir>/node_modules',
    );
  }
}

function parseArgs(argv) {
  const options = {
    url: null,
    port: null,
    evidence: null,
    timeout: DEFAULT_TIMEOUT_MS,
    verbose: false,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index];
    const value = argv[index + 1];
    if (flag === '--url' && value) {
      let parsed;
      try {
        parsed = new URL(value);
      } catch (error) {
        throw new Error(`--url is not a URL: ${value} (${error.message})`);
      }
      options.url = parsed.toString();
      if (parsed.port) options.port = Number(parsed.port);
      index += 1;
    } else if (flag === '--port' && value) {
      const port = Number(value);
      if (!Number.isInteger(port) || port <= 0 || port > 65535) {
        throw new Error(`--port expects a TCP port, got ${value}`);
      }
      if (options.port !== null && options.port !== port) {
        throw new Error(`--url names port ${options.port} but --port names ${port}`);
      }
      options.port = port;
      index += 1;
    } else if (flag === '--evidence' && value) {
      options.evidence = path.resolve(value);
      index += 1;
    } else if (flag === '--timeout' && value) {
      const parsed = Number(value);
      if (!Number.isFinite(parsed) || parsed <= 0) {
        throw new Error(`--timeout expects a positive number of milliseconds, got ${value}`);
      }
      options.timeout = parsed;
      index += 1;
    } else if (flag === '--verbose') {
      options.verbose = true;
    } else {
      throw new Error(`unknown argument: ${flag}`);
    }
  }
  if (options.port === null) options.port = DEFAULT_PORT;
  if (options.url === null) options.url = `http://127.0.0.1:${options.port}/`;
  return options;
}

/** One recorded check: `ok` plus the label (and an optional measured detail). */
function record(checks, ok, label, detail) {
  checks.push({ ok: Boolean(ok), label, detail: detail === undefined ? null : detail });
  return Boolean(ok);
}

function describe(value) {
  if (typeof value === 'string') return value;
  try {
    return JSON.stringify(value);
  } catch (error) {
    return String(value);
  }
}

// --- static server ---------------------------------------------------------

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
      // A foreign server on this port could answer too; the bundle marker is what
      // says this is the page the script is about to measure.
      if (body.includes('id="root"') && body.includes('/assets/')) {
        // One extra request for the wasm artifact's MIME is not needed here: the
        // page load itself fails loudly if `application/wasm` is wrong.
        return;
      }
      lastError = 'the served index.html is not the built chrome bundle';
    } catch (error) {
      lastError = error.message;
    }
    await sleep(150);
  }
  throw new Error(`the static server never became ready at ${url}: ${lastError}`);
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
      response.on('data', chunk => { body += chunk; });
      response.on('end', () => resolve(body));
    });
    request.on('timeout', () => request.destroy(new Error('request timed out')));
    request.on('error', reject);
  });
}

function sleep(ms) {
  return new Promise(resolve => { setTimeout(resolve, ms); });
}

function waitForExit(child, timeout) {
  if (child.exitCode !== null) return Promise.resolve(true);
  return new Promise(resolve => {
    const timer = setTimeout(() => resolve(false), timeout);
    child.once('exit', () => {
      clearTimeout(timer);
      resolve(true);
    });
  });
}

async function stopServer(child) {
  if (!child || child.exitCode !== null) return;
  child.kill('SIGTERM');
  if (await waitForExit(child, 3000)) return;
  child.kill('SIGKILL');
  await waitForExit(child, 2000);
}

// --- page helpers ----------------------------------------------------------

function newPage(context, bucket) {
  return context.newPage().then(page => {
    page.setDefaultTimeout(30000);
    page.on('pageerror', error => bucket.pageErrors.push(String(error)));
    page.on('console', message => bucket.consoleLines.push(`${message.type()}: ${message.text()}`));
    page.on('requestfailed', request => {
      // A favicon 404 is not an asset the bundle needs; everything else is.
      if (!request.url().endsWith('/favicon.ico')) {
        bucket.failedRequests.push(`${request.url()} (${request.failure()?.errorText ?? 'failed'})`);
      }
    });
    return page;
  });
}

/** The resolved layout attribute, on both owners (`main.shell` and `<html>`). */
function readLayout(page) {
  return page.evaluate(() => {
    const shell = document.querySelector('main.shell');
    return {
      shell: shell ? shell.getAttribute('data-layout') : null,
      documentElement: document.documentElement.getAttribute('data-layout') ?? null,
      viewport: {
        width: window.innerWidth,
        height: window.innerHeight,
        dpr: window.devicePixelRatio || 1,
      },
    };
  });
}

/** The session is sized from one measurement: backing store = round(css * dpr). */
function canvasMatchesCss() {
  const canvas = document.getElementById('cad-canvas');
  if (!canvas) return false;
  const rect = canvas.getBoundingClientRect();
  const dpr = window.devicePixelRatio || 1;
  if (!(rect.width > 0) || !(rect.height > 0)) return false;
  return canvas.width > 0 && canvas.height > 0
    && Math.abs(canvas.width - Math.round(rect.width * dpr)) <= 1
    && Math.abs(canvas.height - Math.round(rect.height * dpr)) <= 1;
}

/**
 * Readiness: the shell has painted its layout attribute **and** the `dom-canvas`
 * carrier is live (the `<canvas>` only mounts once the host reports
 * `surfaceMode: "dom-canvas"`, and it only has a backing store once the wasm
 * session started and was handed a size).
 *
 * The last step waits for the geometry to *settle*: the start dock also collapses
 * once the session reports its document, so measuring the first matching frame
 * would assert the canvas size of a layout that is still moving. `settleMs` of an
 * unchanged `layout|rect|backing|dpr` key is the point being measured.
 */
async function waitForSession(page, timeout) {
  await page.waitForFunction(
    () => document.querySelector('main.shell')?.hasAttribute('data-layout') === true,
    undefined,
    { timeout },
  );
  await page.waitForFunction(canvasMatchesCss, undefined, { timeout });
  await page.waitForFunction(
    settleMs => {
      const shell = document.querySelector('main.shell');
      const canvas = document.getElementById('cad-canvas');
      if (!shell || !canvas) return false;
      const rect = canvas.getBoundingClientRect();
      const dpr = window.devicePixelRatio || 1;
      const probe = window;
      if (!(canvas.width > 0) || !(canvas.height > 0)) return false;
      if (Math.abs(canvas.width - Math.round(rect.width * dpr)) > 1) return false;
      if (Math.abs(canvas.height - Math.round(rect.height * dpr)) > 1) return false;
      const key = [
        shell.getAttribute('data-layout'),
        rect.width.toFixed(2),
        rect.height.toFixed(2),
        canvas.width,
        canvas.height,
        dpr,
      ].join('|');
      const state = probe.__yacrUiSmokeStable;
      if (!state || state.key !== key) {
        probe.__yacrUiSmokeStable = { key, since: performance.now() };
        return false;
      }
      return performance.now() - state.since >= settleMs;
    },
    250,
    { timeout },
  );
}

function readSurface(page) {
  return page.evaluate(() => {
    const rectOf = element => {
      if (!element) return null;
      const rect = element.getBoundingClientRect();
      return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
    };
    const canvas = document.getElementById('cad-canvas');
    return {
      surface: rectOf(document.querySelector('.native-input')),
      canvas: canvas === null
        ? null
        : {
          css: rectOf(canvas),
          backing: { width: canvas.width, height: canvas.height },
          inlineWidth: canvas.getAttribute('width'),
        },
      viewport: {
        width: window.innerWidth,
        height: window.innerHeight,
        dpr: window.devicePixelRatio || 1,
      },
    };
  });
}

/** A raw WebGL2 context on a scratch canvas: it only names the renderer. */
function readRenderer(page) {
  return page.evaluate(() => {
    const scratch = document.createElement('canvas');
    const gl = scratch.getContext('webgl2');
    if (!gl) return { webgl2: false, renderer: null };
    const info = gl.getExtension('WEBGL_debug_renderer_info');
    const renderer = info ? gl.getParameter(info.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER);
    gl.getExtension('WEBGL_lose_context')?.loseContext();
    return { webgl2: true, renderer: renderer === null ? null : String(renderer) };
  });
}

function softwareRenderer(renderer) {
  return /swiftshader|llvmpipe|softpipe|software/i.test(renderer ?? '');
}

async function takeScreenshot(page, directory, name) {
  const file = path.join(directory, name);
  await page.screenshot({ path: file, fullPage: false });
  const stats = fs.statSync(file);
  return { file, bytes: stats.size };
}

/**
 * The **visible** `role="alert"` text the page is showing right now.
 *
 * Every error surface the chrome has is one of these nodes: the desktop
 * `.statusbar__error`, the compact command bar's error row, the diagnostics
 * palette. A visible, non-empty one is the error banner; that is what this suite
 * asserts absent (`checkNoErrorBanner`) and also prints as an observation, so a
 * regression names itself instead of only failing a check.
 *
 * Hidden or empty alert nodes are not an error surface (the components render
 * `null` rather than an empty banner), and a read that throws returns a marker
 * that fails the assertion instead of quietly reporting "no banner".
 */
async function readErrorAlerts(page) {
  try {
    const alerts = page.locator('[role="alert"]');
    const count = await alerts.count();
    const texts = [];
    for (let index = 0; index < count; index += 1) {
      const alert = alerts.nth(index);
      if (!(await alert.isVisible())) continue;
      const text = (await alert.innerText()).trim();
      if (text.length > 0) texts.push(text);
    }
    return texts;
  } catch (error) {
    return [`<alert unreadable: ${error.message}>`];
  }
}

/** The banner must be absent: no visible `role="alert"` carries text. */
function checkNoErrorBanner(checks, alerts, when) {
  record(checks, alerts.length === 0, `no visible error banner after ${when}`, alerts);
}

/** `dom-canvas` sizing: the backing store must equal the CSS box x DPR. */
function canvasSizing(surface) {
  const canvas = surface.canvas;
  if (canvas === null) return { ok: false, detail: 'canvas#cad-canvas is missing' };
  const expectedWidth = Math.round(canvas.css.width * surface.viewport.dpr);
  const expectedHeight = Math.round(canvas.css.height * surface.viewport.dpr);
  const ok = canvas.backing.width > 0 && canvas.backing.height > 0
    && Math.abs(canvas.backing.width - expectedWidth) <= 1
    && Math.abs(canvas.backing.height - expectedHeight) <= 1;
  return {
    ok,
    detail: {
      backing: `${canvas.backing.width}x${canvas.backing.height}`,
      expected: `${expectedWidth}x${expectedHeight}`,
      css: `${canvas.css.width}x${canvas.css.height}`,
      dpr: surface.viewport.dpr,
    },
  };
}

/** All checks that do not depend on the layout mode. */
function checkCommon(checks, bucket, layoutInfo, surface, renderer, expectedLayout) {
  record(checks, bucket.pageErrors.length === 0, 'no uncaught page error', bucket.pageErrors);
  record(checks, bucket.failedRequests.length === 0, 'no failed non-favicon request', bucket.failedRequests);
  record(
    checks,
    layoutInfo.shell === expectedLayout && layoutInfo.documentElement === expectedLayout,
    `data-layout is ${expectedLayout} on main.shell and <html>`,
    { shell: layoutInfo.shell, documentElement: layoutInfo.documentElement },
  );
  record(
    checks,
    renderer.webgl2 && typeof renderer.renderer === 'string' && renderer.renderer.length > 0,
    'WebGL2 renderer string is readable',
    renderer,
  );

  record(checks, surface.canvas !== null, 'canvas#cad-canvas exists', surface.canvas === null ? 'missing' : null);
  const sizing = canvasSizing(surface);
  record(checks, sizing.ok, 'canvas backing store is non-zero and equals CSS size x DPR', sizing.detail);
}

/** Existence / visibility / box of one region, without waiting for it to appear. */
async function regionInfo(page, selector) {
  const locator = page.locator(selector).first();
  const count = await page.locator(selector).count();
  if (count === 0) return { count, visible: false, box: null };
  const visible = await locator.isVisible();
  const box = visible ? await locator.boundingBox() : null;
  return { count, visible, box };
}

function checkMobileChrome(checks, surface, viewport) {
  record(checks, surface.surface !== null, 'CAD partition (.native-input) exists', surface.surface);
  if (surface.surface === null) return;
  const ratio = (surface.surface.width * surface.surface.height)
    / (viewport.width * viewport.height);
  record(
    checks,
    ratio >= MIN_SURFACE_AREA_RATIO,
    `CAD partition keeps >= ${Math.round(MIN_SURFACE_AREA_RATIO * 100)}% of the viewport area`,
    { ratio: Number(ratio.toFixed(3)), rect: surface.surface, viewport },
  );
  record(
    checks,
    Math.abs(surface.surface.x) <= 1 && Math.abs(surface.surface.width - viewport.width) <= 1,
    'CAD partition is full width',
    { x: surface.surface.x, width: surface.surface.width, viewportWidth: viewport.width },
  );
}

// --- the viewport case -----------------------------------------------------

async function runViewportCase(browser, options, spec) {
  const checks = [];
  const bucket = { pageErrors: [], failedRequests: [], consoleLines: [] };
  const expectedLayout = spec.width <= MOBILE_MAX_WIDTH ? 'mobile' : 'desktop';
  const name = `${spec.width}x${spec.height}`;
  const screenshots = [];
  const observations = [];
  const container = await browser.newContext({
    viewport: { width: spec.width, height: spec.height },
    deviceScaleFactor: 1,
  });
  const page = await newPage(container, bucket);
  let renderer = { webgl2: false, renderer: null };
  try {
    const response = await page.goto(options.url, { waitUntil: 'load', timeout: options.timeout });
    if (response === null || !response.ok()) {
      throw new Error(`the bundle URL did not load: ${options.url} -> ${response?.status() ?? 'no response'}`);
    }
    await waitForSession(page, options.timeout);

    const layoutInfo = await readLayout(page);
    const surface = await readSurface(page);
    renderer = await readRenderer(page);

    record(checks, layoutInfo.viewport.width === spec.width, `viewport reports ${spec.width} CSS px`, layoutInfo.viewport);
    checkCommon(checks, bucket, layoutInfo, surface, renderer, expectedLayout);

    // A healthy status is not a visible frame: the composited pixels are the
    // judge (a present that draws nothing still reports success).
    const presented = await measureCanvasPixels(page, options.evidence, `presented-${name}`);
    record(
      checks,
      presented.ok,
      'the presented CAD canvas is not a uniform colour (a frame is on screen)',
      presented.detail,
    );

    if (expectedLayout === 'mobile') {
      const topbar = await regionInfo(page, '.mobile-topbar');
      const commandbar = await regionInfo(page, '.mobile-commandbar');
      const ribbon = await regionInfo(page, '.ribbon');
      const statusbar = await regionInfo(page, '.statusbar');
      record(checks, topbar.visible, '.mobile-topbar is visible', topbar.box);
      record(checks, commandbar.visible, '.mobile-commandbar is visible', commandbar.box);
      record(checks, ribbon.count === 0, 'no .ribbon element on mobile', { count: ribbon.count });
      record(checks, statusbar.count === 0, 'no .statusbar element on mobile', { count: statusbar.count });
      record(checks, (await page.locator('.commandline').count()) === 0, 'no desktop .commandline on mobile', null);
      checkMobileChrome(checks, surface, layoutInfo.viewport);
    } else {
      const ribbon = await regionInfo(page, '.ribbon');
      const statusbar = await regionInfo(page, '.statusbar');
      const topbar = await regionInfo(page, '.mobile-topbar');
      const commandbar = await regionInfo(page, '.mobile-commandbar');
      record(checks, ribbon.visible, '.ribbon is visible on desktop', ribbon.box ?? { count: ribbon.count });
      record(checks, statusbar.visible, '.statusbar is visible on desktop', statusbar.box ?? { count: statusbar.count });
      record(checks, topbar.count === 0, 'no .mobile-topbar on desktop', { count: topbar.count });
      record(checks, commandbar.count === 0, 'no .mobile-commandbar on desktop', { count: commandbar.count });
    }

    // Chrome evidence first: the interaction below opens the drawer and the
    // completion sheet, which would hide the composition this screenshot is for.
    if (options.evidence) {
      const shot = await takeScreenshot(page, options.evidence, `${spec.width}.png`);
      screenshots.push(shot);
      record(checks, shot.bytes > 0, `screenshot ${spec.width}.png written`, shot.file);
    }
    // The startup banner is asserted, not merely observed: no refusal may reach
    // `setError` on this carrier, so a visible `role="alert"` is a defect.
    const alerts = await readErrorAlerts(page);
    checkNoErrorBanner(checks, alerts, 'load and settle');
    observations.push(...alerts.map(text => `visible alert: ${text}`));

    if (spec.width === 390) {
      await checkMobileInteraction(page, checks, options, screenshots);
    }
  } catch (error) {
    record(checks, false, 'case ran to completion', error.message);
  } finally {
    await container.close();
  }
  return { name, expectedLayout, checks, bucket, renderer, screenshots, observations };
}

/** Requirement 4: drawer open / tab switch / Escape / command draft at 390px. */
async function checkMobileInteraction(page, checks, options, screenshots) {
  const menu = page.locator('.mobile-topbar__menu');
  const drawerId = await menu.getAttribute('aria-controls');
  const drawer = drawerId ? page.locator(`#${drawerId}`) : page.locator('.mobile-drawer');

  record(checks, (await menu.getAttribute('aria-expanded')) === 'false', 'drawer starts closed (aria-expanded="false")', null);
  await menu.click();
  await drawer.waitFor({ state: 'visible', timeout: options.timeout });
  // The drawer drops in with a 200ms fade/slide (`mobile.css`); its box and its
  // opacity are only the final ones once that animation ends, and a screenshot or
  // a geometry read of the first frame would describe neither state.
  await page.waitForFunction(
    () => {
      const drawer = document.querySelector('.mobile-drawer');
      return !!drawer && getComputedStyle(drawer).opacity === '1';
    },
    undefined,
    { timeout: options.timeout },
  );
  await page.waitForFunction(
    () => document.querySelector('.mobile-topbar__menu')?.getAttribute('aria-expanded') === 'true',
    undefined,
    { timeout: options.timeout },
  );
  record(checks, await drawer.isVisible(), 'menu button opens the drawer', drawerId);
  record(checks, (await menu.getAttribute('aria-expanded')) === 'true', 'aria-expanded flips to "true"', null);

  if (options.evidence) {
    const shot = await takeScreenshot(page, options.evidence, '390-drawer.png');
    screenshots.push(shot);
  }

  const panel = page.locator('.mobile-drawer__body');
  const chips = page.locator('.mobile-drawer__tabs [role="tab"]');
  const chipCount = await chips.count();
  const activeBefore = await page.locator('.mobile-drawer__tabs [role="tab"][aria-selected="true"]').first().textContent();
  const panelBefore = await panel.getAttribute('aria-label');
  const textBefore = (await panel.innerText()).trim();

  let otherIndex = -1;
  for (let index = 0; index < chipCount; index += 1) {
    if ((await chips.nth(index).getAttribute('aria-selected')) !== 'true') {
      otherIndex = index;
      break;
    }
  }
  record(checks, chipCount > 1 && otherIndex >= 0, 'drawer offers more than one tab chip', { chipCount });
  if (otherIndex >= 0) {
    await chips.nth(otherIndex).click();
    await page.waitForFunction(
      previous => document.querySelector('.mobile-drawer__body')?.getAttribute('aria-label') !== previous,
      panelBefore,
      { timeout: options.timeout },
    );
    const activeAfter = await page.locator('.mobile-drawer__tabs [role="tab"][aria-selected="true"]').first().textContent();
    const panelAfter = await panel.getAttribute('aria-label');
    const textAfter = (await panel.innerText()).trim();
    record(
      checks,
      activeAfter !== activeBefore,
      'clicking a chip moves the selected tab',
      { before: activeBefore, after: activeAfter },
    );
    record(
      checks,
      panelAfter !== panelBefore && textAfter !== textBefore,
      'the panel content changes with the tab',
      { before: panelBefore, after: panelAfter },
    );
  }

  await page.keyboard.press('Escape');
  await drawer.waitFor({ state: 'detached', timeout: options.timeout });
  await page.waitForFunction(
    () => document.querySelector('.mobile-topbar__menu')?.getAttribute('aria-expanded') === 'false',
    undefined,
    { timeout: options.timeout },
  );
  record(checks, (await page.locator('.mobile-drawer').count()) === 0, 'Escape closes the drawer', null);
  record(checks, (await menu.getAttribute('aria-expanded')) === 'false', 'aria-expanded returns to "false"', null);

  // The command draft lives in the shared shell state (`shell.command`), not in
  // the field: the field echoes it back, and the completion sheet is filtered by
  // it, which is the same draft the desktop command line would render.
  const field = page.locator('#cad-command');
  await field.waitFor({ state: 'visible', timeout: options.timeout });
  record(checks, await field.isEnabled(), 'the mobile command field accepts input (session ready)', null);
  await field.fill(TYPED_COMMAND);
  const echoed = await field.inputValue();
  record(checks, echoed === TYPED_COMMAND, 'typed text is reflected back from the shared draft', { typed: TYPED_COMMAND, echoed });

  const sheet = page.locator('.mobile-sheet [role="listbox"]');
  let filtered = [];
  try {
    await sheet.waitFor({ state: 'visible', timeout: 10000 });
    filtered = await page.locator('.mobile-sheet .completion__id').allTextContents();
  } catch (error) {
    filtered = [];
  }
  record(
    checks,
    filtered.some(id => id.trim().toUpperCase().startsWith(TYPED_COMMAND)),
    'the completion sheet is filtered by the same draft',
    { typed: TYPED_COMMAND, rows: filtered.slice(0, 8) },
  );
}

// --- the live resize case --------------------------------------------------

/**
 * The threshold's own two edges, crossed live: `1025x900` is the first `desktop`
 * width, `1024x900` the last `mobile` one (`max-width` is inclusive). One pixel is
 * the whole difference, so this is the only check that catches a constant/query
 * disagreement; region presence is asserted too, because an attribute flip alone
 * does not prove the stylesheet switched with it.
 */
async function crossTightBreakpoint(page, options, checks) {
  const at = async (width, height) => {
    await page.setViewportSize({ width, height });
    await waitForSession(page, options.timeout);
    return readLayout(page);
  };
  const desktop = await at(MOBILE_MAX_WIDTH + 1, 900);
  record(checks, desktop.shell === 'desktop' && desktop.documentElement === 'desktop',
    `${MOBILE_MAX_WIDTH + 1}x900 stays desktop (1024 is the last mobile width)`,
    { shell: desktop.shell, documentElement: desktop.documentElement });
  record(checks, await page.locator('.ribbon').isVisible(), 'the desktop ribbon is present at 1025', null);

  const mobile = await at(MOBILE_MAX_WIDTH, 900);
  record(checks, mobile.shell === 'mobile' && mobile.documentElement === 'mobile',
    `${MOBILE_MAX_WIDTH}x900 flips to mobile (max-width is inclusive)`,
    { shell: mobile.shell, documentElement: mobile.documentElement });
  record(checks, await page.locator('.mobile-commandbar').isVisible(), 'the compact command bar is present at 1024', null);
  record(checks, (await page.locator('.ribbon').count()) === 0, 'the desktop ribbon is gone at 1024', null);
  const sizing = canvasSizing(await readSurface(page));
  record(checks, sizing.ok, 'the CAD canvas is resized again at the breakpoint crossing', sizing.detail);
  const pixels = await measureCanvasPixels(page, options.evidence, 'presented-resize-1024');
  record(checks, pixels.ok, 'the CAD canvas still shows a frame at 1024x900', pixels.detail);
  record(checks, (await page.evaluate(() => window.__yacrProbe)) === 1,
    'no reload across the 1px threshold crossing (marker survives)', null);
}

async function runResizeCase(browser, options) {
  const checks = [];
  const bucket = { pageErrors: [], failedRequests: [], consoleLines: [] };
  const name = 'live-resize 1280x800 <-> 390x844 and 1025x900 <-> 1024x900';
  const screenshots = [];
  const observations = [];
  const container = await browser.newContext({ viewport: { ...DESKTOP_VIEWPORT }, deviceScaleFactor: 1 });
  const page = await newPage(container, bucket);
  let renderer = { webgl2: false, renderer: null };
  try {
    const response = await page.goto(options.url, { waitUntil: 'load', timeout: options.timeout });
    if (response === null || !response.ok()) {
      throw new Error(`the bundle URL did not load: ${options.url} -> ${response?.status() ?? 'no response'}`);
    }
    await waitForSession(page, options.timeout);
    record(checks, (await readLayout(page)).shell === 'desktop', 'starts in the desktop layout', null);
    const alertsAtLoad = await readErrorAlerts(page);

    // The marker is the proof that no navigation happened: a reload would drop it.
    await page.evaluate(() => { window.__yacrProbe = 1; });

    await page.setViewportSize({ width: MOBILE_VIEWPORT.width, height: MOBILE_VIEWPORT.height });
    await waitForSession(page, options.timeout);
    const mobileLayout = await readLayout(page);
    const mobileSurface = await readSurface(page);
    record(
      checks,
      mobileLayout.shell === 'mobile' && mobileLayout.documentElement === 'mobile',
      'resizing to 390x844 flips data-layout to mobile (live)',
      { shell: mobileLayout.shell, documentElement: mobileLayout.documentElement },
    );
    record(checks, await page.locator('.mobile-topbar').isVisible(), 'mobile top bar appears after the resize', null);
    record(checks, await page.locator('.mobile-commandbar').isVisible(), 'mobile command bar appears after the resize', null);
    record(checks, (await page.locator('.ribbon').count()) === 0, 'the desktop ribbon is gone after the resize', null);
    record(checks, (await page.locator('.statusbar').count()) === 0, 'the desktop status bar is gone after the resize', null);
    record(
      checks,
      (await page.evaluate(() => window.__yacrProbe)) === 1,
      'no reload: the __yacrProbe marker survives the resize',
      null,
    );
    checkMobileChrome(checks, mobileSurface, mobileLayout.viewport);
    const mobileSizing = canvasSizing(mobileSurface);
    record(checks, mobileSizing.ok, 'the CAD canvas was resized with the layout', mobileSizing.detail);
    // The resize re-renders at the new size; a present that draws nothing would
    // leave a uniform band here even though every size is correct.
    const resizedPixels = await measureCanvasPixels(page, options.evidence, 'presented-resize-390');
    record(
      checks,
      resizedPixels.ok,
      'the CAD canvas still shows a frame after the resize to 390x844',
      resizedPixels.detail,
    );
    renderer = await readRenderer(page);
    const alertsAfterMobile = await readErrorAlerts(page);

    if (options.evidence) {
      const shot = await takeScreenshot(page, options.evidence, 'resize-390.png');
      screenshots.push(shot);
    }

    await page.setViewportSize({ ...DESKTOP_VIEWPORT });
    await waitForSession(page, options.timeout);
    const backLayout = await readLayout(page);
    record(
      checks,
      backLayout.shell === 'desktop' && backLayout.documentElement === 'desktop',
      'resizing back to 1280x800 restores the desktop layout',
      { shell: backLayout.shell, documentElement: backLayout.documentElement },
    );
    record(checks, await page.locator('.ribbon').isVisible(), 'the desktop ribbon is back', null);
    record(checks, await page.locator('.statusbar').isVisible(), 'the desktop status bar is back', null);
    record(checks, (await page.locator('.mobile-topbar').count()) === 0, 'no mobile top bar after resizing back', null);
    record(checks, (await page.locator('.mobile-commandbar').count()) === 0, 'no mobile command bar after resizing back', null);
    record(
      checks,
      (await page.evaluate(() => window.__yacrProbe)) === 1,
      'no reload: the __yacrProbe marker survives both crossings',
      null,
    );
    const restoredPixels = await measureCanvasPixels(page, options.evidence, 'presented-resize-back');
    record(
      checks,
      restoredPixels.ok,
      'the CAD canvas shows a frame again after the resize back to desktop',
      restoredPixels.detail,
    );
    record(checks, bucket.pageErrors.length === 0, 'no uncaught page error during the resizes', bucket.pageErrors);

    // The banner must be absent at the load and stay absent across both
    // crossings: the carrier does not change, so nothing may start refusing
    // `probe_input` mid-session either.
    const alertsAfterRestore = await readErrorAlerts(page);
    record(
      checks,
      alertsAtLoad.length === 0 && alertsAfterMobile.length === 0 && alertsAfterRestore.length === 0,
      'no visible error banner after load/settle and after both resizes',
      { load: alertsAtLoad, mobile: alertsAfterMobile, restored: alertsAfterRestore },
    );
    for (const text of alertsAtLoad) observations.push(`visible alert (load): ${text}`);
    for (const text of alertsAfterMobile) observations.push(`visible alert (390): ${text}`);
    for (const text of alertsAfterRestore) observations.push(`visible alert (restored): ${text}`);

    // The wide crossing above cannot see an off-by-one in the constant: at 1025 the
    // layout is desktop and at 1024 it is mobile, and one pixel is the whole
    // difference. Both crossings are live (the marker is asserted again at the end).
    await crossTightBreakpoint(page, options, checks);
    await page.setViewportSize({ ...DESKTOP_VIEWPORT });
    await waitForSession(page, options.timeout);
    record(
      checks,
      (await readLayout(page)).shell === 'desktop',
      'and back to 1280x800 leaves the desktop layout in place',
      null,
    );

    if (options.evidence) {
      const shot = await takeScreenshot(page, options.evidence, 'resize-back-1280.png');
      screenshots.push(shot);
    }
  } catch (error) {
    record(checks, false, 'resize case ran to completion', error.message);
  } finally {
    await container.close();
  }
  return { name, checks, bucket, renderer, screenshots, observations };
}

// --- reporting -------------------------------------------------------------

function printCase(result, verbose) {
  const failed = result.checks.filter(check => !check.ok);
  const passed = result.checks.length - failed.length;
  const head = failed.length === 0 ? 'PASS' : 'FAIL';
  const rendering = softwareRenderer(result.renderer?.renderer)
    ? `software (SwiftShader: ${result.renderer.renderer})`
    : `NOT software-labelled (${result.renderer?.renderer ?? 'no WebGL2'}) - still not GPU acceptance`;
  console.log(
    `${head} ${result.name} (${passed}/${result.checks.length} checks) ` +
      `gl_renderer=${describe(result.renderer?.renderer ?? 'unknown')} rendering=${rendering}`,
  );
  if (verbose) {
    for (const check of result.checks) {
      const mark = check.ok ? 'ok' : 'FAILED';
      console.log(`  ${mark} ${check.label}${check.detail === null ? '' : ` -> ${describe(check.detail)}`}`);
    }
  }
  for (const check of failed) {
    if (verbose) break;
    console.log(`  FAILED ${check.label}${check.detail === null ? '' : ` -> ${describe(check.detail)}`}`);
  }
  for (const shot of result.screenshots) {
    console.log(`  screenshot ${shot.file} (${shot.bytes} bytes)`);
  }
  for (const observation of result.observations) {
    console.log(`  observed (not asserted): ${observation}`);
  }
  if (failed.length > 0) {
    for (const line of result.bucket.pageErrors.slice(-10)) console.log(`  pageerror: ${line}`);
    for (const line of result.bucket.failedRequests.slice(-10)) console.log(`  requestfailed: ${line}`);
    for (const line of result.bucket.consoleLines.slice(-15)) console.log(`  console: ${line}`);
  }
  return failed.length === 0;
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  if (!fs.existsSync(path.join(BUNDLE_DIR, 'index.html'))) {
    throw new Error(
      `no browser bundle at ${BUNDLE_DIR} (build it first: bash scripts/build-web-ui.sh)`,
    );
  }
  if (!fs.existsSync(SERVER_SCRIPT)) {
    throw new Error(`the static server script is missing: ${SERVER_SCRIPT}`);
  }
  if (options.evidence) fs.mkdirSync(options.evidence, { recursive: true });

  const serverLog = [];
  const child = spawn(
    'python3',
    [SERVER_SCRIPT, '--directory', BUNDLE_DIR, '--port', String(options.port)],
    { cwd: REPO_ROOT, stdio: ['ignore', 'pipe', 'pipe'] },
  );
  child.stdout.setEncoding('utf8');
  child.stderr.setEncoding('utf8');
  child.stdout.on('data', chunk => serverLog.push(String(chunk)));
  child.stderr.on('data', chunk => serverLog.push(String(chunk)));
  // Belt and braces: a killed process must still take the server down.
  process.on('exit', () => {
    if (child.exitCode === null) child.kill('SIGKILL');
  });

  let browser = null;
  const results = [];
  try {
    await waitForServer(options.url, child, options.timeout, serverLog);
    if (child.exitCode !== null) {
      throw new Error(
        `the static server exited with code ${child.exitCode} after answering; ` +
          `is port ${options.port} already in use by another process?`,
      );
    }
    console.log(`[probe] serving ${BUNDLE_DIR} at ${options.url} (playwright chromium, SwiftShader)`);
    if (options.evidence) console.log(`[probe] evidence directory ${options.evidence}`);

    const { chromium } = loadPlaywright();
    browser = await chromium.launch({ headless: true, args: [...BROWSER_ARGS] });
    for (const spec of VIEWPORT_CASES) {
      results.push(await runViewportCase(browser, options, spec));
    }
    results.push(await runResizeCase(browser, options));
  } finally {
    if (browser !== null) await browser.close();
    await stopServer(child);
  }

  let passed = 0;
  let checks = 0;
  let failedChecks = 0;
  let observations = 0;
  for (const result of results) {
    if (printCase(result, options.verbose)) passed += 1;
    checks += result.checks.length;
    failedChecks += result.checks.filter(check => !check.ok).length;
    observations += result.observations.length;
  }
  console.log(
    `${failedChecks === 0 ? 'PASS' : 'FAIL'} ${passed}/${results.length} cases ` +
      `(${checks - failedChecks}/${checks} checks)`,
  );
  if (observations > 0) {
    console.log(
      `note: ${observations} observation(s) printed above are visible page state this suite does not ` +
        'assert (they are not part of the responsive-chrome contract).',
    );
  }

  const software = results.some(result => softwareRenderer(result.renderer?.renderer));
  if (software) {
    console.log(
      'note: this run rendered through SwiftShader (software rasterisation). ' +
        'It is layout/DOM evidence only - not GPU, pixel, performance or native-host acceptance.',
    );
  }
  if (options.evidence) {
    console.log('note: the screenshots above are diagnostic artefacts, not visual acceptance.');
  }
  return failedChecks === 0 ? 0 : 1;
}

main()
  .then(code => {
    process.exitCode = code;
  })
  .catch(error => {
    console.error(`web-ui-smoke FAILED: ${error.message}`);
    process.exitCode = 1;
  });
