// The ribbon's document-wide panel entries, as one action table.
//
// The `runAction` chain in `App.tsx` is the file closest to the code-size gate,
// so the entries that only open a module-store panel live here instead of adding
// one branch each. Both stores are plain modules (no React), so this table has no
// hook dependency and a test can resolve an action without a DOM.
import { openBimPanel } from '../bim/panel.ts';
import { openParamsPanel } from '../params/panel.ts';

/** Shell action token → the panel it opens (a repeat press re-focuses). */
export const PANEL_ACTIONS: Readonly<Record<string, () => void>> = Object.freeze({
  bim: openBimPanel,
  parameters: openParamsPanel,
});
