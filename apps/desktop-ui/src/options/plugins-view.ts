// The Plugins tab's view model (pure data, no JSX and no DOM).
//
// The wire contract — `readPluginStatus`, `setPluginEnabled`, `installPluginFile`,
// `uninstallPlugin` — belongs to `plugins/plugin-manager.ts`. This module only
// derives what the tab *shows* from one `PluginRow`, so the three decisions that
// are easy to get subtly wrong can be unit-tested without a window:
//
//   * which of the four states a row is in. `status` is the host's own verdict
//     (`invalid` wins over everything); otherwise the pair
//     `enabled`/`sessionActive` says whether the row is running now, is saved for
//     the next start, or is off. The tab never guesses `pendingRestart` from a
//     timer or a mutation it just sent — it reads it back.
//   * which action a row may offer. Only a `user` plugin can be removed; a
//     built-in one is shipped with the application and has no uninstall path.
//   * whether a picked file is even worth sending to the host. The rules mirror
//     the installer's, but the host re-checks and its refusal is authoritative:
//     this is a pre-check that keeps a wrong pick from becoming a request whose
//     only possible answer is a refusal.
//
// The vendor-settled summary lives here too: the event carries a host-shaped
// record, and the tab must survive a record that is missing fields (an older
// host, or a subscriber that fires before the first pass) instead of rendering
// `undefined` as an applied count.
import { MAX_PLUGIN_BYTES, PLUGIN_EXTENSIONS } from '../plugins/plugin-manager.ts';
import type { PluginRow } from '../plugins/types.ts';

/** The four states a decoder row can read as. */
export type PluginState = 'active' | 'pendingRestart' | 'disabled' | 'invalid';

/**
 * The row's state.
 *
 * `status` is the host's word. `ok` is the only healthy value; the manifest
 * failures (`manifest_missing`, `manifest_invalid`) and the short `invalid`
 * spelling all mean the same thing to this tab — the decoder cannot be loaded —
 * so any other status reads as invalid. With a healthy row, `enabled` is the
 * saved choice for the next start and `sessionActive` says whether this process
 * loaded it; the two disagreeing is exactly what "restart required" means.
 */
export function pluginState(row: PluginRow): PluginState {
  const status = typeof row.status === 'string' ? row.status : '';
  if (status !== 'ok') return 'invalid';
  if (!row.enabled) return 'disabled';
  return row.sessionActive ? 'active' : 'pendingRestart';
}

/** The catalog key for one state's pill. */
export function pluginStateKey(state: PluginState): string {
  return `plugins.state.${state}`;
}

/** Whether the row offers "Uninstall": a user plugin, never a built-in one. */
export function canUninstall(row: PluginRow): boolean {
  return row.source === 'user';
}

/** Whether the row offers "Disable" (only the saved-on decoder can be turned off). */
export function canDisable(row: PluginRow): boolean {
  return row.enabled === true;
}

/** The only file kinds the installer accepts (the host's extension check). */
export const PLUGIN_FILE_EXTENSIONS: readonly string[] = PLUGIN_EXTENSIONS;

/**
 * The pre-check size limit, in bytes.
 *
 * `plugin-manager.ts` is the authority and refuses the same file again after
 * reading it, so the pre-check and the actual installer share one constant.
 */
export const PLUGIN_MAX_BYTES = MAX_PLUGIN_BYTES;

/** What is wrong with a picked file (`plugins.fileType` / `plugins.fileTooLarge`). */
export type PluginFileProblem = 'fileType' | 'fileTooLarge' | null;

/** The catalog key for a file problem. */
export function pluginFileProblemKey(problem: Exclude<PluginFileProblem, null>): string {
  return `plugins.${problem}`;
}

/**
 * Whether a `File` may be offered for install.
 *
 * The extension check is case-insensitive because a browser hands over the name
 * as the user typed it on disk; a size of exactly the limit is allowed. A file
 * with no name or no size is treated as the wrong kind — the tab cannot claim a
 * file it cannot describe.
 */
export function pluginFileProblem(
  file: { readonly name?: unknown; readonly size?: unknown } | null | undefined,
): PluginFileProblem {
  if (!file) return 'fileType';
  const name = typeof file.name === 'string' ? file.name.trim().toLowerCase() : '';
  if (!PLUGIN_FILE_EXTENSIONS.some(extension => name.endsWith(extension))) return 'fileType';
  const size = typeof file.size === 'number' && Number.isFinite(file.size) ? file.size : 0;
  return size > PLUGIN_MAX_BYTES ? 'fileTooLarge' : null;
}

/** A DOM-safe stem for one plugin id (`my.plugin` cannot break an `id=`). */
export function pluginRowDomId(id: unknown): string {
  const text = typeof id === 'string' ? id : '';
  const safe = text.replace(/[^A-Za-z0-9_-]/g, '-');
  return safe.length > 0 ? safe : 'plugin';
}

/** Where the last vendor pass left the drawing: how much it applied, and what it said. */
export interface VendorSettledSummary {
  readonly applied: number;
  readonly diagnostics: readonly string[];
}

/**
 * Normalise one `subscribeVendorSettled`/`lastVendorSettled` record.
 *
 * `null` means "no pass has settled yet" (the neutral state, never a zero count
 * presented as a finished pass). A record with missing fields becomes a pass with
 * zero applied objects and no messages, which is honest: the pass happened, the
 * tab just has nothing else to show.
 */
export function vendorSettledSummary(value: unknown): VendorSettledSummary | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const record = value as { readonly applied?: unknown; readonly appliedCount?: unknown; readonly diagnostics?: unknown };
  const raw = typeof record.applied === 'number' ? record.applied : record.appliedCount;
  const applied = typeof raw === 'number' && Number.isFinite(raw) ? raw : 0;
  const diagnostics = Array.isArray(record.diagnostics)
    ? record.diagnostics.filter((entry): entry is string => typeof entry === 'string' && entry.trim().length > 0)
    : [];
  return { applied, diagnostics };
}

/** The status frame's scan diagnostics, or an empty list. */
export function scanDiagnostics(frame: { readonly diagnostics?: unknown } | null | undefined): string[] {
  const list = frame?.diagnostics;
  if (!Array.isArray(list)) return [];
  return list.filter((entry): entry is string => typeof entry === 'string' && entry.trim().length > 0);
}
