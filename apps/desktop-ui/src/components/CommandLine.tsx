// Command line, docked bottom-left above the status bar. Autocomplete opens
// inline *below* the input (never over the drawing area), the history panel
// expands in the same row, and Up/Down step through recent commands.
//
// The same field is also the text step of a text-carrying tool (ADR 0019 A.3):
// while the host reports `tool.awaiting_text`, the field becomes a text prompt.
// The text is the tool's **terminal** step, so Enter submits it through
// `cad_set_tool_text` (never a command), Escape's first press only dismisses the
// field, and the field returns to command-name mode the moment the flag clears.
// The mode is read from that flag alone — never inferred from the tool key or
// the captured point count.
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { Icon } from '../icons.tsx';
import { Button, IconButton } from './primitives.tsx';
import { useI18n, useShell } from '../context.ts';
import { handleCommandKey } from '../cad/cad-command.ts';
import { commandLogView } from '../commands/command-log.ts';
import { defaultSuggestionIndex, filterCommandSuggestions, historyText, splitCommandText, stepHistoryCursor, unavailableReasonKey } from '../commands/commands.ts';
import type { CommandAlias, CommandSuggestion } from '../commands/commands.ts';
import type { FocusReleaseRoot } from '../focus-release.ts';
import type { IconName } from '../icons.tsx';
import { releaseFocusOnOutsidePress } from '../focus-release.ts';
import type { ComposingKeyEvent } from './key-events.ts';
import {
  TEXT_PROMPT_KEY,
  TEXT_STEP_PROMPT_KEY,
  TEXT_TOO_LONG_KEY,
  TOOL_TEXT_MAX_LENGTH,
  awaitingToolText,
  textDraftState,
  textEscapeAction,
  toolTextPrefill,
} from '../tools/text.ts';

