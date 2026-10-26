// Text-edit dialog (ADR 0021 D4): the modal editor for one text entity's raw
// DXF string, opened from the properties palette's "edit in window" entry.
//
// The host owns everything that could be guessed wrong:
//
//   * the **content** — `cad_open_popup {kind:'textEdit'}` carries no text; the
//     host reads the current single selection and injects `payload.text`
//     verbatim, or refuses with `popup.no_text_selection` and opens no window;
//   * the **line-break mapping** — `\n` ⇄ `\P` for the multi-line source
//     classes, a named refusal for TEXT. This module never converts anything,
//     in either direction: it edits the raw string the host sent and sends it
//     back;
//   * the **apply** — `cad_set_selected_text {text}`, the same IPC the docked
//     inline field uses. There is no entity id in the payload, so no second,
//     possibly stale, source of truth.
//
// What lives here is the part that must be true without a renderer: the catalog
// registry, the live draft bounds, what a key press means in each mode, and the
// fate of one Apply (close vs. keep the draft and say why).
import { SELECTED_TEXT_MAX_LENGTH, selectedTextOf } from '../tools/textedit.ts';
import type { SelectedTextResult, TextEditKeyAction, TextEditKeyEvent } from '../tools/textedit.ts';
import { TEXT_TOO_LONG_KEY, textLength } from '../tools/text.ts';
import type { Translator } from '../i18n.ts';

/**
 * Every catalog key this dialog and its entry point render.
 *
 * Declared once, as data, for the same reason `POPUP_KEYS` is: the copy lives
 * only in `crates/cad-ui-core/i18n`, and a component addresses a key through
 * this registry instead of repeating a literal.
 *
 * `tooLong`/`empty`/`refused` reuse the existing text-edit channels so the
 * dialog and the docked field cannot word the same refusal differently. `busy`
 * reuses the app's "another operation is in progress" line: this batch adds no
 * busy copy of its own, and inventing words is worse than reusing the one that
 * already says exactly that.
 */
export const POPUP_TEXT_KEYS = Object.freeze({
  title: 'dialog.textedit.title',
  apply: 'dialog.textedit.apply',
  close: 'dialog.textedit.close',
  count: 'dialog.textedit.count',
  hintMultiline: 'dialog.textedit.hint_multiline',
  hintSingle: 'dialog.textedit.hint_single',
  entry: 'properties.edit_in_window',
  noSelection: 'popup.no_text_selection',
  empty: 'properties.text.empty',
  tooLong: TEXT_TOO_LONG_KEY,
  refused: 'properties.text.failed',
  busy: 'linux.busy',
});

/** The host's bound for the edited string — the same bound as the inline field. */
export const POPUP_TEXT_MAX = SELECTED_TEXT_MAX_LENGTH;

/** The live state of the draft (bounds, and the two local refusals). */
export interface TextEditDraftView {
  readonly value: string;
  readonly length: number;
  readonly max: number;
  readonly empty: boolean;
  readonly tooLong: boolean;
  readonly singleLineBreak: boolean;
  readonly canApply: boolean;
}

/** The fate of one Apply, as the view shows it. */
export interface TextEditApplyOutcome {
  readonly done: boolean;
  readonly message: string | null;
}

/** One Apply's result: the host's own answer, or the in-flight guard's `busy`. */
export type TextEditSubmitResult = SelectedTextResult | { readonly kind: 'busy' };

/** The dialog's single-send guard. */
export interface TextEditSubmitter {
  isBusy(): boolean;
  submit(text: string): Promise<TextEditSubmitResult>;
}

/** The popup-open answer this module only needs to read (`kind` + `error`). */
export interface PopupOpenOutcome {
  readonly kind?: string;
  readonly error?: unknown;
}

/**
 * The live state of the draft.
 *
 * `length` counts **Unicode scalar values**, like the host's `chars()` and like
 * the rest of the app: `String.length` would count UTF-16 code units and refuse
 * an astral-heavy draft the host accepts.
 *
 * `singleLineBreak` is the honest local refusal for TEXT: a break can reach the
 * field through paste, and the host refuses it by name (`textedit.newline_text`)
 * because AutoCAD's TEXT does not interpret `\P`. Nothing is stripped or
 * rewritten — the draft stays exactly as the user left it and Apply is off.
 */
export function textEditDraftView(text: unknown, multiline: boolean): TextEditDraftView {
  const value = typeof text === 'string' ? text : '';
  const length = textLength(value);
  const empty = value.trim().length === 0;
  const tooLong = length > POPUP_TEXT_MAX;
  const singleLineBreak = multiline === false && /[\r\n]/.test(value);
  return {
    value,
    length,
    max: POPUP_TEXT_MAX,
    empty,
    tooLong,
    singleLineBreak,
    canApply: !empty && !tooLong && !singleLineBreak,
  };
}

