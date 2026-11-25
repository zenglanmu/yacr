// The one application-domain switcher row every profile puts under File.
//
// The switcher surface itself (the popover) belongs to `DomainSwitcher` in the
// navigation dock. This module is only the discoverable **entry**, and it must
// read the same in every profile — so all three File surfaces reference this one
// panel instead of each declaring its own row:
//
//   * `cad2d` — the File tab, and the same tab's collapsed menubar;
//   * `bim`   — the accent File tab (`bim.file`);
//   * `concept` — the File menu.
//
// A one-row panel renders inline in a menu: `ribbonMenuEntriesFor` suppresses the
// caption when the only row already carries the panel's name, so the row sits
// directly under File instead of behind a redundant submenu.
import type { RibbonPanel } from './ribbon-model.ts';
import { SHELL_ACTIONS } from './actions.ts';

export const DOMAIN_PANEL: RibbonPanel = Object.freeze({
  id: 'domain',
  titleKey: 'domain.switcher_label',
  items: Object.freeze([
    {
      id: 'DOMAIN',
      labelKey: 'domain.switcher_label',
      icon: 'app',
      action: SHELL_ACTIONS.domainSwitcher,
    },
  ]),
});
