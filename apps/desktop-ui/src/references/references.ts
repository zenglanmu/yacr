// External references: the `RESOURCES` selector plus the display rows the
// References panel renders.
//
// The host owns the reference graph and reports it in `cad_status.references`
// (additive, absent on an older host). This module never resolves a path, never
// retries a load and never offers a "locate" action: no host command exists for
// those, so the panel only shows what the status already says. A row's `status`
// is a closed machine value; the panel words it from the catalog and keeps the
// host's stable reason code visible instead of inventing prose.
import { referencesOf } from '../cad/cad-status.ts';
import type { CadReferenceKind, CadReferenceStatus, CadStatus } from '../cad/cad-status.ts';

/** The chrome-local selector for the References panel. */
export const RESOURCES_COMMAND = 'RESOURCES';

/** Whether typed text is the local `RESOURCES` selector (case-insensitive). */
export function isResourcesCommand(text: string): boolean {
  return text.trim().toUpperCase() === RESOURCES_COMMAND;
}

/** Catalog key for each reference state (wire values are the Rust kebab-case set). */
export const REFERENCE_STATUS_KEYS: Readonly<Record<CadReferenceKind, string>> = Object.freeze({
  'loaded': 'references.status.loaded',
  'missing': 'references.status.missing',
  'browser-unavailable': 'references.status.browser_unavailable',
  'refused-path': 'references.status.refused_path',
  'unloaded': 'references.status.unloaded',
  'cycle': 'references.status.cycle',
  'depth-exceeded': 'references.status.depth_exceeded',
  'skipped-overlay': 'references.status.skipped_overlay',
  'unsupported': 'references.status.unsupported',
});

/** The visual tone of a state badge (the class suffix the panel renders). */
export type ReferenceTone = 'ok' | 'error' | 'muted';

const ERROR_STATES: ReadonlySet<CadReferenceKind> = new Set([
  'missing', 'browser-unavailable', 'refused-path', 'cycle', 'depth-exceeded',
]);

/** One reference's badge tone: loaded is ok, real failures are error. */
export function referenceTone(status: CadReferenceKind): ReferenceTone {
  if (status === 'loaded') return 'ok';
  return ERROR_STATES.has(status) ? 'error' : 'muted';
}

/**
 * The one explanatory sentence a state needs beyond its badge, or `null`.
 *
 * Only two states get extra words, and both are grounded in what the carrier
 * really did: a browser row cannot read local files, and a missing row's saved
 * path is not a readable file. No other state is embellished.
 */
export function referenceHintKey(status: CadReferenceKind): string | null {
  if (status === 'browser-unavailable') return 'references.hint.browser_unavailable';
  if (status === 'missing') return 'references.hint.missing';
  return null;
}

/**
 * The stable reason code: the text before the first `:`. The host may append a
 * human message; the panel shows the code only, so no host prose leaks into a
 * second language.
 */
export function referenceReasonCode(reason: string | null | undefined): string | null {
  if (typeof reason !== 'string') return null;
  const code = reason.split(':', 1)[0]?.trim() ?? '';
  return code.length > 0 ? code : null;
}

/** One renderable reference row (all values are the host's, never inferred). */
export interface ReferenceRow {
  readonly name: string;
  readonly path: string;
  readonly resolvedPath: string | null;
  readonly status: CadReferenceKind;
  readonly statusKey: string;
  readonly tone: ReferenceTone;
  readonly hintKey: string | null;
  readonly reasonCode: string | null;
  readonly overlay: boolean;
  readonly depth: number;
}

/** Project the host's rows; an absent block is an empty list, never invented. */
export function referenceRows(status: Partial<CadStatus> | null | undefined): readonly ReferenceRow[] {
  return referencesOf(status).map((reference: CadReferenceStatus): ReferenceRow => ({
    name: reference.name,
    path: reference.path,
    resolvedPath: reference.resolvedPath,
    status: reference.status,
    statusKey: REFERENCE_STATUS_KEYS[reference.status] ?? 'references.status.unsupported',
    tone: referenceTone(reference.status),
    hintKey: referenceHintKey(reference.status),
    reasonCode: referenceReasonCode(reference.reason),
    overlay: reference.overlay,
    depth: reference.depth,
  }));
}
