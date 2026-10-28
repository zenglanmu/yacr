// The host transport contract: the Rust<->TS status wire, the runtime routing
// table, the browser adapter's command map, and the source/package structure
// that keeps the Tauri import in exactly one place.
//
// Like every suite here this file is `.ts` by filename only: Node's type
// stripping runs it, `tsc` never sees it. Assertions are behaviour (imported
// modules) or source text (structure), never pixels.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readdir, readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { parseCadStatus } from '../src/cad/cad-status.ts';
import { RUNTIME, RUNTIMES, isTauri, runtimeFrom, usesWasmCad } from '../src/host/runtime.ts';
import { SHELL_ONLY_COMMANDS, createInvoke, invokeRoute } from '../src/host/invoke.ts';
import {
  WEB_FAILURE_COMMAND,
  WEB_SESSION_COMMAND,
  WEB_UNAVAILABLE_PREFIX,
  createWebAdapter,
  loadWebShellModule,
  unsupportedRuntimeMessage,
} from '../src/host/web-adapter.ts';

const PKG_ROOT = new URL('../', import.meta.url);
const SRC_ROOT = new URL('../src/', import.meta.url);

// ---------------------------------------------------------------------------
// Fixtures: the two `shell_status()` answers a real wasm session produced
// (`apps/app-web/src/shell.rs`). These are the Rust<->TS contract: the chrome
// rejects a payload missing any field `parseCadStatus` enforces, so if the host
// stops sending one, this suite fails before a browser ever loads.
// ---------------------------------------------------------------------------

const STATUS_NO_SESSION = `{"adapter":null,"backend":null,"camera":null,"canFit":false,"canNew":false,"canOpen":false,"canPlot":false,"canSave":false,"commandLog":[],"crosshair":false,"cursorWorld":null,"documentLabel":null,"entityCount":null,"error":null,"frameCount":0,"gridEnabled":false,"gripDrag":false,"hoverPreview":false,"importCount":0,"importProgress":null,"loading":false,"orthoEnabled":false,"palettesAvailable":false,"popup":null,"selection":{"count":0,"empty":true,"mixedKeys":[],"rows":[],"text":null,"text_multiline":false},"snapEnabled":false,"snapGridEnabled":false,"snapKind":null,"snapSpacing":10.0,"surfaceMode":"dom-canvas","tool":null}`;

const STATUS_LIVE = `{"adapter":"webgl2: example adapter (wasm32 session)","backend":"webgl2","camera":{"center":[10.0,-5.0],"scale":0.25},"canFit":false,"canNew":false,"canOpen":false,"canPlot":false,"canSave":false,"canvasSession":{"batches":4,"draws":4,"label":"synthetic-four-lines.dwg"},"commandLog":[],"crosshair":false,"cursorWorld":null,"documentLabel":"synthetic-four-lines.dwg","entityCount":null,"error":null,"frameCount":7,"gridEnabled":false,"gripDrag":false,"hoverPreview":false,"importCount":1,"importProgress":null,"loading":false,"orthoEnabled":false,"palettesAvailable":false,"popup":null,"selection":{"count":0,"empty":true,"mixedKeys":[],"rows":[],"text":null,"text_multiline":false},"snapEnabled":false,"snapGridEnabled":false,"snapKind":null,"snapSpacing":10.0,"surfaceMode":"dom-canvas","tool":null}`;

const REFUSAL = 'unsupported-runtime: cad_command: "OPEN" is not executable — the browser carrier is view/navigate only';

/** A stand-in wasm session module: the four exports, over the two fixtures. */
function fakeModule({ status = STATUS_NO_SESSION, commands = '[]', aliases = '[]' } = {}) {
  const calls = [];
  return {
    calls,
    module: {
      ready: Promise.resolve(),
      shell_status() { calls.push('shell_status'); return status; },
      shell_commands() { calls.push('shell_commands'); return commands; },
      shell_aliases() { calls.push('shell_aliases'); return aliases; },
      shell_command(command) { calls.push(['shell_command', command]); throw REFUSAL; },
    },
  };
}

// ---------------------------------------------------------------------------
// 1. The status wire
// ---------------------------------------------------------------------------

