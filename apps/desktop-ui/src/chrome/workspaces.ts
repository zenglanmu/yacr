// Workspace presets: a named snapshot of the chrome's own UI state.
//
// A workspace is **docked UI state only** — which chrome regions are visible
// (ribbon, palette layer, drawing tabs) and the palette panel width. It carries
// no CAD document state and is never written to a file: the store lives for the
// lifetime of the document, and a reload starts from the built-in preset, exactly
// like the palette geometry store.
//
// One built-in preset always exists ("Drafting & Annotation"): the startup
// chrome, stored as a catalog key so both locales read the same name. Saved
// presets carry a user-entered name verbatim (user data is never translated).
// `WSSAVE` captures the current UI state, `WSSETTINGS` lists/renames/deletes,
// `WORKSPACE` applies one.
//
// The store is a tiny external store (`subscribe` + an immutable snapshot) so the
// shell and the selector read one source through `useSyncExternalStore`. All
// functions are pure state transitions over frozen rows, which is what the Node
// contract tests exercise without a DOM.
import { PALETTE_DEFAULT_WIDTH, PALETTE_MAX_DEFAULT_WIDTH, PALETTE_MIN_WIDTH } from '../layout/palette-window.ts';
import { DEFAULT_CHROME_VISIBILITY } from './visibility.ts';
import type { ChromeVisibility } from './visibility.ts';
import type { Translator } from '../i18n.ts';

/** The built-in preset's id (stable; `WSSETTINGS` can never delete it). */
export const DRAFTING_WORKSPACE_ID = 'drafting-annotation';

/** The built-in preset's catalog key, so both locales name it once. */
export const DRAFTING_WORKSPACE_NAME_KEY = 'workspace.drafting_annotation';

/** The narrowest panel width a preset may capture (the palette store's own bound). */
export const WORKSPACE_PANEL_MIN_WIDTH = PALETTE_MIN_WIDTH;

/** The widest panel width a preset may capture (the responsive default's ceiling). */
export const WORKSPACE_PANEL_MAX_WIDTH = PALETTE_MAX_DEFAULT_WIDTH;

/** One workspace preset. */
export interface WorkspacePreset {
  readonly id: string;
  /** The catalog key of a built-in name, or `null` for a user-named preset. */
  readonly nameKey: string | null;
  /** The user-entered name, or `null` for a built-in preset. */
  readonly name: string | null;
  readonly visibility: ChromeVisibility;
  readonly panelWidth: number;
  /** Built-ins cannot be renamed or deleted. */
  readonly builtin: boolean;
}

/** The observable store value. */
export interface WorkspaceSnapshot {
  readonly presets: readonly WorkspacePreset[];
  /** The preset last applied or saved (always a real id). */
  readonly currentId: string;
  /** Bumped by every explicit apply, so a repeated apply re-runs its effect. */
  readonly applyRevision: number;
}

/** The result of saving a named preset. */
export type SaveWorkspaceResult =
  | { readonly ok: true; readonly id: string }
  | { readonly ok: false; readonly reason: 'empty' | 'duplicate' };

/** The result of renaming a saved preset. */
export type RenameWorkspaceResult =
  | { readonly ok: true }
  | { readonly ok: false; readonly reason: 'empty' | 'duplicate' | 'builtin' | 'unknown' };

const BUILTIN_PRESETS: readonly WorkspacePreset[] = Object.freeze([
  Object.freeze({
    id: DRAFTING_WORKSPACE_ID,
    nameKey: DRAFTING_WORKSPACE_NAME_KEY,
    name: null,
    visibility: DEFAULT_CHROME_VISIBILITY,
    panelWidth: PALETTE_DEFAULT_WIDTH,
    builtin: true,
  }),
]);

let snapshot: WorkspaceSnapshot = Object.freeze({
  presets: BUILTIN_PRESETS,
  currentId: DRAFTING_WORKSPACE_ID,
  applyRevision: 0,
});
let nextUserNumber = 1;
const LISTENERS = new Set<() => void>();

function publish(next: WorkspaceSnapshot): void {
  snapshot = Object.freeze(next);
  for (const listener of [...LISTENERS]) listener();
}

/** The current store value (referentially stable until something changes). */
export function workspaceSnapshot(): WorkspaceSnapshot {
  return snapshot;
}

/** Subscribe to store changes; returns the unsubscribe function. */
export function subscribeWorkspaces(listener: () => void): () => void {
  LISTENERS.add(listener);
  return () => { LISTENERS.delete(listener); };
}

/** The preset with `id`, or `null`. */
export function workspacePreset(id: string): WorkspacePreset | null {
  return snapshot.presets.find(preset => preset.id === id) ?? null;
}

