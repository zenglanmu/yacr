// 3D view affordances: the typed `cad_standard_view` IPC, the seven standard
// views and the admission rules the ribbon shares with them.
//
// Standard views are the one view family that **cannot** travel on
// `cad_command`: `STANDARDVIEW` maps to a payload-carrying
// `CommandId::StandardView`, and the name-only command funnel refuses a
// payload variant. The host exposes the dedicated typed channel
// `cad_standard_view { view }` for exactly that reason, so this module is the
// only place the chrome names it.
//
// `{accepted:true}` means **queued**, never applied: the owner tick drains the
// queue through `CadSession::standard_view`, and the real camera transition —
// or its refusal — surfaces later through `cad_status.error`. Nothing here may
// present admission as completion, retry a request, or clamp a view name to a
// neighbouring one.
import { NO_DOCUMENT_KEY } from './commands.ts';
import type { Translator } from './i18n.ts';

/**
 * Tauri IPC arguments, the subset of `@tauri-apps/api`'s `InvokeArgs` this
 * module sends. Declared here so the typed `invoke` stays assignable.
 */
export type InvokeArgs = Record<string, unknown> | Uint8Array | number[] | ArrayBuffer;

/** The typed IPC call this module needs (`invoke` satisfies it). */
export type Invoke = (command: string, args?: InvokeArgs) => Promise<unknown>;

/** The seven standard views' host machine keys (`StandardView::key`). */
export type StandardViewName = 'top' | 'bottom' | 'front' | 'back' | 'left' | 'right' | 'isometric';

/** One standard-view button, as the ribbon model walks it. */
export interface StandardViewEntry {
  /** Chrome-local id (also the icon-set entry name for the seven views). */
  readonly id: string;
  /** The host's machine key, which `cad_standard_view` parses. */
  readonly standardView: StandardViewName;
  /** Catalog key for the button's label. */
  readonly labelKey: string;
  /** Shared inline icon-set entry. */
  readonly icon: string;
}

/**
 * The seven standard views in the catalog's own order.
 *
 * `standardView` is the host's stable machine key (`StandardView::key`), which is
 * what `cad_standard_view` parses — never a label and never a command name. `top` is
 * the only one that leaves 3D: it returns to the 2D plan, while every other view
 * is a framed 3D perspective.
 *
 * The list is data so the ribbon model and the tests walk the same sequence; the
 * `icon` names entries in the shared inline icon set.
 */
export const STANDARD_VIEWS: readonly StandardViewEntry[] = Object.freeze([
  { id: 'viewTop', standardView: 'top', labelKey: 'view.standard.top', icon: 'viewTop' },
  { id: 'viewBottom', standardView: 'bottom', labelKey: 'view.standard.bottom', icon: 'viewBottom' },
  { id: 'viewFront', standardView: 'front', labelKey: 'view.standard.front', icon: 'viewFront' },
  { id: 'viewBack', standardView: 'back', labelKey: 'view.standard.back', icon: 'viewBack' },
  { id: 'viewLeft', standardView: 'left', labelKey: 'view.standard.left', icon: 'viewLeft' },
  { id: 'viewRight', standardView: 'right', labelKey: 'view.standard.right', icon: 'viewRight' },
  { id: 'viewIsometric', standardView: 'isometric', labelKey: 'view.standard.isometric', icon: 'viewIsometric' },
]);

/** The wire names, in the same order (`top` first, `isometric` last). */
export const STANDARD_VIEW_KEYS: readonly StandardViewName[] = Object.freeze(
  STANDARD_VIEWS.map(entry => entry.standardView),
);

/**
 * The semantic pairs among the standard views: each pair looks at the same axis
 * from opposite sides, plus the isometric view as the odd one out.
 *
 * This is data (not a comment) because the ribbon's layout contract is built on
 * it: the seven buttons must place each pair in two cells that share an edge
 * (see `pairsShareEdge` in `ribbon-model.mjs`), and the test walks this list
 * instead of restating the intent.
 */
export const STANDARD_VIEW_PAIRS: readonly (readonly [StandardViewName, StandardViewName])[] = Object.freeze([
  Object.freeze(['top', 'bottom'] as const),
  Object.freeze(['front', 'back'] as const),
  Object.freeze(['left', 'right'] as const),
]);

/** Whether `value` is one of the seven machine keys. */
function isStandardViewName(value: string): value is StandardViewName {
  return (STANDARD_VIEW_KEYS as readonly string[]).includes(value);
}

/**
 * The host's machine key for `view`, or `null` when the name is not one of the
 * seven. Matching is trim + lower-case because the host parses ASCII
 * case-insensitively; anything else is refused locally rather than sent to be
 * refused remotely.
 */
export function standardViewName(view: unknown): StandardViewName | null {
  const name = String(view ?? '').trim().toLowerCase();
  return isStandardViewName(name) ? name : null;
}

