// Which host this bundle runs in: one decision, made once, read everywhere.
//
// Three build targets share this tree and only the transport differs:
//
// * `tauri` - the Linux and Windows Tauri hosts (`apps/app-linux`,
//   `apps/app-windows`): every command is a `#[tauri::command]` on the host, and
//   the native shell owns the CAD surface;
// * `web` - the browser bundle (`scripts/build-web-ui.sh` -> `apps/app-web/dist-ui`):
//   there is **no** Tauri IPC at all; the wasm CAD session (`apps/app-web`) owns
//   the document and its four `shell_*` exports answer the chrome;
// * `android` - the same browser bundle inside an Android WebView **with** the
//   Tauri bridge present, so the wasm session owns the document while the two
//   shell-only canvas reports still travel over IPC.
//
// The value is injected by `vite.config.mjs` as a build-time `define`
// (`__YACR_RUNTIME__`), validated there against [`RUNTIMES`], so in a production
// bundle it is a constant and never a runtime probe. The `typeof` guard is what
// keeps this module importable by Node's test runner, where nothing defines it:
// an absent define is the desktop (Tauri) build, the only target that existed
// before the browser one.
//
// [`isTauri`] answers a different question - "does the Tauri IPC bridge exist in
// this page right now" - and is deliberately **not** derived from the runtime:
// an Android WebView carrier is a `web`-like document with a bridge, and
// `host/invoke.ts` uses exactly that fact to route the two shell-only canvas
// reports.

/** The hosts this one React tree is built for. */
export type Runtime = 'tauri' | 'web' | 'android';

/** Every runtime a build may be configured with (also `vite.config.mjs`'s set). */
export const RUNTIMES: readonly Runtime[] = ['tauri', 'web', 'android'];

/**
 * The build-time constant `vite.config.mjs` injects.
 *
 * Declared ambient and module-local: the identifier is replaced by a string
 * literal at build time and never exists at runtime, so a real declaration here
 * would be a lie (and a duplicate one).
 */
declare const __YACR_RUNTIME__: string | undefined;

/** The runtime a build defaults to when no define is present (the desktop hosts). */
export const RUNTIME_DEFAULT: Runtime = 'tauri';

/** The injected value as a runtime, or `null` for anything outside [`RUNTIMES`]. */
export function runtimeFrom(value: unknown): Runtime | null {
  return value === 'tauri' || value === 'web' || value === 'android' ? value : null;
}

/**
 * The runtime this bundle was built for.
 *
 * An absent define (`typeof … === 'undefined'`) is the desktop build. A define
 * that is present but not one of [`RUNTIMES`] is a build mistake, not something
 * to guess around: it throws here, at module load, instead of silently routing
 * every command to the wrong transport.
 */
export const RUNTIME: Runtime = (() => {
  const injected: unknown = typeof __YACR_RUNTIME__ === 'undefined' ? RUNTIME_DEFAULT : __YACR_RUNTIME__;
  const runtime = runtimeFrom(injected);
  if (runtime === null) {
    throw new Error(`runtime: __YACR_RUNTIME__ must be one of ${RUNTIMES.join(', ')} (got ${JSON.stringify(injected)})`);
  }
  return runtime;
})();

/**
 * Whether the wasm CAD session owns the document on this runtime.
 *
 * `web` and `android` draw into the in-page `<canvas>` (ADR 0024); a `tauri`
 * desktop bundle composites a native surface below the WebView instead.
 */
export const usesWasmCad: boolean = RUNTIME === 'web' || RUNTIME === 'android';

/** The scope [`isTauri`] probes: `window` in a page, `globalThis` in a test. */
export interface TauriProbeScope {
  /** What `@tauri-apps/api/core` dereferences; Tauri itself installs it. */
  readonly __TAURI_INTERNALS__?: unknown;
}

/**
 * Whether the Tauri IPC bridge is present in this document.
 *
 * This is a *probe*, never a capability claim: it says the bridge object exists,
 * not that any particular command is implemented. An Android WebView carrier
 * publishes it; the plain browser bundle does not. The default scope is
 * `globalThis` (which **is** `window` in a page) so the same call works in a
 * Node test double.
 */
export function isTauri(scope: unknown = globalThis): boolean {
  if (scope === null || (typeof scope !== 'object' && typeof scope !== 'function')) return false;
  return (scope as TauriProbeScope).__TAURI_INTERNALS__ != null;
}
