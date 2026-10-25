// The text-entry channel of a text-carrying draw tool (ADR 0019 decision A).
//
// TEXT captures its point first and its text **last**: once the points are
// given, the host reports `tool.awaiting_text` and the next submitted text
// completes the command — there is no second confirm. The text travels on its
// own IPC, `cad_set_tool_text`, which mirrors `cad_set_tool_parameter`: a
// `{accepted:true}` answer is **queue admission**, and a refusal (unknown
// state, empty/whitespace-only, over the length bound) is explicit rather than
// a silent success.
//
// Two rules keep this honest:
//
// - `awaiting_text` is the **single source of truth** for "the tool wants text".
//   Nothing here infers it from the tool key or from the captured point count —
//   the host's own `requires_text()` decides that, and a Rust-side change must
//   not drift from a React-side guess.
// - The UI never rewrites what the user typed: an over-long draft is refused
//   with the bound shown, never truncated, and a refused submit leaves the draft
//   in place so nothing is swallowed.

/** The one IPC that submits the active tool's text. */
export const TOOL_TEXT_COMMAND = 'cad_set_tool_text';

/**
 * The host's length bound, in characters. The host counts `str::chars()` (Unicode
 * scalar values), so the UI counts the same way — `String.length` would count
 * UTF-16 code units and refuse a value the host accepts (or the reverse for
 * astral-plane input).
 */
export const TOOL_TEXT_MAX_LENGTH = 1024;

/** The catalog key naming the text step (the terminal step of the command). */
export const TEXT_STEP_PROMPT_KEY = 'tool.prompt.text.type_text';

/** The catalog key of the preceding pick, for the step chip while it is pending. */
export const TEXT_PICK_PROMPT_KEY = 'tool.prompt.text.pick_point';

/** The catalog key of the length refusal, shown with the real bound. */
export const TEXT_TOO_LONG_KEY = 'status.tool_text.too_long';

/** The catalog key of the short text-mode prompt (the field's placeholder). */
export const TEXT_PROMPT_KEY = 'status.tool_text.prompt';

/** One local refusal reason, with the stable error name the caller reports. */
export type ToolTextRefusal =
  | { readonly kind: 'refused'; readonly reason: 'kind'; readonly error: 'CAD_TOOL_TEXT_KIND' }
  | { readonly kind: 'refused'; readonly reason: 'empty'; readonly error: 'CAD_TOOL_TEXT_EMPTY' }
  | {
    readonly kind: 'refused';
    readonly reason: 'too_long';
    readonly length: number;
    readonly error: 'CAD_TOOL_TEXT_TOO_LONG';
  };

/** The verdict on a draft before it is sent. */
export type ToolTextCheck =
  | ToolTextRefusal
  | { readonly kind: 'ok'; readonly text: string; readonly length: number };

/** The fate of one `cad_set_tool_text` call. */
export type ToolTextSubmitResult =
  | ToolTextRefusal
  | { readonly kind: 'accepted' }
  | { readonly kind: 'failed'; readonly error: string };

/** The live limit state of a draft, for the prompt line's warning. */
export interface TextDraftState {
  readonly length: number;
  readonly max: number;
  readonly tooLong: boolean;
  readonly empty: boolean;
}

/** What Escape means while a text step owns the keyboard. */
export interface TextEscapeInput {
  readonly awaitingText?: boolean;
  readonly focused?: boolean;
}

/** The `invoke` handle this module needs (the Tauri IPC). */
type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value);
}

/**
 * Whether the host reports `tool` as awaiting its text step.
 *
 * Strictly `=== true`: an older host without the field, a `null` tool, or a
 * truthy non-boolean all mean "no text step", so a malformed payload can never
 * open the text prompt. The field is read from the tool block only.
 */
export function toolAwaitingText(tool: unknown): boolean {
  return isRecord(tool) && tool.awaiting_text === true;
}

/** [`toolAwaitingText`] for a whole `cad_status` payload. */
export function awaitingToolText(status: unknown): boolean {
  return toolAwaitingText(isRecord(status) ? status.tool : null);
}

/**
 * The host's prefill for the current text step (`tool.text_prefill`), or `null`.
 *
 * `TEXTEDIT` pre-fills the terminal text step with the selected entity's current
 * string so the user edits instead of retyping; a fresh `TEXT` sends none. Read
 * defensively like [`toolAwaitingText`]: an older host without the field, or a
 * value that is not a string, means "no prefill" — never a fabricated one, and
 * never an excuse to start the step differently.
 */
export function toolTextPrefill(tool: unknown): string | null {
  if (!isRecord(tool)) return null;
  return typeof tool.text_prefill === 'string' ? tool.text_prefill : null;
}

/** The length of `text` in characters (Unicode scalar values), like the host. */
export function textLength(text: unknown): number {
  return typeof text === 'string' ? [...text].length : 0;
}

/**
 * Whether `text` would be admitted by the host, and why not when it would not.
 *
 * `kind: 'ok'` carries the exact string to send (never trimmed or rewritten).
 * Empty/whitespace-only and over-long input are refused here so the obvious
 * cases never reach IPC; the host refuses them too.
 */
export function checkToolText(text: unknown): ToolTextCheck {
  if (typeof text !== 'string') {
    return { kind: 'refused', reason: 'kind', error: 'CAD_TOOL_TEXT_KIND' };
  }
  if (text.trim().length === 0) {
    return { kind: 'refused', reason: 'empty', error: 'CAD_TOOL_TEXT_EMPTY' };
  }
  const length = textLength(text);
  if (length > TOOL_TEXT_MAX_LENGTH) {
    return { kind: 'refused', reason: 'too_long', length, error: 'CAD_TOOL_TEXT_TOO_LONG' };
  }
  return { kind: 'ok', text, length };
}

/** The live limit state of a draft, for the prompt line's warning. */
export function textDraftState(text: unknown): TextDraftState {
  const length = textLength(text);
  return {
    length,
    max: TOOL_TEXT_MAX_LENGTH,
    tooLong: length > TOOL_TEXT_MAX_LENGTH,
    empty: typeof text !== 'string' || text.trim().length === 0,
  };
}

/**
 * Ask the host to submit the active tool's text.
 *
 * Admission semantics match `setToolParameter*`: the result is queue admission,
 * and a refusal is explicit. A value the host would refuse (empty, whitespace,
 * over the bound) is refused before IPC with its `reason`, so the caller can
 * show the real bound instead of a generic failure.
 */
export async function setToolText(
  invoke: Invoke,
  text: unknown,
): Promise<ToolTextSubmitResult> {
  const check = checkToolText(text);
  if (check.kind !== 'ok') return check;
  try {
    const response = await invoke(TOOL_TEXT_COMMAND, { text: check.text });
    const accepted = Boolean(response) && typeof response === 'object'
      && (response as { accepted?: unknown }).accepted === true;
    if (!accepted) throw Error('CAD_TOOL_TEXT_NOT_ACCEPTED');
    return { kind: 'accepted' };
  } catch (error) { return { kind: 'failed', error: String(error) }; }
}

/**
 * What Escape means while a text-carrying tool is active (ADR 0019 A.1).
 *
 * The first Escape only **dismisses text focus**: the tool stays awaiting text,
 * so nothing is cancelled and the pointer lane is handed back. Only the next
 * Escape — with no field focused, which is the global handler's `CANCEL` — ends
 * the tool. A single Escape therefore never cancels.
 */
export function textEscapeAction(
  { awaitingText = false, focused = false }: TextEscapeInput = {},
): 'dismiss' | 'cancel' | null {
  if (!awaitingText) return null;
  return focused ? 'dismiss' : 'cancel';
}
