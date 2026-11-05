// The plugin registry's logic: status, install / enable / uninstall, source
// validation, and the remembered `vendor-settled` frame.
//
// # Why this is not a CAD command layer
//
// A plugin is an ES module that decodes one vendor's opaque records (the current
// one is the bundled Tianzheng decoder). Installing it writes a file into the
// **shell's** config directory, so the five `cad_plugin_*` commands are
// shell-only: `host/invoke.ts` routes them to the Tauri bridge whenever one
// exists, and `host/web-adapter.ts` refuses them by name on a page with no
// shell. This module never pretends otherwise.
//
// # No fabricated state
//
// A carrier without the commands (or without a registry) answers
// `supported: false` with the real refusal in `diagnostics`; it does not answer
// an empty plugin list, which the chrome would render as "nothing installed".
// Any other host failure is rethrown, because a broken shell is not the same
// thing as an unsupported carrier.
//
// # Validation
//
// `installPluginFile` checks the extension and the 4 MiB cap before reading the
// file, then imports the source as a real ES module and requires the ABI v1
// contract (`abiVersion === 1` + `decode`). The module is loaded through a Blob
// URL because that is the only way to evaluate caller-supplied source without
// touching the document; the global it registers on is restored afterwards so a
// validation run cannot replace the plugin a live pass is using.

import type {
  PluginImporter,
  PluginInvoke,
  PluginMeta,
  PluginMutationResult,
  PluginRow,
  PluginSource,
  PluginRowStatus,
  PluginStatusFrame,
  VendorSettled,
} from './types.ts';

/** The bundled decoder's id. */
export const BUILTIN_PLUGIN_ID = 'tch';

/** The largest plugin source this UI will hand to the shell (4 MiB). */
export const MAX_PLUGIN_BYTES = 4 * 1024 * 1024;

/** The only accepted plugin source extensions. */
export const PLUGIN_EXTENSIONS: readonly string[] = ['.js', '.mjs'];

const PLUGIN_STATUS_COMMAND = 'cad_plugin_status';
const PLUGIN_INSTALL_COMMAND = 'cad_plugin_install';
const PLUGIN_ENABLE_COMMAND = 'cad_plugin_enable';
const PLUGIN_UNINSTALL_COMMAND = 'cad_plugin_uninstall';

const CONTRACT_INVALID = 'plugin.contract_invalid';
const EXTENSION_INVALID = 'plugin.extension_invalid';
const PAYLOAD_TOO_LARGE = 'plugin.payload_too_large';

/** The global a vendor plugin registers itself on. */
interface VendorGlobal {
  __YACR_VENDOR_PLUGIN__?: unknown;
}

/** The optional manifest fields a validated module may carry. */
interface PluginCandidate {
  readonly abiVersion?: unknown;
  readonly decode?: unknown;
  readonly id?: unknown;
  readonly name?: unknown;
  readonly version?: unknown;
}