test('parseCadStatus accepts a real shell_status() with no session', () => {
  const status = parseCadStatus(JSON.parse(STATUS_NO_SESSION));
  assert.equal(status.surfaceMode, 'dom-canvas');
  assert.equal(status.documentLabel, null);
  assert.equal(status.backend, null);
  assert.equal(status.canOpen, false);
  assert.equal(status.snapSpacing, 10);
});

test('parseCadStatus accepts a real shell_status() with a live session', () => {
  const status = parseCadStatus(JSON.parse(STATUS_LIVE));
  assert.equal(status.surfaceMode, 'dom-canvas');
  assert.equal(status.documentLabel, 'synthetic-four-lines.dwg');
  assert.equal(status.backend, 'webgl2');
  assert.match(status.adapter, /wasm32 session/);
  assert.deepEqual(status.camera, { center: [10, -5], scale: 0.25 });
  assert.equal(status.frameCount, 7);
  assert.equal(status.importCount, 1);
  assert.deepEqual(status.canvasSession, { batches: 4, draws: 4, label: 'synthetic-four-lines.dwg' });
  // A live session still advertises no document capability.
  for (const key of ['canNew', 'canOpen', 'canSave', 'canPlot', 'canFit']) {
    assert.equal(status[key], false, `${key} must stay false`);
  }
});

test('a status missing a required field is rejected, not coerced', () => {
  const broken = JSON.parse(STATUS_LIVE);
  delete broken.frameCount;
  assert.throws(() => parseCadStatus(broken), /CAD_STATUS_INVALID/);
});

// ---------------------------------------------------------------------------
// 2. The runtime and the routing table
// ---------------------------------------------------------------------------

test('the runtime falls back to tauri when no build define is present', () => {
  // Node imports this module without Vite's `define`; that must be the desktop
  // build, and an unknown injected value must be rejected outright.
  assert.equal(RUNTIME, 'tauri');
  assert.equal(usesWasmCad, false);
  assert.deepEqual([...RUNTIMES], ['tauri', 'web', 'android']);
  assert.equal(runtimeFrom('web'), 'web');
  assert.equal(runtimeFrom('android'), 'android');
  assert.equal(runtimeFrom('tauri'), 'tauri');
  assert.equal(runtimeFrom('browser'), null);
  assert.equal(runtimeFrom(undefined), null);
});

test('isTauri probes the injected bridge object, not the build', () => {
  assert.equal(isTauri({}), false);
  assert.equal(isTauri({ __TAURI_INTERNALS__: undefined }), false);
  assert.equal(isTauri({ __TAURI_INTERNALS__: {} }), true);
  assert.equal(isTauri(null), false);
});

test('every runtime x command class resolves to adapter or Tauri', () => {
  const shellOnly = 'canvas_runtime_failed';
  const shellOnlyOther = 'canvas_session_status';
  const ordinary = ['cad_status', 'cad_command', 'probe_input', 'cad_layers'];

  // A `tauri` host is Tauri for everything, bridge probe irrelevant.
  for (const present of [true, false]) {
    for (const command of [...ordinary, shellOnly, shellOnlyOther]) {
      assert.equal(invokeRoute('tauri', command, present), 'tauri', `tauri/${command}/${present}`);
    }
  }
  for (const runtime of ['web', 'android']) {
    for (const command of ordinary) {
      assert.equal(invokeRoute(runtime, command, true), 'web', `${runtime}/${command} (bridge)`);
      assert.equal(invokeRoute(runtime, command, false), 'web', `${runtime}/${command} (no bridge)`);
    }
    // The two shell-only reports prefer an existing Tauri bridge.
    for (const command of SHELL_ONLY_COMMANDS) {
      assert.equal(invokeRoute(runtime, command, true), 'tauri', `${runtime}/${command} (bridge)`);
      assert.equal(invokeRoute(runtime, command, false), 'web', `${runtime}/${command} (no bridge)`);
    }
  }
  assert.deepEqual([...SHELL_ONLY_COMMANDS], ['canvas_runtime_failed', 'canvas_session_status']);
});

