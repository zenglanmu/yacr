// The frontend vendor-plugin host (ADR 0028 D3/D4/D5).
//
// One module owns three things and nothing else:
//
// * **Registration** — load the plugin module once, validate its optional
//   `register`, and collect the handlers it subscribes with. Loading or
//   registration failure rejects; callers fall back to the default palette and
//   the ordinary command funnel instead of blocking the app.
// * **Dispatch** — `properties` (first non-null panel wins), `command` (first
//   `{handled:true}` wins) and `cad-status` (every handler, errors isolated).
//   Handler exceptions are "not handled": the next handler is asked and the
//   router/default chrome still runs. They never become a silent success.
// * **The one external store** for the current panel and the diagnostic notice,
//   read by React through `useSyncExternalStore`. No React state lives here.
//
// The plugin remains a same-privilege module script (ADR 0028 D6): this file is
// interop glue, not a security boundary, and it never writes to `cad-db`.

import { useSyncExternalStore } from 'react';
import { loadVendorPlugin } from '../vendor/vendor-pass.ts';
import type { CadSelection, CadStatus } from '../cad/cad-status.ts';
import type {
  PluginCommandDecision,
  PluginCommandRequest,
  PluginPropertyPanel,
  PluginSelection,
  VendorPluginHost,
  VendorPluginModule,
} from './types.ts';

type PropertiesHandler = (selection: PluginSelection) => PluginPropertyPanel | null;
type CommandHandler = (request: PluginCommandRequest) => PluginCommandDecision;
type CadStatusHandler = (status: CadStatus) => void;

interface HandlerSet {
  readonly properties: PropertiesHandler[];
  readonly command: CommandHandler[];
  readonly 'cad-status': CadStatusHandler[];
}

/** One diagnostic notice. `key` must resolve in the shared catalogs. */
export interface PluginNotice {
  readonly key: string;
  readonly params?: Readonly<Record<string, string>>;
}

function emptyHandlers(): HandlerSet {
  return { properties: [], command: [], 'cad-status': [] };
}

let handlers: HandlerSet = emptyHandlers();
let registration: Promise<void> | null = null;
let panel: PluginPropertyPanel | null = null;
let notice: PluginNotice | null = null;
const listeners = new Set<() => void>();

