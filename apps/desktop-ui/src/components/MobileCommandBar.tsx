// The mobile command bar: the single bottom component.
//
// It replaces the desktop command line **and** the status bar. The bar holds the
// same field (through the shared controller and `CommandField`), a send button
// for the on-screen keyboard's own Enter-less habits, and the suggestion/history
// lists — which open upward as one bounded sheet so the drawing above stays
// visible.
//
// Why a sheet and not the desktop's inline lists: the bar is the last thing on
// the screen, so an inline list would have nowhere to go but over the drawing,
// and the drawing is the point. The sheet is capped at 45dvh (`mobile.css`) and
// the history is capped at `MOBILE_HISTORY_ROWS` entries, so it can never grow
// into a second screen; it scrolls inside itself. The desktop panel opens upward
// for the same reason, which is why both presentations pop over the drawing and
// neither reflows the shell.
//
// **No refusal line under the field.** A refused command is reported once, in the
// history (the red row of the same sheet, above this bar). One surface, one
// sentence. The host's own error banner below is a different thing and stays: it
// reports host failures (a failed canvas module, a failed poll), not a refusal
// the command line produced.
import React, { useEffect, useRef } from 'react';
import { Icon } from '../icons.tsx';
import type { IconName } from '../icons.tsx';
import { IconButton } from './primitives.tsx';
import { CommandField } from './CommandField.tsx';
import { useI18n, useShell } from '../context.ts';
import { useCommandLine } from '../commands/use-command-line.ts';
import { mobileHistoryRows } from '../commands/command-line-model.ts';
import { unavailableReasonKey } from '../commands/commands.ts';
import { releaseFocusOnOutsidePress } from '../focus-release.ts';
import type { FocusReleaseRoot } from '../focus-release.ts';

export function MobileCommandBar() {
  const { t } = useI18n();
  const shell = useShell();
  const line = useCommandLine();
  const rootRef = useRef<HTMLDivElement | null>(null);
  // Read once: the hand-off effect depends on the boolean, not on the controller
  // object it is rebuilt into on every render.
  const focused = line.focused;
  const historyRows = mobileHistoryRows(shell.historyEntries);
  const showHistory = shell.historyOpen && !line.textMode;

  // The same outside-press hand-off the docked field installs: a press on the
  // drawing must blur the field **before** it reaches `.native-input`, or the
  // suspended pointer lane drops the press that was meant to refocus the CAD
  // surface. The bar's own controls (including the sheet) are inside the root, so
  // they keep the keyboard.
  useEffect(() => {
    if (!focused) return undefined;
    const input = line.inputRef.current;
    return releaseFocusOnOutsidePress(rootRef.current as unknown as FocusReleaseRoot | null, () => input?.blur());
  }, [focused]);

  // The start hint is its own strip: with no drawing on screen and no host
  // failure to report, the bar says what to do first instead of leaving an empty
  // slot. A command-line refusal is not its business — that lives in the history
  // sheet, so the hint is not suppressed by one.
  const showHint = !shell.documentVisible && !shell.loading && !shell.displayedError;

  return (
    <div ref={rootRef} className="mobile-commandbar app-region">
      {line.listOpen || showHistory ? (
        <div className="mobile-sheet">
          <div className="mobile-sheet__head">
            <span>{showHistory ? t('command.history') : t('command.category.commands')}</span>
            <div className="mobile-sheet__actions">
              {showHistory ? (
                <IconButton
                  icon="clearSelection"
                  size="sm"
                  label={t('command.clear_history')}
                  disabled={shell.historyEntries.length === 0}
                  onClick={shell.clearHistory}
                />
              ) : null}
              <IconButton
                icon="close"
                size="sm"
                label={t('mobile.close')}
                onClick={() => (showHistory ? shell.setHistoryOpen(false) : line.inputRef.current?.blur())}
              />
            </div>
          </div>
          {showHistory ? (
            historyRows.length === 0 ? (
              <div className="mobile-sheet__empty">{t('command.history_empty')}</div>
            ) : (
              // The same log the docked panel renders, newest first and capped: a
              // command row recalls into the field, a refusal row is red text and
              // not a control (`.is-error`, styled in `mobile.css`).
              <ol className="mobile-sheet__rows mobile-sheet__list" aria-label={t('command.history')}>
                {historyRows.map((entry, index) => (
                  <li key={`${entry.kind}-${index}`}>
                    {entry.kind === 'command' ? (
                      <button
                        type="button"
                        className="mobile-sheet__row"
                        onClick={() => { line.choose(entry.text); shell.setHistoryOpen(false); }}
                      >
                        {entry.text}
                      </button>
                    ) : (
                      <p className="mobile-sheet__row is-error">{entry.text}</p>
                    )}
                  </li>
                ))}
              </ol>
            )
          ) : line.suggestions.length === 0 ? (
            <div className="mobile-sheet__empty">{t('command.no_matches')}</div>
          ) : (
            <div className="mobile-sheet__rows" role="listbox" aria-label={t('command.completions')}>
              {line.suggestions.map((suggestion, index) => (
                <button
                  key={suggestion.id}
                  type="button"
                  role="option"
                  id={line.optionId(suggestion)}
                  aria-selected={index === line.highlight}
                  className={['completion', index === line.highlight ? 'is-highlight' : '', suggestion.supported ? 'is-supported' : ''].filter(Boolean).join(' ')}
                  onMouseEnter={() => line.setHighlight(index)}
                  onMouseDown={event => event.preventDefault()}
                  onClick={() => line.choose(suggestion.id)}
                  title={suggestion.supported ? undefined : t(unavailableReasonKey(suggestion.id), { command: suggestion.id })}
                >
                  <Icon name={suggestion.icon as IconName} size={18} />
                  <span className="completion__id">{suggestion.id}</span>
                  <span className="completion__label">{suggestion.target ?? t(suggestion.labelKey ?? '')}</span>
                  {suggestion.supported ? null : <em className="completion__badge">{t('command.badge.unwired')}</em>}
                </button>
              ))}
            </div>
          )}
        </div>
      ) : null}
      {showHint ? <p className="mobile-commandbar__hint">{t('start.hint')}</p> : null}
      <div className="mobile-commandbar__row">
        <Icon name={line.textMode ? 'text' : 'terminal'} size={16} className="mobile-commandbar__prompt" />
        <CommandField id="cad-command" className="mobile-commandbar__input" line={line} enterKeyHint="send" />
        {line.textMode ? null : (
          // The history toggle for this presentation: same state, same log, same
          // name ("history"). It opens the sheet above the bar and nothing else.
          <IconButton
            icon="terminal"
            label={t('command.toggle_history')}
            pressed={shell.historyOpen}
            onClick={() => shell.setHistoryOpen(!shell.historyOpen)}
          />
        )}
        <IconButton
          icon="chevronRight"
          variant="solid"
          label={t('mobile.send')}
          disabled={line.disabled || !line.canSubmit}
          onClick={line.submit}
        />
      </div>
      {shell.displayedError ? (
        <p className="mobile-commandbar__message error" role="alert" title={shell.displayedError}>
          {shell.displayedError}
        </p>
      ) : null}
    </div>
  );
}
