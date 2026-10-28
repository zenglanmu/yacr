// The command line's history log, as React state.
//
// The rules (append, bound, dedupe, the derived command-only recall list) live in
// `command-history.ts`; this hook owns only the state and the two write paths the
// shell exposes:
//
//   * `appendCommand` — one admitted command line (`submitCommand`);
//   * `recordError`   — one refusal sentence, called with the already-built text
//     (the record is the only surface it has, so it is written once per submit).
//
// There is **one** log for both presentations: the desktop region and the mobile
// bar read `ShellValue.history` (command-only, for Up/Down recall and ranking) and
// `ShellValue.historyEntries` (the typed record they render). Neither view keeps
// a copy of its own.
import { useCallback, useMemo, useState } from 'react';
import { appendCommand, appendError, commandOnly } from './command-history.ts';
import type { CommandHistoryEntry } from './command-history.ts';

/** The empty log, as one frozen value so a clear is a reference-stable reset. */
const EMPTY: readonly CommandHistoryEntry[] = Object.freeze([]);

/** What a presentation and the shell read from the log. */
export interface CommandHistoryState {
  /** The typed log, oldest first (commands and refusals). */
  readonly entries: readonly CommandHistoryEntry[];
  /** The derived command-only list: the one Up/Down and ranking may read. */
  readonly commands: readonly string[];
  /** Record one admitted command line. */
  readonly appendCommand: (text: string) => void;
  /** Record one refusal sentence — pass the sentence the builder produced. */
  readonly recordError: (text: string) => void;
  /** Drop the whole log (the panel's clear button). */
  readonly clear: () => void;
}

export function useCommandHistory(): CommandHistoryState {
  const [entries, setEntries] = useState<readonly CommandHistoryEntry[]>(EMPTY);
  const append = useCallback((text: string) => {
    setEntries(current => appendCommand(current, text));
  }, []);
  const recordError = useCallback((text: string) => {
    setEntries(current => appendError(current, text));
  }, []);
  const clear = useCallback(() => setEntries(EMPTY), []);
  // Memoized on the log: the suggestion ranking and the shortcut menu's "repeat"
  // read this list on every render.
  const commands = useMemo(() => commandOnly(entries), [entries]);
  return { entries, commands, appendCommand: append, recordError, clear };
}
