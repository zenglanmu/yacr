// Editing the text of the single selected text entity (ADR 0020 addendum B).
//
// Where the editor lives, and why: the CAD rectangle is a **native,
// input-transparent** surface — the webview sits below it and DOM cannot be
// painted over that rect (ADR 0010; the geometry is in ADR 0020). The
// properties palette is **chrome**, outside the rectangle, so an editable field
// there is allowed. That is the whole point of this module, and the reason
// there is deliberately no in-canvas editor anywhere in the tree.
//
// The host owns the selection. `cad_set_selected_text` carries **no entity id**:
// the host resolves and validates "the current single selection" itself, and a
// refusal arrives the same way the tool-text channel's refusals do. The
// round-trippable current string arrives as `selection.text` — it is *not* one
// of the display `PropertyRow`s, so nothing here reuses those strings (their
// values are formatted for display, not for round-tripping).
//
// Three rules keep this honest:
//
// - **verbatim.** The field shows the host's string exactly as it was sent — no
//   trim, no normalisation, no MTEXT markup rendering — and returns exactly what
//   the user leaves in it. A single-line `input` strips line breaks from its
//   value, so the editor's field is a `textarea`.
// - **no silent success.** Empty/whitespace-only and over-long drafts are
//   refused with the same messages the text step uses, a host refusal is
//   surfaced, and a refused draft is never truncated or cleared.
// - **no silent draft loss.** A selection change under focus keeps the draft and
//   says why Apply is disabled (see [`textEditHostChanged`]); the pointer lane
//   and the selection block are never touched from here.
import { checkToolText, textLength, TOOL_TEXT_MAX_LENGTH } from './text.ts';
import type { ToolTextCheck, ToolTextRefusal } from './text.ts';

/** The one IPC that edits the current single selection's text. */
export const SELECTED_TEXT_COMMAND = 'cad_set_selected_text';

/** The host's bound for the edited string — the same bound as the text step. */
export const SELECTED_TEXT_MAX_LENGTH = TOOL_TEXT_MAX_LENGTH;

/**
 * The catalog keys of the panel editor.
 *
 * Exported as constants (the same convention the text step's keys follow) so the
 * copy lives only in the canonical catalog and never in JSX.
 */
export const TEXT_EDIT_KEYS = Object.freeze({
  field: 'properties.text.field',
  apply: 'properties.text.apply',
  count: 'properties.text.count',
  empty: 'properties.text.empty',
  detached: 'properties.text.detached',
  failed: 'properties.text.failed',
});

/** The editor's own state: the host baseline, the draft, and detach. */
export interface TextEditState {
  readonly base: string | null;
  readonly draft: string;
  readonly detached: boolean;
}

/** The editor with no host text and no draft. */
export const TEXT_EDIT_INITIAL: TextEditState = Object.freeze({
  base: null,
  draft: '',
  detached: false,
});

/** The rendered state of the editor. */
export interface TextEditView {
  readonly visible: boolean;
  readonly value: string;
  readonly length: number;
  readonly max: number;
  readonly tooLong: boolean;
  readonly empty: boolean;
  readonly detached: boolean;
  readonly canApply: boolean;
}

/** What a key press in a text field means. */
export type TextEditKeyAction = 'none' | 'submit' | 'block';

/**
 * The key facts a text field reports. Structural, so a real `KeyboardEvent`, a
 * React synthetic keyboard event and the plain objects a test passes all fit.
 */
export interface TextEditKeyEvent {
  readonly isComposing?: boolean;
  readonly keyCode?: number;
  readonly key?: string;
  readonly repeat?: boolean;
  readonly ctrlKey?: boolean;
  readonly metaKey?: boolean;
  readonly shiftKey?: boolean;
  readonly altKey?: boolean;
}

/** The fate of one `cad_set_selected_text` call. */
export type SelectedTextResult =
  | ToolTextRefusal
  | { readonly kind: 'accepted' }
  | { readonly kind: 'failed'; readonly error: string };

/** The `invoke` handle this module needs (the Tauri IPC). */
type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value);
}

