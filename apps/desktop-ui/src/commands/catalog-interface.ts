// The 2026-11-12 interface wave: the chrome-owned UI selectors.
//
// All eighteen names are **chrome-local** (`LOCAL_CHROME_COMMANDS`): the command
// line answers them before `cad_command`, and the ribbon entries dispatch the
// same words through the same local selector table. None reaches the router and
// none needs a document. The first fourteen are the interface wave; `QCCLOSE`,
// `UPDATETHUMBSNOW`, `TOOLBAR` and `-TOOLBAR` are its amendment.
//
// They are appended last so no shipped catalog row moves. The rows reuse the
// existing glyphs: `menu`/`app` for the ribbon, toolbar and tab chrome,
// `eye`/`eyeOff` for the show/hide pair, `drawing`/`layers` for the two Quick
// View panels, `close` for their close words, `refresh` for the preview refresh,
// and `settings`/`save` for the calculator and workspace run.
//
// The old Wave 8/10 rows for RIBBON, FILETAB, FILETABCLOSE, HIDEPALETTES,
// SHOWPALETTES and RIBBONCLOSE were removed from their batches and are
// re-declared here with the same ids, labels and glyphs: they are no longer
// named refusals, so their hints now describe the real chrome state.
import type { CommandCatalogEntry } from './catalog.ts';

/** The interface-wave catalog rows, in declaration order. */
export const INTERFACE_COMMAND_CATALOG: readonly CommandCatalogEntry[] = Object.freeze([
  // Ribbon visibility: RIBBON shows it, RIBBONCLOSE hides it. Both set the one
  // `ChromeVisibility.ribbon` flag; neither toggles.
  { id: 'RIBBON', labelKey: 'draw.kind.ribbon', icon: 'menu', group: 'view' },
  { id: 'RIBBONCLOSE', labelKey: 'draw.kind.ribbonclose', icon: 'menu', group: 'view' },
  // The quick-access toolbar strip: TOOLBAR shows it, `-TOOLBAR SHOW|HIDE`
  // spells the same set explicitly. It is the title row's QAT, not a palette.
  { id: 'TOOLBAR', labelKey: 'draw.kind.toolbar', icon: 'app', group: 'view' },
  { id: '-TOOLBAR', labelKey: 'draw.kind.dashtoolbar', icon: 'app', group: 'view' },
  // The palette/clean-screen-like pair: HIDEPALETTES hides ribbon + palettes +
  // drawing tabs as one explicit state; SHOWPALETTES restores all three.
  { id: 'HIDEPALETTES', labelKey: 'draw.kind.hidepalettes', icon: 'eyeOff', group: 'view' },
  { id: 'SHOWPALETTES', labelKey: 'draw.kind.showpalettes', icon: 'eye', group: 'view' },
  // PMTOGGLE flips the same value between the show and hide states; two calls
  // restore the original state, and hiding closes the open palette windows.
  { id: 'PMTOGGLE', labelKey: 'draw.kind.pmtoggle', icon: 'eye', group: 'view' },
  // Drawing-tab strip visibility. The single-document build still has the strip
  // (Start + the open drawing), so this is a real show/hide pair.
  { id: 'FILETAB', labelKey: 'draw.kind.filetab', icon: 'app', group: 'view' },
  { id: 'FILETABCLOSE', labelKey: 'draw.kind.filetabclose', icon: 'app', group: 'view' },
  // Quick View: two floating panels plus their idempotent close words.
  { id: 'QVDRAWING', labelKey: 'draw.kind.qvdrawing', icon: 'drawing', group: 'view' },
  { id: 'QVDRAWINGCLOSE', labelKey: 'draw.kind.qvdrawingclose', icon: 'close', group: 'view' },
  { id: 'QVLAYOUT', labelKey: 'draw.kind.qvlayout', icon: 'layers', group: 'view' },
  { id: 'QVLAYOUTCLOSE', labelKey: 'draw.kind.qvlayoutclose', icon: 'close', group: 'view' },
  // QuickCalc is a four-function calculator: no engineering/trigonometric
  // functions exist in this build, and the catalog hint says so.
  { id: 'QUICKCALC', labelKey: 'draw.kind.quickcalc', icon: 'settings', group: 'properties' },
  { id: 'QCCLOSE', labelKey: 'draw.kind.qcclose', icon: 'close', group: 'properties' },
  // Refresh the Quick View preview cards in place. Nothing is written to disk;
  // the vector previews are recomputed from the current session.
  { id: 'UPDATETHUMBSNOW', labelKey: 'draw.kind.updatethumbsnow', icon: 'refresh', group: 'view' },
  // Workspace presets: apply (`WORKSPACE`), save current (`WSSAVE`), manage
  // (`WSSETTINGS`). Session in-memory only; nothing is written to a file.
  { id: 'WORKSPACE', labelKey: 'draw.kind.workspace', icon: 'app', group: 'view' },
  { id: 'WSSAVE', labelKey: 'draw.kind.wssave', icon: 'save', group: 'view' },
  { id: 'WSSETTINGS', labelKey: 'draw.kind.wssettings', icon: 'settings', group: 'view' },
]);
