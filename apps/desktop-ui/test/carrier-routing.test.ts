// The **observed carrier** contract: one state machine (`host/runtime.ts`), one
// routing table built on it (`host/invoke.ts`), and the chrome's reaction to a
// flip (`App.tsx`).
//
// Why this file exists: a build target is not a carrier. The Linux host decides
// at startup whether it runs `native-below` (native Wayland: a native CAD session
// under the WebView) or `dom-canvas` (X11/XWayland: the wasm session in the page,
// `apps/app-linux/src/host/surface.rs` `native_cad_session: false`) and reports it
// through `cad_status.hostProfile.surfaceMode`. A `tauri` bundle must therefore be
// able to **change its mind** about who serves CAD data; before this contract it
// kept the desktop vocabulary on X11 and advertised commands nothing could execute.
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

const HOOK_TS = new URL('../src/commands/use-host-vocabulary.ts', import.meta.url);
const APP_TSX = new URL('../src/App.tsx', import.meta.url);
const EVENTS_TS = new URL('../src/cad/use-cad-events.ts', import.meta.url);
const INVOKE_TS = new URL('../src/host/invoke.ts', import.meta.url);

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
  // The running app version is host data: the native host answers it, the wasm
  // session refuses by name. It is deliberately **not** shell-only: on a
  // `dom-canvas` carrier the adapter owns the answer (the honest refusal).
  'cad_app_version',
];

// ---------------------------------------------------------------------------
// 1. The state machine
// ---------------------------------------------------------------------------

