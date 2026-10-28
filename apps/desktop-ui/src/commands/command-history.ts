// The command line's history: one ordered log of what was typed and what the
// line refused.
//
// The history used to be a `string[]` of admitted commands only, which is why a
// refusal the user had just read under the field was nowhere in the record: the
// red line appeared and the next attempt wiped it. The log models both kinds, so
// the desktop panel and the mobile sheet can show one honest list:
//
//   * `kind: 'command'` — an admitted command line, the only thing Up/Down recalls;
//   * `kind: 'error'`   — the sentence the message line showed, verbatim.
//
// Two invariants keep it usable:
//
//   1. the log is bounded (`COMMAND_HISTORY_LIMIT`, oldest dropped from the
//      front), so a long session cannot grow it without limit;
//   2. an entry identical to the one before it *and of the same kind* is dropped,
//      so pressing Enter twice on the same refused line records one row, while a
//      command and an error that happen to read the same stay two rows.
//
// `commandOnly` is the derived recall list. It is derived, never stored: the
// Up/Down cursor and the suggestion ranking read commands only, so an error
// sentence can never be put into the field. Nothing here draws — the row markup
// and the colours are the presentations' business.

/** What one row of the log is: an admitted line, or a refusal sentence. */
export type CommandHistoryKind = 'command' | 'error';

/** One row of the log, oldest first in the array. */
export interface CommandHistoryEntry {
  readonly kind: CommandHistoryKind;
  readonly text: string;
}

/**
 * How many entries the log keeps.
 *
 * The same bound the string history always had: a session's 65th command drops
 * the first, and the panel's own scroller (`max-height`) is what keeps a long log
 * from becoming a wall — the cap is about memory and recall depth, not layout.
 */
export const COMMAND_HISTORY_LIMIT = 64;

function entryText(value: unknown): string {
  return String(value ?? '').trim();
}

function bounded(limit: number): number {
  return Number.isFinite(limit) ? Math.max(0, Math.trunc(limit)) : COMMAND_HISTORY_LIMIT;
}

/**
 * Append one entry: trimmed, empty dropped, an identical same-kind neighbour
 * dropped, and the oldest entries dropped past `limit`.
 *
 * The caller's array is never mutated; a rejected entry returns the very same
 * reference, so a `setState` with it bails out instead of re-rendering.
 */
export function appendHistory(
  entries: readonly CommandHistoryEntry[],
  entry: CommandHistoryEntry,
  limit: number = COMMAND_HISTORY_LIMIT,
): readonly CommandHistoryEntry[] {
  const text = entryText(entry.text);
  if (!text) return entries;
  const last = entries.at(-1);
  if (last && last.kind === entry.kind && last.text === text) return entries;
  const size = bounded(limit);
  if (size === 0) return [];
  const next = [...entries, Object.freeze({ kind: entry.kind, text })];
  return next.length > size ? next.slice(next.length - size) : next;
}

/** Append an admitted command line (`result.command`'s own text). */
export function appendCommand(
  entries: readonly CommandHistoryEntry[],
  text: unknown,
  limit: number = COMMAND_HISTORY_LIMIT,
): readonly CommandHistoryEntry[] {
  return appendHistory(entries, { kind: 'command', text: String(text ?? '') }, limit);
}

/**
 * Append the sentence the message line is showing.
 *
 * The caller passes the **built** sentence, not its inputs: `App` hands over the
 * `commandMessage` memo and the command line hands over the same string it puts
 * in `line.messageText`, so the record and the line cannot drift.
 */
export function appendError(
  entries: readonly CommandHistoryEntry[],
  text: unknown,
  limit: number = COMMAND_HISTORY_LIMIT,
): readonly CommandHistoryEntry[] {
  return appendHistory(entries, { kind: 'error', text: String(text ?? '') }, limit);
}

/** The command texts, oldest first: the only list Up/Down and ranking may read. */
export function commandOnly(entries: readonly CommandHistoryEntry[]): readonly string[] {
  return entries.filter(entry => entry.kind === 'command').map(entry => entry.text);
}

/**
 * The log newest-first, capped at `limit` rows — what both presentations render.
 *
 * Unlike the recall list this keeps the errors: the panel and the sheet are the
 * record, and a capped sheet is still the newest rows of that record.
 */
export function historyRows(
  entries: readonly CommandHistoryEntry[],
  limit: number = COMMAND_HISTORY_LIMIT,
): CommandHistoryEntry[] {
  const size = bounded(limit);
  if (size === 0 || entries.length === 0) return [];
  return [...entries].reverse().slice(0, size);
}
