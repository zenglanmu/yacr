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
// word can never be read as a command plus an argument. The one declared
// exception is `-TOOLBAR`, whose own grammar is `-TOOLBAR [SHOW|HIDE]`: that row
// reads the argument it documents and nothing else, and every other argument is
// still a router line.
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

/** The typed word that shows the ribbon (a set, never a toggle). */
export const RIBBON_COMMAND = 'RIBBON';

/** The typed word that hides the ribbon (a set, never a toggle). */
export const RIBBONCLOSE_COMMAND = 'RIBBONCLOSE';

/** The typed word that hides ribbon + palettes + drawing tabs in one state. */
export const HIDEPALETTES_COMMAND = 'HIDEPALETTES';

/** The typed word that restores ribbon + palettes + drawing tabs. */
export const SHOWPALETTES_COMMAND = 'SHOWPALETTES';

/** The typed word that toggles the palette-chrome state (`PMTOGGLE`). */
export const PMTOGGLE_COMMAND = 'PMTOGGLE';

/** The typed word that opens the classic external-references panel. */
export const CLASSICXREF_COMMAND = 'CLASSICXREF';

/** The typed word that shows the drawing-tab strip (a set, never a toggle). */
export const FILETAB_COMMAND = 'FILETAB';

/** The typed word that hides the drawing-tab strip (a set, never a toggle). */
export const FILETABCLOSE_COMMAND = 'FILETABCLOSE';

/** The typed word that opens Quick View Drawings. */
export const QVDRAWING_COMMAND = 'QVDRAWING';

/** The typed word that closes Quick View Drawings (a no-op when already closed). */
export const QVDRAWINGCLOSE_COMMAND = 'QVDRAWINGCLOSE';

/** The typed word that opens Quick View Layouts. */
export const QVLAYOUT_COMMAND = 'QVLAYOUT';

/** The typed word that closes Quick View Layouts (a no-op when already closed). */
export const QVLAYOUTCLOSE_COMMAND = 'QVLAYOUTCLOSE';

/** The typed word that opens the QuickCalc dialog. */
export const QUICKCALC_COMMAND = 'QUICKCALC';

/** The typed word that closes QuickCalc (a no-op when already closed). */
export const QCCLOSE_COMMAND = 'QCCLOSE';

/** The typed word that refreshes the Quick View preview cards now. */
export const UPDATETHUMBSNOW_COMMAND = 'UPDATETHUMBSNOW';

/** The typed word that shows the quick-access toolbar strip. */
export const TOOLBAR_COMMAND = 'TOOLBAR';

/** The spelled-out show/hide form of the quick-access toolbar (accepts SHOW/HIDE). */
export const DASH_TOOLBAR_COMMAND = '-TOOLBAR';

/** The typed word that opens the workspace selector. */
export const WORKSPACE_COMMAND = 'WORKSPACE';

/** The typed word that saves the current UI state as a named workspace. */
export const WSSAVE_COMMAND = 'WSSAVE';

/** The typed word that opens the workspace settings dialog. */
export const WSSETTINGS_COMMAND = 'WSSETTINGS';


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

/** Whether typed text is the bare `RIBBON` selector. */
export function isRibbonCommand(text: unknown): boolean {
  return normalized(text) === RIBBON_COMMAND;
}

/** Whether typed text is the bare `RIBBONCLOSE` selector. */
export function isRibbonCloseCommand(text: unknown): boolean {
  return normalized(text) === RIBBONCLOSE_COMMAND;
}

/** Whether typed text is the bare `HIDEPALETTES` selector. */
export function isHidePalettesCommand(text: unknown): boolean {
  return normalized(text) === HIDEPALETTES_COMMAND;
}

/** Whether typed text is the bare `SHOWPALETTES` selector. */
export function isShowPalettesCommand(text: unknown): boolean {
  return normalized(text) === SHOWPALETTES_COMMAND;
}

/** Whether typed text is the bare `PMTOGGLE` selector. */
export function isPmToggleCommand(text: unknown): boolean {
  return normalized(text) === PMTOGGLE_COMMAND;
}

/** Whether typed text is the bare `CLASSICXREF` selector. */
export function isClassicXRefCommand(text: unknown): boolean {
  return normalized(text) === CLASSICXREF_COMMAND;
}

/** Whether typed text is the bare `FILETAB` selector. */
export function isFileTabCommand(text: unknown): boolean {
  return normalized(text) === FILETAB_COMMAND;
}

/** Whether typed text is the bare `FILETABCLOSE` selector. */
export function isFileTabCloseCommand(text: unknown): boolean {
  return normalized(text) === FILETABCLOSE_COMMAND;
}

/** Whether typed text is the bare `QVDRAWING` selector. */
export function isQuickViewDrawingCommand(text: unknown): boolean {
  return normalized(text) === QVDRAWING_COMMAND;
}

/** Whether typed text is the bare `QVDRAWINGCLOSE` selector. */
export function isQuickViewDrawingCloseCommand(text: unknown): boolean {
  return normalized(text) === QVDRAWINGCLOSE_COMMAND;
}

