// The plugin manager's wire types, exactly as the shell's `cad_plugin_*`
// commands answer them.
//
// The plugin registry is a **shell** capability (it installs `.js` files into
// the host's own config directory), not a CAD-document capability, so the
// commands route to the Tauri shell on every carrier where a bridge exists. A
// carrier with no such bridge (the browser, an X11 page with no native shell)
// refuses by name instead of inventing an empty list.
//
// Nothing here is "supported by default": a host that cannot read its registry
// answers `supported: false` plus a diagnostic, and the chrome renders that
// truth rather than an empty plugin table that would look like "no plugins
// installed".

/** Where one plugin came from. */
export type PluginSource = 'builtin' | 'user';

/**
 * The validated state of one installed plugin's manifest.
 *
 * The shell contract spells the failure `manifest_invalid`; `invalid` is
 * accepted as an equivalent short spelling from a host. The value is never
 * coerced: the row reports exactly what the host said.
 */
export type PluginRowStatus = 'ok' | 'manifest_missing' | 'manifest_invalid' | 'invalid';

/** One installed plugin row (`cad_plugin_status().plugins[]`). */
export interface PluginRow {
  readonly id: string;
  readonly name: string;
  readonly version: string;
  /** The decoder ABI the plugin declares. */
  readonly abi: number;
  readonly source: PluginSource;
  readonly enabled: boolean;
  /** Whether the running session actually loaded this decoder. */
  readonly sessionActive: boolean;
  readonly status: PluginRowStatus;
  /** When the plugin was installed, if the host keeps that fact. */
  readonly installedAt?: string;
}

/**
 * The shell's plugin registry snapshot.
 *
 * `activeDecoder` is the **configured** decoder for the next document;
 * `sessionActiveDecoder` is what the running session has loaded. They differ
 * while a restart is pending, which is why `restartRequired` exists rather than
 * the chrome guessing.
 */
export interface PluginStatusFrame {
  readonly supported: boolean;
  readonly carrier: string;
  readonly plugins: readonly PluginRow[];
  readonly activeDecoder: string | null;
  readonly sessionActiveDecoder: string | null;
  readonly restartRequired: boolean;
  readonly diagnostics: readonly string[];
}

/** The answer every plugin mutation (`install` / `enable` / `uninstall`) gives. */
export interface PluginMutationResult {
  readonly accepted: boolean;
  /** Stable machine reason for a refusal (`plugin.*`, `vendor.*`, ...). */
  readonly reason?: string;
  readonly restartRequired: boolean;
  /** The registry snapshot **after** the mutation (or the current one on refusal). */
  readonly status: PluginStatusFrame;
}

/** Optional manifest fields a plugin module may declare for itself. */
export interface PluginMeta {
  readonly id?: string;
  readonly name?: string;
  readonly version?: string;
}

/** One `vendor-settled` event, as the chrome remembers it. */
export interface VendorSettled {
  readonly applied: number;
  readonly diagnostics: readonly string[];
  /** `Date.now()` when this frame was recorded. */
  readonly at: number;
}

/** The `invoke` seam the plugin manager takes (host IPC or a test double). */
export type PluginInvoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;

/** The ES-module importer seam (`validatePluginSource`; a test double in Node). */
export type PluginImporter = (url: string) => Promise<unknown>;

// Structural contracts between the chrome and a vendor plugin (ADR 0028 D3).
//
// The plugin is a separately built ES module. It never imports this file: the
// shapes are duplicated there on purpose so the plugin keeps depending on the
// published descriptor, not on host source. This side is the one declaration the
// host validates against.
//
// Every row/panel here is **display-only** derived data. Nothing in this file
// describes a write path into `cad-db`; the plugin cannot change the document,
// the selection or an entity identity (ADR 0028 D6).

import type { CadStatus } from '../cad/cad-status.ts';

/** One selection row as the plugin reads it (`cad_status.selection.rows`). */
export interface PluginSelectionRow {
  readonly key: string;
  readonly value: string | null;
  readonly group: string;
  readonly mixed: boolean;
  readonly editable: boolean;
  readonly kind: string;
  readonly options: readonly string[];
}

/** The current Rust selection snapshot handed to `properties`/`command`. */
export interface PluginSelection {
  readonly count: number;
  readonly empty: boolean;
  readonly rows: readonly PluginSelectionRow[];
}

/** One read-only row of a plugin property panel. `key` is a stable machine key. */
export interface PluginPropertyRow {
  readonly key: string;
  readonly value: string;
}

/** One group of plugin property rows (`identity` / `geometry` / `text` / ...). */
export interface PluginPropertyGroup {
  readonly key: string;
  readonly rows: readonly PluginPropertyRow[];
}

/**
 * The panel a plugin may return from a `properties` handler.
 *
 * `handle` is the source provenance of the single selected entity; the chrome
 * only renders the panel while the live selection still carries that handle.
 */
export interface PluginPropertyPanel {
  readonly kind: string;
  readonly handle: string;
  readonly groups: readonly PluginPropertyGroup[];
}

/** One command request, after admission and before `cad_command` is sent. */
export interface PluginCommandRequest {
  readonly command: string;
  readonly selection: PluginSelection;
}

/** A command veto. `null` means "not handled, let the router run it". */
export type PluginCommandDecision = { readonly handled: true; readonly messageKey: string } | null;

/** Every event a plugin may subscribe to, and its handler signature. */
export interface VendorPluginHost {
  on(event: 'properties', handler: (selection: PluginSelection) => PluginPropertyPanel | null): void;
  on(event: 'command', handler: (request: PluginCommandRequest) => PluginCommandDecision): void;
  on(event: 'cad-status', handler: (status: CadStatus) => void): void;
}

/**
 * The extended ABI v1 module shape: `decode` (required) plus the optional
 * startup registration hook. `vendor-pass.ts` re-exports the same contract.
 */
export interface VendorPluginModule {
  readonly abiVersion: number;
  decode(records: ArrayBuffer): ArrayBuffer;
  register?(host: VendorPluginHost): void;
}
