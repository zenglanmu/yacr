// The collapsed ribbon as a menu bar: the band's own model, folded into menus.
//
// When the ribbon is collapsed there is no band to hold the panels, so the tab
// names become top-level menus and each tab's panels become menu content. This
// module is the pure half of that: it reads the very same `RIBBON_TABS` the band
// renders and applies the very same carrier reduction (`offeredPanels` /
// `offeredItems`), so a command added to a panel appears in the menu with no
// second definition to maintain.
//
// Two rules turn a panel into menu structure:
//
//   * **Small panels stay inline.** A panel with fewer than
//     `RIBBON_MENU_SUBMENU_MIN` offered items renders as a titled section: the
//     panel title as a muted caption, then one row per item, with a separator
//     between sections. That is AutoCAD classic's File menu: a short list, no
//     extra click.
//   * **Big panels become a submenu.** A panel with `RIBBON_MENU_SUBMENU_MIN`
//     or more offered items would push the whole menu past the window, so it
//     collapses to one row (panel title plus a right chevron) that opens the
//     items as a flyout. The Home tab's Draw panel (28 items) is the case this
//     exists for.
//
// An item that already owns `variants` becomes a further submenu level (its
// variant rows), which is exactly the caret the band shows on the same item. No
// synthetic rows are invented: the variant list is used verbatim, so a split
// button whose parent is a refusal (`PEDIT`, `HATCHEDIT`) does not grow a menu
// row the panel deliberately does not have.
//
// Panels that expose a dialog launcher keep it only in the band: the launcher is
// a ribbon-band affordance (a small arrow in the panel's title row) and has no
// menu equivalent here, so this module never emits a row for it.
//
// Everything here is DOM-free and deterministic, so the menu contract is
// testable with plain Node.
import { RIBBON_TABS, offeredItems, offeredPanels, ribbonTab } from './ribbon-model.ts';
import type { RibbonItem, RibbonTab } from './ribbon-model.ts';
import { CONTEXT_MENU_MARGIN, placeContextMenu } from '../commands/context-menu.ts';
import type { MenuPlacement } from '../commands/context-menu.ts';

/**
 * Offered-item count at which a panel stops rendering inline and becomes a
 * submenu entry.
 *
 * Seven is the first count that no longer reads as a short list inside a single
 * dropdown: the band's narrowest inline panels (Clipboard, Navigate, Display)
 * top out at six, and the Draw panel's twenty-eight rows are the other end. The
 * number is exported so the renderer, the tests and the docs share one value.
 */
export const RIBBON_MENU_SUBMENU_MIN = 7;

/**
 * Pointer-intent delay before a hovered submenu row opens its flyout, in ms.
 *
 * Resting on a row is the Windows/AutoCAD way to open a submenu, but opening on
 * the first `mouseenter` would fire while the pointer is merely crossing the
 * list on its way somewhere else. The delay is short enough to feel immediate
 * once the pointer stops and long enough that a pass-through row never flashes
 * a flyout. Explicit activation (click, Enter, ArrowRight) bypasses it entirely
 * and opens at once.
 */
export const RIBBON_MENU_HOVER_OPEN_MS = 220;

/** One command row of a menu, with its own variant flyout (if any). */
export interface RibbonMenuRow {
  readonly kind: 'item';
  readonly id: string;
  readonly item: RibbonItem;
  /** The item's `variants`, one row each; `[]` when the item has none. */
  readonly children: readonly RibbonMenuRow[];
}

