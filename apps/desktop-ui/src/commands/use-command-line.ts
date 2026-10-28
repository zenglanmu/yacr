// The command line's controller: the React state and effects half.
//
// `command-line-model.ts` owns every rule; this hook owns only the state that a
// render actually needs (the focused flag, the highlighted row, the history
// cursor, the text step's draft and its last refusal) plus the effects that keep
// those in step with the host. Both command-line presentations render from the
// returned controller:
//
//   * `components/CommandLine.tsx` — the desktop region, with its command-log
//     echo, history panel and inline completions;
//   * `components/MobileCommandBar.tsx` — the mobile bottom bar, with the same
//     field and the same sheets opening upward instead of inline.
//
// Nothing here draws: the 44px row, the sheet position and the label sizes are
// the presentations' business. Nothing here is duplicated either — a change to
// Enter, Up/Down, Tab, Escape or the text step lands in both bars at once.
//
// The text step (ADR 0019 A.3) is the one mode with its own draft: while the host
// reports `tool.awaiting_text` the field becomes a text prompt, Enter submits it
// through `cad_set_tool_text` (never a command), Escape's first press only
// dismisses the field, and the field returns to command-name mode the moment the
// flag clears. The mode is read from that flag alone — never inferred from the
// tool key or the captured point count.
import { useEffect, useMemo, useRef, useState } from 'react';
import type { RefObject } from 'react';
import { useI18n, useShell } from '../context.ts';
import { handleCommandKey } from '../cad/cad-command.ts';
import { defaultSuggestionIndex } from './commands.ts';
import type { CommandAlias, CommandSuggestion } from './commands.ts';
import type { ComposingKeyEvent } from '../components/key-events.ts';
import {
  TEXT_STEP_PROMPT_KEY,
  TEXT_TOO_LONG_KEY,
  TOOL_TEXT_MAX_LENGTH,
  awaitingToolText,
  textDraftState,
  textEscapeAction,
  toolTextPrefill,
} from '../tools/text.ts';
import {
  commandLineCanSubmit,
  commandLineDisabled,
  commandLineHistoryStep,
  commandLineListOpen,
  commandLineNavIntent,
  commandLineOptionId,
  commandLinePlaceholderKey,
  commandLineSubmitText,
  commandLineSuggestions,
  commandLineValue,
} from './command-line-model.ts';

/** Everything a command-line presentation renders from. */
export interface CommandLineController {
  /** Whether the field is the active tool's terminal text step. */
  readonly textMode: boolean;
  /** The text step's name (`TEXT_STEP_PROMPT_KEY`), or `null` outside it. */
  readonly textStepKey: string | null;
  /** The text in the field. */
  readonly value: string;
  /** The placeholder, already translated. */
  readonly placeholder: string;
  /** Whether the field is disabled (a queued command, or no session yet). */
  readonly disabled: boolean;
  /** Whether the field has something worth sending. */
  readonly canSubmit: boolean;
  /** `aria-invalid` for an over-long text step. */
  readonly invalid: boolean;
  /** `aria-autocomplete` (`'list'` while command names are suggested). */
  readonly autoComplete: 'list' | undefined;
  /** `aria-expanded` for the suggestion list. */
  readonly expanded: boolean | undefined;
  /** `aria-activedescendant` for the highlighted row. */
  readonly activeOptionId: string | undefined;
  /** `aria-describedby` for the text step's own message, when one is showing. */
  readonly describedBy: string | undefined;
  /** The story the text step reports (over-long draft or a refusal), or `null`. */
  readonly messageText: string | null;
  /** Whether the field currently owns the keyboard. */
  readonly focused: boolean;
  /** Whether the suggestion list is offered. */
  readonly listOpen: boolean;
  /** The rows, newest relevance first. */
  readonly suggestions: readonly CommandSuggestion[];
  /** The highlighted row index (`-1` before the first render). */
  readonly highlight: number;
  /** Point at a row (pointer hover, or the arrow keys' own step). */
  readonly setHighlight: (index: number) => void;
  /** The DOM id of one row. */
  readonly optionId: (suggestion: CommandSuggestion) => string;
  /** The `<input>` node, so a presentation can focus or blur it. */
  readonly inputRef: RefObject<HTMLInputElement | null>;
  readonly onFocus: () => void;
  readonly onBlur: () => void;
  readonly onChange: (next: string) => void;
  readonly onKeyDown: (event: ComposingKeyEvent<HTMLInputElement>) => void;
  /** The Enter action: submit the text step, or the admitted command line. */
  readonly submit: () => void;
  /** Fill the line with `id` and hand the keyboard back to the field. */
  readonly choose: (id: string) => void;
}