function emit(): void {
  for (const listener of [...listeners]) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

function setPanel(next: PluginPropertyPanel | null): void {
  if (panel === next) return;
  panel = next;
  emit();
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/**
 * Minimal structural validation of a plugin panel.
 *
 * This is deliberately shallow (strings, arrays, row key/value strings): the
 * panel is display-only data and the host renders no plugin HTML. A malformed
 * shape is rejected as `null` rather than repaired into a guessed panel.
 */
function validPanel(value: unknown): PluginPropertyPanel | null {
  if (!isRecord(value)) return null;
  if (typeof value.kind !== 'string' || typeof value.handle !== 'string' || !Array.isArray(value.groups)) {
    return null;
  }
  for (const group of value.groups) {
    if (!isRecord(group) || typeof group.key !== 'string' || !Array.isArray(group.rows)) return null;
    for (const row of group.rows) {
      if (!isRecord(row) || typeof row.key !== 'string' || typeof row.value !== 'string') return null;
    }
  }
  return value as unknown as PluginPropertyPanel;
}

/** The plugin-facing selection: the same rows, without the wire-only extras. */
function pluginSelection(selection: CadSelection | null): PluginSelection {
  if (!selection) return { count: 0, empty: true, rows: [] };
  return {
    count: selection.count,
    empty: selection.empty,
    rows: selection.rows.map(row => ({ ...row, options: row.options ?? [] })),
  };
}

function addHandler(target: HandlerSet, event: string, handler: unknown): void {
  if (typeof handler !== 'function') return;
  if (event === 'properties') target.properties.push(handler as PropertiesHandler);
  else if (event === 'command') target.command.push(handler as CommandHandler);
  else if (event === 'cad-status') target['cad-status'].push(handler as CadStatusHandler);
}

/**
 * Call `module.register` once and commit the handlers it installed.
 *
 * The set is committed only after `register` returns: a throw leaves the host
 * with no providers (the previous set is already reset by the caller), exactly
 * like a missing `register`.
 */
function registerModule(module: VendorPluginModule): void {
  const next = emptyHandlers();
  const host = {
    on(event: string, handler: unknown): void { addHandler(next, event, handler); },
  } as unknown as VendorPluginHost;
  if (typeof module.register === 'function') module.register(host);
  handlers = next;
}

/**
 * Load and register the plugin once. Safe to call repeatedly.
 *
 * Resolves with no handlers when the module has no `register`; rejects when the
 * module cannot load or `register` throws, so `use-cad-events` can report one
 * diagnostic line. The rejected promise is forgotten, so a later call retries
 * rather than pinning the failure forever.
 */
export function ensurePluginRegistered(): Promise<void> {
  if (registration !== null) return registration;
  registration = loadVendorPlugin()
    .then(module => { registerModule(module); })
    .catch((error: unknown) => {
      registration = null;
      throw error;
    });
  return registration;
}

/** Deliver one `status-changed` snapshot to every `cad-status` handler. */
export function dispatchCadStatus(status: CadStatus): void {
  for (const handler of [...handlers['cad-status']]) {
    try { handler(status); } catch { /* a plugin error is not a host failure */ }
  }
}

/**
 * Ask the `properties` providers for a panel for the current selection.
 *
 * The first non-null panel wins and becomes the current panel. `null` from every
 * handler (or no handlers) clears it, so the default palette is shown. A panel
 * returned for anything other than a single selection violates the descriptor:
 * it is rejected and reported once as `vendor.plugin.multi_selection_unsupported`.
 */
export function requestProperties(selection: CadSelection | null): void {
  const current = pluginSelection(selection);
  for (const handler of [...handlers.properties]) {
    let candidate: PluginPropertyPanel | null;
    try {
      candidate = handler(current);
    } catch {
      continue;
    }
    if (candidate === null || candidate === undefined) continue;
    if (current.count !== 1) {
      setPanel(null);
      showPluginNotice('vendor.plugin.multi_selection_unsupported');
      return;
    }
    const valid = validPanel(candidate);
    setPanel(valid);
    return;
  }
  setPanel(null);
}

/**
 * Ask the `command` providers whether they veto an admitted command.
 *
 * The first `{handled:true, messageKey}` wins; a throwing handler is "not
 * handled" and the next provider is asked. `null` means the ordinary router
 * command must run.
 */
export function pluginCommand(command: string, selection: CadSelection | null): PluginCommandDecision {
  const request: PluginCommandRequest = { command, selection: pluginSelection(selection) };
  for (const handler of [...handlers.command]) {
    let decision: PluginCommandDecision;
    try {
      decision = handler(request);
    } catch {
      continue;
    }
    if (decision && decision.handled === true
      && typeof decision.messageKey === 'string' && decision.messageKey !== '') {
      return { handled: true, messageKey: decision.messageKey };
    }
  }
  return null;
}

/** The current plugin panel (`null` when the default palette should show). */
export function currentPluginPanelForTests(): PluginPropertyPanel | null {
  return panel;
}

/** The current diagnostic notice (`null` when no modal should render). */
export function currentPluginNoticeForTests(): PluginNotice | null {
  return notice;
}

/** React subscription: the current plugin panel. */
export function usePluginPanel(): PluginPropertyPanel | null {
  return useSyncExternalStore(subscribe, currentPluginPanelForTests, currentPluginPanelForTests);
}

/** React subscription: the current diagnostic notice. */
export function usePluginNotice(): PluginNotice | null {
  return useSyncExternalStore(subscribe, currentPluginNoticeForTests, currentPluginNoticeForTests);
}

/** Drop the plugin panel; the dock stays open and shows the default palette. */
export function clearPluginPanel(): void {
  setPanel(null);
}

/** Show one diagnostic modal. The key must exist in the shared catalogs. */
export function showPluginNotice(key: string, params?: Readonly<Record<string, string>>): void {
  notice = params ? { key, params } : { key };
  emit();
}

/** Dismiss the diagnostic modal. */
export function dismissPluginNotice(): void {
  if (notice === null) return;
  notice = null;
  emit();
}

/**
 * Test seam: install a module directly, without loading `plugins/tch.js`.
 *
 * The previous handler set, panel and notice are discarded first so one test
 * cannot observe another's registration (the real app registers once).
 */
export function registerPluginModuleForTests(module: VendorPluginModule): void {
  resetPluginHostForTests();
  registerModule(module);
  registration = Promise.resolve();
}

/** Test seam: forget every handler, panel, notice and pending registration. */
export function resetPluginHostForTests(): void {
  handlers = emptyHandlers();
  registration = null;
  const changed = panel !== null || notice !== null;
  panel = null;
  notice = null;
  if (changed) emit();
}
