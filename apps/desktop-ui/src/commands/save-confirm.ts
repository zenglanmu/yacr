// The pre-save confirmation for a `.dwg` source.
//
// The host never overwrites the opened source and always offers a NEW
// `<stem>-export.dwg` name, but the picker used to be the first place the user
// learned that DWG output is experimental. This module owns the one decision the
// chrome can make **before** dispatching SAVE: is the open source a DWG, and if
// so what default name does the host's DWG export offer?
//
// The source identity is the shell's existing `documentLabel` (the host's
// `document_name_hint` on `native-below`, the live session's label on
// `dom-canvas`) — the same string the doc tab and the status bar show. Nothing
// here reads the save picker, and nothing invents a capability: if the label is
// not a `.dwg`, no confirmation is requested.
//
// The name derivation mirrors `cad_host::policy::source_is_dwg` /
// `default_dwg_export_name` (Path::file_stem, `<stem>-export.dwg`, `drawing` as
// the fallback stem) so the dialog states the name the host will actually
// suggest. Both functions are pure and therefore runnable in the Node contracts.

/** The router names whose real host path is the SAVE funnel. */
export const DWG_SAVE_COMMANDS: readonly string[] = Object.freeze(['SAVE', 'SAVEAS', 'QSAVE']);

/** One pending confirmation: the command to re-dispatch and its default name. */
export interface DwgSaveConfirmation {
  /** The exact admitted line to dispatch after the user proceeds. */
  readonly command: string;
  /** The host's default new-file name (`<stem>-export.dwg`). */
  readonly exportName: string;
}

/** What the confirmation's two actions resolve to. */
export type DwgSaveConfirmAction = 'proceed' | 'cancel';

/** The outcome of one action; `confirmation` is always cleared. */
export interface DwgSaveConfirmResolution {
  readonly confirmation: DwgSaveConfirmation | null;
  /** The command to dispatch exactly once, or `null` for cancel. */
  readonly dispatch: string | null;
}

/** The last path/name segment, for both POSIX and Windows separators. */
function sourceFileName(label: unknown): string {
  if (typeof label !== 'string') return '';
  const trimmed = label.trim();
  if (trimmed === '') return '';
  const cut = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'));
  return cut >= 0 ? trimmed.slice(cut + 1) : trimmed;
}

/**
 * The lower-case extension of the source label, or `''`.
 *
 * A leading dot is a hidden file, not an extension (Rust's `Path::extension`
 * agrees), so `.dwg` alone is not a DWG source. A trailing dot is no extension
 * either.
 */
export function sourceExtension(label: unknown): string {
  const name = sourceFileName(label);
  const dot = name.lastIndexOf('.');
  if (dot <= 0 || dot === name.length - 1) return '';
  return name.slice(dot + 1).toLowerCase();
}

/** Whether the open source label denotes a DWG drawing. */
export function isDwgSourceName(label: unknown): boolean {
  return sourceExtension(label) === 'dwg';
}

/** The source's file stem, or `"drawing"` when the label carries none. */
export function sourceStem(label: unknown): string {
  const name = sourceFileName(label);
  const dot = name.lastIndexOf('.');
  const stem = dot > 0 ? name.slice(0, dot) : name;
  return stem === '' ? 'drawing' : stem;
}

/** The host's suggested DWG export name for this source (`<stem>-export.dwg`). */
export function dwgExportName(label: unknown): string {
  return `${sourceStem(label)}-export.dwg`;
}

/**
 * Whether `command` is a SAVE-family line on a `.dwg` source.
 *
 * This is the cheap synchronous pre-check (no vocabulary read): the caller
 * uses it to decide whether admission has to be consulted at all, so local
 * selectors and every other command keep their immediate path.
 */
export function isDwgSaveCandidate(command: unknown, documentLabel: unknown): boolean {
  const name = String(command ?? '').trim().toUpperCase();
  return DWG_SAVE_COMMANDS.includes(name) && isDwgSourceName(documentLabel);
}

/**
 * The confirmation request for one command, or `null` when none is due.
 *
 * Only an admitted SAVE-family line on a `.dwg` source asks: a `.dxf` source,
 * no document and a different command all keep today's immediate dispatch.
 */
export function dwgSaveConfirmationRequest(
  command: unknown,
  documentLabel: unknown,
): DwgSaveConfirmation | null {
  if (!isDwgSaveCandidate(command, documentLabel)) return null;
  return Object.freeze({
    command: String(command).trim().toUpperCase(),
    exportName: dwgExportName(documentLabel),
  });
}

/**
 * Resolve one action on a pending confirmation.
 *
 * The confirmation is cleared in every outcome, so a second call (a double
 * click, a repeat) can only ever answer "nothing to dispatch". `proceed` returns
 * the command once; `cancel` never returns one.
 */
export function resolveDwgSaveConfirmation(
  confirmation: DwgSaveConfirmation | null,
  action: DwgSaveConfirmAction,
): DwgSaveConfirmResolution {
  if (confirmation === null) return { confirmation: null, dispatch: null };
  if (action === 'cancel') return { confirmation: null, dispatch: null };
  return { confirmation: null, dispatch: confirmation.command };
}
