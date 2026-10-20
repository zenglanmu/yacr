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
import { Icon } from '../icons.jsx';
import { Button, IconButton } from './primitives.jsx';
import { useI18n, useShell } from '../context.mjs';
import { handleCommandKey } from '../cad.mjs';
import { defaultSuggestionIndex, filterCommandSuggestions, historyText, stepHistoryCursor, unavailableReasonKey } from '../commands.mjs';
import {
  TEXT_PROMPT_KEY,
  TEXT_STEP_PROMPT_KEY,
  TEXT_TOO_LONG_KEY,
  TOOL_TEXT_MAX_LENGTH,
  awaitingToolText,
  textDraftState,
  textEscapeAction,
} from '../text.mjs';

export function CommandLine() {
  const { t } = useI18n();
  const shell = useShell();
  const inputRef = useRef(null);
  const [focused, setFocused] = useState(false);
  const [highlight, setHighlight] = useState(-1);
  const [cursor, setCursor] = useState(null);
  // The text step's own draft and its last outcome message. The draft is never
  // mirrored into `shell.command`: the command line keeps its content and gets
  // it back once the tool ends.
  const [textDraft, setTextDraft] = useState('');
  const [textMessage, setTextMessage] = useState(null);
  const tool = shell.status?.tool ?? null;
  const toolKey = typeof tool?.key === 'string' ? tool.key : null;
  const textMode = awaitingToolText(shell.status);

  const suggestions = useMemo(
    () => (textMode ? [] : filterCommandSuggestions(shell.command, { history: shell.history })),
    [textMode, shell.command, shell.history],
  );
  // Suggestions appear once the user starts typing; with an empty line the
  // Up/Down keys step through command history instead.
  const listOpen = !textMode && focused && shell.command.trim().length > 0 && !shell.historyOpen;

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

  // A new text step starts empty and focuses the field so typing can begin
  // immediately. Escape's first press gives the keyboard back and is never
  // re-grabbed while the same tool still awaits its text.
  useEffect(() => {
    if (!textMode) return;
    setTextDraft('');
    setTextMessage(null);
    inputRef.current?.focus();
  }, [textMode, toolKey]);

  const effectiveText = () => {
    if (highlight >= 0 && suggestions[highlight] && shell.command.trim()) return suggestions[highlight].id;
    return shell.command;
  };

  function stepSuggestions(delta) {
    setHighlight(index => ((index < 0 ? 0 : index) + delta + suggestions.length) % suggestions.length);
  }

  function handleNavKey(event) {
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
  const activeOptionId = listOpen && suggestions[highlight] ? `completion-${suggestions[highlight].id}` : undefined;

  return (
    <div className="region region--cmdline commandline app-region">
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
          onKeyDown={event => {
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
              id={`completion-${suggestion.id}`}
              aria-selected={index === highlight}
              className={['completion', index === highlight ? 'is-highlight' : '', suggestion.supported ? 'is-supported' : ''].filter(Boolean).join(' ')}
              onMouseEnter={() => setHighlight(index)}
              onMouseDown={event => event.preventDefault()}
              onClick={() => { shell.setCommand(suggestion.id); inputRef.current?.focus(); }}
              title={suggestion.supported ? undefined : t(unavailableReasonKey(suggestion.id), { command: suggestion.id })}
            >
              <Icon name={suggestion.icon} size={15} />
              <span className="completion__id">{suggestion.id}</span>
              <span className="completion__label">{t(suggestion.labelKey)}</span>
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
