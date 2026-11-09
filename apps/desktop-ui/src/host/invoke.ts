// The one transport seam of the shared chrome.
//
// This is the **only** module in the tree that imports `@tauri-apps/api`: every
// component and every feature module calls `invoke(...)` from here, so switching
// carrier is one decision (`host/runtime.ts`) and not fifty call sites.
//
// # Routing, in full
//
// The carrier is **observed**, not built: a `tauri` or `android` bundle starts at
// `unknown` and learns from the first `cad_status` answer whether a native host
// owns the document or the `dom-canvas` carrier does
// (`apps/app-linux/src/host/surface.rs` runs X11 with `native_cad_session: false`;
// the Android host reports `surfaceMode: "native-below"`, `documentOwner:
// "native"`). A `web` bundle has no host to ask and starts on `dom-canvas`. The
// rule is one sentence: **the carrier that owns the document serves the data
// commands, and a bundle with no Tauri bridge never calls one.**
//
// | build           | carrier        | bridge  | CAD data commands  | `canvas_runtime_failed` / `canvas_session_status` |
// |-----------------|----------------|---------|--------------------|---------------------------------------------------|
// | `tauri`         | `unknown`      | present | Tauri (bootstrap)  | Tauri                                             |
// | `tauri`         | `native-below` | present | Tauri              | Tauri                                             |
// | `tauri`         | `dom-canvas`   | present | `host/web-adapter.ts` | Tauri (the bridge exists)                      |
// | `android`       | `unknown`      | present | Tauri (bootstrap)  | Tauri                                             |
// | `android`       | `native-below` | present | Tauri              | Tauri                                             |
// | `android`       | `dom-canvas`   | present | `host/web-adapter.ts` | Tauri (the bridge exists)                      |
// | `web`           | `dom-canvas`   | absent  | `host/web-adapter.ts` | `host/web-adapter.ts` (no bridge)              |
// | `tauri`/`android` | any          | absent  | `host/web-adapter.ts` | `host/web-adapter.ts` (no bridge)              |
//
// The two `absent` rows are the guard, not a scenario any shipped host targets: a
// bundle with no Tauri bridge must never call one, so every command falls to the
// adapter, whose refusal by name (`unsupported-runtime: <command>`) is the honest
// answer. A `web` bundle is that same case by construction. An `android` bundle
// reporting `dom-canvas` stays possible only if a host reports it; the adapter
// then owns the data on that carrier, exactly as on X11.
//
// # The one bootstrap exception: `cad_status`
//
// `cad_status` is how a host **announces** its carrier, so on a `tauri`/`android`
// build the first one must be able to reach the native host. Precisely:
//
// 1. carrier `unknown` -> `cad_status` goes to Tauri, and that answer carries
//    `hostProfile.surfaceMode` (the X11 host names `dom-canvas`; a native host -
//    including the Android one - names `native-below`);
// 2. the chrome feeds it to `observeCarrier(...)` (`host/runtime.ts`), which
//    flips the carrier to `dom-canvas` when the host said so;
// 3. from the next poll on, **every** `cad_status` comes from the adapter - the
//    wasm session's own snapshot - on a carrier that flipped; a `native-below`
//    answer keeps the native transport for the whole session.
//
// There is no second exception and no per-command half-switch: a command is
// routed by the carrier alone, so a flipped bundle answers exactly what the
// browser answers (same adapter, same vocabulary, same refusals). A `web` build
// starts at `dom-canvas` and never reaches the Tauri branch. An `android` build
// starts where a `tauri` one does, so the native host's bootstrap applies to it:
// the first `cad_status` and the `cad_event` subscription
// (`cadEventSource`/`cadEventRoute`) are native, the reported profile flips the
// carrier to `native-below`, and the uniform rule keeps the native transport -
// and, if an Android host ever reports `dom-canvas`, moves everything to the
// adapter exactly like X11.
//
// The shell-only names are shell capabilities and *reports the chrome sends*. On
// a browser carrier the wasm session owns the canvas facts, so the web adapter
// remembers them and resolves; on an Android WebView (and on an X11 host, whose
// `dom-canvas` carrier still maps a failure step to its marker line and exit
// code) the document has the Tauri bridge, and the native host is the one that
// acts on the report. The `cad_plugin_*` commands likewise belong to the shell's
// plugin registry, never to the wasm session. That is why the rule is "route
// these to Tauri **when the bridge is present**", not "route them by carrier": a
// plain browser has no bridge and must never call one.
//
// A `native-below` carrier is byte-for-byte the previous behaviour: the same
// `@tauri-apps/api` function, the same command names, the same arguments object
// (an omitted `args` stays omitted rather than becoming `{}`).
//
// # One rejection identity
//
// `invoke()` resolves the raw reply value on both transports and rejects with an
// `Error` whose `message` is the host's verbatim refusal. The wasm adapter
// already rejects that way; the native branch wraps Tauri's bare-string
// rejection once, here, so a caller never branches on the carrier to read a
// refusal.

