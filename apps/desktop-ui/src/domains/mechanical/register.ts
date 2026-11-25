// Register the mechanical profile's UI once at module load.
//
// Same contract as the BIM and concept profiles: the domain contributes **data**
// only (ribbon tabs, tray panels); every clickable row is a `RibbonItem` that
// passes the shared admission. `domains/chrome.tsx` imports this module for its
// side effect.
import { registerDomainUi } from '../ui/registry.ts';
import { MECHANICAL_RIBBON_TABS } from './ribbon.ts';
import { MECHANICAL_TRAY } from './tray.ts';

registerDomainUi('mechanical', {
  ribbon: MECHANICAL_RIBBON_TABS,
  tray: MECHANICAL_TRAY,
});