/**
 * The round-trippable text of the current selection, or `null` when there is
 * nothing to edit.
 *
 * Only a **single** selected object with a real string qualifies; a
 * multi-selection, an empty one, a non-text object and an older host that omits
 * the field all read as `null`. The reader never throws: the field is optional
 * in the wire contract, and a malformed value must render as "not editable"
 * rather than as a guessed string or a blanked status block.
 */
export function selectedTextOf(selection: unknown): string | null {
  if (!isRecord(selection)) return null;
  if (selection.count !== 1) return null;
  return typeof selection.text === 'string' ? selection.text : null;
}

/**
 * The source class's line-break policy, as the host reports it — or `null` when
 * the host does not tell us.
 *
 * `CadSelection.text_multiline` (wire `text_multiline`) is the one honest source
 * for this: `true` for the multi-line source classes (MTEXT markup and leader
 * text), `false` for TEXT, which the host refuses a newline for by name. The
 * mode is **never** inferred from a display row's `type`: those values are
 * formatted for reading, and behaviour must not hang off display copy. The
 * reader is deliberately tolerant in one direction only — a missing or
 * non-boolean field is `null` ("unknown"), never a guess, so the field can keep
 * its previous behaviour on a build that predates the field instead of
 * inventing a mode.
 *
 * Only a single selected object with a real string has an editable mode; a
 * mode without text would describe something the panel does not edit.
 */
export function selectedTextMultilineOf(selection: unknown): boolean | null {
  if (!isRecord(selection)) return null;
  if (selection.count !== 1) return null;
  if (typeof selection.text !== 'string') return null;
  return typeof selection.text_multiline === 'boolean' ? selection.text_multiline : null;
}

/**
 * Whether `text` would be admitted by the host, and why not when it would not.
 *
 * Identical rules to the text step ([`checkToolText`]): the host counts Unicode
 * scalar values (`str::chars()`), so the UI counts the same way, and the obvious
 * refusals (empty/whitespace-only, over the bound) never reach IPC. The host
 * refuses them too; this only saves a round trip and keeps the copy identical.
 */
export function checkSelectedText(text: unknown): ToolTextCheck {
  return checkToolText(text);
}

/**
 * Ask the host to apply `text` to the **current single selection**.
 *
 * The payload is `{ text }` and nothing else: there is no entity id, because the
 * host resolves and validates the selection itself — sending an id from the UI
 * would be a second, possibly stale, source of truth. `{kind:'accepted'}` is
 * queue admission, never completion: the drawing result (and any host-side
 * refusal) surfaces through the next `cad_status`.
 */
export async function setSelectedText(
  invoke: Invoke,
  text: unknown,
): Promise<SelectedTextResult> {
  const check = checkSelectedText(text);
  if (check.kind !== 'ok') return check;
  try {
    const response = await invoke(SELECTED_TEXT_COMMAND, { text: check.text });
    const accepted = Boolean(response) && typeof response === 'object'
      && (response as { accepted?: unknown }).accepted === true;
    if (!accepted) throw Error('CAD_SELECTED_TEXT_NOT_ACCEPTED');
    return { kind: 'accepted' };
  } catch (error) { return { kind: 'failed', error: String(error) }; }
}

/**
 * Fold a new host `selection.text` into the editor state.
 *
 * - The same text is still selected (or still nothing): only the `detached` flag
 *   can clear. Re-selecting the object a parked draft was typed for therefore
 *   re-attaches it, which is the natural recovery in a CAD session.
 * - An **untouched** field follows the host, including to "no editable text".
 * - An **edited** draft is kept verbatim and marked `detached` when the host's
 *   text is no longer the one it was seeded from (deselected, multi-selected, or
 *   another entity). Apply is then disabled and the UI says why — the one thing
 *   this must never do is silently discard what the user typed, or apply it to a
 *   different object behind their back.
 */
export function textEditHostChanged(
  state: TextEditState | null | undefined,
  hostText: unknown,
): TextEditState {
  const current = state ?? TEXT_EDIT_INITIAL;
  const draft = typeof current.draft === 'string' ? current.draft : '';
  const base = typeof current.base === 'string' ? current.base : null;
  const host = typeof hostText === 'string' ? hostText : null;
  if (host === base) {
    return current.detached === true ? { base, draft, detached: false } : current;
  }
  if (draft === (base ?? '')) return { base: host, draft: host ?? '', detached: false };
  return { base, draft, detached: true };
}

