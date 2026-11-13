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
// While a plugin `Editor` prompt is pending the row shows that prompt instead of
// a command: its own sentence in the accent label beside the field, its keywords
// as chips, and its last invalid-input sentence in the echo strip above the row.
// The echo strip is also where a plugin's `writeMessage` lines appear. Both are
// host/plugin strings rendered verbatim, both float over the drawing (the region
// is the same containing block as the two overlays), and neither is ever a modal
// or a row that reflows the bar.
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
import { ContextMenu } from './ContextMenu.tsx';
import { useI18n, useShell } from '../context.ts';
import { useCommandLine } from '../commands/use-command-line.ts';
import { usePluginEcho } from '../commands/use-plugin-echo.ts';
import { PROMPT_KEYWORDS_LABEL_KEY } from '../commands/plugin-prompt.ts';
import { commandWindowRows } from '../commands/command-window.ts';
import { unavailableReasonKey } from '../commands/commands.ts';
import { commandWindowText, entryText, isHistoryCopyChord } from '../commands/history-copy.ts';
import { useHistoryCopy } from '../commands/use-history-copy.ts';
import { historySurfaceReading } from '../history-copy-surface.ts';
import { isTextEntryTarget } from '../defaults.ts';
import type { ContextMenuItem, ContextMenuModel } from '../commands/context-menu.ts';
import type { IconName } from '../icons.tsx';
import { releaseFocusOnOutsidePress } from '../focus-release.ts';
import type { FocusReleaseRoot } from '../focus-release.ts';

/** The open right-click menu of the panel: where it sits, and the row it hit. */
interface HistoryMenuState {
  readonly x: number;
  readonly y: number;
  readonly key: string | null;
}

