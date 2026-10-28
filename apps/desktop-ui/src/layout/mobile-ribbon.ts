// The mobile ribbon sheet: the desktop ribbon model, folded for a touch panel.
//
// The drawer is **not** a second ribbon definition. It reads the same
// `RIBBON_TABS` the desktop band renders and applies the same carrier filter
// (`offeredPanels` / `offeredItems`), so a command added to a panel appears in
// both places and a carrier rule can never disagree with itself.
//
// Two deliberate reductions, both of them honesty rules rather than styling:
//
//   * **No docked palettes.** The mobile layout never renders the 300px dock
//     (`Workspace`'s left column and the nav rail are hidden by CSS, and no
//     mobile surface re-opens them), so the drawer asks `offeredPanels` with
//     `palettesAvailable: false`. That is the exact rule a `dom-canvas` carrier
//     already uses: a toggle for a panel that is never rendered is a dead
//     affordance, so it is dropped with the panel instead of being shown.
//   * **No chrome-mode switches.** `cleanScreen` and `commandLine` collapse
//     desktop chrome regions. The mobile layout has no ribbon band, no status
//     bar and no docking, and its top bar is the only way back to the menu, so
//     neither switch has an effect worth offering. They are dropped here only;
//     the desktop ribbon keeps both.
import { RIBBON_TABS, SHELL_ACTIONS, offeredItems, offeredPanels, ribbonTab } from '../ribbon/ribbon-model.ts';
import type { RibbonItem, RibbonTab } from '../ribbon/ribbon-model.ts';

/**
 * Shell actions that the mobile layout does not offer.
 *
 * Both are chrome-collapse switches with nothing to collapse on a phone (see the
 * module comment). Every other shell action (`about`, `options`, `diagnostics`,
 * `properties`, `language`) stays in the drawer and keeps its desktop meaning —
 * `diagnostics`/`properties` are removed by the palette rule above, not here.
 */
export const MOBILE_HIDDEN_ACTIONS: readonly string[] = Object.freeze([
  SHELL_ACTIONS.cleanScreen,
  SHELL_ACTIONS.commandLine,
]);

/** One titled group of the drawer: a desktop ribbon panel, minus dropped items. */
export interface MobileRibbonSection {
  readonly id: string;
  readonly titleKey: string;
  readonly items: readonly RibbonItem[];
}

/** Whether `item` survives the mobile reduction (see the module comment). */
function mobileItem(item: RibbonItem): boolean {
  return !(typeof item.action === 'string' && MOBILE_HIDDEN_ACTIONS.includes(item.action));
}

/**
 * One tab's drawer sections, in the ribbon's own order.
 *
 * `palettesAvailable` is the same carrier flag the band passes to
 * `offeredPanels`; the drawer passes `false` because the compact layout never
 * renders the 300px dock, whatever the host reports (see the module comment).
 *
 * A panel that loses every item is dropped whole (title included), exactly like
 * `offeredPanels` drops a palette panel on a carrier that has no palette: a
 * titled empty box would claim a group that has nothing to offer.
 */
export function mobileRibbonSectionsFor(tab: RibbonTab, palettesAvailable = false): MobileRibbonSection[] {
  const sections: MobileRibbonSection[] = [];
  for (const panel of offeredPanels(tab.panels, palettesAvailable)) {
    const items = offeredItems(panel.items, palettesAvailable).filter(mobileItem);
    if (items.length > 0) sections.push({ id: panel.id, titleKey: panel.titleKey, items });
  }
  return sections;
}

/** The drawer sections for `tabId` (an unknown id resolves like `ribbonTab`). */
export function mobileRibbonSections(tabId: string, palettesAvailable = false): MobileRibbonSection[] {
  return mobileRibbonSectionsFor(ribbonTab(tabId), palettesAvailable);
}

/**
 * The drawer's tab chips, in ribbon order.
 *
 * Exposed as a function so the component and the tests read one list: the chips
 * are `RIBBON_TABS` itself, never a hand-written subset that could lose a tab.
 */
export function mobileRibbonTabs(): readonly RibbonTab[] {
  return RIBBON_TABS;
}

/**
 * A whitespace-free DOM id for one drawer item's expanded option list.
 *
 * Two ribbon ids contain a space (`CHAMFER POLYLINE`, `ZOOM WINDOW`), and an
 * `id`/`aria-controls` pair must not: `aria-controls` is matched as a token
 * list, so a space would name two ids and neither would be found.
 */
export function mobileRibbonDomId(itemId: string): string {
  return `mobile-ribbon-${String(itemId).trim().replace(/\s+/g, '-')}`;
}

/**
 * The one refusal a whole panel shares, or `null`.
 *
 * With no drawing open every document-scoped tool in a panel is refused for the
 * same reason ("No drawing is open"), and repeating that reason under twenty
 * cells is noise, not information. When **every** item is refused for the same
 * reason the drawer prints one note under the panel title; a panel with any
 * enabled item, or with different reasons, keeps the per-cell copy where it
 * belongs.
 *
 * The input is a per-item **reason identifier** (a catalog key, `null` =
 * enabled), not a rendered sentence — and that is the whole point. The rendered
 * sentence names the command it belongs to ("Command `COPY` is not supported
 * yet."), so comparing sentences never matches even when the reason is one and
 * the same: on the web carrier, whose command vocabulary is empty, that used to
 * print 37 different near-identical sub-labels instead of one panel note. Keys
 * compare equal; sentences do not.
 *
 * The helper reads no shell state and invents no sentence.
 */
export function sharedRefusal(reasons: readonly (string | null)[]): string | null {
  if (reasons.length === 0) return null;
  const [first] = reasons;
  if (first === null) return null;
  return reasons.every(reason => reason === first) ? first : null;
}

/**
 * Panel-level wording for a reason every item in a panel shares.
 *
 * A shared reason still has to be said **without** naming one command: the
 * catalog's own sentence carries a `{command}` placeholder
 * (`command.unsupported` -> "Command {command} is not supported yet."), which
 * would be a lie for a whole panel. The reasons a whole panel can really share
 * have a command-free sentence of their own; `plot.no_document`,
 * `view.standard_unavailable` and `linux.busy` already read that way and are
 * reused verbatim, and only `command.unsupported` needs new words.
 *
 * A reason that is not in this table returns `null`: an unknown shared reason
 * falls back to the per-cell sentences rather than printing a key or inventing
 * copy for a state the catalog does not describe.
 */
const PANEL_REFUSAL_KEY: Readonly<Record<string, string>> = Object.freeze({
  'command.unsupported': 'mobile.refusal.unsupported',
  'plot.no_document': 'plot.no_document',
  'view.standard_unavailable': 'view.standard_unavailable',
  'linux.busy': 'linux.busy',
});

/** The catalog key for a shared reason's panel note, or `null` (see above). */
export function panelRefusalKey(reasonKey: string | null): string | null {
  if (typeof reasonKey !== 'string') return null;
  return PANEL_REFUSAL_KEY[reasonKey] ?? null;
}
