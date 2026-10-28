// The one transport seam of the shared chrome.
//
// This is the **only** module in the tree that imports `@tauri-apps/api`: every
// component and every feature module calls `invoke(...)` from here, so switching
// carrier is one decision (`host/runtime.ts`) and not fifty call sites.
//
// Routing, in full:
//
// | runtime            | command                                   | goes to |
// |--------------------|-------------------------------------------|---------|
// | `tauri`            | anything                                  | Tauri   |
// | `web` / `android`  | `canvas_runtime_failed`, `canvas_session_status` and the Tauri bridge exists | Tauri |
// | `web` / `android`  | everything else                           | `host/web-adapter.ts` |
//
// The two shell-only names are *reports the chrome sends*. On a browser carrier
// the wasm session already owns those facts, so the web adapter remembers them
// and resolves; on an Android WebView the same document still has the Tauri
// bridge, and the native host is the one that maps a failure step to its marker
// line and exit code. That is why the rule is "route these two to Tauri **when
// the bridge is present**", not "route them by runtime": a plain browser has no
// bridge and must never call one.
//
// The `tauri` branch is byte-for-byte the previous behaviour: the same
// `@tauri-apps/api` function, the same command names, the same arguments object
// (an omitted `args` stays omitted rather than becoming `{}`).

import { invoke as tauriInvoke } from '@tauri-apps/api/core';
import { RUNTIME, isTauri } from './runtime.ts';
import type { Runtime } from './runtime.ts';
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
 * They are the only names that prefer Tauri on a `web`/`android` runtime, and
 * only when the bridge is actually there (see the module header).
 */
export const SHELL_ONLY_COMMANDS: readonly string[] = ['canvas_runtime_failed', 'canvas_session_status'];

/** The Tauri callable this module routes to (injectable for tests). */
export type TauriTransport = (command: string, args?: InvokeArgs) => Promise<unknown>;

/** The web callable this module routes to (`host/web-adapter.ts`). */
export interface WebTransport {
  invoke(command: string, args?: unknown): Promise<unknown>;
}

/** What a caller may inject; all four default to the real transports. */
export interface InvokeDeps {
  /** The build's runtime; defaults to [`RUNTIME`]. */
  readonly runtime?: Runtime;
  /** The Tauri transport; defaults to `@tauri-apps/api`'s `invoke`. */
  readonly tauri?: TauriTransport;
  /** The browser transport; defaults to the lazy wasm-session adapter. */
  readonly web?: WebTransport;
  /** Whether the Tauri bridge exists; defaults to [`isTauri`]. */
  readonly tauriPresent?: () => boolean;
}

/** The `invoke` surface every feature module is typed against. */
export type Invoke = <T = unknown>(command: string, args?: InvokeArgs) => Promise<T>;

/**
 * The transport one command goes to.
 *
 * Pure, and the single statement of the routing table: `tauri` hosts use Tauri
 * for everything; `web`/`android` hosts use the web adapter, except the two
 * shell-only reports when the Tauri bridge is present.
 */
export function invokeRoute(runtime: Runtime, command: string, tauriPresent: boolean): InvokeRoute {
  if (runtime === 'tauri') return 'tauri';
  if (tauriPresent && SHELL_ONLY_COMMANDS.includes(command)) return 'tauri';
  return 'web';
}

/**
 * Build an `invoke` over the given transports.
 *
 * The returned function decides **per call** (not per module load), so a
 * document that gains or loses the bridge is routed by what exists at call time -
 * which is exactly the Android case the table above describes.
 */
export function createInvoke({
  runtime = RUNTIME,
  tauri = tauriInvoke,
  web = createWebAdapter(() => loadWebShellModule()),
  tauriPresent = () => isTauri(),
}: InvokeDeps = {}): Invoke {
  return <T = unknown>(command: string, args?: InvokeArgs): Promise<T> => {
    const route = invokeRoute(runtime, command, tauriPresent());
    return (route === 'tauri' ? tauri(command, args) : web.invoke(command, args)) as Promise<T>;
  };
}

/** The chrome's `invoke`: Tauri on the desktop hosts, the wasm session in a browser. */
export const invoke: Invoke = createInvoke();