/**
 * The status of the shell as the standard-view admission rules read it.
 *
 * Structural and permissive: before the first `cad_status` answer the field may
 * be absent, and an older host may omit `loading`.
 */
export interface StandardViewContext {
  readonly status?: { readonly loading?: unknown } | null;
  readonly documentOpen?: unknown;
}

/**
 * The catalog key for why the standard views cannot run right now.
 *
 * They need a live CAD session with a drawing: framing an empty session in a
 * perspective is meaningless. `plot.no_document` is the catalog's existing
 * wording for "no drawing is open" and is reused here instead of adding a
 * second sentence that says the same thing. Before the first `cad_status`
 * answer (and while an import is loading) the honest answer stays the generic
 * unsupported key, because the host has not told us anything yet.
 */
export function standardViewUnavailableReasonKey({ status = null, documentOpen = false }: StandardViewContext = {}): string {
  if (status && !status.loading && !documentOpen) return NO_DOCUMENT_KEY;
  return 'command.unsupported';
}

/**
 * Whether the chrome may send a standard view at all.
 *
 * Only a loaded status with an open drawing qualifies. A pending command is not
 * a reason to block: a standard view has its own bounded queue and must not
 * wait on an unrelated `cad_command`.
 */
export function standardViewAvailable({ status = null, documentOpen = false }: StandardViewContext = {}): boolean {
  return Boolean(status) && !status?.loading && Boolean(documentOpen);
}

/**
 * The outcome of one standard-view request: admission only, never completion.
 */
export type StandardViewResult =
  | { readonly kind: 'accepted'; readonly view: StandardViewName }
  | { readonly kind: 'refused'; readonly view: string; readonly error: string };

/** Whether an ack is the host's `{ accepted: true }` (queued) answer. */
function ackAccepted(ack: unknown): boolean {
  if (ack === null || typeof ack !== 'object') return false;
  return (ack as { readonly accepted?: unknown }).accepted === true;
}

/**
 * One `cad_standard_view { view }` request, reporting **admission** only.
 *
 * - `{ kind: 'accepted', view }` — queued by the host; the outcome arrives in
 *   `cad_status` (a refusal included).
 * - `{ kind: 'refused', view, error }` — the request never reached the session:
 *   an unknown local name, a host refusal (`view.standard_unknown`,
 *   `view.standard_empty`, a modal popup, a full queue, a disabled session) or a
 *   rejected/`{accepted:false}` answer.
 *
 * An unknown name is refused **before** IPC: the model only ever produces the
 * seven keys, so a bad name is a programming error, not a user-visible camera
 * change, and the host's own refusal stays the authority for everything else.
 */
export async function requestStandardView(invoke: Invoke, view: unknown): Promise<StandardViewResult> {
  const name = standardViewName(view);
  if (!name) {
    return { kind: 'refused', view: String(view ?? ''), error: 'CAD_STANDARD_VIEW_UNKNOWN' };
  }
  try {
    const ack = await invoke('cad_standard_view', { view: name });
    if (!ackAccepted(ack)) throw Error('CAD_STANDARD_VIEW_NOT_ACCEPTED');
    return { kind: 'accepted', view: name };
  } catch (error) {
    return { kind: 'refused', view: name, error: String(error) };
  }
}

/**
 * The banner line for a request that was not admitted, or `''` when it was.
 *
 * A refusal is always named: the host's own reason string is what the user (and
 * the bug report) needs, so it is translated through the existing
 * `command.failed` line rather than replaced by a vaguer message here.
 */
export function standardViewFailure(result: StandardViewResult | null | undefined, t: Translator): string {
  if (result && result.kind === 'accepted') return '';
  // A missing result is a programming error (the caller dropped the answer), not
  // a success: name it instead of quietly showing nothing.
  return t('command.failed', { error: result?.error ?? 'CAD_STANDARD_VIEW_NO_RESULT' });
}

/** What [`submitStandardView`] needs from the chrome. */
export interface StandardViewSubmission {
  readonly invoke: Invoke;
  readonly view: unknown;
  readonly t: Translator;
  /** The error banner; a refusal is reported through it. */
  readonly report?: (message: string) => void;
}

/**
 * Run one request and report its failure through `report` (the error banner).
 *
 * The main window's only entry point for a standard view: the ribbon item and
 * the tests share this function, so neither can invent a different outcome.
 * A successful admission reports nothing — the view is not applied yet, and
 * claiming it here would be exactly the fabricated completion this module
 * avoids.
 */
export async function submitStandardView({ invoke, view, t, report }: StandardViewSubmission): Promise<StandardViewResult> {
  const result = await requestStandardView(invoke, view);
  const failure = standardViewFailure(result, t);
  if (failure && typeof report === 'function') report(failure);
  return result;
}