/** One panel's menu entry: an inline titled section or a submenu parent. */
export interface RibbonMenuEntry {
  /**
   * `'section'` renders inline (caption plus rows); `'panel'` renders as one
   * submenu row whose flyout holds `rows`. The decision is `rows.length`
   * against [`RIBBON_MENU_SUBMENU_MIN`], taken once here so the component never
   * re-derives it.
   */
  readonly kind: 'section' | 'panel';
  readonly id: string;
  readonly titleKey: string;
  /**
   * Whether an inline section prints its caption row. A section whose title is
   * the **same words as the menu's own trigger** would repeat the menu title
   * (the File tab's File panel is the case), so that caption is suppressed and
   * the rows render without one. The same applies when the section's **first
   * row already carries that name** (Home > Properties: a 属性 caption over a
   * 属性 row says nothing twice), so the caption is dropped there too and the
   * row speaks for the group. Submenu entries (`kind: 'panel'`) never print a
   * caption — their title is the row label — so they are always `false`.
   */
  readonly caption: boolean;
  readonly rows: readonly RibbonMenuRow[];
}

/** One item as a menu row, recursing through its variants. */
export function ribbonMenuRow(item: RibbonItem): RibbonMenuRow {
  const variants = Array.isArray(item.variants) ? item.variants : [];
  return { kind: 'item', id: item.id, item, children: variants.map(ribbonMenuRow) };
}

/**
 * Whether `count` offered items render inline or as a submenu entry.
 *
 * The boundary is inclusive on the submenu side: `RIBBON_MENU_SUBMENU_MIN - 1`
 * items render inline, `RIBBON_MENU_SUBMENU_MIN` items become a submenu.
 */
export function ribbonMenuEntryKind(count: number): 'section' | 'panel' {
  return Number(count) >= RIBBON_MENU_SUBMENU_MIN ? 'panel' : 'section';
}

/**
 * One tab's menu entries, in the ribbon's own panel order.
 *
 * `palettesAvailable` is the same carrier flag the band passes to
 * `offeredPanels`: a carrier that never renders the desktop palettes drops their
 * toggles, and a panel left with nothing is dropped whole (title included)
 * rather than becoming a titled empty section. Every panel that survives the
 * carrier rule survives here too — this is a reduction, not a curation, so a
 * command cannot quietly disappear from the collapsed ribbon while the expanded
 * ribbon still offers it.
 */
export function ribbonMenuEntriesFor(tab: RibbonTab, palettesAvailable?: boolean): RibbonMenuEntry[] {
  const entries: RibbonMenuEntry[] = [];
  for (const panel of offeredPanels(tab.panels, palettesAvailable)) {
    const items = offeredItems(panel.items, palettesAvailable);
    if (items.length === 0) continue;
    const kind = ribbonMenuEntryKind(items.length);
    entries.push({
      kind,
      id: panel.id,
      titleKey: panel.titleKey,
      // A section caption is dropped when it says nothing the menu has not
      // already said: when it repeats the trigger's own words (File > File), or
      // when its first row is already labelled with the panel title (Home >
      // Properties). The rows stay either way; only the header goes. A submenu
      // row carries its title as the row label instead.
      caption: kind === 'section'
        && panel.titleKey !== tab.labelKey
        && items[0].labelKey !== panel.titleKey,
      rows: items.map(ribbonMenuRow),
    });
  }
  return entries;
}

/**
 * Whether the entry at `index` opens a new visual group.
 *
 * A group is a section boundary, and only an **inline section** can be one: a
 * run of consecutive `panel` entries is a list of sibling submenus, so drawing
 * a hairline above each of them would box three ordinary rows into three
 * sections. The first entry never needs a rule (there is nothing above it), and
 * a panel that follows another panel keeps the block open.
 */
export function panelStartsGroup(entries: readonly RibbonMenuEntry[], index: number): boolean {
  const previous = index > 0 ? entries[index - 1] : undefined;
  return previous !== undefined && previous.kind === 'section';
}

/** The menu entries for `tabId` (an unknown id resolves like `ribbonTab`). */
export function ribbonMenuEntries(tabId: string, palettesAvailable?: boolean): RibbonMenuEntry[] {
  return ribbonMenuEntriesFor(ribbonTab(tabId), palettesAvailable);
}