/** Record what the user typed in the field (never trimmed or rewritten). */
export function textEditDraftChanged(
  state: TextEditState | null | undefined,
  next: unknown,
): TextEditState {
  const current = state ?? TEXT_EDIT_INITIAL;
  const draft = typeof next === 'string' ? next : '';
  if (draft === current.draft) return current;
  return { ...current, draft };
}

/**
 * Record an accepted edit: the sent string becomes the new baseline, so the
 * host's echo of it is a no-op instead of looking like a selection change (which
 * would park the draft). Anything typed while the edit was in flight is kept.
 */
export function textEditApplied(
  state: TextEditState | null | undefined,
  sentText: unknown,
): TextEditState {
  const current = state ?? TEXT_EDIT_INITIAL;
  if (typeof sentText !== 'string') return current;
  return { base: sentText, draft: current.draft, detached: false };
}

/**
 * The rendered state of the editor.
 *
 * `visible` is false only when there is nothing to show at all (no host text and
 * no parked draft), so a draft is never hidden by a selection change.
 * `canApply` is false while the draft is detached (a different or no selection)
 * or over the bound; an empty draft stays **clickable** so the refusal is
 * surfaced instead of leaving a mysteriously disabled button.
 */
export function textEditView(state: TextEditState | null | undefined): TextEditView {
  const current = state ?? TEXT_EDIT_INITIAL;
  const draft = typeof current.draft === 'string' ? current.draft : '';
  const base = typeof current.base === 'string' ? current.base : null;
  const detached = current.detached === true;
  const length = textLength(draft);
  return {
    visible: base !== null || detached,
    value: draft,
    length,
    max: SELECTED_TEXT_MAX_LENGTH,
    tooLong: length > SELECTED_TEXT_MAX_LENGTH,
    empty: draft.trim().length === 0,
    detached,
    canApply: base !== null && !detached && length <= SELECTED_TEXT_MAX_LENGTH,
  };
}

/**
 * What a key press in the editor means, per line-break mode.
 *
 * The map is the text-edit popup's (`popup-textedit.ts`), applied to the docked
 * field so the two editors cannot promise different keys for the same entity:
 *
 * - **single-line** (`text_multiline === false`, TEXT): `Enter` **applies**,
 *   mirroring the command line and the popup. `Shift+Enter`/`Alt+Enter` are
 *   `'block'` — they insert nothing and send nothing, because the host refuses a
 *   newline for TEXT by name (`textedit.newline_text`) and offering the break
 *   here would only earn that refusal after the fact.
 * - **multi-line** (`text_multiline === true`, MTEXT and leader text): unchanged —
 *   `Enter` stays a line break (`'none'` lets the textarea insert it) and the
 *   submit is the explicit `Ctrl+Enter`/`Cmd+Enter`.
 * - **unknown** (`null`/absent, an older host that does not send the field):
 *   the pre-`text_multiline` behaviour, kept deliberately — `Enter` is a line
 *   break and `Ctrl+Enter` applies. Guessing a mode here could either swallow a
 *   legitimate line break or send half an edit, so the field waits for the host
 *   to say which it is.
 * - An IME composition owns every key it reports — including the Enter that ends
 *   a composition, and the legacy `keyCode === 229` signal some IMEs send
 *   without `isComposing` — so composing never submits, in any mode.
 * - A held `Enter` is one action, not a burst (`repeat` is ignored).
 */
export function textEditKeyAction(
  event: TextEditKeyEvent | null | undefined = {},
  multiline: boolean | null = null,
): TextEditKeyAction {
  if (!event || typeof event !== 'object') return 'none';
  if (event.isComposing || event.keyCode === 229) return 'none';
  if (event.key !== 'Enter' || event.repeat) return 'none';
  if (event.ctrlKey || event.metaKey) return 'submit';
  if (multiline === true) return 'none';
  if (multiline === false) return event.shiftKey || event.altKey ? 'block' : 'submit';
  // Unknown mode: the behaviour before `text_multiline` existed (line break,
  // explicit Ctrl/Cmd+Enter to apply).
  return 'none';
}
