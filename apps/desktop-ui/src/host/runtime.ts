// Which host this bundle runs in, and which **carrier** that host reports.
//
// Three build targets share this tree and only the transport differs:
//
// * `tauri` - the Linux and Windows Tauri hosts (`apps/app-linux`,
//   `apps/app-windows`): every command is a `#[tauri::command]` on the host, and
//   the native shell owns the CAD surface;
// * `web` - the browser bundle (`scripts/build-web-ui.sh` -> `apps/app-web/dist-ui`):
//   there is **no** Tauri IPC at all; the wasm CAD session (`apps/app-web`) owns
//   the document and its `shell_*` exports answer the chrome;
// * `android` - the same browser bundle inside an Android WebView **with** the
//   Tauri bridge present, so the wasm session owns the document while the two
//   shell-only canvas reports still travel over IPC.
//
// The build value is injected by `vite.config.mjs` as a build-time `define`
// (`__YACR_RUNTIME__`), validated there against [`RUNTIMES`], so in a production
// bundle it is a constant and never a runtime probe. The `typeof` guard is what
// keeps this module importable by Node's test runner, where nothing defines it:
// an absent define is the desktop (Tauri) build, the only target that existed
// before the browser one.
//
// # Why a build target is not enough: the carrier
//
// A build target says which **bundle** this is; it does not say which **carrier**
// the process ended up running. The Linux host decides its carrier at startup
// (`apps/app-linux/src/backend.rs`) and reports it through `cad_status.surfaceMode`:
// native Wayland composites a native surface (`native-below`), while the
// X11/XWayland host runs the browser's `dom-canvas` path with **no** native CAD
// session at all (`apps/app-linux/src/host/surface.rs`, `native_cad_session: false`).
//
// That distinction is the whole point of [`Carrier`]: a `tauri` build on X11
// serves no CAD data, so the chrome must route its CAD commands to the **wasm
// adapter** exactly like the browser and the Android carrier do - otherwise it
// offers a desktop vocabulary to a window where nothing drains the queue.
//
// The state machine has three states and one transition rule:
//
// ```text
// initial: web/android -> dom-canvas      initial: tauri -> unknown
// observe(mode): "dom-canvas" -> dom-canvas, anything else -> native-below
// ```
//
// `unknown` is therefore the honest "native until proven otherwise" state of a
// `tauri` build: it routes to the native host (which is how the X11 host gets to
// announce its carrier at all) and stops being reachable after the first report.
// Only the exact string `"dom-canvas"` selects the wasm carrier - the same rule
// `cad/surface.ts` uses to read the flag, so the two readings cannot disagree.
//
// [`isTauri`] answers a different question - "does the Tauri IPC bridge exist in
// this page right now" - and is deliberately **not** derived from the carrier: an
// Android WebView carrier is a wasm document with a bridge, and `host/invoke.ts`
// uses exactly that fact to route the two shell-only canvas reports.

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
 * The carrier this chrome is driving: who owns the CAD document.
 *
 * * `unknown` - a `tauri` build that has not received a `cad_status` answer yet.
 *   The native host is asked first, because its answer is the only way an X11
 *   host can announce the carrier it really runs. Nothing else is inferred from
 *   this state.
 * * `native-below` - the native host owns the document and the surface below the
 *   WebView (Linux native Wayland, Windows); every command is a Tauri command.
 * * `dom-canvas` - the wasm session inside the page is the only CAD authority
 *   (browser, Android, X11/XWayland): the data commands belong to the adapter.
 */
export type Carrier = 'unknown' | 'native-below' | 'dom-canvas';

/** The state a build starts in, before any host has reported a carrier. */
export function initialCarrier(runtime: Runtime): Carrier {
  return runtime === 'tauri' ? 'unknown' : 'dom-canvas';
}

/** A listener told about every **modelled** carrier change. */
export type CarrierListener = (carrier: Carrier) => void;

/** The observable carrier state (see the module header for the transition rule). */
export interface CarrierStore {
  /** The carrier right now. */
  now(): Carrier;
  /**
   * Apply one reported `cad_status.surfaceMode` and report whether the modelled
   * carrier **changed**.
   *
   * `"dom-canvas"` selects that carrier; anything else - a missing field on an
   * older host, a `null`, an unrecognised string - resolves to `native-below`,
   * exactly like `cad/surface.ts`'s `surfaceModeFromShell`. The answer is
   * "the value is different now", so `unknown -> native-below` counts as a
   * change: it is announced like any other transition (a listener that only
   * acts on the wasm carrier ignores it, which is why the first native report
   * cannot disturb a `native-below` host).
   */
  observe(surfaceMode: unknown): boolean;
  /** Whether the wasm session owns the CAD document, i.e. the carrier is `dom-canvas`. */
  usesWasmCad(): boolean;
  /** Listen for changes; returns the unsubscribe function. */
  subscribe(listener: CarrierListener): () => void;
}

/**
 * A carrier store over one initial state.
 *
 * Exported so the transitions are testable without touching the module
 * singleton (a test that flips the singleton would leak into every later test in
 * the same process). `apps/desktop-ui` uses the singleton below.
 */
export function createCarrierStore(initial: Carrier = initialCarrier(RUNTIME)): CarrierStore {
  let carrier: Carrier = initial;
  const listeners = new Set<CarrierListener>();
  return {
    now: () => carrier,
    observe(surfaceMode: unknown): boolean {
      const next: Carrier = surfaceMode === 'dom-canvas' ? 'dom-canvas' : 'native-below';
      if (next === carrier) return false;
      carrier = next;
      // Snapshot: a listener that unsubscribes (or subscribes) during the
      // notification must not change what this transition announces.
      for (const listener of [...listeners]) listener(carrier);
      return true;
    },
    usesWasmCad: () => carrier === 'dom-canvas',
    subscribe(listener: CarrierListener): () => void {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
  };
}

/** The process-wide carrier state of this bundle. */
const state: CarrierStore = createCarrierStore();

/** The carrier right now (see [`Carrier`]). */
export function carrierNow(): Carrier {
  return state.now();
}

/** Apply one reported `cad_status.surfaceMode`; `true` when the carrier changed. */
export function observeCarrier(surfaceMode: unknown): boolean {
  return state.observe(surfaceMode);
}

/** Whether the wasm CAD session owns the document on the observed carrier. */
export function usesWasmCad(): boolean {
  return state.usesWasmCad();
}

/** Listen for carrier changes; returns the unsubscribe function. */
export function subscribeCarrier(listener: CarrierListener): () => void {
  return state.subscribe(listener);
}

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
