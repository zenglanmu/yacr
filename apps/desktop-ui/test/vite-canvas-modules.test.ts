// Contracts for the dev-only Vite plugin that serves the generated `dom-canvas`
// artifacts (see `vite-canvas-modules.mjs`).
//
// Scope, stated honestly: these are **unit** tests against the plugin object and
// its pure id/path helpers, plus a **source-structure** assertion that
// `vite.config.mjs` registers it. There is no dev server, no HTTP request and no
// browser here; the live verification (200 + verbatim bytes for
// `/canvas/bootstrap.js` and `/canvas/app_web.js`) is a separate, manual check.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { canvasModulePath, canvasPublicModules, isCanvasModuleRequest } from '../vite-canvas-modules.mjs';

const CONFIG = new URL('../vite.config.mjs', import.meta.url);
const PACKAGE_DIR = fileURLToPath(new URL('..', import.meta.url));

test('the plugin is dev-only, pre-ordered, and answers ids through resolveId/load', () => {
  const plugin = canvasPublicModules();
  assert.equal(plugin.name, 'yacr-canvas-public-modules');
  // Production must stay untouched: at build time `public/**` is copied verbatim
  // and the runtime request is answered by the static file server.
  assert.equal(plugin.apply, 'serve');
  // Must win against Vite's public-dir guard, which refuses these files otherwise.
  assert.equal(plugin.enforce, 'pre');
  assert.equal(typeof plugin.resolveId, 'function');
  assert.equal(typeof plugin.load, 'function');
  // `configResolved` re-points the directory at Vite's real `publicDir`.
  assert.equal(typeof plugin.configResolved, 'function');
  // Rebuilt artifacts must not keep serving a cached transform.
  assert.equal(typeof plugin.configureServer, 'function');
});

test('only flat /canvas/<name>.js requests are claimed', () => {
  assert.equal(isCanvasModuleRequest('/canvas/bootstrap.js'), true);
  assert.equal(isCanvasModuleRequest('/canvas/app_web.js'), true);
  assert.equal(isCanvasModuleRequest('/canvas/some.artifact-1.js'), true);

  for (const id of [
    '/canvas/app_web_bg.wasm',   // a binary, served by Vite's static handler
    '/canvas/',                  // no file name
    '/canvas/.js',               // no stem
    '/canvas/.hidden.js',        // dot-file
    '/canvas/sub/bootstrap.js',  // nested: not the generated layout
    '/canvas/../secret.js',      // traversal
    '/canvas/..%2fsecret.js',    // encoded traversal, still a dot-file name
    '/canvas/bootstrap.js?import', // the query Vite's asset plugin would append
    '/canvas/bootstrap.js#x',
    '/other/bootstrap.js',
    'bootstrap.js',
    '',
    0,
    null,
    undefined,
  ]) {
    assert.equal(isCanvasModuleRequest(id), false, `must not claim ${JSON.stringify(id)}`);
  }
});

test('a resolved path is always a direct child of public/canvas', () => {
  const dir = '/repo/apps/desktop-ui/public/canvas';
  assert.equal(canvasModulePath(dir, '/canvas/bootstrap.js'), '/repo/apps/desktop-ui/public/canvas/bootstrap.js');
  assert.equal(canvasModulePath(dir, '/canvas/../bootstrap.js'), null);
  assert.equal(canvasModulePath(dir, '/canvas/../../package.json'), null);
  assert.equal(canvasModulePath(dir, '/canvas/%2e%2e%2fpackage.json'), null);
  assert.equal(canvasModulePath(dir, '/canvas/app_web_bg.wasm'), null);
  assert.equal(canvasModulePath('', '/canvas/bootstrap.js'), null);
});