import { invoke as tauriInvoke } from '@tauri-apps/api/core';
import { listen as tauriListen } from '@tauri-apps/api/event';
import { RUNTIME, carrierNow, initialCarrier, isTauri } from './runtime.ts';
import type { Carrier, Runtime } from './runtime.ts';
import { createWebAdapter, loadWebShellModule } from './web-adapter.ts';
import { CAD_EVENT_NAME, createTauriEventSource, createWasmEventSource } from '../cad/cad-events.ts';
import type { CadEventListener, CadEventSource } from '../cad/cad-events.ts';

/**
 * Tauri IPC arguments.
 *
 * The same union `@tauri-apps/api`'s `InvokeArgs` spells, declared here so this
 * module's signatures do not leak the Tauri type into feature modules (which
 * each declare their own narrow subset; `layers/layers.ts` and
 * `cad/view3d.ts` are the two that need the binary members).
 */
export type InvokeArgs = Record<string, unknown> | Uint8Array | number[] | ArrayBuffer;

/** Which transport serves a command. */
export type InvokeRoute = 'tauri' | 'web';

/**
 * The shell-only commands: shell capabilities and reports the chrome *sends*.
 *
 * They are the only names that prefer Tauri while an adapter owns the CAD data,
 * and only when the bridge is actually there (see the module header). The two
 * canvas reports have no answer; the five `cad_plugin_*` commands read or mutate
 * the **shell's** plugin registry, which is not CAD document data and therefore
 * never belongs to the wasm session. `cad_open_release_page` is the same kind of
 * shell capability: it asks the shell (or, on a bridge-less page, the browser
 * transport) to open the download page, and the wasm session has nothing to say
 * about it.
 */
export const SHELL_ONLY_COMMANDS: readonly string[] = [
  'canvas_runtime_failed',
  'canvas_session_status',
  'cad_plugin_status',
  'cad_plugin_install',
  'cad_plugin_enable',
  'cad_plugin_uninstall',
  'cad_plugin_read_entry',
  'cad_open_release_page',
];

/** The Tauri callable this module routes to (injectable for tests). */
export type TauriTransport = (command: string, args?: InvokeArgs) => Promise<unknown>;

/** The web callable this module routes to (`host/web-adapter.ts`). */
export interface WebTransport {
  invoke(command: string, args?: unknown): Promise<unknown>;
}

/** What a caller may inject; every field defaults to the real thing. */
export interface InvokeDeps {
  /** The build's runtime; defaults to [`RUNTIME`]. */
  readonly runtime?: Runtime;
  /** The Tauri transport; defaults to `@tauri-apps/api`'s `invoke`. */
  readonly tauri?: TauriTransport;
  /** The browser transport; defaults to the lazy wasm-session adapter. */
  readonly web?: WebTransport;
  /** Whether the Tauri bridge exists; defaults to [`isTauri`]. */
  readonly tauriPresent?: () => boolean;
  /**
   * The observed carrier, read **per call**; defaults to [`carrierNow`].
   *
   * Injectable so the bootstrap order (native first, adapter after the flip) is
   * testable without mutating the module singleton.
   */
  readonly carrier?: () => Carrier;
}

/** The `invoke` surface every feature module is typed against. */
export type Invoke = <T = unknown>(command: string, args?: InvokeArgs) => Promise<T>;

