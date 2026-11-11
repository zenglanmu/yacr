// The View tab's interface panel: the chrome-owned visibility and window words.
//
// Every item dispatches a **command token**, not a local `action`: each name is
// in `LOCAL_CHROME_COMMANDS`, so the shared admission (`shell.canCommand`)
// enables it and `shell.runCommand` reaches the local selector table before
// `cad_command`. That keeps the ribbon on the same one funnel as the command
// line instead of adding a second dispatch path for chrome state.
//
// AutoCAD exposes these on its View tab (Windows / User Interface): RIBBON,
// FILETAB, HIDEPALETTES, QVLAYOUT, QUICKCALC and WORKSPACE. The closing words
// (RIBBONCLOSE, FILETABCLOSE, SHOWPALETTES, QVDRAWING, QVDRAWINGCLOSE,
// QVLAYOUTCLOSE, WSSAVE, WSSETTINGS) stay on the command line only, so the
// ribbon shows one live affordance per state instead of a wall of duplicates.
import type { RibbonPanel } from './ribbon-model.ts';

/** The View tab's interface panel (`ribbon.panel.interface`). */
export const INTERFACE_PANEL: RibbonPanel = Object.freeze({
  id: 'interface',
  titleKey: 'ribbon.panel.interface',
  items: Object.freeze([
    // Ribbon visibility: the show word. Its hide twin is the typed RIBBONCLOSE.
    { id: 'RIBBON', labelKey: 'draw.kind.ribbon', icon: 'menu', command: 'RIBBON' },
    // The quick-access toolbar strip: the show word; `-TOOLBAR HIDE` stays typed.
    { id: 'TOOLBAR', labelKey: 'draw.kind.toolbar', icon: 'app', command: 'TOOLBAR' },
    // The palette/clean-screen-like pair: hide all three regions in one state,
    // or restore them. Both are explicit sets.
    { id: 'HIDEPALETTES', labelKey: 'draw.kind.hidepalettes', icon: 'eyeOff', command: 'HIDEPALETTES' },
    // Drawing tabs: the strip exists in the single-document build.
    { id: 'FILETAB', labelKey: 'draw.kind.filetab', icon: 'app', command: 'FILETAB' },
    // Quick View Layouts is the layout switcher (the strip is the always-on
    // surface; this is the overview). Quick View Drawings is command-line only
    // in this panel because one drawing makes it a one-card overview.
    { id: 'QVLAYOUT', labelKey: 'draw.kind.qvlayout', icon: 'layers', command: 'QVLAYOUT' },
    // The four-function calculator dialog.
    { id: 'QUICKCALC', labelKey: 'draw.kind.quickcalc', icon: 'settings', command: 'QUICKCALC' },
    // Workspace presets: apply one; WSSAVE/WSSETTINGS stay typed because they
    // need a name or a list, not one click.
    { id: 'WORKSPACE', labelKey: 'draw.kind.workspace', icon: 'app', command: 'WORKSPACE' },
  ]),
});
