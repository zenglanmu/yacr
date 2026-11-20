// The file-level rows of the chrome's display catalog, split out of `catalog.ts`
// so neither module approaches the 1000-line code gate (the `catalog-tables.ts`
// precedent). This is a plain sibling module, not a barrel: `catalog.ts` spreads
// it in listing order.
//
// The file actions are the ones the host answers through `cad_status` capability
// booleans (`canNew`/`canSave`/`canPlot`), so this block is also the list a
// reader checks against `gating.ts`. `EXPORT` is a selector of its own since
// ADR-0041: the host opens an export picker and the chosen extension decides the
// writer, which is why there is no per-format row here.
import type { CommandCatalogEntry } from './catalog.ts';

/** OPEN … CLOSE: the file group, in the catalog's listing order. */
export const FILE_COMMAND_CATALOG: readonly CommandCatalogEntry[] = Object.freeze([
  { id: 'OPEN', labelKey: 'file.open', icon: 'open', group: 'file' },
  { id: 'NEW', labelKey: 'qat.new', icon: 'new', group: 'file' },
  { id: 'SAVE', labelKey: 'qat.save', icon: 'save', group: 'file' },
  // EXPORT is its own selector (ADR-0041): the host picker's extension decides
  // the writer, and it carries SAVE's `canSave` gate, not PLOT's.
  { id: 'EXPORT', labelKey: 'file.export', icon: 'export', group: 'file' },
  // PLOT renders the drawing to SVG/PDF through the platform save picker and
  // refuses `.png` for the GUI. `PRINT` stays its alias, gated by `canPlot`.
  { id: 'PLOT', labelKey: 'file.plot', icon: 'save', group: 'file' },
  // CLOSE ends the open drawing (document-scoped): on the start page it is
  // disabled with the catalog's own "no drawing is open" sentence.
  { id: 'CLOSE', labelKey: 'doctabs.close', icon: 'close', group: 'file' },
]);
