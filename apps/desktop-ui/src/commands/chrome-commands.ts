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
// `CLEANSCREENON` turns on the same clean-screen state the File tab's toggle
// flips — it sets it, never toggles it. `CLEANSCREENOFF` leaves that same state
// the same way: an explicit set, never a toggle. `PROPERTIESCLOSE` closes the
// properties window the palette's own close button closes; closing an already
// closed window is an idempotent success, never a refusal. `LAYERCLOSE` is its
// layers-palette twin: the palette's close control and the typed word close the
// same window, and a second close is still a success.

/** The typed word that opens the existing About dialog. */
export const ABOUT_COMMAND = 'ABOUT';

/** The typed word that enters clean-screen mode (sets it, never toggles it). */
export const CLEANSCREENON_COMMAND = 'CLEANSCREENON';

/** The typed word that leaves clean-screen mode (sets it, never toggles it). */
export const CLEANSCREENOFF_COMMAND = 'CLEANSCREENOFF';

/** The typed word that closes the properties window (a no-op when already closed). */
export const PROPERTIESCLOSE_COMMAND = 'PROPERTIESCLOSE';

/** The typed word that closes the layers palette (a no-op when already closed). */
export const LAYERCLOSE_COMMAND = 'LAYERCLOSE';

/** The typed word that opens the existing Options dialog (Drafting settings). */
export const DSETTINGS_COMMAND = 'DSETTINGS';

/** The typed word that opens the existing properties palette. */
export const QUICKPROPERTIES_COMMAND = 'QUICKPROPERTIES';

/** The typed word that opens the existing layers palette. */
export const CLASSICLAYER_COMMAND = 'CLASSICLAYER';

/** The typed word that shows and focuses the command line. */
export const COMMANDLINE_COMMAND = 'COMMANDLINE';

/** The typed word that hides the command line. */
export const COMMANDLINEHIDE_COMMAND = 'COMMANDLINEHIDE';

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

/** Whether typed text is the bare `CLEANSCREENON` selector (`cleanscreenon` included). */
export function isCleanScreenOnCommand(text: unknown): boolean {
  return normalized(text) === CLEANSCREENON_COMMAND;
}

/** Whether typed text is the bare `CLEANSCREENOFF` selector (`cleanscreenoff` included). */
export function isCleanScreenOffCommand(text: unknown): boolean {
  return normalized(text) === CLEANSCREENOFF_COMMAND;
}

/** Whether typed text is the bare `PROPERTIESCLOSE` selector (`propertiesclose` included). */
export function isPropertiesCloseCommand(text: unknown): boolean {
  return normalized(text) === PROPERTIESCLOSE_COMMAND;
}

/** Whether typed text is the bare `LAYERCLOSE` selector (`layerclose` included). */
export function isLayerCloseCommand(text: unknown): boolean {
  return normalized(text) === LAYERCLOSE_COMMAND;
}

/** Whether typed text is the bare `DSETTINGS` selector. */
export function isDSettingsCommand(text: unknown): boolean {
  return normalized(text) === DSETTINGS_COMMAND;
}

/** Whether typed text is the bare `QUICKPROPERTIES` selector. */
export function isQuickPropertiesCommand(text: unknown): boolean {
  return normalized(text) === QUICKPROPERTIES_COMMAND;
}

/** Whether typed text is the bare `CLASSICLAYER` selector. */
export function isClassicLayerCommand(text: unknown): boolean {
  return normalized(text) === CLASSICLAYER_COMMAND;
}

/** Whether typed text is the bare `COMMANDLINE` selector. */
export function isCommandLineCommand(text: unknown): boolean {
  return normalized(text) === COMMANDLINE_COMMAND;
}

/** Whether typed text is the bare `COMMANDLINEHIDE` selector. */
export function isCommandLineHideCommand(text: unknown): boolean {
  return normalized(text) === COMMANDLINEHIDE_COMMAND;
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
  { name: CLEANSCREENON_COMMAND, matches: isCleanScreenOnCommand },
  { name: CLEANSCREENOFF_COMMAND, matches: isCleanScreenOffCommand },
  { name: PROPERTIESCLOSE_COMMAND, matches: isPropertiesCloseCommand },
  { name: LAYERCLOSE_COMMAND, matches: isLayerCloseCommand },
  { name: DSETTINGS_COMMAND, matches: isDSettingsCommand },
  { name: QUICKPROPERTIES_COMMAND, matches: isQuickPropertiesCommand },
  { name: CLASSICLAYER_COMMAND, matches: isClassicLayerCommand },
  { name: COMMANDLINE_COMMAND, matches: isCommandLineCommand },
  { name: COMMANDLINEHIDE_COMMAND, matches: isCommandLineHideCommand },
]);

/** The bare names of [`CHROME_COMMANDS`], for exhaustive admission tests. */
export const CHROME_COMMAND_NAMES: readonly string[] = Object.freeze(
  CHROME_COMMANDS.map(selector => selector.name),
);
