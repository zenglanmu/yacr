// The **observed carrier** contract: one state machine (`host/runtime.ts`), one
// routing table built on it (`host/invoke.ts`), and the chrome's reaction to a
// flip (`App.tsx`).
//
// Why this file exists: a build target is not a carrier. The Linux host decides
// at startup whether it runs `native-below` (native Wayland: a native CAD session
// under the WebView) or `dom-canvas` (X11/XWayland: the wasm session in the page,
// `apps/app-linux/src/host/surface.rs` `native_cad_session: false`) and reports it
// through `cad_status.surfaceMode`. A `tauri` bundle must therefore be able to
// **change its mind** about who serves CAD data; before this contract it kept the
// desktop vocabulary on X11 and advertised commands nothing could execute.
//
// Same conventions as the rest of `test/`: `.ts` by filename only, Node's type
// stripping runs it, `tsc` never sees it, and every assertion is behaviour or
// source text - never pixels.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import {
  RUNTIME,
  carrierNow,
  createCarrierStore,
  initialCarrier,
  observeCarrier,
  usesWasmCad,
} from '../src/host/runtime.ts';
import { SHELL_ONLY_COMMANDS, createInvoke, invokeRoute } from '../src/host/invoke.ts';

const APP_TSX = new URL('../src/App.tsx', import.meta.url);

/** Commands that read or write CAD document data: the adapter owns them on `dom-canvas`. */
const DATA_COMMANDS = [
  'cad_status',
  'cad_commands',
  'cad_command_aliases',
  'cad_command',
  'cad_layers',
  'cad_set_layer_visible',
  'cad_setting',
  'cad_settings',
  'probe_input',
  'cad_standard_view',
  'cad_open_popup',
];

// ---------------------------------------------------------------------------
// 1. The state machine
// ---------------------------------------------------------------------------

test('a build starts on the carrier it can prove', () => {
  // The desktop bundle cannot know before the host answers - and its first
  // `cad_status` is the question, so guessing "adapter" would deadlock the X11
  // host out of ever announcing itself.
  assert.equal(initialCarrier('tauri'), 'unknown');
  // The two carriers whose authority is decided by the build itself.
  assert.equal(initialCarrier('web'), 'dom-canvas');
  assert.equal(initialCarrier('android'), 'dom-canvas');

  // Node imports the module without Vite's define: the desktop build, still
  // unproven, so nothing is routed to the wasm session yet.
  assert.equal(RUNTIME, 'tauri');
  assert.equal(carrierNow(), 'unknown');
  assert.equal(usesWasmCad(), false);
});

test('observeCarrier selects only the exact dom-canvas spelling', () => {
  const store = createCarrierStore('unknown');
  assert.equal(store.usesWasmCad(), false);

  assert.equal(store.observe('dom-canvas'), true);
  assert.equal(store.now(), 'dom-canvas');
  assert.equal(store.usesWasmCad(), true);
  // The same report twice is not a change: a poll must not churn the chrome.
  assert.equal(store.observe('dom-canvas'), false);

  // Everything else resolves to `native-below`, exactly like
  // `cad/surface.ts`'s `surfaceModeFromShell`: near misses are not matched
  // loosely and a missing field (an older host) is the historic carrier.
  for (const value of ['DOM-CANVAS', 'dom-canvas ', 'dom_canvas', 'domCanvas', '',
    'native-below', null, undefined, 7, {}, []]) {
    const probe = createCarrierStore('dom-canvas');
    assert.equal(probe.observe(value), true, `${JSON.stringify(value)} must flip to native-below`);
    assert.equal(probe.now(), 'native-below', `${JSON.stringify(value)} must resolve to native-below`);
    assert.equal(probe.usesWasmCad(), false);
  }
});

test('unknown -> native-below is a change, and it is announced', () => {
  // Decision, documented in `host/runtime.ts`: "changed" means the modelled value
  // is different now, so the first native report is a transition like any other.
  // Announcing it is inert (the chrome only reacts to the wasm carrier), and
  // keeping one rule is what makes the answer predictable.
  const store = createCarrierStore('unknown');
  const seen = [];
  const unsubscribe = store.subscribe(carrier => seen.push(carrier));

  assert.equal(store.observe(undefined), true);
  assert.deepEqual(seen, ['native-below']);
  // No churn afterwards: a native host polls at 4 Hz.
  assert.equal(store.observe('native-below'), false);
  assert.deepEqual(seen, ['native-below']);

  assert.equal(store.observe('dom-canvas'), true);
  assert.deepEqual(seen, ['native-below', 'dom-canvas']);

  unsubscribe();
  assert.equal(store.observe('native-below'), true);
  assert.deepEqual(seen, ['native-below', 'dom-canvas'], 'no notification after unsubscribe');

  // A listener that unsubscribes *during* a notification does not disturb it.
  const other = createCarrierStore('unknown');
  const both = [];
  const off = other.subscribe(() => { both.push('a'); off(); });
  other.subscribe(() => both.push('b'));
  other.observe('dom-canvas');
  assert.deepEqual(both, ['a', 'b']);
});

