// Register the BIM profile's UI once at module load.
//
// Same contract as the concept profile: the domain contributes **data** only
// (ribbon tabs, tray panels), and every clickable row is still a `RibbonItem`
// that passes the shared admission. `domains/chrome.tsx` imports this module for
// its side effect.
import { registerDomainUi } from '../ui/registry.ts';
import { BIM_RIBBON_TABS } from './ribbon.ts';
import { BIM_TRAY } from './tray.ts';

registerDomainUi('bim', {
  ribbon: BIM_RIBBON_TABS,
  tray: BIM_TRAY,
});