test('a build starts on the carrier it can prove', () => {
  // The desktop and Android bundles cannot know before the host answers - and
  // their first `cad_status` is the question, so guessing "adapter" would
  // deadlock an X11 host out of ever announcing itself (and would keep an
  // Android native session from serving a single command).
  assert.equal(initialCarrier('tauri'), 'unknown');
  assert.equal(initialCarrier('android'), 'unknown');
  // The browser bundle has no host to ask: its authority is the build itself.
  assert.equal(initialCarrier('web'), 'dom-canvas');

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

  // Everything else resolves to `native-below`: the carrier store keeps its
  // bootstrap collapse unchanged, and `cad/surface.ts` is the stricter reader for
  // the chrome's own surface decision. A missing profile (a pre-P1 host) is the
  // historic carrier.
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

test('every build x observed carrier x bridge x command class resolves the same way', () => {
  for (const command of DATA_COMMANDS) {
    // A `tauri` bundle routes data natively while the carrier is unproven or
    // native **and the bridge is there**...
    assert.equal(invokeRoute('tauri', command, true, 'unknown'), 'tauri', `tauri/unknown/${command}`);
    assert.equal(invokeRoute('tauri', command, true, 'native-below'), 'tauri', `tauri/native/${command}`);
    // ...and to the adapter as soon as the host reports `dom-canvas`...
    assert.equal(invokeRoute('tauri', command, true, 'dom-canvas'), 'web', `tauri/dom-canvas/${command}`);
    // ...and on every carrier when there is no bridge to call at all. Shipping a
    // bundle whose page has no Tauri IPC is a build mistake; the adapter's named
    // `unsupported-runtime` refusal is the only honest answer, never a fake Tauri
    // call into a missing object.
    for (const carrier of ['unknown', 'native-below', 'dom-canvas']) {
      assert.equal(invokeRoute('tauri', command, false, carrier), 'web', `tauri/${carrier}/${command} (no bridge)`);
    }

    // Android is the same client as `tauri` once its host reports a carrier: the
    // native host owns the document, so the data commands are its own. The
    // `dom-canvas` row stays possible for a host that reports it (the adapter
    // then owns the data), and a bridge-less Android bundle can only refuse.
    assert.equal(invokeRoute('android', command, true, 'unknown'), 'tauri', `android/unknown/${command}`);
    assert.equal(invokeRoute('android', command, true, 'native-below'), 'tauri', `android/native/${command}`);
    assert.equal(invokeRoute('android', command, true, 'dom-canvas'), 'web', `android/dom-canvas/${command}`);
    for (const carrier of ['unknown', 'native-below', 'dom-canvas']) {
      assert.equal(invokeRoute('android', command, false, carrier), 'web', `android/${carrier}/${command} (no bridge)`);
    }

    // The browser bundle never answers CAD data out of Tauri, whatever was
    // reported and whether or not a bridge is somehow present.
    for (const present of [true, false]) {
      assert.equal(invokeRoute('web', command, present, 'dom-canvas'), 'web', `web/dom-canvas/${command}`);
    }
  }

  assert.deepEqual([...SHELL_ONLY_COMMANDS], [
    'canvas_runtime_failed', 'canvas_session_status',
    'cad_plugin_status', 'cad_plugin_install', 'cad_plugin_enable', 'cad_plugin_uninstall', 'cad_plugin_read_entry',
    'cad_open_release_page',
  ]);
  for (const command of SHELL_ONLY_COMMANDS) {
    // The two reports prefer an existing bridge on every carrier the adapter
    // owns data on - an Android WebView, and an X11 host (which is where the
    // marker line and exit code live).
    assert.equal(invokeRoute('android', command, true, 'dom-canvas'), 'tauri', `android/dom-canvas/${command}`);
    assert.equal(invokeRoute('android', command, true, 'native-below'), 'tauri', `android/native/${command}`);
    assert.equal(invokeRoute('tauri', command, true, 'dom-canvas'), 'tauri', `tauri/dom-canvas/${command}`);
    // ...and stay on the adapter when there is no bridge to report to.
    assert.equal(invokeRoute('web', command, false, 'dom-canvas'), 'web', `web/${command}`);
    assert.equal(invokeRoute('tauri', command, false, 'dom-canvas'), 'web', `tauri/${command} (no bridge)`);
    assert.equal(invokeRoute('android', command, false, 'unknown'), 'web', `android/${command} (no bridge)`);
    // `native-below` on a bridged `tauri` host is untouched: Tauri for everything.
    assert.equal(invokeRoute('tauri', command, true, 'native-below'), 'tauri', `native/${command}`);
  }
});

test('a bridged native carrier routes exactly what it always did', () => {
  // Regression guard for the rule "no behaviour change on native-below": the
  // default carrier of a `tauri` call is the build's initial (`unknown`), which
  // a bridged host answers the same way as an explicit `native-below` report.
  for (const command of [...DATA_COMMANDS, ...SHELL_ONLY_COMMANDS]) {
    assert.equal(invokeRoute('tauri', command, true), invokeRoute('tauri', command, true, 'native-below'));
    assert.equal(invokeRoute('tauri', command, true), 'tauri', command);
  }
});

test('the update commands route by capability, not by build', () => {
  // Opening the download page is a shell capability performed by the transport
  // when a bridge exists (`window.open` is not a CAD answer)...
  assert.equal(invokeRoute('tauri', 'cad_open_release_page', true, 'dom-canvas'), 'tauri');
  assert.equal(invokeRoute('android', 'cad_open_release_page', true, 'dom-canvas'), 'tauri');
  // ...and by the browser adapter when there is no bridge to ask.
  assert.equal(invokeRoute('tauri', 'cad_open_release_page', false, 'dom-canvas'), 'web');
  assert.equal(invokeRoute('web', 'cad_open_release_page', false, 'dom-canvas'), 'web');

  // The running app version is host data, so it follows the data routing: the
  // native host answers it on `native-below`; on `dom-canvas` the wasm session
  // owns the route and refuses by name (which the store maps to `unsupported`).
  assert.equal(invokeRoute('tauri', 'cad_app_version', true, 'native-below'), 'tauri');
  assert.equal(invokeRoute('android', 'cad_app_version', true, 'native-below'), 'tauri');
  assert.equal(invokeRoute('tauri', 'cad_app_version', true, 'dom-canvas'), 'web');
  assert.equal(invokeRoute('web', 'cad_app_version', false, 'dom-canvas'), 'web');
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
      return { hostProfile: { surfaceMode: 'dom-canvas' }, documentLabel: null };
    },
    web: { invoke: async (command) => { calls.push(['web', command]); return { hostProfile: { surfaceMode: 'dom-canvas' } }; } },
    tauriPresent: () => true,
    carrier: () => carrier.now(),
  });

  const first = await invoke('cad_status');
  assert.deepEqual(calls, [['tauri', 'cad_status']]);
  // The chrome feeds the answer into the state machine (`App.tsx`), which is the
  // one place the flip can come from.
  assert.equal(carrier.observe(first.hostProfile?.surfaceMode), true);
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
  // A pre-P1 host omits the profile (a current native host names `native-below`).
  assert.equal(carrier.observe(first.hostProfile?.surfaceMode), true);
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

test('an Android native session bootstraps natively and stays native', async () => {
  // P5: the Android host runs a native Rust `CadSession` and reports
  // `surfaceMode: "native-below"`, `documentOwner: "native"`. Its first
  // `cad_status` is the same bootstrap a desktop host performs - and because the
  // answer is `native-below`, the native transport is kept for the session.
  const calls = [];
  const carrier = createCarrierStore('unknown');
  const invoke = createInvoke({
    runtime: 'android',
    tauri: async (command) => {
      calls.push(['tauri', command]);
      return command === 'cad_status'
        ? { hostProfile: { surfaceMode: 'native-below', documentOwner: 'native' } }
        : { accepted: true };
    },
    web: { invoke: async (command) => { calls.push(['web', command]); return {}; } },
    tauriPresent: () => true,
    carrier: () => carrier.now(),
  });

  // The bootstrap command (and any command before the first answer) is native.
  assert.equal(invokeRoute('android', 'cad_command', true, carrier.now()), 'tauri');
  const first = await invoke('cad_status');
  assert.deepEqual(calls, [['tauri', 'cad_status']]);
  assert.equal(carrier.observe(first.hostProfile?.surfaceMode), true);
  assert.equal(carrier.now(), 'native-below');

  // `cadEventRoute` is this very table asked about `cad_status`, so the event
  // stream is native before and after the answer: the carrier never flips.
  assert.equal(invokeRoute('android', 'cad_status', true, 'unknown'), 'tauri');
  assert.equal(invokeRoute('android', 'cad_status', true, carrier.now()), 'tauri');

  await invoke('cad_command', { command: 'FIT' });
  await invoke('probe_input', { input: { kind: 'pointer_down', sequence: 1 } });
  await invoke('canvas_session_status', { label: 'plan.dwg' });
  assert.deepEqual(calls, [
    ['tauri', 'cad_status'],
    ['tauri', 'cad_command'],
    ['tauri', 'probe_input'],
    ['tauri', 'canvas_session_status'],
  ]);
});

test('the event transport is the same routing table, android included', async () => {
  // `cadEventRoute` must stay one delegation to `invokeRoute`, otherwise the
  // Android bootstrap could subscribe to the wasm session while its commands go
  // to the native host.
  const source = await readFile(INVOKE_TS, 'utf8');
  assert.match(
    source,
    /export function cadEventRoute\(\): InvokeRoute \{\n\s+return invokeRoute\(RUNTIME, 'cad_status', isTauri\(\), carrierNow\(\)\);/,
  );
});

test('a bridgeless android bundle cannot serve a command', async () => {
  // The explicit refusal path: no Tauri bridge means there is no host to ask, so
  // the adapter answers - with `unsupported-runtime: cad_command` in a real page,
  // never by calling the missing bridge.
  const calls = [];
  const invoke = createInvoke({
    runtime: 'android',
    tauri: async (command) => { calls.push(['tauri', command]); return {}; },
    web: { invoke: async (command) => { calls.push(['web', command]); return {}; } },
    tauriPresent: () => false,
    carrier: () => 'unknown',
  });
  await invoke('cad_status');
  await invoke('cad_command', { command: 'FIT' });
  assert.deepEqual(calls, [
    ['web', 'cad_status'],
    ['web', 'cad_command'],
  ]);
});

// ---------------------------------------------------------------------------
// 4. The chrome's reaction: a flip reloads the vocabulary
// ---------------------------------------------------------------------------

test('a carrier flip reloads the command vocabulary and the alias table', async () => {
  const app = await readFile(APP_TSX, 'utf8');
  const events = await readFile(EVENTS_TS, 'utf8');
  const hook = await readFile(HOOK_TS, 'utf8');

  // Every received status feeds the state machine from the one subscription
  // (`cad/use-cad-events.ts`), exactly once per status frame — never from a
  // render pass and never from a poll.
  assert.match(events, /observeCarrier\(reportedSurfaceMode\(\{ status: frame\.status \}\)\)/);
  assert.match(app, /useCadEvents\(/);

  // The chrome reads the vocabulary through the hook (which owns the retry, the
  // in-flight promise and the carrier subscription).
  assert.match(app, /useHostVocabulary\(setError\)/);

  // The flip is observed through the published subscription, and only the wasm
  // carrier bumps the epoch: a first native report must reload nothing.
  assert.match(hook, /useEffect\(\(\) => subscribeCarrier\(\(\) => \{\n\s+if \(usesWasmCad\(\)\) setCarrierEpoch\(/);

  // The loader effect depends on that epoch, so it runs again on a flip. Without
  // this the X11 chrome would keep the desktop table the native host answered.
  const loader = hook.slice(hook.indexOf("const pending = loadCommandVocabulary(() => invoke('cad_commands'));"));
  assert.ok(loader.includes('vocabulary.current = pending;'), 'the in-flight promise must be replaced');
  assert.ok(hook.includes('}, [carrierEpoch, onError]);'), 'the vocabulary/alias loader must depend on the carrier epoch');

  // The no-op path stays a no-op: nothing else in the hook may bump the epoch.
  assert.equal(hook.split('setCarrierEpoch(').length - 1, 1, 'one bump site');
});
