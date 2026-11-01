// The expanded command history: one ordered record that the popup renders.
//
// The popup is the **single surface** for what the command line did. It replaces
// the desktop echo, so there is no second place to look: a submitted line, the
// output the host echoed for it and a refusal the chrome itself produced all read
// as rows of this one list.
//
// Two sources meet here, and each owns exactly one fact:
//
//   * the **typed record** (`command-history.ts`) owns the rows and their order —
//     it is the log of submitted lines, including the client-only refusals the
//     host never saw, so nothing can reorder it or add a row ahead of it;
//   * the **host echo** (`command-log.ts`) owns the output only — the measurement,
//     point and error lines the host actually produced for an admitted command.
//
// An echo row is attached to a typed command only when the two texts name the same
// selector (`sameCommandLine`, alias and argument aware). An echo that matches
// nothing is **never invented as its own row**: its chronological position
// relative to the typed record is unknown, so guessing a slot would be a claim
// nobody made. A command whose echo has not arrived (queued, capped, alias table
// not loaded yet) keeps an empty `lines`, and the next call picks the echo up —
// the function is pure and stateless, so re-running it with a fuller log is the
// whole catch-up mechanism.
//
// A refusal row is copy, not a control: it has no recall text, never swallows a
// host echo and carries no output. Nothing here draws or stores anything.
import { commandLogView } from './command-log.ts';
import type { CommandLogLine, CommandLogRow } from './command-log.ts';
import type { CommandHistoryEntry } from './command-history.ts';
import { aliasTarget, splitCommandText } from './commands.ts';
import type { Translator } from '../i18n.ts';

/** One row of the expanded command history. */
export interface CommandWindowRow {
  readonly key: string;
  readonly kind: 'command' | 'error';
  /** The typed command text, or the refusal sentence. */
  readonly text: string;
  /** The text a click puts back in the field; `null` for a refusal row. */
  readonly recall: string | null;
  /** The host error code carried by an attached echo, for a `title`; else null. */
  readonly code: string | null;
  /** Host output lines echoed for this command, oldest first (empty when none). */
  readonly lines: readonly CommandLogLine[];
}

/**
 * The selector a host `command` row echoed, or `null` when it carries none.
 *
 * The formatted row keeps the raw command as its one `command`-tone line (see
 * `formatCommandLogEntry`), which is the name the match reads.
 */
function hostCommandName(row: CommandLogRow): string | null {
  if (row.kind !== 'command') return null;
  const line = row.lines.find(candidate => candidate.tone === 'command');
  return line && typeof line.text === 'string' && line.text !== '' ? line.text : null;
}

/**
 * Whether a typed line and a host echo name are the same selector.
 *
 * Both sides are normalized by `splitCommandText` (trim + upper-case), then the
 * host name is compared against the whole typed line, the typed line's base name
 * (when it had an argument, so `ZOOM 2` matches an echoed `ZOOM`) and the alias
 * target of either form in either direction (`L` matches an echoed `LINE` once the
 * host's alias table is loaded, and an echoed alias matches the canonical name).
 * A non-string or empty side is not a selector and matches nothing.
 */
export function sameCommandLine(clientText: unknown, hostName: unknown): boolean {
  if (typeof clientText !== 'string' || typeof hostName !== 'string') return false;
  const client = splitCommandText(clientText);
  const host = splitCommandText(hostName);
  if (client.text === '' || host.text === '') return false;
  if (host.text === client.text) return true;
  if (client.argument !== null && host.text === client.name) return true;
  const clientForms = client.argument === null ? [client.text] : [client.text, client.name];
  for (const form of clientForms) {
    if (aliasTarget(form) === host.text) return true;
  }
  const hostTarget = aliasTarget(host.text);
  return hostTarget !== null && clientForms.includes(hostTarget);
}

/**
 * The index of the next host `command` row at or after `from` that echoes `typed`,
 * or `null`. Host rows before a match are skipped, never attached: they belong to
 * an earlier command whose client row is outside the typed record.
 */
function findEchoRow(hostRows: readonly CommandLogRow[], from: number, typed: string): number | null {
  for (let index = from; index < hostRows.length; index += 1) {
    const name = hostCommandName(hostRows[index]);
    if (name !== null && sameCommandLine(typed, name)) return index;
  }
  return null;
}

/** The end (exclusive) of the echo group that follows the command row at `start`. */
function echoGroupEnd(hostRows: readonly CommandLogRow[], start: number): number {
  let end = start + 1;
  while (end < hostRows.length && hostRows[end].kind !== 'command') end += 1;
  return end;
}

/** The typed record in order, each command carrying its own echoed output. */
export function commandWindowRows(
  entries: readonly CommandHistoryEntry[],
  logEntries: unknown,
  t: Translator,
): CommandWindowRow[] {
  const hostRows = commandLogView(logEntries, t);
  const rows: CommandWindowRow[] = [];
  let cursor = 0;
  const source: readonly CommandHistoryEntry[] = Array.isArray(entries) ? entries : [];
  for (const [index, entry] of source.entries()) {
    const text = String(entry?.text ?? '');
    const key = `${index}:${entry?.kind}:${text}`;
    if (entry?.kind !== 'command') {
      // A refusal sentence: copy, never a control, and never a place to hang an
      // echo on — the pointer does not move for it.
      rows.push({ key, kind: 'error', text, recall: null, code: null, lines: [] });
      continue;
    }
    const match = findEchoRow(hostRows, cursor, text);
    if (match === null) {
      // Queued command, capped log or alias table not loaded yet: show the typed
      // row now and attach its output on a later call. The pointer stays put so
      // the echo cannot be claimed by a command submitted after it.
      rows.push({ key, kind: 'command', text, recall: text, code: null, lines: [] });
      continue;
    }
    const end = echoGroupEnd(hostRows, match);
    const attached = hostRows.slice(match + 1, end);
    const first = attached[0];
    rows.push({
      key,
      kind: 'command',
      text,
      recall: text,
      code: first && first.kind === 'error' ? first.code : null,
      lines: attached.flatMap(row => [...row.lines]),
    });
    cursor = end;
  }
  return rows;
}
