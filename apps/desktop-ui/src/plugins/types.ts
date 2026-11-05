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
