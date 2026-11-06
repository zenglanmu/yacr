// Chrome-local command-line selectors that reach no router command.
//
// The command line answers these words itself: typing one opens a chrome-owned
// surface (a window, panel or dialog) and the text is never forwarded to
// `cad_command`. This module owns only the **typed-word predicates**; the
// dispatch table lives in `commands/local-selectors.ts` and admission in
// `commands/gating.ts`.
//
// Append-only growth: a new selector is one more row at the end of
// [`CHROME_COMMANDS`], and its name lands in [`CHROME_COMMAND_NAMES`] without a
// second list to keep in sync. Predicates normalize exactly like
// `normalizeCommandText` (trim + upper-case) and match the bare word only, so a
// word can never be read as a command plus an argument.
//
// `ABOUT` opens the same dialog the File tab's About ribbon action opens (the
// one host-owned About window); this module only recognises the typed word.

/** The typed word that opens the existing About dialog. */
export const ABOUT_COMMAND = 'ABOUT';

/** One chrome-local selector: its canonical word and how typed text matches it. */
export interface ChromeCommandSelector {
  readonly name: string;
  readonly matches: (text: unknown) => boolean;
}

/** Normalize typed text the same way the command line does (trim + upper-case). */
function normalized(text: unknown): string {
  return String(text ?? '').trim().toUpperCase();
}

/** Whether typed text is the bare `ABOUT` selector (`about`, `  About ` included). */
export function isAboutCommand(text: unknown): boolean {
  return normalized(text) === ABOUT_COMMAND;
}

/**
 * Every chrome-local selector this module owns, in append order.
 *
 * A row is a name plus its exact-match predicate; adding a selector appends one
 * row (and, if the dispatch needs it, one `LocalSelectorResult` kind). The list
 * is the single source for [`CHROME_COMMAND_NAMES`], so a name and its matcher
 * can never drift apart.
 */
export const CHROME_COMMANDS: readonly ChromeCommandSelector[] = Object.freeze([
  { name: ABOUT_COMMAND, matches: isAboutCommand },
]);

/** The bare names of [`CHROME_COMMANDS`], for exhaustive admission tests. */
export const CHROME_COMMAND_NAMES: readonly string[] = Object.freeze(
  CHROME_COMMANDS.map(selector => selector.name),
);