export function CommandLine() {
  const { t } = useI18n();
  const shell = useShell();
  const inputRef = useRef<HTMLInputElement | null>(null);
  const rootRef = useRef<HTMLDivElement | null>(null);
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
    () => (textMode
      ? []
      : filterCommandSuggestions(shell.command, { history: shell.history, aliases: shell.commandAliases as readonly CommandAlias[] })),
    // `shell.commandAliases` is a dependency because it is the row source (module
    // state loaded once at startup): without it a memo from before the table
    // arrived would hide the shorthand rows.
    [textMode, shell.command, shell.history, shell.commandAliases],
  );
  // The command-window echo: `status.commandLog`, oldest first, newest at the
  // bottom. The host log is append-only (sequences are monotonic, a document
  // replacement clears it, the ring drops from the front at capacity), so the
  // length plus the newest sequence identifies its content exactly. The rows are
  // memoized on that signature: the status is a fresh object every poll (4 Hz),
  // and re-formatting up to 200 entries each time would be wasted work.
  const logEntries = shell.status?.commandLog ?? [];
  const logSignature = logEntries.length > 0
    ? `${logEntries.length}:${logEntries[logEntries.length - 1]?.sequence}`
    : '';
  const logRows = useMemo(() => commandLogView(logEntries, t), [logSignature, t]);
  const logRef = useRef<HTMLDivElement | null>(null);
  // Whether new echoes should keep the view at the newest line. The user owns the
  // position once they scroll up, and it is never yanked back.
  const logStick = useRef(true);
  // Suggestions appear once the user starts typing; with an empty line the
  // Up/Down keys step through command history instead.
  const listOpen = !textMode && focused && shell.command.trim().length > 0 && !shell.historyOpen;

  // A click that returns focus must also act. The browser changes focus as the
  // press's *default action*, after the event has been dispatched — too late for
  // the lane's `pointerdown`, which a still-suspended lane drops. This
  // capture-phase listener blurs the field before the press reaches
  // `.native-input`, so `onBlur` re-enables the lane for that very same event. It
  // never prevents or stops the press, and it exists only while the field is
  // focused; `focused` is the sole dependency, so a re-render never duplicates
  // it and unmount (or blur) removes it.
  useEffect(() => {
    if (!focused) return undefined;
    const input = inputRef.current;
    // The release hook only needs `contains`; its structural root narrows the
    // target to `EventTarget`, which `Element#contains` does not declare.
    return releaseFocusOnOutsidePress(rootRef.current as unknown as FocusReleaseRoot | null, () => input?.blur());
  }, [focused]);

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

  // Follow the newest echo when (and only when) the view is already at the
  // bottom. This runs on the log signature, so a poll that repeats the same log
  // never touches the scroll position.
  useEffect(() => {
    const node = logRef.current;
    if (!node || !logStick.current) return;
    node.scrollTop = node.scrollHeight;
  }, [logSignature]);

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

  const effectiveText = () => {
    // A completion row is a **name**; a typed argument belongs to the line
    // (`ZOOM 2`), so Enter keeps the full text instead of replacing it with the
    // bare suggestion. Tab still replaces the line with the name alone.
    if (shell.command.trim() && splitCommandText(shell.command).argument === null
      && highlight >= 0 && suggestions[highlight]) return suggestions[highlight].id;
    return shell.command;
  };

  function stepSuggestions(delta: number) {
    setHighlight(index => ((index < 0 ? 0 : index) + delta + suggestions.length) % suggestions.length);
  }

  function handleNavKey(event: React.KeyboardEvent<HTMLInputElement>) {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      const delta = event.key === 'ArrowDown' ? 1 : -1;
      if (listOpen && suggestions.length) {
        event.preventDefault();
        stepSuggestions(delta);
        return true;
      }
      const next = stepHistoryCursor(shell.history, cursor, delta);
      if (next !== cursor) {
        event.preventDefault();
        setCursor(next);
        shell.setCommand(historyText(shell.history, next, ''));
      }
      return next !== cursor;
    }
    if (event.key === 'Tab' && listOpen && suggestions[highlight]) {
      event.preventDefault();
      shell.setCommand(suggestions[highlight].id);
      return true;
    }
    return false;
  }

  const draft = textDraftState(textDraft);
  // Over the bound, the limit itself is the only message worth showing; the
  // draft is left exactly as typed (never truncated) and Enter sends nothing.
  const tooLongText = draft.tooLong ? t(TEXT_TOO_LONG_KEY, { max: TOOL_TEXT_MAX_LENGTH }) : null;
  const textMessageText = textMode ? (tooLongText ?? textMessage?.text ?? null) : null;

  function trackLogScroll(event: React.UIEvent<HTMLDivElement>) {
    const node = event.currentTarget;
    logStick.current = node.scrollHeight - node.scrollTop - node.clientHeight <= 4;
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
        setTextMessage({ kind: 'error', text: result.error });
        return;
      }
      setTextMessage({
        kind: 'error',
        text: result.reason === 'too_long'
          ? t(TEXT_TOO_LONG_KEY, { max: TOOL_TEXT_MAX_LENGTH })
          : result.error,
      });
    });
  }

  // The text step never disables the field while a submit is queued: disabling
  // it would drop focus mid-edit. The submit guard handles the double Enter.
  const disabledInput = !shell.cadReady || (shell.pending && !textMode);
  // The option id is a slug: a multi-word alias (`ZOOM EXTENTS`) would otherwise
  // put a space in a DOM id, and `aria-activedescendant` has to name it exactly.
  const optionId = (suggestion: CommandSuggestion) => `completion-${suggestion.id.replace(/\s+/g, '-')}`;
  const activeOptionId = listOpen && suggestions[highlight] ? optionId(suggestions[highlight]) : undefined;

  return (
    <div ref={rootRef} className="region region--cmdline commandline app-region">
      {/* The echo sits above the input row, inside the same region: the region
          grows upward (the input keeps its distance from the status bar) and the
          list is bounded with its own scrollbar, so a long log never pushes the
          ribbon or the drawing area any further. It is output, not a control, so
          the list is deliberately not focusable and offers no clear affordance
          (the host exposes no clear API; the F2 history panel above is a
          different, input-only record). */}
      {logRows.length > 0 ? (
        <div
          ref={logRef}
          className="commandlog"
          role="log"
          aria-label={t('command.log.label')}
          onScroll={trackLogScroll}
        >
          {logRows.map((row, index) => (
            <div
              key={row.sequence ?? `entry-${index}`}
              className={`commandlog__entry is-${row.kind}`}
              title={row.kind === 'error' ? row.code ?? undefined : undefined}
            >
              {row.lines.map((line, lineIndex) => (
                <div key={lineIndex} className={`commandlog__line is-${line.tone}`}>
                  {line.label ? (
                    <span className="commandlog__label">
                      {line.tone === 'command' ? `${line.label} ` : `${line.label} = `}
                    </span>
                  ) : null}
                  <span className="commandlog__text">{line.text}</span>
                  {line.unit ? <span className="commandlog__unit">{` ${line.unit}`}</span> : null}
                  {line.meta ? <span className="commandlog__meta">{line.meta}</span> : null}
                </div>
              ))}
            </div>
          ))}
        </div>
      ) : null}
      <div className="commandline__row">
        <Icon name={textMode ? 'text' : 'terminal'} size={15} className="commandline__prompt" />
        <label className="sr-only" htmlFor="cad-command">{textMode ? t(TEXT_STEP_PROMPT_KEY) : t('command.title')}</label>
        <input
          id="cad-command"
          ref={inputRef}
          className="commandline__input"
          type="text"
          value={textMode ? textDraft : shell.command}
          placeholder={textMode ? t(TEXT_PROMPT_KEY) : t('command.placeholder')}
          autoComplete="off"
          spellCheck={false}
          disabled={disabledInput}
          aria-autocomplete={textMode ? undefined : 'list'}
          aria-expanded={textMode ? undefined : listOpen && suggestions.length > 0}
          aria-activedescendant={textMode ? undefined : activeOptionId}
          aria-invalid={textMode && draft.tooLong ? true : undefined}
          aria-describedby={textMessageText ? 'cad-command-message' : undefined}
          onChange={event => {
            if (textMode) {
              setTextDraft(event.target.value);
              setTextMessage(null);
              return;
            }
            setCursor(null);
            shell.setCommand(event.target.value);
          }}
          onFocus={() => { setFocused(true); shell.suspendPointer(true); }}
          onBlur={() => { setFocused(false); shell.suspendPointer(false); }}
          onKeyDown={(event: ComposingKeyEvent<HTMLInputElement>) => {
            if (!textMode) {
              if (handleNavKey(event)) return;
              handleCommandKey(event, () => shell.submitCommand(effectiveText()));
              return;
            }
            if (event.key === 'Escape') {
              // The IME owns Escape while composing (it cancels the composition).
              // Keep it away from the global handler, which would blur the field
              // in the middle of a composition.
              if (event.isComposing) {
                event.stopPropagation();
                return;
              }
              // One Escape only dismisses this field: the tool stays awaiting
              // text and the pointer lane comes back. The next Escape has no
              // field focused, so the global handler sends CANCEL — a single
              // press never cancels the tool.
              if (textEscapeAction({ awaitingText: textMode, focused: true }) === 'dismiss') {
                event.preventDefault();
                inputRef.current?.blur();
              }
              return;
            }
            // Enter submits the text; an IME-composing Enter is ignored by the
            // shared key rule, so it never commits half a composition.
            handleCommandKey(event, submitText);
          }}
        />
        {textMode ? (
          // The named step, in the shared bounded accent label: it ellipsizes
          // instead of pushing the field, and the field never moves between modes.
          <span className="statusbar__tool-step" title={t(TEXT_STEP_PROMPT_KEY)}>{t(TEXT_STEP_PROMPT_KEY)}</span>
        ) : null}
        {textMode ? null : (
          <IconButton
            icon={shell.historyOpen ? 'chevronDown' : 'chevronUp'}
            size="sm"
            label={shell.historyOpen ? t('command.collapse') : t('command.expand')}
            pressed={shell.historyOpen}
            title={`${t('command.expand')} (F2)`}
            onClick={() => shell.setHistoryOpen(!shell.historyOpen)}
          />
        )}
      </div>
      {textMessageText ? (
        <p id="cad-command-message" className="commandline__message error" role="alert">
          {textMessageText}
        </p>
      ) : null}
      {textMode ? null : shell.commandMessage ? (
        <p className={['commandline__message', shell.commandMessage.kind].join(' ')} role="alert">
          {shell.commandMessage.text}
        </p>
      ) : null}
      {listOpen ? (
        <div className="completions" role="listbox" aria-label={t('command.completions')}>
          <div className="completions__title">{t('command.category.commands')}</div>
          {suggestions.length === 0 ? (
            <div className="completions__empty">{t('command.no_matches')}</div>
          ) : suggestions.map((suggestion, index) => (
            <button
              key={suggestion.id}
              type="button"
              role="option"
              id={optionId(suggestion)}
              aria-selected={index === highlight}
              className={['completion', index === highlight ? 'is-highlight' : '', suggestion.supported ? 'is-supported' : ''].filter(Boolean).join(' ')}
              onMouseEnter={() => setHighlight(index)}
              onMouseDown={event => event.preventDefault()}
              onClick={() => { shell.setCommand(suggestion.id); inputRef.current?.focus(); }}
              title={suggestion.supported ? undefined : t(unavailableReasonKey(suggestion.id), { command: suggestion.id })}
            >
              <Icon name={suggestion.icon as IconName} size={15} />
              <span className="completion__id">{suggestion.id}</span>
              {/* An alias row shows the canonical command it stands for as its
                  secondary text: the shorthand is the entry, the target is the
                  meaning. A canonical row keeps its translated label. */}
              <span className="completion__label">{suggestion.target ?? t(suggestion.labelKey ?? '')}</span>
              {suggestion.supported ? null : <em className="completion__badge">{t('command.badge.unwired')}</em>}
            </button>
          ))}
        </div>
      ) : null}
      {shell.historyOpen ? (
        <div className="history-panel" aria-label={t('command.history')}>
          <div className="history-panel__head">
            <span>{t('command.history')}</span>
            <Button variant="plain" size="xs" icon="clearSelection" onClick={shell.clearHistory} disabled={!shell.history.length}>
              {t('command.clear_history')}
            </Button>
          </div>
          {shell.history.length === 0 ? (
            <div className="history-panel__empty">{t('command.history_empty')}</div>
          ) : (
            <ol className="history-panel__list">
              {[...shell.history].reverse().map((entry, index) => (
                <li key={`${entry}-${index}`}>
                  <button type="button" className="history-panel__entry" onClick={() => shell.setCommand(entry)}>
                    {entry}
                  </button>
                </li>
              ))}
            </ol>
          )}
        </div>
      ) : null}
    </div>
  );
}
