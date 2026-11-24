// The text screen (`TEXTSCR` / `GRAPHSCR`): the session's command history as a
// plain-text window over the drawing.
//
// There is **no second history model**: the window renders `commandWindowRows` —
// the same fold of the typed record (`shell.historyEntries`) and the host echo
// (`status.commandLog`) the command line's expanded history renders — through the
// same plain-text serializer the history's Copy action uses
// (`commandWindowText`). A row the expanded history shows is a row this window
// shows, in the same order.
//
// Visibility is the **host's** session flag (`cad_status.textScreen`, set by
// `TEXTSCR` and cleared by `GRAPHSCR`): the chrome keeps no local mirror, and the
// close word goes through the router so the flag and the window cannot drift.
// The window is mounted only while the flag is `true`.
//
// An absent echo source (an older carrier that sends no `commandLog`) is **named**
// instead of painted as an empty window; a genuinely empty session shows the
// explicit "no commands yet" sentence. Neither case invents history.
import React, { useEffect, useMemo, useRef } from 'react';
import { Icon } from '../icons.tsx';
import { Button, IconButton } from './primitives.tsx';
import { useI18n, useShell } from '../context.ts';
import { commandWindowRows } from '../commands/command-window.ts';
import { commandWindowText } from '../commands/history-copy.ts';
import { textScreenOpen } from '../cad/cad.ts';

/** The close word: the session flag's OFF spelling. */
const GRAPH_SCREEN_COMMAND = 'GRAPHSCR';

export function TextScreen() {
  const { t } = useI18n();
  const shell = useShell();
  const open = textScreenOpen(shell.status);
  // `useSurfacePanelFocus` is not used here: this window is opened and closed by
  // the host flag, and Escape must send the real `GRAPHSCR` command rather than
  // hiding a local copy. The latest dispatch is read through a ref so the one
  // listener never has to be reinstalled when the shell object is rebuilt.
  const runRef = useRef(shell.runCommand);
  runRef.current = shell.runCommand;

  const logEntries = shell.status?.commandLog;
  const logSignature = Array.isArray(logEntries) && logEntries.length > 0
    ? `${logEntries.length}:${logEntries[logEntries.length - 1]?.sequence}`
    : '';
  // Fold only when the two sources change — the status poll hands over a fresh
  // array at 4 Hz, and re-folding the log on every poll would be wasted work.
  const rows = useMemo(
    () => commandWindowRows(shell.historyEntries, logEntries ?? [], t),
    // `logEntries` is covered by `logSignature`; see `CommandLine.tsx`.
    [logSignature, shell.historyEntries, t],
  );
  const body = useMemo(() => commandWindowText(rows, t), [rows, t]);

  useEffect(() => {
    if (!open) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.isComposing || event.keyCode === 229) return;
      if (event.key !== 'Escape' || event.defaultPrevented) return;
      // Capture phase: the text window is the top reading surface, so Escape
      // closes it (through the router) instead of reaching the shell's generic
      // Escape branch, which would close unrelated palettes underneath.
      event.preventDefault();
      event.stopPropagation();
      runRef.current(GRAPH_SCREEN_COMMAND);
    };
    window.addEventListener('keydown', onKeyDown, true);
    return () => window.removeEventListener('keydown', onKeyDown, true);
  }, [open]);

  if (!open) return null;

  const close = () => runRef.current(GRAPH_SCREEN_COMMAND);
  const emptyKey = Array.isArray(logEntries) || shell.historyEntries.length > 0
    ? 'textscreen.empty'
    : 'textscreen.unavailable';

  return (
    <section className="text-screen" role="dialog" aria-label={t('textscreen.title')}>
      <header className="text-screen__head">
        <span className="text-screen__title">
          <Icon name="terminal" size={14} />
          {t('textscreen.title')}
        </span>
        <IconButton icon="close" size="xs" label={t('textscreen.close')} onClick={close} />
      </header>
      {rows.length === 0 ? (
        <p className="text-screen__empty">{t(emptyKey)}</p>
      ) : (
        <pre className="text-screen__body" tabIndex={0}>{body}</pre>
      )}
      <footer className="text-screen__foot">
        <span className="text-screen__hint">{t('textscreen.hint')}</span>
        <Button variant="chip" size="sm" icon="drawing" iconSize={13} onClick={close}>
          {t('textscreen.close')}
        </Button>
      </footer>
    </section>
  );
}