/**
 * The transport one command goes to.
 *
 * Pure, and the single statement of the routing table above. `carrier` is the
 * observed carrier and defaults to the build's *initial* one, so a caller with no
 * observation yet is routed by what the build can prove about itself: `web` to
 * the adapter, `tauri`/`android` to the native host that must announce itself
 * first (or to the adapter when that host's bridge is missing).
 */
export function invokeRoute(
  runtime: Runtime,
  command: string,
  tauriPresent: boolean,
  carrier: Carrier = initialCarrier(runtime),
): InvokeRoute {
  // The adapter serves the documents whose authority is the wasm session (every
  // `dom-canvas` carrier), and it also serves a bundle with **no** Tauri bridge:
  // there is nothing else to call, and the adapter's named refusal is the honest
  // answer. Nothing here is inferred from the build target - an `android` bundle
  // on a native session is a Tauri client exactly like a desktop one.
  const adapterOwnsData = carrier === 'dom-canvas' || !tauriPresent;
  if (!adapterOwnsData) return 'tauri';
  if (tauriPresent && SHELL_ONLY_COMMANDS.includes(command)) return 'tauri';
  return 'web';
}

/**
 * Build an `invoke` over the given transports.
 *
 * The returned function decides **per call** (not per module load), so a
 * document that gains or loses the bridge - and a `tauri` bundle whose host
 * reported the `dom-canvas` carrier - is routed by what is true at call time,
 * which is exactly the bootstrap order the header describes.
 */
export function createInvoke({
  runtime = RUNTIME,
  tauri = tauriInvoke,
  web = createWebAdapter(() => loadWebShellModule()),
  tauriPresent = () => isTauri(),
  carrier = carrierNow,
}: InvokeDeps = {}): Invoke {
  return <T = unknown>(command: string, args?: InvokeArgs): Promise<T> => {
    const route = invokeRoute(runtime, command, tauriPresent(), carrier());
    if (route === 'web') return web.invoke(command, args) as Promise<T>;
    // One rejection identity on both transports: a refused command rejects with
    // an `Error` whose `message` is the host's verbatim refusal. The Tauri IPC
    // rejects with a bare string; the wasm adapter already throws `Error(reason)`.
    // Without this wrap, `String(error)` differs by transport (`"reason"` vs
    // `"Error: reason"`) and every caller that renders or compares the refusal
    // would need a per-carrier branch.
    return tauri(command, args).then(
      value => value as T,
      (error: unknown) => { throw error instanceof Error ? error : new Error(String(error)); },
    );
  };
}

/** The chrome's `invoke`: the native host, or the wasm session, as the carrier says. */
export const invoke: Invoke = createInvoke();

/**
 * Register one Tauri `cad_event` listener and hand back its unlisten.
 *
 * The only `@tauri-apps/api` import for the event stream lives beside the
 * `invoke` import above, so `host/invoke.ts` stays the single transport seam.
 * The host emits one JSON object per event; a payload that is already an object
 * is serialised here so the event source always parses one wire shape.
 */
export const listenCadEvents: CadEventListener = handler =>
  tauriListen<unknown>(CAD_EVENT_NAME, event => {
    const payload = event.payload;
    const json = typeof payload === 'string' ? payload : JSON.stringify(payload) ?? '';
    handler(json);
  }).then(unlisten => () => { unlisten(); });

/**
 * Which transport serves the push event stream right now.
 *
 * This is the **existing** routing table (`invokeRoute`), asked about the status
 * command: the native host announces its carrier through the first `cad_status`,
 * so an unproven `tauri`/`android` build listens natively until that answer
 * arrives (an Android host's answer is `native-below`, so it stays native), and an
 * X11/XWayland host's `dom-canvas` answer moves the stream to the wasm session.
 */
export function cadEventRoute(): InvokeRoute {
  return invokeRoute(RUNTIME, 'cad_status', isTauri(), carrierNow());
}

/**
 * The event source of the current route: Tauri events on the native transport,
 * the wasm session's `cad_subscribe` on the adapter transport.
 *
 * Kept here (rather than in a component) so no UI module branches on the host or
 * the runtime; callers only ever see a [`CadEventSource`].
 */
export function cadEventSource(route: InvokeRoute = cadEventRoute()): CadEventSource {
  return route === 'tauri'
    ? createTauriEventSource(invoke, listenCadEvents)
    : createWasmEventSource(() => loadWebShellModule());
}