test('nothing is resolved or loaded for a missing artifact or a symlink escape', async () => {
  const root = await mkdtemp(join(tmpdir(), 'yacr-vite-canvas-'));
  try {
    const canvasDir = join(root, 'public', 'canvas');
    await mkdir(canvasDir, { recursive: true });
    await writeFile(join(canvasDir, 'bootstrap.js'), 'export const ok = 1;\n');
    // A file outside the canvas directory, reachable through a symlink inside it.
    const outside = join(root, 'outside.js');
    await writeFile(outside, 'export const leaked = true;\n');
    await symlink(outside, join(canvasDir, 'escape.js'));

    const plugin = canvasPublicModules({ root });

    // A real artifact is claimed and read verbatim.
    assert.equal(plugin.resolveId('/canvas/bootstrap.js'), '/canvas/bootstrap.js');
    assert.deepEqual(plugin.load('/canvas/bootstrap.js'), {
      code: 'export const ok = 1;\n',
      map: null,
    });

    // A missing artifact is declined, so Vite's own diagnostics stay in charge.
    assert.equal(plugin.resolveId('/canvas/missing.js'), null);
    assert.equal(plugin.load('/canvas/missing.js'), null);
    // A symlink out of the directory is declined too (realpath, not just the id).
    assert.equal(plugin.resolveId('/canvas/escape.js'), null);
    assert.equal(plugin.load('/canvas/escape.js'), null);
    // Non-.js and traversal ids never reach the file system.
    assert.equal(plugin.resolveId('/canvas/app_web_bg.wasm'), null);
    assert.equal(plugin.load('/canvas/../outside.js'), null);

    // `configResolved` re-points the plugin at Vite's resolved public directory.
    const other = join(root, 'public');
    const redirected = canvasPublicModules({ root: join(root, 'nowhere') });
    redirected.configResolved({ root: join(root, 'nowhere'), publicDir: other });
    assert.equal(redirected.load('/canvas/bootstrap.js').code, 'export const ok = 1;\n');
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('a rebuilt artifact invalidates the module it was loaded as', async () => {
  const root = await mkdtemp(join(tmpdir(), 'yacr-vite-watch-'));
  try {
    const canvasDir = join(root, 'public', 'canvas');
    await mkdir(canvasDir, { recursive: true });
    await writeFile(join(canvasDir, 'bootstrap.js'), 'export const ok = 1;\n');

    const watched = [];
    const listeners = new Map();
    const invalidated = [];
    const modules = { '/canvas/bootstrap.js': { id: '/canvas/bootstrap.js' } };
    const server = {
      watcher: {
        add(path) { watched.push(path); },
        on(event, handler) { listeners.set(event, handler); },
      },
      moduleGraph: {
        getModuleById(id) { return modules[id] ?? null; },
        invalidateModule(module) { invalidated.push(module.id); },
      },
    };
    const plugin = canvasPublicModules({ root });
    plugin.configureServer(server);
    assert.deepEqual(watched, [canvasDir]);

    // A change inside the canvas directory drops the cached module for that URL.
    listeners.get('change')(join(canvasDir, 'bootstrap.js'));
    assert.deepEqual(invalidated, ['/canvas/bootstrap.js']);
    // An unrelated change, a non-JS artifact or a module Vite never loaded is ignored.
    listeners.get('change')(join(root, 'index.html'));
    listeners.get('change')(join(canvasDir, 'app_web_bg.wasm'));
    listeners.get('add')(join(canvasDir, 'orphan.js'));
    assert.deepEqual(invalidated, ['/canvas/bootstrap.js']);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('vite.config.mjs registers the dev-only plugin', async () => {
  const config = await readFile(CONFIG, 'utf8');
  assert.match(config, /import \{ canvasPublicModules \} from '\.\/vite-canvas-modules\.mjs';/);
  assert.match(config, /plugins: \[canvasPublicModules\(\)\]/);
  // The plugin module itself must not sneak into the shipped UI sources.
  const src = await readFile(join(PACKAGE_DIR, 'src', 'cad', 'canvas-host.ts'), 'utf8');
  assert.ok(!src.includes('vite-canvas-modules'), 'the loader must not import dev tooling');
});
