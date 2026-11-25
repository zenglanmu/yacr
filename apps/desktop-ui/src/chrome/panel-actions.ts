// The ribbon's document-wide panel entries, as one action table.
//
// The `runAction` chain in `App.tsx` is the file closest to the code-size gate,
// so the entries that only open a module-store panel live here instead of adding
// one branch each. The stores are plain modules (no React), so this table has no
// hook dependency and a test can resolve an action without a DOM.
import { openBimPanel } from '../bim/panel.ts';
import { toggleSwitcher, toggleTray } from '../domains/store.ts';
import { openParamsPanel } from '../params/panel.ts';
import { openRecorderPanel } from '../recorder/panel.ts';

/** Shell action token → the panel it opens (a repeat press re-focuses). */
export const PANEL_ACTIONS: Readonly<Record<string, () => void>> = Object.freeze({
  bim: openBimPanel,
  parameters: openParamsPanel,
  actionRecorder: openRecorderPanel,
  // Not a palette: flips the domain tray's visibility (the reference apps'
  // Show/Hide Tray). It is a plain store transition, so it belongs here rather
  // than as another `runAction` branch in `App.tsx`.
  domainTray: toggleTray,
  // The application-domain switcher: one popover flag shared with the dock's
  // app button and the typed `DOMAIN` command. BIM/concept have no command line,
  // so their ribbon/menu entries are the in-profile path to it.
  domainSwitcher: toggleSwitcher,
});