/** The reader-facing name: the built-in key through `t`, a saved name verbatim. */
export function workspaceName(preset: WorkspacePreset, t: Translator): string {
  return preset.nameKey !== null ? t(preset.nameKey) : preset.name ?? '';
}

/** Trim a typed workspace name and collapse internal runs of whitespace. */
export function normalizeWorkspaceName(name: unknown): string {
  return String(name ?? '').trim().replace(/\s+/g, ' ');
}

/** Clamp a requested panel width into the palette store's own bounds. */
export function clampWorkspaceWidth(width: unknown): number {
  const value = typeof width === 'number' ? width : Number(width);
  if (!Number.isFinite(value)) return PALETTE_DEFAULT_WIDTH;
  return Math.max(PALETTE_MIN_WIDTH, Math.min(PALETTE_MAX_DEFAULT_WIDTH, Math.round(value)));
}

function nameTaken(name: string, exceptId: string | null = null): boolean {
  const needle = name.toLowerCase();
  return snapshot.presets.some(preset => preset.id !== exceptId
    && preset.name !== null
    && preset.name.toLowerCase() === needle);
}

/**
 * Save `visibility`/`panelWidth` as a new named preset and make it current.
 *
 * A blank name and a duplicate (case-insensitive) user name are explicit
 * refusals; the caller words them from the catalog. The state itself already
 * matches, so saving does not bump `applyRevision` (no re-apply is needed).
 */
export function saveWorkspace(
  name: unknown,
  visibility: ChromeVisibility,
  panelWidth: unknown,
): SaveWorkspaceResult {
  const normalized = normalizeWorkspaceName(name);
  if (!normalized) return { ok: false, reason: 'empty' };
  if (nameTaken(normalized)) return { ok: false, reason: 'duplicate' };
  const id = `workspace-${nextUserNumber}`;
  nextUserNumber += 1;
  const preset: WorkspacePreset = Object.freeze({
    id,
    nameKey: null,
    name: normalized,
    visibility: Object.freeze({ ...visibility }),
    panelWidth: clampWorkspaceWidth(panelWidth),
    builtin: false,
  });
  publish({
    presets: Object.freeze([...snapshot.presets, preset]),
    currentId: id,
    applyRevision: snapshot.applyRevision,
  });
  return { ok: true, id };
}

/** Rename a saved preset. Built-ins and unknown ids are explicit refusals. */
export function renameWorkspace(id: string, name: unknown): RenameWorkspaceResult {
  const preset = workspacePreset(id);
  if (!preset) return { ok: false, reason: 'unknown' };
  if (preset.builtin) return { ok: false, reason: 'builtin' };
  const normalized = normalizeWorkspaceName(name);
  if (!normalized) return { ok: false, reason: 'empty' };
  if (nameTaken(normalized, id)) return { ok: false, reason: 'duplicate' };
  publish({
    ...snapshot,
    presets: Object.freeze(snapshot.presets.map(row => (row.id === id
      ? Object.freeze({ ...row, name: normalized })
      : row))),
  });
  return { ok: true };
}

/**
 * Delete a saved preset. Built-ins and unknown ids are refused. Deleting the
 * current preset falls back to the built-in one (without re-applying: the
 * running chrome is the user's, not a preset's to overwrite).
 */
export function deleteWorkspace(id: string): boolean {
  const preset = workspacePreset(id);
  if (!preset || preset.builtin) return false;
  publish({
    presets: Object.freeze(snapshot.presets.filter(row => row.id !== id)),
    currentId: snapshot.currentId === id ? DRAFTING_WORKSPACE_ID : snapshot.currentId,
    applyRevision: snapshot.applyRevision,
  });
  return true;
}

/** Make `id` current without applying it (used by the settings list). */
export function setCurrentWorkspace(id: string): boolean {
  if (!workspacePreset(id)) return false;
  if (snapshot.currentId === id) return true;
  publish({ ...snapshot, currentId: id });
  return true;
}

/**
 * Apply `id`: make it current and bump `applyRevision` so the shell re-applies
 * its visibility/width even when it is already the current preset.
 */
export function requestApplyWorkspace(id: string): boolean {
  if (!workspacePreset(id)) return false;
  publish({
    ...snapshot,
    currentId: id,
    applyRevision: snapshot.applyRevision + 1,
  });
  return true;
}

/** The preset an explicit apply should paint, or `null` before the first apply. */
export function appliedWorkspace(): WorkspacePreset | null {
  return snapshot.applyRevision === 0 ? null : workspacePreset(snapshot.currentId);
}

/** Reset to the built-in preset. Contract tests call this between cases. */
export function resetWorkspaces(): void {
  nextUserNumber = 1;
  publish({ presets: BUILTIN_PRESETS, currentId: DRAFTING_WORKSPACE_ID, applyRevision: 0 });
}