/**
 * The menu entries of the tab at `index` in ribbon order.
 *
 * The collapsed menu bar walks `RIBBON_TABS` by index (one trigger per tab), so
 * it needs the same lookup without re-finding the tab by id.
 */
export function ribbonMenuEntriesAt(index: number, palettesAvailable?: boolean): RibbonMenuEntry[] {
  const tab = RIBBON_TABS[index];
  return tab ? ribbonMenuEntriesFor(tab, palettesAvailable) : [];
}

/** One row rectangle an overlay hangs from (a trigger or a parent menu row). */
export interface RibbonMenuAnchor {
  readonly left: number;
  readonly right: number;
  readonly top: number;
  readonly bottom: number;
}

export interface RibbonMenuSize {
  readonly width: number;
  readonly height: number;
}

export interface RibbonMenuViewport {
  readonly width: number;
  readonly height: number;
}

/**
 * The gap between a trigger's bottom edge and the top of its dropdown, in CSS px.
 *
 * One value for the placement and for the height budget: a dropdown that was
 * sized with one offset and painted with another would either flip or leave a
 * one-pixel gap, and the flip is exactly what must not happen.
 */
export const RIBBON_MENU_TRIGGER_GAP = 1;

/**
 * Smallest height a menu level may be given when the window is short.
 *
 * A floor keeps a level usable (and scrollable) instead of collapsing to a
 * sliver; below this the window is smaller than any sensible menu anyway.
 */
export const RIBBON_MENU_MIN_HEIGHT = 160;

/**
 * The surface inset a flyout's anchor compensates for, in CSS px.
 *
 * A menu surface (`.ribbon-menu`) paints a 1px border and a 4px padding inside
 * its own box, and a flyout's **first row** has to line up with the parent row
 * it belongs to — not the surface's top edge. Since the surface's top edge sits
 * just above that row, the flyout has to be lifted by exactly the inset to put
 * its first row back on the row's own baseline. Both numbers live in the
 * `.ribbon-menu` rule; this constant is their sum and the CSS contract test
 * keeps the two in step.
 */
export const RIBBON_MENU_SURFACE_INSET = 5;

/**
 * The anchor a flyout hangs from: the parent **surface's** horizontal edges,
 * the parent **row's** vertical position, lifted by the surface inset.
 *
 * A flyout must never overlap the menu it springs from. Anchoring it to the
 * row's right edge does exactly that: the row ends inside the parent's padding,
 * so the flyout would cover the parent's padding, its right border and its
 * shadow, and leave the parent's bottom-right corner open. The surface's outer
 * right edge is the only edge a sibling surface can abut: `left = surface.right`
 * is flush, the parent's border stays visible, and the flip rule measures from
 * `surface.left` for the same reason.
 *
 * The vertical half is the other half of the same rule: the flyout's top goes
 * to `row.top - RIBBON_MENU_SURFACE_INSET`, so the flyout's own border and
 * padding (the same inset again) put its first row level with the parent row.
 */
export function ribbonMenuFlyoutAnchor(row: RibbonMenuAnchor, surface: RibbonMenuAnchor): RibbonMenuAnchor {
  return {
    left: surface.left,
    right: surface.right,
    top: row.top - RIBBON_MENU_SURFACE_INSET,
    bottom: row.bottom - RIBBON_MENU_SURFACE_INSET,
  };
}

export interface PlaceRibbonMenuLevelInput {
  readonly anchor?: RibbonMenuAnchor;
  /** `0` is a top-level dropdown; `> 0` is a flyout beside its parent row. */
  readonly depth?: number;
  readonly size?: RibbonMenuSize;
  readonly viewport?: RibbonMenuViewport;
  readonly margin?: number;
}

