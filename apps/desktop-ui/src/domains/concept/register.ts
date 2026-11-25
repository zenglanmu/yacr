// Register the concept profile's UI once at module load.
//
// `domains/chrome.tsx` imports this module for its side effect, next to the
// concept components. Keeping registration in one tiny module means a test or a
// carrier build can import the layout data without mounting React, and a future
// domain does the same (`registerDomainUi('<id>', {...})`).
import { registerDomainUi } from '../ui/registry.ts';
import { CONCEPT_MENUS } from './menus.ts';
import { CONCEPT_TOOLBAR } from './toolbar.ts';
import { CONCEPT_TRAY } from './tray.ts';

registerDomainUi('concept', {
  menu: CONCEPT_MENUS,
  toolbar: CONCEPT_TOOLBAR,
  tray: CONCEPT_TRAY,
});