/** Whether typed text is the bare `QVLAYOUT` selector. */
export function isQuickViewLayoutCommand(text: unknown): boolean {
  return normalized(text) === QVLAYOUT_COMMAND;
}

/** Whether typed text is the bare `QVLAYOUTCLOSE` selector. */
export function isQuickViewLayoutCloseCommand(text: unknown): boolean {
  return normalized(text) === QVLAYOUTCLOSE_COMMAND;
}

/** Whether typed text is the bare `QUICKCALC` selector. */
export function isQuickCalcCommand(text: unknown): boolean {
  return normalized(text) === QUICKCALC_COMMAND;
}

/** Whether typed text is the bare `WORKSPACE` selector. */
export function isWorkspaceCommand(text: unknown): boolean {
  return normalized(text) === WORKSPACE_COMMAND;
}

/** Whether typed text is the bare `WSSAVE` selector. */
export function isWorkspaceSaveCommand(text: unknown): boolean {
  return normalized(text) === WSSAVE_COMMAND;
}

/** Whether typed text is the bare `WSSETTINGS` selector. */
export function isWorkspaceSettingsCommand(text: unknown): boolean {
  return normalized(text) === WSSETTINGS_COMMAND;
}

/** Whether typed text is the bare `QCCLOSE` selector. */
export function isQuickCalcCloseCommand(text: unknown): boolean {
  return normalized(text) === QCCLOSE_COMMAND;
}

/** Whether typed text is the bare `UPDATETHUMBSNOW` selector. */
export function isUpdateThumbsNowCommand(text: unknown): boolean {
  return normalized(text) === UPDATETHUMBSNOW_COMMAND;
}

/** Whether typed text is the bare `TOOLBAR` selector (it shows the strip). */
export function isToolbarCommand(text: unknown): boolean {
  return normalized(text) === TOOLBAR_COMMAND;
}

/**
 * The visibility `-TOOLBAR` asks for, or `null` when the line is not that form.
 *
 * `-TOOLBAR SHOW` shows the quick-access strip, `-TOOLBAR HIDE` hides it, and
 * the **bare** word shows it (the same idempotent "show" as `TOOLBAR`). Any
 * other argument is not this selector, so it falls through to the router line
 * rather than being read as a silent toggle.
 */
export function dashToolbarVisibility(text: unknown): boolean | null {
  const line = normalized(text).replace(/\s+/g, ' ');
  if (line === DASH_TOOLBAR_COMMAND || line === `${DASH_TOOLBAR_COMMAND} SHOW`) return true;
  if (line === `${DASH_TOOLBAR_COMMAND} HIDE`) return false;
  return null;
}

/** Whether typed text is a `-TOOLBAR` show/hide form. */
export function isDashToolbarCommand(text: unknown): boolean {
  return dashToolbarVisibility(text) !== null;
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
  { name: RIBBON_COMMAND, matches: isRibbonCommand },
  { name: RIBBONCLOSE_COMMAND, matches: isRibbonCloseCommand },
  { name: HIDEPALETTES_COMMAND, matches: isHidePalettesCommand },
  { name: SHOWPALETTES_COMMAND, matches: isShowPalettesCommand },
  { name: PMTOGGLE_COMMAND, matches: isPmToggleCommand },
  { name: CLASSICXREF_COMMAND, matches: isClassicXRefCommand },
  { name: FILETAB_COMMAND, matches: isFileTabCommand },
  { name: FILETABCLOSE_COMMAND, matches: isFileTabCloseCommand },
  { name: QVDRAWING_COMMAND, matches: isQuickViewDrawingCommand },
  { name: QVDRAWINGCLOSE_COMMAND, matches: isQuickViewDrawingCloseCommand },
  { name: QVLAYOUT_COMMAND, matches: isQuickViewLayoutCommand },
  { name: QVLAYOUTCLOSE_COMMAND, matches: isQuickViewLayoutCloseCommand },
  { name: QUICKCALC_COMMAND, matches: isQuickCalcCommand },
  { name: WORKSPACE_COMMAND, matches: isWorkspaceCommand },
  { name: WSSAVE_COMMAND, matches: isWorkspaceSaveCommand },
  { name: WSSETTINGS_COMMAND, matches: isWorkspaceSettingsCommand },
  { name: QCCLOSE_COMMAND, matches: isQuickCalcCloseCommand },
  { name: UPDATETHUMBSNOW_COMMAND, matches: isUpdateThumbsNowCommand },
  { name: TOOLBAR_COMMAND, matches: isToolbarCommand },
  { name: DASH_TOOLBAR_COMMAND, matches: isDashToolbarCommand },
]);

/** The bare names of [`CHROME_COMMANDS`], for exhaustive admission tests. */
export const CHROME_COMMAND_NAMES: readonly string[] = Object.freeze(
  CHROME_COMMANDS.map(selector => selector.name),
);
