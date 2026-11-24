#!/usr/bin/env node
/*
 * Domain chrome golden screenshots (ADR-0053 D7).
 *
 * Captures the four domain profiles at one fixed viewport and compares them with
 * the checked-in baseline in `apps/desktop-ui/test/golden/ui/`. The baselines are
 * **our own** screenshots (never the reference applications' manual images,
 * which are copyrighted and not in this repository).
 *
 * What this proves: the rendered DOM chrome (tabs, panels, palette, placeholder
 * card) is stable at the captured viewport and that switching domains still
 * changes the profile. What it does NOT prove: GPU rendering, pixels of the CAD
 * surface, performance, or any real Windows/macOS/Android/Web acceptance. The
 * page is Chromium + SwiftShader/software; treat every diff as layout evidence.
 *
 * Usage:
 *   node scripts/ui-golden.cjs --url http://127.0.0.1:8090/ --out /tmp/ui-golden
 *   node scripts/ui-golden.cjs --url ... --update     # rewrite the baselines
 *
 * Requires a Chromium build. `CHROME_BIN` wins; otherwise the newest
 * `~/.cache/ms-playwright/chromium_headless_shell-<build>/chrome-headless-shell-linux64/chrome-headless-shell`
 * is used. Chromium's CDP is spoken directly (no Playwright dependency).
 */
'use strict';

const { spawn } = require('node:child_process');
const { once } = require('node:events');
const { existsSync, readdirSync, mkdirSync, writeFileSync, readFileSync } = require('node:fs');
const { tmpdir, homedir } = require('node:os');
const { join } = require('node:path');
const { pathToFileURL } = require('node:url');
const { setTimeout: sleep } = require('node:timers/promises');

const ROOT = join(__dirname, '..');
const BASELINE_DIR = join(ROOT, 'apps', 'desktop-ui', 'test', 'golden', 'ui');
const VIEWPORT = { width: 1600, height: 1000 };
const DOMAINS = [
  { id: 'cad2d', row: 0, label: '2D drafting (AutoCAD profile)' },
  { id: 'bim', row: 1, label: 'BIM placeholder' },
  { id: 'concept', row: 2, label: 'Concept placeholder' },
  { id: 'mechanical', row: 3, label: 'Mechanical placeholder' },
];
// A pixel is "different" when any channel moves more than this. Anti-aliasing
// and font hinting move a few pixels around glyph edges; the ratio gate below is
// what catches a real layout/profile change.
const PIXEL_DELTA = 12;
const MAX_DIFF_RATIO = 0.02;

function parseArgs(argv) {
  const options = { update: false, url: 'http://127.0.0.1:8090/', out: null };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--update') options.update = true;
    else if (arg === '--url') options.url = argv[++i];
    else if (arg === '--out') options.out = argv[++i];
    else throw new Error(`unknown argument ${arg}`);
  }
  if (!options.out) options.out = join(tmpdir(), 'yacr-ui-golden');
  return options;
}

function findChrome() {
  if (process.env.CHROME_BIN) return process.env.CHROME_BIN;
  const cache = join(homedir(), '.cache', 'ms-playwright');
  if (!existsSync(cache)) {
    throw new Error('no Chromium found: set CHROME_BIN or install a Playwright Chromium');
  }
  const candidates = readdirSync(cache)
    .filter(name => name.startsWith('chromium_headless_shell-'))
    .sort()
    .reverse()
    .map(name => join(cache, name, 'chrome-headless-shell-linux64', 'chrome-headless-shell'))
    .filter(existsSync);
  if (!candidates.length) throw new Error('no chrome-headless-shell under ~/.cache/ms-playwright');
  return candidates[0];
}

async function launchChrome(bin, userDataDir) {
  const child = spawn(bin, [
    '--headless',
    '--disable-gpu',
    '--no-sandbox',
    '--hide-scrollbars',
    `--window-size=${VIEWPORT.width},${VIEWPORT.height}`,
    '--remote-debugging-port=0',
    `--user-data-dir=${userDataDir}`,
    'about:blank',
  ], { stdio: ['ignore', 'ignore', 'pipe'] });
  const endpoint = await new Promise((resolve, reject) => {
    let buffer = '';
    const timer = setTimeout(() => reject(new Error(`Chromium did not announce CDP:\n${buffer}`)), 15000);
    child.stderr.setEncoding('utf8');
    child.stderr.on('data', chunk => {
      buffer += chunk;
      const match = buffer.match(/DevTools listening on (ws:\/\/\S+)/);
      if (match) {
        clearTimeout(timer);
        resolve(match[1]);
      }
    });
    child.on('exit', code => reject(new Error(`Chromium exited early (${code}):\n${buffer}`)));
  });
  const host = new URL(endpoint).host;
  return { child, httpBase: `http://${host}` };
}