test('the module singleton is the same state machine', () => {
  // Flipped through the exported functions (not the factory) so the production
  // wiring is covered too. This suite is the last reader in its own process.
  assert.equal(observeCarrier('dom-canvas'), true);
  assert.equal(carrierNow(), 'dom-canvas');
  assert.equal(usesWasmCad(), true);
  assert.equal(observeCarrier('dom-canvas'), false);
});

// ---------------------------------------------------------------------------
// 2. The routing table
// ---------------------------------------------------------------------------

test('every build x observed carrier x command class resolves the same way', () => {
  for (const command of DATA_COMMANDS) {
    // A `tauri` bundle routes data natively while the carrier is unproven or
    // native, and to the adapter the moment the host reports `dom-canvas`.
    assert.equal(invokeRoute('tauri', command, false, 'unknown'), 'tauri', `tauri/unknown/${command}`);
    assert.equal(invokeRoute('tauri', command, true, 'unknown'), 'tauri', `tauri/unknown/${command} (bridge)`);
    assert.equal(invokeRoute('tauri', command, false, 'native-below'), 'tauri', `tauri/native/${command}`);
    assert.equal(invokeRoute('tauri', command, true, 'native-below'), 'tauri', `tauri/native/${command} (bridge)`);
    assert.equal(invokeRoute('tauri', command, true, 'dom-canvas'), 'web', `tauri/dom-canvas/${command}`);

    // The browser and the Android carrier never answer CAD data out of Tauri,
    // whatever was reported.
    for (const runtime of ['web', 'android']) {
      for (const carrier of ['dom-canvas', 'native-below', 'unknown']) {
        assert.equal(
          invokeRoute(runtime, command, true, carrier),
          'web',
          `${runtime}/${carrier}/${command}`,
        );
      }
    }
  }

  assert.deepEqual([...SHELL_ONLY_COMMANDS], ['canvas_runtime_failed', 'canvas_session_status']);
  for (const command of SHELL_ONLY_COMMANDS) {
    // The two reports prefer an existing bridge on every carrier the adapter
    // owns data on - an Android WebView, and an X11 host (which is where the
    // marker line and exit code live).
    assert.equal(invokeRoute('android', command, true, 'dom-canvas'), 'tauri', `android/${command}`);
    assert.equal(invokeRoute('tauri', command, true, 'dom-canvas'), 'tauri', `tauri/${command}`);
    // ...and stay on the adapter when there is no bridge to report to.
    assert.equal(invokeRoute('web', command, false, 'dom-canvas'), 'web', `web/${command}`);
    assert.equal(invokeRoute('tauri', command, false, 'dom-canvas'), 'web', `tauri/${command} (no bridge)`);
    // `native-below` is untouched: Tauri for everything, bridge or not.
    assert.equal(invokeRoute('tauri', command, false, 'native-below'), 'tauri', `native/${command}`);
  }
});

test('a native-below carrier routes exactly what it always did', () => {
  // Regression guard for the rule "no behaviour change on native-below": the
  // default carrier of a `tauri` call is the build's initial one, so an
  // unobserved call still reaches the native host.
  for (const command of [...DATA_COMMANDS, ...SHELL_ONLY_COMMANDS]) {
    assert.equal(invokeRoute('tauri', command, true), invokeRoute('tauri', command, true, 'native-below'));
    assert.equal(invokeRoute('tauri', command, true), 'tauri', command);
  }
});

// ---------------------------------------------------------------------------
// 3. The bootstrap exception: the first `cad_status` is the native host's
// ---------------------------------------------------------------------------