test('createInvoke routes per call through the injected transports', async () => {
  const calls = [];
  const tauri = async (command, args) => { calls.push(['tauri', command, args]); return 'from-tauri'; };
  const web = { invoke: async (command, args) => { calls.push(['web', command, args]); return 'from-web'; } };

  const desktop = createInvoke({ runtime: 'tauri', tauri, web, tauriPresent: () => false });
  assert.equal(await desktop('cad_status'), 'from-tauri');

  const browser = createInvoke({ runtime: 'web', tauri, web, tauriPresent: () => false });
  assert.equal(await browser('cad_status'), 'from-web');
  assert.equal(await browser('canvas_runtime_failed', { step: 'webgl2' }), 'from-web');

  // Android: the same document has the bridge, so the two reports go to Tauri.
  const android = createInvoke({ runtime: 'android', tauri, web, tauriPresent: () => true });
  assert.equal(await android('cad_status'), 'from-web');
  assert.equal(await android('canvas_session_status', { label: 'x' }), 'from-tauri');

  assert.deepEqual(calls, [
    ['tauri', 'cad_status', undefined],
    ['web', 'cad_status', undefined],
    ['web', 'canvas_runtime_failed', { step: 'webgl2' }],
    ['web', 'cad_status', undefined],
    ['tauri', 'canvas_session_status', { label: 'x' }],
  ]);
});

// ---------------------------------------------------------------------------
// 3. The browser adapter's command map
// ---------------------------------------------------------------------------

test('the adapter serves the four shell commands from the wasm module', async () => {
  const fake = fakeModule({ commands: '["ZOOM"]', aliases: '[["Z","ZOOM"]]' });
  const adapter = createWebAdapter(fake.module);

  const status = await adapter.invoke('cad_status');
  assert.equal(status.surfaceMode, 'dom-canvas');
  assert.equal(status.documentLabel, null);
  assert.deepEqual(await adapter.invoke('cad_commands'), ['ZOOM']);
  assert.deepEqual(await adapter.invoke('cad_command_aliases'), [['Z', 'ZOOM']]);
  assert.deepEqual(fake.calls, ['shell_status', 'shell_commands', 'shell_aliases']);
});

test('cad_command rethrows the session refusal verbatim', async () => {
  const fake = fakeModule();
  const adapter = createWebAdapter(fake.module);
  await assert.rejects(adapter.invoke('cad_command', { command: 'OPEN' }), error => {
    assert.ok(error instanceof Error);
    assert.equal(error.message, REFUSAL);
    return true;
  });
  assert.deepEqual(fake.calls, [['shell_command', 'OPEN']]);
});

test('cad_command without a command name fails loudly', async () => {
  const adapter = createWebAdapter(fakeModule().module);
  await assert.rejects(adapter.invoke('cad_command'), /web-adapter: cad_command: missing command/);
  await assert.rejects(adapter.invoke('cad_command', { command: 7 }), /missing command/);
});

test('the two canvas reports are remembered and answered with no value', async () => {
  const logged = [];
  const adapter = createWebAdapter(fakeModule().module, { console: { error: (...args) => logged.push(args) } });

  assert.equal(await adapter.invoke(WEB_SESSION_COMMAND, { label: 'plan.dwg', batches: 4, draws: 3 }), undefined);
  assert.deepEqual(adapter.lastSession, { label: 'plan.dwg', batches: 4, draws: 3 });

  assert.equal(await adapter.invoke(WEB_FAILURE_COMMAND, { step: 'webgl2', detail: 'no context' }), undefined);
  assert.deepEqual(adapter.lastFailure, { step: 'webgl2', detail: 'no context', marker: 'yacr: unsupported-runtime: webgl2' });
  assert.deepEqual(logged, [['yacr: unsupported-runtime: webgl2', 'no context']]);

  // A malformed report is still loud, and never claims a step it was not given.
  assert.equal(await adapter.invoke(WEB_SESSION_COMMAND, {}), undefined);
  assert.deepEqual(adapter.lastSession, { label: null, batches: null, draws: null });
});

