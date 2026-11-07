// Plain-text serialization of the command history, for clipboard copy.
//
// The expanded history has **one log and two presentations** (`CommandLine.tsx`
// on desktop, `MobileCommandBar.tsx` on mobile). Copying is a third reading of
// the same record, so it is derived here from the very rows each surface already
// renders — never from a second walk over `shell.historyEntries` or the host log,
// which is how the two views could start disagreeing about what was shown.
//
// Two serializers, one per presentation:
//
//   * `commandWindowText` / `entryText` — the desktop panel's merged rows: the
//     typed line, its attached host echo (label / value / unit / precision chip)
//     and refusal rows, **oldest first**, matching the panel's top-to-bottom
//     order.
//   * `commandHistoryText` — the mobile sheet's typed rows, **in the order the
//     sheet shows them** (newest first), one entry per line.
//
// Everything is pure and DOM-free, so the exact text a user would paste is
// asserted with `node --test` rather than eyeballed through a WebView.
//
// Indentation mirrors the panel: the command line starts at column 0, every
// attached output line is indented by two spaces — the copy's version of the
// panel's `22px` output margin.
import type { CommandHistoryEntry } from './command-history.ts';
import type { CommandLogLine } from './command-log.ts';
import type { CommandWindowRow } from './command-window.ts';
import type { Translator } from '../i18n.ts';

/** The two-space indent of an attached output line (the panel's output margin). */
export const HISTORY_OUTPUT_INDENT = '  ';

/** The key the desktop panel's prompt label comes from (`Command:`). */
export const HISTORY_PROMPT_KEY = 'command.log.prompt';

/**
 * One output line, without the indent: the panel's own joining rules.
 *
 *   label + (`" = "` for a value/delta/error tone, `" "` for a command tone)
 *         + text
 *         + (`" "` + unit when the host reported one)
 *         + meta
 *
 * The precision chip is a separate span with a 6px left margin in the panel, so
 * the text copy keeps one space in front of it — otherwise `5.0000 mm` and
 * `Approximate` would paste as `mmApproximate`, which is not what is on screen.
 */
export function commandLogLineText(line: CommandLogLine): string {
  let text = '';
  if (line.label) text += line.tone === 'command' ? `${line.label} ` : `${line.label} = `;
  text += line.text ?? '';
  if (line.unit) text += ` ${line.unit}`;
  if (line.meta) text += ` ${line.meta}`;
  return text;
}

/**
 * One desktop row: the command (or refusal sentence) plus its attached output.
 *
 * A command row reads `prompt text` (`Command: LINE`); a refusal is its own
 * sentence, verbatim — the same thing the red row shows and the same sentence
 * `commandMessageFor` wrote into the log. Output lines follow, indented.
 */
export function entryText(
  row: CommandWindowRow,
  t: Translator,
  prompt: string = t(HISTORY_PROMPT_KEY),
): string {
  const head = row.kind === 'command' ? `${prompt} ${row.text}` : row.text;
  const lines = row.lines.map(line => `${HISTORY_OUTPUT_INDENT}${commandLogLineText(line)}`);
  return [head, ...lines].join('\n');
}

/**
 * Every desktop row, oldest first — exactly the panel's top-to-bottom order.
 *
 * `rows` must be the array `commandWindowRows` produced for the panel, not a
 * re-derivation: this is the panel's visible record, output included.
 */
export function commandWindowText(
  rows: readonly CommandWindowRow[],
  t: Translator,
  prompt: string = t(HISTORY_PROMPT_KEY),
): string {
  if (!Array.isArray(rows)) return '';
  return rows.map(row => entryText(row, t, prompt)).join('\n');
}

/**
 * The mobile sheet's rows as plain text, **in the order they are shown**
 * (newest first), one line per entry — the sheet renders the typed log only, so
 * a line here is the entry's own text and nothing else.
 */
export function commandHistoryText(entries: readonly CommandHistoryEntry[]): string {
  if (!Array.isArray(entries)) return '';
  return entries
    .map(entry => String(entry?.text ?? ''))
    .filter(text => text !== '')
    .join('\n');
}

/** The key-event fields the copy chord reads (plain values, like `accelerators.ts`). */
export interface HistoryCopyKeyEvent {
  readonly key?: string;
  readonly ctrlKey?: boolean;
  readonly metaKey?: boolean;
  readonly altKey?: boolean;
  readonly shiftKey?: boolean;
  readonly isComposing?: boolean;
  readonly keyCode?: number;
}

/**
 * Whether a press is the history-copy chord: `Ctrl+C` (or `Cmd+C` on a Mac).
 *
 * Shift is **not** part of it: `Ctrl+Shift+C` is AutoCAD's `COPYBASE` in
 * `accelerators.ts`, and claiming it here would silently change what that chord
 * means. IME composition and `Alt`/`Meta` combinations are declined for the same
 * reason every other accelerator declines them — the press belongs to someone
 * else.
 */
export function isHistoryCopyChord(event: HistoryCopyKeyEvent = {}): boolean {
  if (event.isComposing === true || event.keyCode === 229) return false;
  if (event.altKey === true || event.shiftKey === true) return false;
  if (event.ctrlKey !== true && event.metaKey !== true) return false;
  return typeof event.key === 'string' && event.key.toLowerCase() === 'c';
}