function connect(url) {
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(url);
    const pending = new Map();
    let next = 0;
    socket.onmessage = event => {
      const message = JSON.parse(event.data);
      const waiter = pending.get(message.id);
      if (!waiter) return;
      pending.delete(message.id);
      message.error ? waiter.reject(new Error(JSON.stringify(message.error))) : waiter.resolve(message.result);
    };
    socket.onerror = reject;
    socket.onopen = () => resolve({
      socket,
      send(method, params = {}) {
        return new Promise((res, rej) => {
          const id = ++next;
          pending.set(id, { resolve: res, reject: rej });
          socket.send(JSON.stringify({ id, method, params }));
        });
      },
    });
  });
}

async function evaluate(client, expression) {
  const result = await client.send('Runtime.evaluate', {
    expression,
    returnByValue: true,
    awaitPromise: true,
  });
  if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
  return result.result.value;
}

async function waitFor(client, expression, label, timeout = 20000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await evaluate(client, expression)) return;
    await sleep(150);
  }
  throw new Error(`timed out waiting for ${label}`);
}

async function capture(client, path) {
  const result = await client.send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
  writeFileSync(path, Buffer.from(result.data, 'base64'));
}

async function chooseDomain(client, row) {
  await evaluate(client, `document.querySelector('.domain-switcher__button').click()`);
  await waitFor(client, `document.querySelectorAll('.domain-switcher__row').length >= 4`, 'domain menu');
  await evaluate(client, `document.querySelectorAll('.domain-switcher__row')[${row}].click()`);
  await sleep(350);
}

async function compare(actual, expected) {
  const { decodePng } = await import(pathToFileURL(join(ROOT, 'scripts', 'web-pixels.mjs')).href);
  const a = decodePng(actual);
  const b = decodePng(expected);
  if (a.width !== b.width || a.height !== b.height) {
    return { ok: false, reason: `size ${a.width}x${a.height} != ${b.width}x${b.height}` };
  }
  let different = 0;
  for (let i = 0; i < a.pixels.length; i += a.channels) {
    const delta = Math.max(
      Math.abs(a.pixels[i] - b.pixels[i]),
      Math.abs(a.pixels[i + 1] - b.pixels[i + 1]),
      Math.abs(a.pixels[i + 2] - b.pixels[i + 2]),
    );
    if (delta > PIXEL_DELTA) different += 1;
  }
  const total = a.width * a.height;
  const ratio = different / total;
  return { ok: ratio <= MAX_DIFF_RATIO, ratio, different, total };
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const bin = findChrome();
  mkdirSync(options.out, { recursive: true });
  mkdirSync(BASELINE_DIR, { recursive: true });
  const profile = join(tmpdir(), `yacr-ui-golden-profile-${process.pid}`);
  const { child, httpBase } = await launchChrome(bin, profile);
  let failures = 0;
  try {
    const targets = await (await fetch(`${httpBase}/json/list`)).json();
    const page = targets.find(target => target.type === 'page');
    if (!page) throw new Error('Chromium exposed no page target');
    const client = await connect(page.webSocketDebuggerUrl);
    await client.send('Page.enable');
    await client.send('Runtime.enable');
    await client.send('Page.navigate', { url: options.url });
    await waitFor(client, `Boolean(document.querySelector('.domain-switcher__button'))`, 'the domain switcher');
    await evaluate(client, `document.fonts ? document.fonts.ready.then(() => true) : true`);
    await sleep(600);

    for (const domain of DOMAINS) {
      if (domain.row > 0) await chooseDomain(client, domain.row);
      const current = await evaluate(client, `document.documentElement.dataset.domain`);
      if (current !== domain.id) throw new Error(`${domain.id}: rendered domain is ${current}`);
      const shot = join(options.out, `${domain.id}.png`);
      await capture(client, shot);
      const baseline = join(BASELINE_DIR, `${domain.id}.png`);
      if (options.update || !existsSync(baseline)) {
        writeFileSync(baseline, readFileSync(shot));
        console.log(`baseline ${domain.id}: ${baseline}`);
        continue;
      }
      const result = await compare(readFileSync(shot), readFileSync(baseline));
      if (result.ok) {
        console.log(`ok ${domain.id}: ${(result.ratio * 100).toFixed(3)}% pixels differ`);
      } else {
        failures += 1;
        console.log(`FAIL ${domain.id}: ${(result.ratio * 100).toFixed(3)}% pixels differ (${result.different}/${result.total})`);
      }
    }
  } finally {
    child.kill('SIGTERM');
    await Promise.race([once(child, 'exit'), sleep(3000)]);
  }
  console.log('layout-only evidence: software Chromium; not GPU, pixel or performance acceptance');
  if (failures) process.exitCode = 1;
}

main().catch(error => {
  console.error(error);
  process.exitCode = 2;
});