test('every command this carrier does not implement rejects with unsupported-runtime', async () => {
  const adapter = createWebAdapter(fakeModule().module);
  const unmapped = [
    'probe_input',
    'cad_standard_view',
    'cad_settings',
    'cad_set_setting',
    'cad_layers',
    'cad_set_layer_visible',
    'cad_restore_layers',
    'cad_set_active_layer',
    'cad_create_layer',
    'cad_rename_layer',
    'cad_delete_layer',
    'cad_set_tool_text',
    'cad_set_selected_text',
    'cad_set_tool_parameter',
    'cad_set_selected_property',
    'cad_open_popup',
    'cad_close_popup',
  ];
  for (const command of unmapped) {
    await assert.rejects(adapter.invoke(command, {}), error => {
      assert.equal(error.message, unsupportedRuntimeMessage(command));
      assert.ok(error.message.startsWith('unsupported-runtime: '));
      return true;
    });
  }
});

test('an unreadable module answer is a loud failure, never an empty success', async () => {
  const broken = fakeModule({ status: 'not json', commands: '{}', aliases: 'null' });
  const adapter = createWebAdapter(broken.module);
  await assert.rejects(adapter.invoke('cad_status'), /web-adapter: cad_status:/);
  await assert.rejects(adapter.invoke('cad_commands'), /web-adapter: cad_commands: expected a JSON array/);
  await assert.rejects(adapter.invoke('cad_command_aliases'), /web-adapter: cad_command_aliases: expected a JSON array/);
});

// ---------------------------------------------------------------------------
// 4. The lazy loader: a page that has not loaded the bootstrap gets one attempt
// ---------------------------------------------------------------------------

/** A page whose `<script>` insertion publishes `published` and fires `load`. */
function publishOnAppend(env, published, { fail = false } = {}) {
  const scripts = [];
  const document = {
    createElement(tag) {
      const listeners = {};
      const script = {
        tag,
        type: '',
        src: '',
        async: true,
        addEventListener(type, fn) { (listeners[type] ??= []).push(fn); },
        remove() {},
        fire(type) { for (const fn of listeners[type] ?? []) fn(); },
      };
      scripts.push(script);
      return script;
    },
    head: {
      appendChild(script) {
        if (fail) {
          script.fire('error');
          return;
        }
        env.__YACR_CANVAS_MODULE__ = published;
        script.fire('load');
      },
    },
  };
  return { env: Object.assign(env, { document, location: { origin: 'http://localhost:8000', href: 'http://localhost:8000/' } }), scripts };
}

/** The four navigation exports the `dom-canvas` loader also validates. */
function sessionStubs() {
  return { start() {}, resize() {}, zoom_at() {}, pan() {} };
}

test('a page without the canvas module fails with the unavailable prefix', async () => {
  await assert.rejects(loadWebShellModule({}), error => {
    assert.ok(error.message.startsWith(WEB_UNAVAILABLE_PREFIX));
    assert.match(error.message, /is not published/);
    // The loader's own words are carried, not swallowed.
    assert.match(error.message, /bootstrap did not load/);
    return true;
  });
  // A published object without `ready`, a rejected init, and a missing export
  // are all the same class of failure, and none of them answers a status.
  await assert.rejects(loadWebShellModule({ __YACR_CANVAS_MODULE__: {} }), /ready is missing/);
  await assert.rejects(
    loadWebShellModule({ __YACR_CANVAS_MODULE__: { ready: Promise.reject(new Error('no wasm')) } }),
    /init failed: Error: no wasm/,
  );
  await assert.rejects(
    loadWebShellModule({ __YACR_CANVAS_MODULE__: { ready: Promise.resolve(), shell_status() { return STATUS_NO_SESSION; } } }),
    /missing export shell_commands/,
  );
});

