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
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { Icon } from '../icons.tsx';
import type { IconName } from '../icons.tsx';
import { IconButton } from './primitives.tsx';
import { CommandField } from './CommandField.tsx';
import { ContextMenu } from './ContextMenu.tsx';
import { useI18n, useShell } from '../context.ts';
import { useCommandLine } from '../commands/use-command-line.ts';
import { mobileHistoryRows } from '../commands/command-line-model.ts';
import { unavailableReasonKey } from '../commands/commands.ts';
import { commandHistoryText, isHistoryCopyChord } from '../commands/history-copy.ts';
import { useHistoryCopy } from '../commands/use-history-copy.ts';
import { historySurfaceReading } from '../history-copy-surface.ts';
import { isTextEntryTarget } from '../defaults.ts';
import type { ContextMenuItem, ContextMenuModel } from '../commands/context-menu.ts';
import { releaseFocusOnOutsidePress } from '../focus-release.ts';
import type { FocusReleaseRoot } from '../focus-release.ts';
import { statusBannerText } from '../host/notice.ts';
import { reportError } from '../errors/error-report.ts';

/** The open history sheet's right-click menu: where it sits, and the row it hit. */
interface HistoryMenuState {
  readonly x: number;
  readonly y: number;
  readonly index: number | null;
}

export function MobileCommandBar() {
  const { t } = useI18n();
  const shell = useShell();
  const line = useCommandLine();
  const rootRef = useRef<HTMLDivElement | null>(null);
  // Read once: the hand-off effect depends on the boolean, not on the controller
  // object it is rebuilt into on every render.
  const focused = line.focused;
  // Memoized on the log's identity: the effect below depends on this array, and
  // `mobileHistoryRows` returns a new list on every call.
  const historyRows = useMemo(() => mobileHistoryRows(shell.historyEntries), [shell.historyEntries]);
  const showHistory = shell.historyOpen && !line.textMode;

  // Copy state shared with the docked panel. The serialized text is the sheet's
  // own visible order — newest first — never the array's storage order.
  const copy = useHistoryCopy();
  const copyText = copy.copy;
  const sheetRef = useRef<HTMLDivElement | null>(null);
  const copyButtonRef = useRef<HTMLButtonElement | null>(null);
  const hovered = useRef(false);
  const [copyMenu, setCopyMenu] = useState<HistoryMenuState | null>(null);
  const menuIndex = copyMenu?.index ?? null;
  const menuEntry = menuIndex !== null ? historyRows[menuIndex] ?? null : null;

  function copyAll() {
    copyText(commandHistoryText(historyRows));
  }

  function runCopyMenu(item: ContextMenuItem) {
    setCopyMenu(null);
    if (item.id === 'history.copy_entry') {
      // The sheet renders entries only (no attached output), so the entry's own
      // text is the whole per-row copy.
      if (menuEntry !== null) copyText(commandHistoryText([menuEntry]));
      return;
    }
    copyAll();
  }

  function openCopyMenu(event: React.MouseEvent<HTMLDivElement>) {
    if (!showHistory) return;
    event.preventDefault();
    const target = event.target as Element | null;
    const holder = target !== null && typeof target.closest === 'function'
      ? target.closest('[data-history-index]')
      : null;
    const raw = holder !== null ? holder.getAttribute('data-history-index') : null;
    const index = raw !== null && /^\d+$/.test(raw) ? Number(raw) : null;
    setCopyMenu({ x: event.clientX, y: event.clientY, index });
  }

  // Ctrl+C / Cmd+C on a hardware keyboard while the sheet is the reading surface
  // (focus or a selection inside it). The same non-interception rule as the
  // desktop panel: a focused text field keeps native copy, and a press the sheet
  // does not own reaches the shell's COPYCLIP path untouched.
  useEffect(() => {
    if (!showHistory) {
      // A closed sheet is not a reading surface, and its menu must close with it.
      hovered.current = false;
      setCopyMenu(null);
      return undefined;
    }
    const onKeyDown = (event: KeyboardEvent) => {
      if (!isHistoryCopyChord(event)) return;
      if (isTextEntryTarget(event.target) || isTextEntryTarget(document.activeElement)) return;
      const reading = historySurfaceReading(sheetRef.current);
      // The open menu belongs to this sheet, so Ctrl+C there is still a history
      // copy rather than COPYCLIP.
      const menuOpen = copyMenu !== null;
      if (!menuOpen && !hovered.current && !reading.focused && !reading.selectionInside) return;
      event.preventDefault();
      event.stopPropagation();
      copyText(reading.selectedText !== '' ? reading.selectedText : commandHistoryText(historyRows));
    };
    window.addEventListener('keydown', onKeyDown, true);
    return () => window.removeEventListener('keydown', onKeyDown, true);
  }, [showHistory, copyMenu, copyText, historyRows]);

  const menuModel = useMemo<ContextMenuModel>(() => ({
    sections: [{
      id: 'history-copy',
      items: [
        { id: 'history.copy_entry', key: 'command.history.copy_entry', command: null, enabled: menuEntry !== null },
        { id: 'history.copy_all', key: 'command.history.copy_all', command: null, enabled: historyRows.length > 0 },
      ],
    }],
  }), [menuEntry, historyRows.length]);

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
  // Same banner mapping as the desktop status bar: the known GPU-fallback token
  // reads as one short sentence, and the raw host string stays in `title`.
  const bannerText = shell.displayedError ? statusBannerText(shell.displayedError, t) : '';

  return (
    <div ref={rootRef} className="mobile-commandbar app-region">
      {line.listOpen || showHistory ? (
        <div
          className="mobile-sheet"
          ref={sheetRef}
          onPointerEnter={() => { hovered.current = true; }}
          onPointerLeave={() => { hovered.current = false; }}
          onContextMenu={openCopyMenu}
        >
          <div className="mobile-sheet__head">
            <span>{showHistory ? t('command.history') : t('command.category.commands')}</span>
            <div className="mobile-sheet__actions">
              {showHistory ? (
                <>
                  {/* The same live region as the docked panel: the outcome of a
                      copy by button, long-press menu or hardware chord. */}
                  <span
                    className={[
                      'mobile-sheet__status',
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
                    disabled={historyRows.length === 0}
                    onClick={copyAll}
                  />
                  <IconButton
                    icon="clearSelection"
                    size="sm"
                    label={t('command.clear_history')}
                    disabled={shell.historyEntries.length === 0}
                    onClick={shell.clearHistory}
                  />
                </>
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
                  <li key={`${entry.kind}-${index}`} data-history-index={index}>
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
        // The compact banner is the same trigger as the desktop status bar: a
        // finger-sized button that opens the global error dialog, because the
        // sentence itself is clipped/wrapped in the bar.
        <button
          type="button"
          className="mobile-commandbar__message error"
          role="alert"
          title={shell.displayedError}
          aria-label={t('error.status.open')}
          onClick={() => reportError({ kind: 'host', message: shell.displayedError, detail: shell.displayedError })}
        >
          {bannerText}
        </button>
      ) : null}
      {copyMenu !== null ? (
        // Reuses the shortcut-menu overlay: fixed at the press, keyboard-driven,
        // dismissed by Escape/Tab, an outside press, blur, resize or scroll.
        <ContextMenu
          menu={menuModel}
          anchor={{ x: copyMenu.x, y: copyMenu.y }}
          onRun={runCopyMenu}
          onDismiss={({ restoreFocus }) => {
            setCopyMenu(null);
            if (restoreFocus) copyButtonRef.current?.focus();
          }}
        />
      ) : null}
    </div>
  );
}