export function useCommandLine(): CommandLineController {
  const { t } = useI18n();
  const shell = useShell();
  const inputRef = useRef<HTMLInputElement | null>(null);
  const [focused, setFocused] = useState(false);
  const [highlight, setHighlight] = useState(-1);
  const [cursor, setCursor] = useState<number | null>(null);
  // The text step's own draft and its last outcome message. The draft is never
  // mirrored into `shell.command`: the command line keeps its content and gets
  // it back once the tool ends.
  const [textDraft, setTextDraft] = useState('');
  const [textMessage, setTextMessage] = useState<{ readonly kind: string; readonly text?: string } | null>(null);
  const tool = shell.status?.tool ?? null;
  const toolKey = typeof tool?.key === 'string' ? tool.key : null;
  const textMode = awaitingToolText(shell.status);
  // The host's prefill for this text step: `TEXTEDIT` sends the selected text so
  // the user edits it instead of retyping; a fresh `TEXT` sends nothing.
  const prefill = textMode ? toolTextPrefill(tool) : null;

  const suggestions = useMemo(
    () => commandLineSuggestions({
      textMode,
      command: shell.command,
      history: shell.history,
      aliases: shell.commandAliases as readonly CommandAlias[],
    }),
    // `shell.commandAliases` is a dependency because it is the row source (module
    // state loaded once at startup): without it a memo from before the table
    // arrived would hide the shorthand rows.
    [textMode, shell.command, shell.history, shell.commandAliases],
  );
  const listOpen = commandLineListOpen({
    textMode,
    focused,
    command: shell.command,
    historyOpen: shell.historyOpen,
  });

  useEffect(() => {
    if (textMode) return;
    setHighlight(defaultSuggestionIndex(suggestions, shell.command));
  }, [textMode, shell.command, suggestions]);

  // Leaving the text step (the command completed, or the tool was cancelled)
  // drops the draft, so the field is a command line again — never a stale prompt.
  useEffect(() => {
    if (textMode) return;
    setTextDraft('');
    setTextMessage(null);
  }, [textMode]);

  // A new text step starts with the host's prefill — `TEXTEDIT` puts the current
  // string in the field so the user edits, a fresh `TEXT` starts empty — and
  // focuses the field so typing can begin immediately. Escape's first press gives
  // the keyboard back and is never re-grabbed while the same tool still awaits
  // its text.
  useEffect(() => {
    if (!textMode) return;
    setTextDraft(toolTextPrefill(tool) ?? '');
    setTextMessage(null);
    inputRef.current?.focus();
    // Read on the step transition only: a later poll never re-seeds, so it can
    // never overwrite what the user has typed.
  }, [textMode, toolKey]);

  // A prefill that lands after the step opened (a separate status payload) fills
  // the field only while it is still empty. It is idempotent and one-way: an
  // empty draft is exactly the state in which nothing can be lost.
  useEffect(() => {
    if (!textMode || !prefill) return;
    setTextDraft(current => (current === '' ? prefill : current));
  }, [textMode, prefill]);

  const draft = textDraftState(textDraft);
  // Over the bound, the limit itself is the only message worth showing; the
  // draft is left exactly as typed (never truncated) and Enter sends nothing.
  const tooLongText = draft.tooLong ? t(TEXT_TOO_LONG_KEY, { max: TOOL_TEXT_MAX_LENGTH }) : null;
  const messageText = textMode ? (tooLongText ?? textMessage?.text ?? null) : null;

  function stepSuggestions(delta: number) {
    setHighlight(index => ((index < 0 ? 0 : index) + delta + suggestions.length) % suggestions.length);
  }

  // One sentence, two readers: the line under the field and the history log get
  // the *same* string from this one call, so the record of a refusal can never
  // word it differently from the line that was on screen. The live line is left
  // exactly as it was — the log is the record, not a replacement.
  function refuse(text: string) {
    setTextMessage({ kind: 'error', text });
    shell.recordHistoryError(text);
  }

  function submitText() {
    if (draft.empty || draft.tooLong) return;
    void shell.submitToolText(textDraft).then(result => {
      if (result.kind === 'accepted') {
        setTextDraft('');
        setTextMessage(null);
        return;
      }
      // The first submit still owns the IPC; a re-entrant Enter is not an answer.
      if (result.kind === 'busy') return;
      // A host-side failure carries only its error text; a local refusal names
      // the bound it broke (see `setToolText`).
      if (result.kind === 'failed') {
        refuse(result.error);
        return;
      }
      refuse(result.reason === 'too_long'
        ? t(TEXT_TOO_LONG_KEY, { max: TOOL_TEXT_MAX_LENGTH })
        : result.error);
    });
  }

  function submit() {
    if (textMode) {
      submitText();
      return;
    }
    void shell.submitCommand(commandLineSubmitText({ command: shell.command, highlight, suggestions }));
  }

  function onKeyDown(event: ComposingKeyEvent<HTMLInputElement>) {
    if (!textMode) {
      const intent = commandLineNavIntent({
        key: event.key,
        listOpen,
        suggestionCount: suggestions.length,
        highlight,
      });
      if (intent?.kind === 'suggestion') {
        event.preventDefault();
        stepSuggestions(intent.delta);
        return;
      }
      if (intent?.kind === 'accept') {
        event.preventDefault();
        shell.setCommand(suggestions[highlight].id);
        return;
      }
      if (intent?.kind === 'history') {
        const step = commandLineHistoryStep({ history: shell.history, cursor, delta: intent.delta, draft: '' });
        if (step.changed) {
          event.preventDefault();
          setCursor(step.cursor);
          shell.setCommand(step.text);
        }
        return;
      }
      // Enter submits the line; an IME-composing or held Enter is ignored by the
      // shared key rule, so it never commits half a composition.
      handleCommandKey(event, submit);
      return;
    }
    if (event.key === 'Escape') {
      // The IME owns Escape while composing (it cancels the composition). Keep it
      // away from the global handler, which would blur the field in the middle of
      // a composition.
      if (event.isComposing) {
        event.stopPropagation();
        return;
      }
      // One Escape only dismisses this field: the tool stays awaiting text and the
      // pointer lane comes back. The next Escape has no field focused, so the
      // global handler sends CANCEL — a single press never cancels the tool.
      if (textEscapeAction({ awaitingText: textMode, focused: true }) === 'dismiss') {
        event.preventDefault();
        inputRef.current?.blur();
      }
      return;
    }
    // Enter submits the text; an IME-composing Enter is ignored by the shared key
    // rule, so it never commits half a composition.
    handleCommandKey(event, submit);
  }

  const activeOptionId = listOpen && suggestions[highlight] ? commandLineOptionId(suggestions[highlight]) : undefined;

  return {
    textMode,
    textStepKey: textMode ? TEXT_STEP_PROMPT_KEY : null,
    value: commandLineValue(textMode, shell.command, textDraft),
    placeholder: t(commandLinePlaceholderKey(textMode)),
    disabled: commandLineDisabled({ cadReady: shell.cadReady, pending: shell.pending, textMode }),
    canSubmit: commandLineCanSubmit(textMode, shell.command, draft.empty, draft.tooLong),
    invalid: textMode && draft.tooLong,
    autoComplete: textMode ? undefined : 'list',
    expanded: textMode ? undefined : listOpen && suggestions.length > 0,
    activeOptionId,
    describedBy: messageText ? 'cad-command-message' : undefined,
    messageText,
    focused,
    listOpen,
    suggestions,
    highlight,
    setHighlight,
    optionId: commandLineOptionId,
    inputRef,
    // Focus is also the pointer lane's suspension reason: typing must never leave
    // a pressed pointer behind on the drawing surface.
    onFocus: () => { setFocused(true); shell.suspendPointer(true); },
    onBlur: () => { setFocused(false); shell.suspendPointer(false); },
    onChange: (next: string) => {
      if (textMode) {
        setTextDraft(next);
        setTextMessage(null);
        return;
      }
      setCursor(null);
      shell.setCommand(next);
    },
    onKeyDown,
    submit,
    choose: (id: string) => { shell.setCommand(id); inputRef.current?.focus(); },
  };
}