test('an unloaded page injects the canvas bootstrap instead of deadlocking', async () => {
  // The chrome only learns `surfaceMode: "dom-canvas"` from `cad_status`, so the
  // pixel transport cannot have injected the bootstrap yet: the first status poll
  // is what brings it up. This is the browser's boot order, driven headlessly.
  const fake = fakeModule();
  const page = publishOnAppend({}, { ...sessionStubs(), ...fake.module });
  const adapter = createWebAdapter(async () => loadWebShellModule(page.env));

  const status = await adapter.invoke('cad_status');
  assert.equal(status.surfaceMode, 'dom-canvas');
  assert.equal(page.scripts.length, 1);
  assert.equal(page.scripts[0].src, '/canvas/bootstrap.js');
  assert.equal(page.scripts[0].type, 'module');

  // The module is published now, so later calls never inject a second tag.
  assert.deepEqual(await adapter.invoke('cad_commands'), []);
  assert.equal(page.scripts.length, 1);
});

test('a page that cannot load the bootstrap reports the failure loudly', async () => {
  const page = publishOnAppend({}, undefined, { fail: true });
  await assert.rejects(loadWebShellModule(page.env), error => {
    assert.ok(error.message.startsWith(WEB_UNAVAILABLE_PREFIX));
    assert.match(error.message, /bootstrap did not load/);
    assert.match(error.message, /failed: error event/);
    return true;
  });
});

test('the lazy adapter loads the published module on first use', async () => {
  const fake = fakeModule();
  const scope = { __YACR_CANVAS_MODULE__: fake.module };
  let loads = 0;
  const adapter = createWebAdapter(async () => { loads++; return loadWebShellModule(scope); });
  const status = await adapter.invoke('cad_status');
  assert.equal(status.surfaceMode, 'dom-canvas');
  assert.equal(loads, 1);
});

test('the lazy adapter reports an unloaded page instead of a fabricated status', async () => {
  const adapter = createWebAdapter(async () => loadWebShellModule({}));
  await assert.rejects(adapter.invoke('cad_status'), /web-adapter: canvas module unavailable/);
});

// ---------------------------------------------------------------------------
// 5. Source structure: one Tauri import, one runtime define, two build scripts
// ---------------------------------------------------------------------------

async function sourceFiles(dir) {
  const entries = await readdir(dir, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const url = new URL(`${entry.name}${entry.isDirectory() ? '/' : ''}`, dir);
    if (entry.isDirectory()) files.push(...await sourceFiles(url));
    else if (/\.(ts|tsx)$/.test(entry.name)) files.push(url);
  }
  return files;
}

test('only host/invoke.ts imports the Tauri API', async () => {
  const files = await sourceFiles(SRC_ROOT);
  const importers = [];
  for (const url of files) {
    const source = await readFile(url, 'utf8');
    if (/from\s+['"]@tauri-apps\/api/.test(source)) importers.push(fileURLToPath(url));
  }
  assert.equal(importers.length, 1, `importers: ${importers.join(', ')}`);
  assert.ok(importers[0].endsWith('/src/host/invoke.ts'));
});

test('the package declares the web and android builds next to the tauri one', async () => {
  const pkg = JSON.parse(await readFile(new URL('package.json', PKG_ROOT), 'utf8'));
  assert.equal(pkg.scripts.build, 'vite build');
  assert.equal(pkg.scripts['build:web'], 'YACR_RUNTIME=web vite build --outDir ../app-web/dist-ui --emptyOutDir');
  // The Android host embeds its own artifact (`apps/app-android/dist`); it must
  // never overwrite the desktop hosts' `dist/`.
  assert.equal(pkg.scripts['build:android'], 'YACR_RUNTIME=android vite build --outDir ../app-android/dist --emptyOutDir');
  // The test gate stays typecheck + node tests.
  assert.equal(pkg.scripts.test, 'tsc --noEmit && node --test test/*.test.ts');
});

test('vite.config.mjs injects and validates __YACR_RUNTIME__', async () => {
  const config = await readFile(new URL('vite.config.mjs', PKG_ROOT), 'utf8');
  assert.match(config, /__YACR_RUNTIME__:\s*JSON\.stringify\(runtime\)/);
  assert.match(config, /process\.env\.YACR_RUNTIME/);
  assert.match(config, /\['tauri', 'web', 'android'\]/);
  assert.match(config, /throw new Error/);
  // The existing base and canvas plugin must survive.
  assert.match(config, /base: '\.\/'/);
  assert.match(config, /canvasPublicModules\(\)/);
});
