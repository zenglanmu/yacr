// Command line, docked bottom-left above the status bar. Autocomplete opens
// inline *below* the input (never over the drawing area), the history panel
// expands in the same row, and Up/Down step through recent commands.
//
// This file is the **docked presentation** and nothing else: the draft, the
// suggestion rules, the history cursor, the text-step mode and the submit paths
// are the shared controller in `commands/use-command-line.ts`, which the mobile
// command bar renders from as well. What stays here is what only the desktop
// region has — the command-window echo above the input, the inline suggestion
// list, the history panel and the outside-press focus hand-off.
import React, { useEffect, useMemo, useRef } from 'react';
import { Icon } from '../icons.tsx';
import { Button, IconButton } from './primitives.tsx';
import { CommandField } from './CommandField.tsx';
import { useI18n, useShell } from '../context.ts';
import { useCommandLine } from '../commands/use-command-line.ts';
import { commandLogView } from '../commands/command-log.ts';
import { historyRows } from '../commands/command-history.ts';
import { unavailableReasonKey } from '../commands/commands.ts';
import type { IconName } from '../icons.tsx';
import { releaseFocusOnOutsidePress } from '../focus-release.ts';
import type { FocusReleaseRoot } from '../focus-release.ts';

export function CommandLine() {
  const { t } = useI18n();
  const shell = useShell();
  const line = useCommandLine();
  const rootRef = useRef<HTMLDivElement | null>(null);
  // Read once so the hand-off effect depends on the boolean, not on the
  // controller object it is rebuilt into on every render.
  const focused = line.focused;
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
    const input = line.inputRef.current;
    // The release hook only needs `contains`; its structural root narrows the
    // target to `EventTarget`, which `Element#contains` does not declare.
    return releaseFocusOnOutsidePress(rootRef.current as unknown as FocusReleaseRoot | null, () => input?.blur());
  }, [focused]);

  // Follow the newest echo when (and only when) the view is already at the
  // bottom. This runs on the log signature, so a poll that repeats the same log
  // never touches the scroll position.
  useEffect(() => {
    const node = logRef.current;
    if (!node || !logStick.current) return;
    node.scrollTop = node.scrollHeight;
  }, [logSignature]);

  function trackLogScroll(event: React.UIEvent<HTMLDivElement>) {
    const node = event.currentTarget;
    logStick.current = node.scrollHeight - node.scrollTop - node.clientHeight <= 4;
  }

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
              {row.lines.map((logLine, lineIndex) => (
                <div key={lineIndex} className={`commandlog__line is-${logLine.tone}`}>
                  {logLine.label ? (
                    <span className="commandlog__label">
                      {logLine.tone === 'command' ? `${logLine.label} ` : `${logLine.label} = `}
                    </span>
                  ) : null}
                  <span className="commandlog__text">{logLine.text}</span>
                  {logLine.unit ? <span className="commandlog__unit">{` ${logLine.unit}`}</span> : null}
                  {logLine.meta ? <span className="commandlog__meta">{logLine.meta}</span> : null}
                </div>
              ))}
            </div>
          ))}
        </div>
      ) : null}
      <div className="commandline__row">
        <Icon name={line.textMode ? 'text' : 'terminal'} size={15} className="commandline__prompt" />
        <CommandField id="cad-command" className="commandline__input" line={line} />
        {line.textMode ? (
          // The named step, in the shared bounded accent label: it ellipsizes
          // instead of pushing the field, and the field never moves between modes.
          <span className="statusbar__tool-step" title={line.textStepKey ?? undefined}>{line.textStepKey ? t(line.textStepKey) : null}</span>
        ) : null}
        {line.textMode ? null : (
          // The history toggle, and **only** the history toggle: it opens the log
          // below this row and nothing else. The message lines are separate
          // elements outside that panel, so pressing this never hides, moves or
          // swallows the sentence the last attempt produced. The accessible name
          // says "history" — the chevron alone read as "collapse the red line".
          //
          // `onMouseDown` keeps the press from blurring the field first: a blur
          // closes the completion list, which shortens the region, shifts this
          // row between the press and its release, and the browser then fires the
          // click on an ancestor — the first press did nothing at all. Keeping
          // focus makes one press open the log (the list closes by itself as soon
          // as `historyOpen` is true).
          <IconButton
            icon={shell.historyOpen ? 'chevronDown' : 'chevronUp'}
            size="sm"
            label={t('command.toggle_history')}
            pressed={shell.historyOpen}
            title={`${t('command.toggle_history')} (F2)`}
            onMouseDown={event => event.preventDefault()}
            onClick={() => shell.setHistoryOpen(!shell.historyOpen)}
          />
        )}
      </div>
      {line.messageText ? (
        <p id="cad-command-message" className="commandline__message error" role="alert">
          {line.messageText}
        </p>
      ) : null}
      {line.textMode ? null : shell.commandMessage ? (
        <p className={['commandline__message', shell.commandMessage.kind].join(' ')} role="alert">
          {shell.commandMessage.text}
        </p>
      ) : null}
      {line.listOpen ? (
        <div className="completions" role="listbox" aria-label={t('command.completions')}>
          <div className="completions__title">{t('command.category.commands')}</div>
          {line.suggestions.length === 0 ? (
            <div className="completions__empty">{t('command.no_matches')}</div>
          ) : line.suggestions.map((suggestion, index) => (
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
        // The log, newest first: admitted commands stay clickable (recall into the
        // field), a refusal is red text and not a control — there is no command in
        // it to run again. Both rows come from `shell.historyEntries`, so the panel
        // and the mobile sheet can never describe the same session differently.
        <div className="history-panel" aria-label={t('command.history')}>
          <div className="history-panel__head">
            <span>{t('command.history')}</span>
            <Button variant="plain" size="xs" icon="clearSelection" onClick={shell.clearHistory} disabled={shell.historyEntries.length === 0}>
              {t('command.clear_history')}
            </Button>
          </div>
          {shell.historyEntries.length === 0 ? (
            <div className="history-panel__empty">{t('command.history_empty')}</div>
          ) : (
            <ol className="history-panel__list">
              {historyRows(shell.historyEntries).map((entry, index) => (
                <li key={`${entry.kind}-${index}`}>
                  {entry.kind === 'command' ? (
                    <button type="button" className="history-panel__entry" onClick={() => shell.setCommand(entry.text)}>
                      {entry.text}
                    </button>
                  ) : (
                    <p className="history-panel__entry is-error">{entry.text}</p>
                  )}
                </li>
              ))}
            </ol>
          )}
        </div>
      ) : null}
    </div>
  );
}