/**
 * Where one menu level paints, in viewport CSS px.
 *
 * Both cases delegate the flip-then-clamp rule to `placeContextMenu`, so the
 * menu bar cannot drift from the shortcut menu's own geometry:
 *
 *   * **level 0** hangs from the trigger's bottom-left corner, one gap below
 *     the row;
 *   * **a flyout** opens flush to the parent menu **surface's** outer right
 *     edge. The caller hands in a surface-anchored rectangle
 *     (`ribbonMenuFlyoutAnchor`): the surface's horizontal edges, the parent
 *     row's vertical position and the surface inset already accounted for.
 *     When that side cannot hold it, the helper is asked again with its right
 *     edge on the surface's **left** edge (`x = anchor.left - width`), which
 *     opens the flyout beside the parent instead of back over it. Anchoring
 *     the retry at `anchor.left` (rather than `anchor.left - width`) would
 *     paint the flyout on top of the parent dropdown — the bug this rule
 *     exists to avoid.
 *
 * A level wider than the window is pinned to the margin and scrolls internally.
 */
export function placeRibbonMenuLevel({
  anchor,
  depth = 0,
  size,
  viewport,
  margin = CONTEXT_MENU_MARGIN,
}: PlaceRibbonMenuLevelInput = {}): MenuPlacement {
  const rect: RibbonMenuAnchor = anchor ?? { left: 0, right: 0, top: 0, bottom: 0 };
  const box: RibbonMenuSize = {
    width: Math.max(0, Number(size?.width) || 0),
    height: Math.max(0, Number(size?.height) || 0),
  };
  const view: RibbonMenuViewport = {
    width: Math.max(0, Number(viewport?.width) || 0),
    height: Math.max(0, Number(viewport?.height) || 0),
  };
  if (depth <= 0) {
    return placeContextMenu({
      anchor: { x: rect.left, y: rect.bottom + RIBBON_MENU_TRIGGER_GAP },
      size: box,
      viewport: view,
      margin,
    });
  }
  const beside = placeContextMenu({
    anchor: { x: rect.right, y: rect.top },
    size: box,
    viewport: view,
    margin,
  });
  // No flip and no clamp back over the row means the right edge is clear.
  if (beside.left >= rect.right) return beside;
  // The retry anchors the flyout so its **right** edge lands on the surface's
  // left edge: `x = anchor.left - width`. Anchoring at `anchor.left` would paint
  // the flyout over the parent dropdown.
  return placeContextMenu({
    anchor: { x: rect.left - box.width, y: rect.top },
    size: box,
    viewport: view,
    margin,
  });
}

export interface RibbonMenuMaxHeightInput {
  readonly anchor?: RibbonMenuAnchor;
  readonly depth?: number;
  readonly viewport?: RibbonMenuViewport;
  readonly margin?: number;
  readonly floor?: number;
}

/**
 * The inline `max-height` for one menu level, so a level **always opens
 * downward**.
 *
 * A menu sized to the space between its anchor and the window's bottom margin
 * can never trip `placeContextMenu`'s upward flip, which would otherwise paint a
 * tall dropdown over the title bar and the menu bar itself. The trigger gap is
 * part of the budget for a dropdown, because that is where its top edge lands.
 * A flyout is measured from the anchor it is given, which is the
 * surface-anchored rectangle (`ribbonMenuFlyoutAnchor`, already lifted by the
 * surface inset) — the same rectangle placement uses, so budget and paint
 * cannot disagree. The result has a floor so a short window still yields a
 * usable, scrolling menu.
 */
export function ribbonMenuMaxHeight({
  anchor,
  depth = 0,
  viewport,
  margin = CONTEXT_MENU_MARGIN,
  floor = RIBBON_MENU_MIN_HEIGHT,
}: RibbonMenuMaxHeightInput = {}): number {
  const rect: RibbonMenuAnchor = anchor ?? { left: 0, right: 0, top: 0, bottom: 0 };
  const height = Math.max(0, Number(viewport?.height) || 0);
  const top = depth <= 0 ? rect.bottom + RIBBON_MENU_TRIGGER_GAP : rect.top;
  return Math.max(Number(floor) || 0, height - top - margin);
}