export function CommandLine() {
  const { t } = useI18n();
  const shell = useShell();
  const line = useCommandLine();
  // The plugin echo (the last `editor.writeMessage` lines) is the region's own
  // bounded strip; it has exactly one source (`commands/plugin-message-echo.ts`).
  const echo = usePluginEcho();
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

  // Copying the record. `copy` owns the transient "Copied"/"Copy failed" status
  // and its timer (`clipboard.ts` owns the write itself); the string it receives
  // is always one of the serializers in `commands/history-copy.ts`, so the
  // pasted text is the panel's own visible record — output included.
  const copy = useHistoryCopy();
  const copyText = copy.copy;
  const panelRef = useRef<HTMLDivElement | null>(null);
  const copyButtonRef = useRef<HTMLButtonElement | null>(null);
  // Hover is this component's own fact: the panel is the reading surface while
  // the pointer is over it, focus is inside it, or a selection starts in it.
  // Enter/leave keep the keydown handler (installed on `window`) from having to
  // ask the layout on every press.
  const hovered = useRef(false);
  const [copyMenu, setCopyMenu] = useState<HistoryMenuState | null>(null);
  // The row the open menu was pressed on, resolved through the same key the
  // `<li>` carries; `null` when the press hit the panel's padding or head.
  const menuKey = copyMenu?.key ?? null;
  const menuRow = useMemo(
    () => (menuKey !== null ? windowRows.find(row => row.key === menuKey) ?? null : null),
    [menuKey, windowRows],
  );

  function copyAll() {
    copyText(commandWindowText(windowRows, t));
  }

  function runCopyMenu(item: ContextMenuItem) {
    setCopyMenu(null);
    if (item.id === 'history.copy_entry') {
      if (menuRow !== null) copyText(entryText(menuRow, t));
      return;
    }
    copyAll();
  }

  function openCopyMenu(event: React.MouseEvent<HTMLDivElement>) {
    // `bindBrowserDefaults` already suppresses the webview's own menu for a
    // non-text target; preventing here keeps that true even if the guards change.
    event.preventDefault();
    const target = event.target as Element | null;
    const holder = target !== null && typeof target.closest === 'function'
      ? target.closest('[data-history-key]')
      : null;
    const key = holder !== null ? holder.getAttribute('data-history-key') : null;
    setCopyMenu({ x: event.clientX, y: event.clientY, key });
  }

  // Ctrl+C / Cmd+C while the history is the reading surface. Capture phase: the
  // shell's window handler maps Ctrl+C to COPYCLIP, and only a press this handler
  // actually took is prevented and stopped — a press with the pointer elsewhere
  // (or a focused text field) reaches COPYCLIP exactly as before. A non-collapsed
  // selection inside the panel copies just that text; otherwise the whole visible
  // record is copied.
  useEffect(() => {
    if (!shell.historyOpen) {
      // A pane that is no longer on screen is not a reading surface, and a menu
      // opened on it must not outlive it.
      hovered.current = false;
      setCopyMenu(null);
      return undefined;
    }
    const onKeyDown = (event: KeyboardEvent) => {
      if (!isHistoryCopyChord(event)) return;
      // Native field copy wins: the command field and every popup field keep
      // their own Ctrl+C, exactly as `accelerators.ts` leaves them alone.
      if (isTextEntryTarget(event.target) || isTextEntryTarget(document.activeElement)) return;
      const reading = historySurfaceReading(panelRef.current);
      // An open copy menu is part of the same reading surface (it was opened on
      // this pane and may cover the pointer), so its Ctrl+C is a copy too — never
      // COPYCLIP.
      const menuOpen = copyMenu !== null;
      if (!menuOpen && !hovered.current && !reading.focused && !reading.selectionInside) return;
      event.preventDefault();
      event.stopPropagation();
      copyText(reading.selectedText !== '' ? reading.selectedText : commandWindowText(windowRows, t));
    };
    window.addEventListener('keydown', onKeyDown, true);
    return () => window.removeEventListener('keydown', onKeyDown, true);
  }, [shell.historyOpen, copyMenu, copyText, windowRows, t]);

  // The two copy menu items always exist; an unavailable one is disabled, never
  // dropped (the same rule as the CAD shortcut menu).
  const menuModel = useMemo<ContextMenuModel>(() => ({
    sections: [{
      id: 'history-copy',
      items: [
        { id: 'history.copy_entry', key: 'command.history.copy_entry', command: null, enabled: menuRow !== null },
        { id: 'history.copy_all', key: 'command.history.copy_all', command: null, enabled: windowRows.length > 0 },
      ],
    }],
  }), [menuRow, windowRows.length]);

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

  // The echo strip carries two facts, both host/plugin data and never copy the
  // chrome invents: the pending prompt's last invalid-input sentence (the line
  // the reader needs right under the question) and the plugin's own messages. It
  // yields the space it shares with the suggestion list and the history panel, so
  // the three overlays can never paint on top of each other.
  const promptError = line.promptError;
  const showEcho = !line.listOpen && !shell.historyOpen && (echo.length > 0 || promptError !== null);

  return (
    <div ref={rootRef} className="region region--cmdline commandline app-region">
      <div className="commandline__row">
        <Icon name={line.textMode ? 'text' : 'terminal'} size={15} className="commandline__prompt" />
        <CommandField id="cad-command" className="commandline__input" line={line} />
        {line.promptMode ? (
          // The pending plugin prompt: the plugin's own sentence, in the command
          // line's accent label — quiet, inline, and the field never moves. It is
          // deliberately its own class rather than the status bar's tool-step
          // chip, which is hidden on a narrow window: the question being answered
          // must not disappear.
          <span className="commandline__prompt-step" role="status" title={line.promptMessage ?? undefined}>
            {line.promptMessage}
          </span>
        ) : line.textMode ? (
          // The named step, in the shared bounded accent label: it ellipsizes
          // instead of pushing the field, and the field never moves between modes.
          <span className="statusbar__tool-step" title={line.textStepKey ?? undefined}>{line.textStepKey ? t(line.textStepKey) : null}</span>
        ) : null}
        {line.promptKeywords.length > 0 ? (
          // The prompt's accepted keywords, as chips: a press answers the prompt
          // with that keyword (the host matches it), and `onMouseDown` keeps the
          // press from blurring the field so the keyboard stays where it is. The
          // cluster scrolls sideways instead of wrapping, so the row keeps its
          // one-line height however many options the plugin declared.
          <span className="commandline__keywords" role="group" aria-label={t(PROMPT_KEYWORDS_LABEL_KEY)}>
            {line.promptKeywords.map(keyword => (
              <button
                key={keyword}
                type="button"
                className="commandline__keyword"
                onMouseDown={event => event.preventDefault()}
                onClick={() => line.chooseKeyword(keyword)}
              >
                {keyword}
              </button>
            ))}
          </span>
        ) : null}
        {line.textMode || line.promptMode ? null : (
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
      {showEcho ? (
        // The command line's echo area (see `style.css`): the bounded plugin
        // messages and the prompt's invalid-input sentence. It floats above the
        // row in the same containing block as the two overlays, so the bar keeps
        // its height and neither the ribbon nor the status bar ever moves.
        <div className="commandline__echo" role="log" aria-label={t('command.plugin_echo.label')}>
          {echo.map((message, index) => (
            <p key={`plugin-message-${index}`} className="commandline__echo-line">{message}</p>
          ))}
          {promptError !== null ? (
            // The host's own sentence, not a chrome re-wording: it is the answer
            // to *why* the last input was refused, and it stays while the prompt
            // is still pending.
            <p className="commandline__echo-line is-invalid" role="status">{promptError}</p>
          ) : null}
        </div>
      ) : null}
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
        <div
          className="history-panel"
          ref={panelRef}
          aria-label={t('command.history')}
          data-more-below={moreBelow ? 'true' : undefined}
          onPointerEnter={() => { hovered.current = true; }}
          onPointerLeave={() => { hovered.current = false; }}
          onContextMenu={openCopyMenu}
        >
          <div className="history-panel__head">
            <span className="history-panel__head-title">{t('command.history')}</span>
            {/* One live region for the outcome of any copy (button, menu, chord).
                It stays mounted and empty between copies, so a failure is spoken
                only when it happens. */}
            <span
              className={[
                'history-panel__status',
                copy.status === 'failed' ? 'is-failed' : '',
                copy.status === 'copied' ? 'is-copied' : '',
              ].filter(Boolean).join(' ')}
              role="status"
            >
              {copy.status === null
                ? ''
                : t(copy.status === 'failed' ? 'command.history.copy_failed' : 'command.history.copied')}
            </span>
            <IconButton
              ref={copyButtonRef}
              icon="copy"
              size="sm"
              label={t('command.history.copy')}
              title={`${t('command.history.copy')} (Ctrl+C)`}
              disabled={windowRows.length === 0}
              onClick={copyAll}
            />
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
                  <li key={row.key} className="history-panel__row" data-history-key={row.key}>
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
      {copyMenu !== null ? (
        // The same overlay the CAD shortcut menu uses: fixed at the press,
        // keyboard-driven (arrows/Home/End/Enter, Escape/Tab close), dismissed by
        // a press outside, blur, resize or scroll. It has no backdrop, so the
        // press that dismisses it still reaches the row underneath.
        <ContextMenu
          menu={menuModel}
          anchor={{ x: copyMenu.x, y: copyMenu.y }}
          onRun={runCopyMenu}
          onDismiss={({ restoreFocus }) => {
            setCopyMenu(null);
            // Escape/Tab came from the keyboard; hand focus back to the panel's
            // own control instead of dropping it on the document body.
            if (restoreFocus) copyButtonRef.current?.focus();
          }}
        />
      ) : null}
    </div>
  );
}