/**
 * What a key press in the field means.
 *
 * - **multiline** (a source class that stores MTEXT markup): plain `Enter` is a
 *   **line break** — the field is the entity's text, so a second line is data,
 *   not a confirmation. `Ctrl+Enter`/`Cmd+Enter` applies. `'none'` lets the
 *   textarea do its own thing (insert the break) without this layer swallowing
 *   the key.
 * - **single-line** (TEXT): `Enter` **applies**, mirroring the command line.
 *   `Shift+Enter` is `'block'`: it inserts nothing, because the host refuses a
 *   newline for TEXT and offering it would only produce a refusal after the
 *   fact. `hint_single` is on screen to say so.
 * - An **IME composition owns every key it reports**, including the Enter that
 *   commits it, and the legacy `keyCode === 229` signal some IMEs send without
 *   `isComposing` — composing therefore never submits, in either mode.
 * - A held `Enter` is one action, not a burst.
 */
export function textEditKeyAction(
  event: TextEditKeyEvent | null | undefined,
  multiline: boolean,
): TextEditKeyAction {
  if (!event || typeof event !== 'object') return 'none';
  if (event.isComposing || event.keyCode === 229) return 'none';
  if (event.key !== 'Enter' || event.repeat) return 'none';
  if (event.ctrlKey || event.metaKey) return 'submit';
  if (multiline === true) return 'none';
  return event.shiftKey || event.altKey ? 'block' : 'submit';
}

/**
 * The local refusal for this draft, or `null` when it can be sent.
 *
 * Empty/whitespace-only and over-long drafts never reach IPC (the host refuses
 * them too, but the copy is right here and the bound is real). A single-line
 * draft that carries a break is explained by the single-line rule itself.
 */
export function textEditLocalRefusal(
  view: TextEditDraftView | null | undefined,
  t: Translator,
): string | null {
  if (!view || view.canApply) return null;
  if (view.empty) return t(POPUP_TEXT_KEYS.empty);
  if (view.tooLong) return t(POPUP_TEXT_KEYS.tooLong, { max: view.max });
  return t(POPUP_TEXT_KEYS.hintSingle);
}

/**
 * The fate of one Apply.
 *
 * `{ done: true }` is the only outcome that closes the window. Everything else
 * keeps the draft **exactly as typed** and returns the line to show: a host
 * refusal (by name, in `error`), a local refusal, or `busy`. Discarding or
 * clearing a draft on refusal would throw away the user's work; this function
 * cannot do that because it never touches the draft.
 */
export function textEditApplyOutcome(
  result: TextEditSubmitResult | null | undefined,
  t: Translator,
): TextEditApplyOutcome {
  if (!result) {
    return { done: false, message: t(POPUP_TEXT_KEYS.refused, { error: 'CAD_SELECTED_TEXT_NO_RESULT' }) };
  }
  if (result.kind === 'accepted') return { done: true, message: null };
  if (result.kind === 'busy') return { done: false, message: t(POPUP_TEXT_KEYS.busy) };
  if (result.kind === 'refused') {
    if (result.reason === 'empty') return { done: false, message: t(POPUP_TEXT_KEYS.empty) };
    if (result.reason === 'too_long') {
      return { done: false, message: t(POPUP_TEXT_KEYS.tooLong, { max: POPUP_TEXT_MAX }) };
    }
  }
  return {
    done: false,
    message: t(POPUP_TEXT_KEYS.refused, { error: String(result.error ?? 'CAD_SELECTED_TEXT_REFUSED') }),
  };
}

/**
 * The one in-flight guard of the dialog's Apply.
 *
 * A second Apply while the first is still queued must **report busy instead of
 * sending twice** — the same single-send rule the command line and the inline
 * field follow. `apply` is injected so the guard is testable without IPC; a
 * rejection or throw is the caller's result to interpret, not a busy state.
 */
export function createTextEditSubmitter(
  apply: (text: string) => Promise<SelectedTextResult>,
): TextEditSubmitter {
  let inFlight = false;
  return {
    isBusy: () => inFlight,
    async submit(text) {
      if (inFlight) return { kind: 'busy' };
      inFlight = true;
      try {
        return await apply(text);
      } finally {
        inFlight = false;
      }
    },
  };
}

/**
 * Whether the palette offers the "edit in window" entry.
 *
 * Only a **non-null string** does: the host derives the payload from the current
 * single selection and refuses anything else with `popup.no_text_selection`
 * (opening no window at all), so an entry without text could only produce a
 * refusal. This never changes the selection to make an edit fit.
 */
export function canOpenTextEditPopup(selection: unknown): boolean {
  return typeof selectedTextOf(selection) === 'string';
}

/**
 * What the main window shows when `cad_open_popup` does not open a window.
 *
 * The host's refusal is named by a catalog key (`popup.no_text_selection`), so
 * it is translated instead of leaking a machine name into the error banner.
 * Any other failure is surfaced verbatim, exactly like the About entry — never a
 * silent no-op and never a fabricated "opened".
 */
export function textEditOpenError(
  result: PopupOpenOutcome | null | undefined,
  t: Translator,
): string {
  if (!result || result.kind === 'accepted') return '';
  const error = String(result.error ?? '');
  if (error.includes(POPUP_TEXT_KEYS.noSelection)) return t(POPUP_TEXT_KEYS.noSelection);
  // `openPopup` always reports a non-empty error, so an empty one can only mean
  // a caller handed over a malformed result: name the outcome instead of showing
  // nothing (never a silent no-op).
  return error || String(result.kind ?? '');
}
