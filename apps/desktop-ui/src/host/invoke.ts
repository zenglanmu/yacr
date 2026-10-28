// The one transport seam of the shared chrome.
//
// This is the **only** module in the tree that imports `@tauri-apps/api`: every
// component and every feature module calls `invoke(...)` from here, so switching
// carrier is one decision (`host/runtime.ts`) and not fifty call sites.
//
// # Routing, in full
//
// The carrier is **observed**, not built: a `tauri` bundle starts at `unknown`
// and learns from the first `cad_status` answer whether it is a native host or
// the `dom-canvas` carrier (`apps/app-linux/src/host/surface.rs` runs X11 with
// `native_cad_session: false`). The rule is one sentence: **the carrier that owns
// the document serves the data commands.**
//
// | build           | carrier      | CAD data commands  | `canvas_runtime_failed` / `canvas_session_status` |
// |-----------------|--------------|--------------------|---------------------------------------------------|
// | `tauri`         | `unknown`    | Tauri (bootstrap)  | Tauri                                             |
// | `tauri`         | `native-below` | Tauri            | Tauri                                             |
// | `tauri`         | `dom-canvas` | `host/web-adapter.ts` | Tauri (the bridge exists)                      |
// | `web`           | `dom-canvas` | `host/web-adapter.ts` | `host/web-adapter.ts` (no bridge)              |
// | `android`       | `dom-canvas` | `host/web-adapter.ts` | Tauri (the bridge exists)                      |
// | `web`/`android` | `native-below` | `host/web-adapter.ts` | Tauri when the bridge exists                 |
//
// The last row cannot be produced by a real carrier (neither bundle has a native
// document). It is routed to the adapter on purpose: a bundle with no native
// session must never answer a command out of one, and the adapter's refusal by
// name (`unsupported-runtime: <command>`) is the honest answer.
//
// # The one bootstrap exception: `cad_status`
//
// `cad_status` is how a host **announces** its carrier, so on a `tauri` build the
// first one must be able to reach the native host. Precisely:
//
// 1. carrier `unknown` -> `cad_status` goes to Tauri, and that answer carries
//    `surfaceMode` (the X11 host adds it, `native-below` hosts omit it);
// 2. the chrome feeds it to `observeCarrier(...)` (`host/runtime.ts`), which
//    flips the carrier to `dom-canvas` when the host said so;
// 3. from the next poll on, **every** `cad_status` comes from the adapter - the
//    wasm session's own snapshot.
//
// There is no second exception and no per-command half-switch: a command is
// routed by the carrier alone, so a flipped `tauri` bundle answers exactly what
// the browser answers (same adapter, same vocabulary, same refusals). On `web`
// and `android` builds the carrier starts at `dom-canvas`, so nothing here
// changes their behaviour: they never reach the Tauri branch.
//
// The two shell-only names are *reports the chrome sends*. On a browser carrier
// the wasm session already owns those facts, so the web adapter remembers them
// and resolves; on an Android WebView (and on an X11 host, whose `dom-canvas`
// carrier still maps a failure step to its marker line and exit code) the
// document has the Tauri bridge, and the native host is the one that acts on the
// report. That is why the rule is "route these two to Tauri **when the bridge is
// present**", not "route them by carrier": a plain browser has no bridge and must
// never call one.
//
// A `native-below` carrier is byte-for-byte the previous behaviour: the same
// `@tauri-apps/api` function, the same command names, the same arguments object
// (an omitted `args` stays omitted rather than becoming `{}`).

import { invoke as tauriInvoke } from '@tauri-apps/api/core';
import { RUNTIME, carrierNow, initialCarrier, isTauri } from './runtime.ts';
import type { Carrier, Runtime } from './runtime.ts';
import { createWebAdapter, loadWebShellModule } from './web-adapter.ts';

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
 * The shell-only canvas reports: commands the chrome *sends*, with no answer.
 *
 * They are the only names that prefer Tauri while an adapter owns the CAD data,
 * and only when the bridge is actually there (see the module header).
 */
export const SHELL_ONLY_COMMANDS: readonly string[] = ['canvas_runtime_failed', 'canvas_session_status'];

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
 * observed carrier and defaults to the build's *initial* one, so a caller that
 * has no observation yet (a fresh bundle, and every existing test) gets exactly
 * the behaviour it had before the carrier state existed.
 */
export function invokeRoute(
  runtime: Runtime,
  command: string,
  tauriPresent: boolean,
  carrier: Carrier = initialCarrier(runtime),
): InvokeRoute {
  // The adapter serves the documents whose authority is the wasm session: every
  // `dom-canvas` carrier, plus every bundle that has no native document at all
  // (a `web`/`android` build never answers out of one, whatever was reported).
  const adapterOwnsData = carrier === 'dom-canvas' || runtime !== 'tauri';
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
    return (route === 'tauri' ? tauri(command, args) : web.invoke(command, args)) as Promise<T>;
  };
}

/** The chrome's `invoke`: the native host, or the wasm session, as the carrier says. */
export const invoke: Invoke = createInvoke();