/** A plain JSON object, or `null` for anything else (arrays included). */
function plainRecord(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

/** The message of a thrown value, without the `Error: ` prefix. */
function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * Whether a host failure means "this carrier does not serve the plugin
 * commands", rather than "the shell failed while serving them".
 *
 * The adapter's own wording is `unsupported-runtime: <command>`; the native
 * hosts use the same stable prefix. A missing Tauri bridge is the same class of
 * absence (the command cannot exist), and a couple of plain phrasings are
 * accepted so a different transport does not turn a capability gap into a loud
 * render error.
 */
function capabilityAbsent(error: unknown): boolean {
  const message = errorMessage(error);
  return /unsupported-runtime:\s*cad_plugin_/.test(message)
    || /no tauri (?:ipc )?bridge/i.test(message)
    || /plugin commands? (?:are )?not (?:available|supported)/i.test(message);
}

/** The honest frame for a carrier that has no plugin registry at all. */
function unsupportedStatus(diagnostic: string): PluginStatusFrame {
  return {
    supported: false,
    carrier: 'unsupported',
    plugins: [],
    activeDecoder: null,
    sessionActiveDecoder: null,
    restartRequired: false,
    diagnostics: [diagnostic],
  };
}

/** One plugin row, validated (never coerced into a plausible-looking row). */
function parseRow(value: unknown): PluginRow {
  const record = plainRecord(value);
  if (record === null) throw new Error('plugin.status_invalid: plugin row');
  const source = record.source;
  const status = record.status;
  if (source !== 'builtin' && source !== 'user') throw new Error('plugin.status_invalid: plugin source');
  if (status !== 'ok' && status !== 'manifest_missing' && status !== 'manifest_invalid' && status !== 'invalid') {
    throw new Error('plugin.status_invalid: plugin status');
  }
  if (
    typeof record.id !== 'string'
    || typeof record.name !== 'string'
    || typeof record.version !== 'string'
    || typeof record.abi !== 'number'
    || typeof record.enabled !== 'boolean'
    || typeof record.sessionActive !== 'boolean'
  ) {
    throw new Error('plugin.status_invalid: plugin fields');
  }
  const row: {
    id: string;
    name: string;
    version: string;
    abi: number;
    source: PluginSource;
    enabled: boolean;
    sessionActive: boolean;
    status: PluginRowStatus;
    installedAt?: string;
  } = {
    id: record.id,
    name: record.name,
    version: record.version,
    abi: record.abi,
    source,
    enabled: record.enabled,
    sessionActive: record.sessionActive,
    status,
  };
  if (record.installedAt !== undefined) {
    if (typeof record.installedAt !== 'string') throw new Error('plugin.status_invalid: installedAt');
    row.installedAt = record.installedAt;
  }
  return row;
}

/** One status frame, validated field by field. */
function parseStatusFrame(value: unknown): PluginStatusFrame {
  const record = plainRecord(value);
  if (record === null) throw new Error('plugin.status_invalid');
  if (typeof record.supported !== 'boolean' || typeof record.carrier !== 'string') {
    throw new Error('plugin.status_invalid');
  }
  if (!Array.isArray(record.plugins)) throw new Error('plugin.status_invalid: plugins');
  const diagnostics = record.diagnostics;
  if (!Array.isArray(diagnostics) || diagnostics.some(item => typeof item !== 'string')) {
    throw new Error('plugin.status_invalid: diagnostics');
  }
  const oneDecoder = (field: unknown): string | null => {
    if (field === null) return null;
    if (typeof field !== 'string') throw new Error('plugin.status_invalid: decoder');
    return field;
  };
  if (typeof record.restartRequired !== 'boolean') throw new Error('plugin.status_invalid: restartRequired');
  return {
    supported: record.supported,
    carrier: record.carrier,
    plugins: record.plugins.map(parseRow),
    activeDecoder: oneDecoder(record.activeDecoder),
    sessionActiveDecoder: oneDecoder(record.sessionActiveDecoder),
    restartRequired: record.restartRequired,
    diagnostics: diagnostics as string[],
  };
}

/** One mutation answer, validated. */
function parseMutationResult(value: unknown): PluginMutationResult {
  const record = plainRecord(value);
  if (record === null || typeof record.accepted !== 'boolean' || typeof record.restartRequired !== 'boolean') {
    throw new Error('plugin.result_invalid');
  }
  if (record.reason !== undefined && typeof record.reason !== 'string') {
    throw new Error('plugin.result_invalid: reason');
  }
  const result: {
    accepted: boolean;
    reason?: string;
    restartRequired: boolean;
    status: PluginStatusFrame;
  } = {
    accepted: record.accepted,
    restartRequired: record.restartRequired,
    status: parseStatusFrame(record.status),
  };
  if (typeof record.reason === 'string') result.reason = record.reason;
  return result;
}

/**
 * The shell's plugin registry snapshot.
 *
 * A carrier that does not serve the commands (or has no Tauri bridge) resolves
 * to a `supported: false` frame carrying the real refusal; every other failure
 * is rethrown, because "the shell is broken" must not look like "no plugins".
 */
export async function readPluginStatus(invoke: PluginInvoke): Promise<PluginStatusFrame> {
  let raw: unknown;
  try {
    raw = await invoke(PLUGIN_STATUS_COMMAND);
  } catch (error) {
    if (capabilityAbsent(error)) return unsupportedStatus(errorMessage(error));
    throw error;
  }
  return parseStatusFrame(raw);
}

/** Enable or disable one installed plugin by id. */
export async function setPluginEnabled(
  invoke: PluginInvoke,
  id: string,
  enabled: boolean,
): Promise<PluginMutationResult> {
  return parseMutationResult(await invoke(PLUGIN_ENABLE_COMMAND, { id, enabled }));
}

/** Remove one installed plugin by id. */
export async function uninstallPlugin(invoke: PluginInvoke, id: string): Promise<PluginMutationResult> {
  return parseMutationResult(await invoke(PLUGIN_UNINSTALL_COMMAND, { id }));
}

/** The lower-cased extension of `name` (including the dot), or `''`. */
function fileExtension(name: unknown): string {
  if (typeof name !== 'string') return '';
  const dot = name.lastIndexOf('.');
  return dot <= 0 ? '' : name.slice(dot).toLowerCase();
}

/** Base64 of a byte buffer (the wire form every `cad_plugin_*` carrier takes). */
export function base64FromBytes(bytes: Uint8Array): string {
  let binary = '';
  const chunk = 0x8000;
  for (let index = 0; index < bytes.length; index += chunk) {
    binary += String.fromCharCode(...bytes.subarray(index, index + chunk));
  }
  return btoa(binary);
}

/** The bytes behind one base64 wire payload. Invalid text throws (`atob`). */
export function bytesFromBase64(text: string): Uint8Array {
  const binary = atob(text.replace(/\s+/g, ''));
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}

/** The production importer: a Blob URL, evaluated as a real ES module. */
const defaultImporter: PluginImporter = url => import(/* @vite-ignore */ url);

/**
 * Import caller-supplied plugin source and validate the ABI v1 contract.
 *
 * The module registers itself on `globalThis.__YACR_VENDOR_PLUGIN__`, exactly
 * like the bundled plugin does. The previous value is saved and restored around
 * the import (never deleted) so validating an upload cannot disturb a live pass;
 * the Blob URL is revoked either way. `importer` is injectable because Node's
 * default ESM loader cannot import a Blob URL.
 *
 * Success answers `{ok: true, meta}` with the optional string manifest fields;
 * every failure answers a stable reason:
 * `plugin.contract_invalid`, `plugin.abi_mismatch: <n>`.
 */
export async function validatePluginSource(
  source: string,
  importer: PluginImporter = defaultImporter,
): Promise<{ ok: true; meta: PluginMeta } | { ok: false; reason: string }> {
  const scope = globalThis as VendorGlobal;
  const previous = scope.__YACR_VENDOR_PLUGIN__;
  let url = '';
  try {
    const blob = new Blob([source], { type: 'text/javascript' });
    url = URL.createObjectURL(blob);
    await importer(url);
    const candidate = plainRecord(scope.__YACR_VENDOR_PLUGIN__) as PluginCandidate | null;
    if (candidate === null || typeof candidate.decode !== 'function') {
      return { ok: false, reason: CONTRACT_INVALID };
    }
    if (typeof candidate.abiVersion !== 'number') return { ok: false, reason: CONTRACT_INVALID };
    if (candidate.abiVersion !== 1) {
      return { ok: false, reason: `plugin.abi_mismatch: ${candidate.abiVersion}` };
    }
    const meta: { id?: string; name?: string; version?: string } = {};
    if (typeof candidate.id === 'string' && candidate.id.length > 0) meta.id = candidate.id;
    if (typeof candidate.name === 'string' && candidate.name.length > 0) meta.name = candidate.name;
    if (typeof candidate.version === 'string' && candidate.version.length > 0) meta.version = candidate.version;
    return { ok: true, meta };
  } catch {
    return { ok: false, reason: CONTRACT_INVALID };
  } finally {
    if (url !== '') URL.revokeObjectURL(url);
    scope.__YACR_VENDOR_PLUGIN__ = previous;
  }
}

/**
 * A refused install, carrying the registry snapshot the refusal applies to.
 *
 * The status is read rather than invented: a validation failure is still a real
 * answer about the current registry. A non-capability status failure propagates
 * (the shell is broken, not merely unsupported).
 */
async function installRejection(invoke: PluginInvoke, reason: string): Promise<PluginMutationResult> {
  return {
    accepted: false,
    reason,
    restartRequired: false,
    status: await readPluginStatus(invoke),
  };
}

/**
 * Validate one uploaded `.js` / `.mjs` file and install it through the shell.
 *
 * Order matters: the extension and the size cap are checked **before** the file
 * is read, and the ABI contract is validated before anything reaches the host.
 * The payload is the file's exact bytes in base64; the optional `meta` is only
 * sent when the module declared one of its fields.
 *
 * `importer` is a test seam (Node cannot import a Blob URL); production callers
 * pass two arguments.
 */
export async function installPluginFile(
  invoke: PluginInvoke,
  file: File,
  importer: PluginImporter = defaultImporter,
): Promise<PluginMutationResult> {
  const name = typeof file?.name === 'string' ? file.name : '';
  if (!PLUGIN_EXTENSIONS.includes(fileExtension(name))) {
    return installRejection(invoke, EXTENSION_INVALID);
  }
  if (typeof file.size === 'number' && file.size > MAX_PLUGIN_BYTES) {
    return installRejection(invoke, PAYLOAD_TOO_LARGE);
  }
  const bytes = new Uint8Array(await file.arrayBuffer());
  if (bytes.byteLength > MAX_PLUGIN_BYTES) {
    return installRejection(invoke, PAYLOAD_TOO_LARGE);
  }
  const source = new TextDecoder().decode(bytes);
  const validated = await validatePluginSource(source, importer);
  if (!validated.ok) return installRejection(invoke, validated.reason);

  const args: Record<string, unknown> = {
    filename: name,
    payload: base64FromBytes(bytes),
  };
  if (Object.keys(validated.meta).length > 0) args.meta = validated.meta;
  return parseMutationResult(await invoke(PLUGIN_INSTALL_COMMAND, args));
}

// ---------------------------------------------------------------------------
// `vendor-settled` state
// ---------------------------------------------------------------------------

let lastSettled: VendorSettled | null = null;
const settledListeners = new Set<(settled: VendorSettled) => void>();

/**
 * Remember one settled vendor pass and notify subscribers.
 *
 * The host emits `vendor-settled { applied, diagnostics }` after a pass settles;
 * this is the chrome's only record of it (the UI derives its diagnostic summary
 * from here instead of polling). A listener exception is swallowed so one bad
 * subscriber cannot break the event pump.
 */
export function recordVendorSettled(applied: number, diagnostics: string[]): void {
  const snapshot: VendorSettled = {
    applied,
    diagnostics: [...diagnostics],
    at: Date.now(),
  };
  lastSettled = snapshot;
  for (const listener of [...settledListeners]) {
    try {
      listener(snapshot);
    } catch {
      // A listener must not take the transport down with it.
    }
  }
}

/** The most recent settled pass, or `null` before the first one. */
export function lastVendorSettled(): VendorSettled | null {
  return lastSettled;
}

/** Subscribe to settled passes; the returned function unsubscribes. */
export function subscribeVendorSettled(listener: (settled: VendorSettled) => void): () => void {
  settledListeners.add(listener);
  return () => { settledListeners.delete(listener); };
}

/** Test seam: forget the remembered pass and every subscriber. */
export function resetPluginManagerForTests(): void {
  lastSettled = null;
  settledListeners.clear();
}
