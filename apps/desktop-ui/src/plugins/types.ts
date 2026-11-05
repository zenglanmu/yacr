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