test('the first cad_status reaches the native host, every later one the adapter', async () => {
  // The precise order of an X11 session: the native host is asked first (that
  // answer is how it announces `dom-canvas`), and from then on the wasm session
  // serves the same call. Everything else switches in the same commit.
  const calls = [];
  const carrier = createCarrierStore('unknown');
  const invoke = createInvoke({
    runtime: 'tauri',
    tauri: async (command) => {
      calls.push(['tauri', command]);
      // What `apps/app-linux`'s `cad_status` answers on the `dom-canvas` carrier.
      return { surfaceMode: 'dom-canvas', documentLabel: null };
    },
    web: { invoke: async (command) => { calls.push(['web', command]); return { surfaceMode: 'dom-canvas' }; } },
    tauriPresent: () => true,
    carrier: () => carrier.now(),
  });

  const first = await invoke('cad_status');
  assert.deepEqual(calls, [['tauri', 'cad_status']]);
  // The chrome feeds the answer into the state machine (`App.tsx`), which is the
  // one place the flip can come from.
  assert.equal(carrier.observe(first.surfaceMode), true);
  assert.equal(carrier.usesWasmCad(), true);

  await invoke('cad_status');
  await invoke('cad_commands');
  await invoke('cad_command', { command: 'FIT' });
  // The two shell-only reports still reach the host that maps them to its marker
  // line and exit code.
  await invoke('canvas_runtime_failed', { step: 'webgl2' });

  assert.deepEqual(calls, [
    ['tauri', 'cad_status'],
    ['web', 'cad_status'],
    ['web', 'cad_commands'],
    ['web', 'cad_command'],
    ['tauri', 'canvas_runtime_failed'],
  ]);
  // No half-switch: the only Tauri calls are the bootstrap status and the
  // shell-only report - not one data command is left on the native transport.
  assert.deepEqual(calls.filter(([route]) => route === 'tauri'), [
    ['tauri', 'cad_status'],
    ['tauri', 'canvas_runtime_failed'],
  ]);
});

test('a native-below answer keeps the desktop transport for the whole session', async () => {
  const calls = [];
  const carrier = createCarrierStore('unknown');
  const invoke = createInvoke({
    runtime: 'tauri',
    tauri: async (command) => { calls.push(['tauri', command]); return { documentLabel: 'plan.dwg' }; },
    web: { invoke: async (command) => { calls.push(['web', command]); return {}; } },
    tauriPresent: () => true,
    carrier: () => carrier.now(),
  });
  const first = await invoke('cad_status');
  // An older host omits the field (and a native host does not add it either).
  assert.equal(carrier.observe(first.surfaceMode), true);
  assert.equal(carrier.usesWasmCad(), false);
  await invoke('cad_status');
  await invoke('cad_commands');
  assert.deepEqual(calls, [
    ['tauri', 'cad_status'],
    ['tauri', 'cad_status'],
    ['tauri', 'cad_commands'],
  ]);
});

test('a browser bundle never routes data to Tauri, even if a status said native-below', async () => {
  const calls = [];
  const invoke = createInvoke({
    runtime: 'web',
    tauri: async (command) => { calls.push(['tauri', command]); return {}; },
    web: { invoke: async (command) => { calls.push(['web', command]); return {}; } },
    // An Android-shaped document would have a bridge; a browser has none.
    tauriPresent: () => false,
    carrier: () => 'native-below',
  });
  await invoke('cad_status');
  await invoke('cad_command', { command: 'FIT' });
  await invoke('canvas_runtime_failed', { step: 'webgl2' });
  assert.deepEqual(calls, [
    ['web', 'cad_status'],
    ['web', 'cad_command'],
    ['web', 'canvas_runtime_failed'],
  ]);
});

// ---------------------------------------------------------------------------
// 4. The chrome's reaction: a flip reloads the vocabulary
// ---------------------------------------------------------------------------

test('a carrier flip reloads the command vocabulary and the alias table', async () => {
  const app = await readFile(APP_TSX, 'utf8');

  // Every received status feeds the state machine, from the poll that receives it
  // (never from a render pass).
  assert.match(app, /observeCarrier\(reportedSurfaceMode\(\{ status: next \}\)\)/);

  // The flip is observed through the published subscription, and only the wasm
  // carrier bumps the epoch: a first native report must reload nothing.
  assert.match(app, /useEffect\(\(\) => subscribeCarrier\(\(\) => \{\n\s+if \(usesWasmCad\(\)\) setCarrierEpoch\(/);

  // The loader effect depends on that epoch, so it runs again on a flip. Without
  // this the X11 chrome would keep the desktop table the native host answered.
  const loader = app.slice(app.indexOf('const vocabulary = invoke(\'cad_commands\');'));
  assert.ok(loader.includes('vocabularyRef.current = vocabulary;'), 'the in-flight promise must be replaced');
  const depsAt = app.indexOf('}, [carrierEpoch]);');
  assert.ok(depsAt > 0, 'the vocabulary/alias loader must depend on the carrier epoch');
  assert.ok(
    app.indexOf('const vocabulary = invoke(\'cad_commands\');') < depsAt,
    'the dependency belongs to the loader effect',
  );

  // The no-op path stays a no-op: nothing else in the tree may bump the epoch.
  assert.equal(app.split('setCarrierEpoch(').length - 1, 1, 'one bump site');
});
