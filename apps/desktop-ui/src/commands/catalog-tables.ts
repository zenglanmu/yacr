// The table command group of the chrome's display catalog, split out of
// `catalog.ts` so neither module approaches the 1000-line code gate. This is a
// plain sibling module, not a barrel: `catalog.ts` spreads it in listing order.

import type { CommandCatalogEntry } from './catalog.ts';

/** `TABLE` … `MATCHCELL`, in the catalog's annotate order. */
export const TABLE_COMMAND_CATALOG: readonly CommandCatalogEntry[] = Object.freeze([
  { id: 'TABLE', labelKey: 'draw.kind.table', icon: 'properties', group: 'annotate' },
  { id: '-TABLE', labelKey: 'draw.kind.dashtable', icon: 'properties', group: 'annotate' },
  { id: 'TABLEDIT', labelKey: 'draw.kind.tabledit', icon: 'properties', group: 'annotate' },
  { id: 'TABLEEXPORT', labelKey: 'draw.kind.tableexport', icon: 'save', group: 'annotate' },
  { id: 'TABLESTYLE', labelKey: 'draw.kind.tablestyle', icon: 'properties', group: 'annotate' },
  { id: 'TINSERT', labelKey: 'draw.kind.tinsert', icon: 'properties', group: 'annotate' },
  { id: 'MATCHCELL', labelKey: 'draw.kind.matchcell', icon: 'matchprop', group: 'annotate' },
]);
