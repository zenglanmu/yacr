// Command line, docked bottom-left above the status bar. Both things it can add
// to the screen — the suggestion list and the command history — are **overlays
// above the row**: the bar itself is one fixed-height line, and neither a
// keystroke nor a click ever makes it taller.
//
// There used to be two records of the same session: an in-flow echo strip stacked
// above the input (what the host reported) and the chevron popup (what was typed).
// They drifted apart, and the strip grew and shrank with the log, so every command
// moved the ribbon, the drawing and the status bar. There is one record now, the
// popup, and it reads `commandWindowRows` — the module that folds the typed
// history and the host's command window into a single oldest-first list — so a
// command and the output it produced sit together and the bar's height depends on
// neither.
//
// This file is the **docked presentation** and nothing else: the draft, the
// suggestion rules, the history cursor, the text-step mode and the submit paths
// are the shared controller in `commands/use-command-line.ts`, which the mobile
// command bar renders from as well. What stays here is what only the desktop
// region has — the single row, the overlay suggestion list, the history popup and
// the outside-press focus hand-off.
//
// **No refusal line under the field.** A refused command used to be reported
// twice: once as a red sentence in this region and once as the same sentence in
// the history. The history log is the record now — the sentence a submit produced
// is written there (red row) and nowhere else, so the line the user reads and the
// line they scroll back to are one line, not a duplicate. The host's own error
// banner (the status bar's, not this region's) is untouched: that reports host
// failures, not command-line refusals.
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { Icon } from '../icons.tsx';
import { IconButton } from './primitives.tsx';
import { CommandField } from './CommandField.tsx';
import { useI18n, useShell } from '../context.ts';
import { useCommandLine } from '../commands/use-command-line.ts';
import { commandWindowRows } from '../commands/command-window.ts';
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

  // The one record. Two sources are folded here and nowhere else: the typed log
  // (`shell.historyEntries`, commands *and* refusals) and the host's command
  // window (`status.commandLog`, echoed output). The merge is memoized on a
  // signature because the status poll is 4 Hz and hands over a fresh array every
  // time — re-folding up to 200 host entries and re-resolving every catalogue
  // label per render would be wasted work. The host log is append-only, so length
  // plus the newest sequence identifies its content exactly; `historyEntries` is
  // immutable and replaced only when a line is appended, so its identity is its
  // signature.
  const logEntries = shell.status?.commandLog ?? [];
  const logSignature = logEntries.length > 0
    ? `${logEntries.length}:${logEntries[logEntries.length - 1]?.sequence}`
    : '';
  const windowRows = useMemo(
    () => commandWindowRows(shell.historyEntries, logEntries, t),
    // `logEntries` is covered by `logSignature`; listing it would re-fold on
    // every poll that carried the very same log.
    [logSignature, shell.historyEntries, t],
  );
  // The rows, not the log: a refetch that changes nothing must not touch the
  // scroll position, and a new command must move the view only when the user is
  // already at the bottom.
  const rowsSignature = windowRows.length > 0
    ? `${windowRows.length}:${windowRows[windowRows.length - 1]?.key}`
    : '';

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

  // The popup's scroller: the user owns the position once they scroll up, and it
  // is never yanked back. Opening the panel always starts at the newest row (the
  // list runs oldest → newest, so "newest" is the bottom — AutoCAD's order), and
  // new rows follow the view only while it is already there.
  const panelListRef = useRef<HTMLOListElement | null>(null);
  const stick = useRef(true);
  function overflowBelow(node: HTMLElement | null): boolean {
    return node !== null && node.scrollHeight - node.scrollTop - node.clientHeight > 2;
  }
  // The visible "there is more under the fold" cue. This WebView draws an
  // **overlay** scrollbar that paints nothing at rest (the ribbon's and the mobile
  // drawer's scrollers carry the same note), so the popup carries the drawer's
  // fade — and only while a row really sits below the edge.
  const [moreBelow, setMoreBelow] = useState(false);
  useEffect(() => {
    const node = panelListRef.current;
    if (!shell.historyOpen) {
      // Whatever the last visit left behind, the next open starts at the bottom.
      stick.current = true;
      return;
    }
    if (node === null) return;
    // Follow only while the view is already at the bottom; a reader who scrolled
    // up keeps their place, but the fade still has to know what is under it now.
    if (stick.current) node.scrollTop = node.scrollHeight;
    setMoreBelow(overflowBelow(node));
  }, [shell.historyOpen, rowsSignature]);

  function trackScroll(event: React.UIEvent<HTMLOListElement>) {
    const node = event.currentTarget;
    stick.current = node.scrollHeight - node.scrollTop - node.clientHeight <= 4;
    setMoreBelow(overflowBelow(node));
  }

  return (
    <div ref={rootRef} className="region region--cmdline commandline app-region">
      <div className="commandline__row">
        <Icon name={line.textMode ? 'text' : 'terminal'} size={15} className="commandline__prompt" />
        <CommandField id="cad-command" className="commandline__input" line={line} />
        {line.textMode ? (
          // The named step, in the shared bounded accent label: it ellipsizes
          // instead of pushing the field, and the field never moves between modes.
          <span className="statusbar__tool-step" title={line.textStepKey ?? undefined}>{line.textStepKey ? t(line.textStepKey) : null}</span>
        ) : null}
        {line.textMode ? null : (
          // The history toggle, and **only** the history toggle: it opens the
          // popup above this row and nothing else. It is the one place a refused
          // command is read back from, so pressing it must never clear the record
          // (the host's log has no clear call either). The accessible name says
          // "history" — the chevron alone read as "collapse the red line".
          //
          // `onMouseDown` keeps the press from blurring the field: a blur closes
          // the suggestion list, and a focus ring flickering off and on is a
          // cheaper loss than dropping the press. The list is an overlay now, so
          // the row cannot shift under the click — the `click` always lands here.
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
      {line.listOpen ? (
        // An overlay above the row (see `style.css`), the same containing block as
        // the popup: a flow child here would add up to 176px under the input and
        // push the status bar off the screen on every keystroke.
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
        // The record: one row per command with the output it produced underneath,
        // oldest first so the newest line sits nearest the input. An admitted
        // command stays clickable (recall into the field); a refusal is red text
        // and not a control — there is no command in it to run again. Both come
        // from `commandWindowRows`, which folds the same typed log the mobile
        // sheet reads, so the two surfaces cannot disagree about *which lines
        // happened*; this one additionally shows the host echo under each line.
        //
        // The panel is an overlay with a **fixed** height: `style.css` pins it
        // above the input (`position: absolute; bottom: calc(100% + 6px)`), sets
        // `height: min(40vh, 340px)` and scrolls the list inside itself. Opening
        // it therefore never moves the status bar off-screen, never grows the
        // window and never moves its own top edge as commands arrive.
        // `data-more-below` turns on the bottom fade when the scroller has rows
        // under the fold.
        <div className="history-panel" aria-label={t('command.history')} data-more-below={moreBelow ? 'true' : undefined}>
          <div className="history-panel__head">
            <span>{t('command.history')}</span>
          </div>
          {windowRows.length === 0 ? (
            <div className="history-panel__empty">{t('command.history_empty')}</div>
          ) : (
            <ol
              className="history-panel__list"
              ref={panelListRef}
              role="log"
              aria-label={t('command.log.label')}
              onScroll={trackScroll}
            >
              {windowRows.map(row => {
                // A command row is abbreviated by dragging the text back into the
                // field; a refusal has nothing to re-run, so it stays plain output.
                const recall = row.recall;
                return (
                  <li key={row.key} className="history-panel__row">
                    {row.kind === 'command' ? (
                      <button
                        type="button"
                        className="history-panel__entry"
                        title={row.code ?? undefined}
                        onClick={() => { if (recall !== null) shell.setCommand(recall); }}
                      >
                        <span className="history-panel__prompt">{t('command.log.prompt')}</span>
                        {' '}
                        <span className="history-panel__command">{row.text}</span>
                      </button>
                    ) : (
                      <p className="history-panel__entry is-error" title={row.code ?? undefined}>{row.text}</p>
                    )}
                    {row.lines.length > 0 ? (
                      // The echoed output, indented under its own command so the
                      // grouping reads at 11px mono: the same label / value / unit
                      // / meta shape the echo strip used to have.
                      <div className="history-panel__output">
                        {row.lines.map((outputLine, index) => (
                          <div key={index} className={`history-panel__line is-${outputLine.tone}`}>
                            {outputLine.label ? (
                              <span className="history-panel__label">
                                {outputLine.tone === 'command' ? `${outputLine.label} ` : `${outputLine.label} = `}
                              </span>
                            ) : null}
                            <span className="history-panel__text">{outputLine.text}</span>
                            {outputLine.unit ? <span className="history-panel__unit">{` ${outputLine.unit}`}</span> : null}
                            {outputLine.meta ? <span className="history-panel__meta">{outputLine.meta}</span> : null}
                          </div>
                        ))}
                      </div>
                    ) : null}
                  </li>
                );
              })}
            </ol>
          )}
        </div>
      ) : null}
    </div>
  );
}
