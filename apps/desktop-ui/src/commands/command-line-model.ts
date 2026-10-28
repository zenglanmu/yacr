// The command line's decisions, as pure functions.
//
// Both presentations of the command line — the desktop docked region and the
// mobile bottom bar — must answer the same questions in the same way: what the
// field shows, when the suggestion list is offered, which row is the one Enter
// submits, what Up/Down does, and which rows the history sheet shows. Those
// answers live here so neither view can drift from the other, and so they are
// testable with plain Node (the hook that holds the React state is
// `use-command-line.ts`; it owns no rule of its own).
//
// The rules themselves are unchanged from the docked region they were extracted
// from: a text step has no suggestions, a typed argument belongs to the line
// (`ZOOM 2` is not replaced by `ZOOM`), Up/Down steps the suggestion list only
// while it is open and otherwise steps the history cursor, and Tab accepts the
// highlighted name.
import { filterCommandSuggestions, splitCommandText, stepHistoryCursor } from './commands.ts';
import type { CommandAlias, CommandSuggestion } from './commands.ts';
import { historyRows } from './command-history.ts';
import type { CommandHistoryEntry } from './command-history.ts';
import { TEXT_PROMPT_KEY } from '../tools/text.ts';

/** The catalog key the field's placeholder comes from. */
export function commandLinePlaceholderKey(textMode: boolean): string {
  return textMode ? TEXT_PROMPT_KEY : 'command.placeholder';
}

/**
 * The text the field displays.
 *
 * A text step keeps its own draft and never mirrors it into `shell.command`, so
 * the command line's content is still there when the tool ends.
 */
export function commandLineValue(textMode: boolean, command: string, textDraft: string): string {
  return textMode ? textDraft : command;
}

export interface CommandLineDisabledInput {
  readonly cadReady: boolean;
  readonly pending: boolean;
  readonly textMode: boolean;
}

/**
 * Whether the field is disabled.
 *
 * A queued command disables the field, **except** during a text step: disabling
 * it there would drop focus mid-edit and lose the user's text (the submit guard,
 * not the `disabled` attribute, handles a double Enter).
 */
export function commandLineDisabled({ cadReady, pending, textMode }: CommandLineDisabledInput): boolean {
  return !cadReady || (pending && !textMode);
}

export interface CommandLineListInput {
  readonly textMode: boolean;
  readonly focused: boolean;
  readonly command: string;
  readonly historyOpen: boolean;
}

/**
 * Whether the suggestion list is offered.
 *
 * Only while typing a command name (`focused`, non-empty) and while the history
 * panel is closed — the two lists never compete for the same space.
 */
export function commandLineListOpen({ textMode, focused, command, historyOpen }: CommandLineListInput): boolean {
  return !textMode && focused && command.trim().length > 0 && !historyOpen;
}

export interface CommandLineSuggestionInput {
  readonly textMode: boolean;
  readonly command: string;
  readonly history: readonly string[];
  readonly aliases: readonly CommandAlias[];
}

/** The suggestion rows for the current draft; none during a text step. */
export function commandLineSuggestions({ textMode, command, history, aliases }: CommandLineSuggestionInput): CommandSuggestion[] {
  if (textMode) return [];
  return filterCommandSuggestions(command, { history, aliases });
}

export interface CommandLineSubmitInput {
  readonly command: string;
  readonly highlight: number;
  readonly suggestions: readonly CommandSuggestion[];
}

/**
 * The line Enter submits.
 *
 * A highlighted completion row is a **name**; a typed argument belongs to the
 * line (`ZOOM 2`), so Enter keeps the full text instead of replacing it with the
 * bare name. Tab (the `accept` intent below) is the key that replaces the line.
 */
export function commandLineSubmitText({ command, highlight, suggestions }: CommandLineSubmitInput): string {
  if (command.trim() && splitCommandText(command).argument === null
    && highlight >= 0 && suggestions[highlight]) return suggestions[highlight].id;
  return command;
}

/** Whether the line has something to send right now. */
export function commandLineCanSubmit(textMode: boolean, command: string, textEmpty: boolean, textTooLong: boolean): boolean {
  return textMode ? !textEmpty && !textTooLong : command.trim().length > 0;
}

/** A stable, whitespace-free DOM id for one suggestion row. */
export function commandLineOptionId(suggestion: { readonly id: string }): string {
  return `completion-${suggestion.id.replace(/\s+/g, '-')}`;
}

/**
 * What ArrowUp/ArrowDown/Tab should do with the field's current state.
 *
 * A listed suggestion consumes the arrow keys; otherwise they step the history
 * cursor. Tab is an accept only while the list is open and a row is highlighted.
 * Every other key answers `null`, which means "not a navigation key" — the
 * caller then hands the press to the Enter/chat rule.
 */
export type CommandLineNavIntent =
  | { readonly kind: 'suggestion'; readonly delta: number }
  | { readonly kind: 'accept' }
  | { readonly kind: 'history'; readonly delta: number };

export interface CommandLineNavInput {
  readonly key: string;
  readonly listOpen: boolean;
  readonly suggestionCount: number;
  readonly highlight: number;
}

export function commandLineNavIntent({
  key, listOpen, suggestionCount, highlight,
}: CommandLineNavInput): CommandLineNavIntent | null {
  const delta = key === 'ArrowDown' ? 1 : key === 'ArrowUp' ? -1 : 0;
  if (delta !== 0) {
    return listOpen && suggestionCount > 0
      ? { kind: 'suggestion', delta }
      : { kind: 'history', delta };
  }
  if (key === 'Tab' && listOpen && suggestionCount > 0 && highlight >= 0 && highlight < suggestionCount) {
    return { kind: 'accept' };
  }
  return null;
}

export interface CommandLineHistoryInput {
  readonly history: readonly string[];
  readonly cursor: number | null;
  readonly delta: number;
  readonly draft: string;
}

/** One history step: the new cursor, the line it shows, and whether it moved. */
export interface CommandLineHistoryStep {
  readonly cursor: number | null;
  readonly text: string;
  readonly changed: boolean;
}

/** Step the history cursor (see `stepHistoryCursor`) and read the line it lands on. */
export function commandLineHistoryStep({ history, cursor, delta, draft }: CommandLineHistoryInput): CommandLineHistoryStep {
  const next = stepHistoryCursor(history, cursor, delta);
  return {
    cursor: next,
    text: next === null || next < 0 || next >= history.length ? draft : history[next],
    changed: next !== cursor,
  };
}

/**
 * Visible rows in the mobile sheet (newest first), commands and refusals alike.
 *
 * Capped so the upward sheet stays a sheet: a long session's 64-entry log would
 * otherwise run past the 45dvh cap and read as a wall. The oldest entries are
 * still in `shell.historyEntries` and remain step-able with Up/Down (the commands
 * among them); the sheet is a shortcut list, not the whole record.
 */
export const MOBILE_HISTORY_ROWS = 12;

export function mobileHistoryRows(
  entries: readonly CommandHistoryEntry[],
  limit: number = MOBILE_HISTORY_ROWS,
): CommandHistoryEntry[] {
  return historyRows(entries, limit);
}
