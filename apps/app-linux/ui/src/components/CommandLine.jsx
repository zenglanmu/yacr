// Command line, docked bottom-left above the status bar. Autocomplete opens
// inline *below* the input (never over the drawing area), the history panel
// expands in the same row, and Up/Down step through recent commands.
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { Icon } from '../icons.jsx';
import { Button, IconButton } from './primitives.jsx';
import { useI18n, useShell } from '../context.mjs';
import { handleCommandKey } from '../cad.mjs';
import { defaultSuggestionIndex, filterCommandSuggestions, historyText, stepHistoryCursor } from '../commands.mjs';

export function CommandLine() {
  const { t } = useI18n();
  const shell = useShell();
  const inputRef = useRef(null);
  const [focused, setFocused] = useState(false);
  const [highlight, setHighlight] = useState(-1);
  const [cursor, setCursor] = useState(null);

  const suggestions = useMemo(
    () => filterCommandSuggestions(shell.command, { history: shell.history }),
    [shell.command, shell.history],
  );
  // Suggestions appear once the user starts typing; with an empty line the
  // Up/Down keys step through command history instead.
  const listOpen = focused && shell.command.trim().length > 0 && !shell.historyOpen;

  useEffect(() => {
    setHighlight(defaultSuggestionIndex(suggestions, shell.command));
  }, [shell.command, suggestions]);

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

  const disabledInput = !shell.cadReady || shell.pending;
  const activeOptionId = listOpen && suggestions[highlight] ? `completion-${suggestions[highlight].id}` : undefined;

  return (
    <div className="region region--cmdline commandline app-region">
      <div className="commandline__row">
        <Icon name="terminal" size={15} className="commandline__prompt" />
        <label className="sr-only" htmlFor="cad-command">{t('command.title')}</label>
        <input
          id="cad-command"
          ref={inputRef}
          className="commandline__input"
          type="text"
          value={shell.command}
          placeholder={t('command.placeholder')}
          autoComplete="off"
          spellCheck={false}
          disabled={disabledInput}
          aria-autocomplete="list"
          aria-expanded={listOpen && suggestions.length > 0}
          aria-activedescendant={activeOptionId}
          onChange={event => { setCursor(null); shell.setCommand(event.target.value); }}
          onFocus={() => { setFocused(true); shell.suspendPointer(true); }}
          onBlur={() => { setFocused(false); shell.suspendPointer(false); }}
          onKeyDown={event => {
            if (handleNavKey(event)) return;
            handleCommandKey(event, () => shell.submitCommand(effectiveText()));
          }}
        />
        <IconButton
          icon={shell.historyOpen ? 'chevronDown' : 'chevronUp'}
          size="sm"
          label={shell.historyOpen ? t('command.collapse') : t('command.expand')}
          pressed={shell.historyOpen}
          title={`${t('command.expand')} (F2)`}
          onClick={() => shell.setHistoryOpen(!shell.historyOpen)}
        />
      </div>
      {shell.commandMessage ? (
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
              title={suggestion.supported ? undefined : t('command.unsupported', { command: suggestion.id })}
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
