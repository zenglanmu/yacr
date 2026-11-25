// Declarative ribbon definition. Pure data (no React), so both the renderer and
// the tests can walk it. An item carries exactly one dispatch token:
//
//   * `command` — a payload-free name admitted through `cad_command`;
//   * `standardView` — one of the seven host view keys, which carries a payload
//     and therefore travels on the dedicated typed IPC `cad_standard_view`
//     (never on `cad_command`);
//   * `action` — a DOM-only shell action that never touches the CAD protocol.
//
// An item with none of the three is labelled but disabled, which is the honest
// state for a command the host does not wire yet.
import type { CadStatusFlagKey } from './ribbon-active.ts';
import type { Translator } from '../i18n.ts';

import { PALETTE_ACTIONS, PALETTE_COMMANDS } from './actions.ts';

export { PALETTE_ACTIONS, PALETTE_COMMANDS, SHEET_PALETTE_ACTIONS, SHELL_ACTIONS } from './actions.ts';

/** The facts every ribbon item carries. */
interface RibbonItemBase {
  readonly id: string;
  readonly labelKey: string;
  /**
   * Literal display text supplied by a plugin contribution.
   *
   * Host-owned items use the catalog (`labelKey`); a plugin cannot add rows to
   * that catalog, so it hands over finished text instead. When present this wins
   * over the catalog lookup, and `labelKey` stays a stable synthetic identifier
   * (never shown). See `docs/domain-ui-slots.md`.
   */
  readonly label?: string;
  readonly icon: string;
  readonly large?: boolean;
  /**
   * The item's own limit, in catalog words, shown as its tooltip **while it is
   * enabled** (`zoom_window.hint` says "2D only", the previous-view form says
   * what is not recorded). It is not the refusal sentence: a disabled item still
   * words its reason through `unavailableReasonKey`, so one state never borrows
   * the other's copy. Same convention as `layers.<flag>.hint` and
   * `status.tool_parameter.*.hint`.
   */
  readonly hintKey?: string;
  /**
   * The `cad_status` boolean this item is a **toggle** for, when it is one: the
   * renderer reads `shell.status[key]`, never a local mirror. See
   * `ribbon-active.ts` for the key set and the pressed-state helpers.
   */
  readonly activeKey?: CadStatusFlagKey;
  /** The item's own popover (a small caret); its entries are items too. */
  readonly variants?: readonly RibbonItem[];
  /**
   * A stable catalog sentence for an item this build **cannot** run (no
   * dispatch token). It is the honest "not wired" state: the item is labelled
   * and disabled, never a fake button, and the sentence names the missing
   * capability instead of the generic refusal. A live item ignores it.
   */
  readonly pendingKey?: string;
  /**
   * Literal refusal sentence for a plugin-supplied pending item. Wins over
   * `pendingKey`'s catalog lookup exactly like `label` wins over `labelKey`.
   */
  readonly pendingText?: string;
}

/** One item's dispatch token: exactly one of the four families. */
export type RibbonItemKind = 'command' | 'standardView' | 'setView' | 'action';

/** A payload-free router command, admitted through `cad_command`. */
export interface RibbonCommandItem extends RibbonItemBase {
  readonly command: string;
  readonly standardView?: never;
  readonly viewPreset?: never;
  readonly action?: never;
}

/** One of the seven standard views, sent on `cad_standard_view { view }`. */
export interface RibbonStandardViewItem extends RibbonItemBase {
  readonly standardView: string;
  readonly command?: never;
  readonly viewPreset?: never;
  readonly action?: never;
}

/**
 * One fitted camera placement, sent on `cad_set_view { preset }`.
 *
 * The host resolves the preset against the active-space bounds; the chrome never
 * knows those bounds. `overview`/`front`/`back`/`left`/`right` are 3D
 * observations, `plan` is the fitted 2D top view.
 */
export interface RibbonSetViewItem extends RibbonItemBase {
  readonly viewPreset: string;
  readonly command?: never;
  readonly standardView?: never;
  readonly action?: never;
}

/** A DOM-only shell action that never touches the CAD protocol. */
export interface RibbonActionItem extends RibbonItemBase {
  readonly action: string;
  readonly command?: never;
  readonly standardView?: never;
  readonly viewPreset?: never;
}

/**
 * A labelled item this build deliberately does not wire.
 *
 * It carries **no** dispatch token, so `itemEnabled` is false and running it is
 * a no-op; `pendingKey` supplies the refusal sentence. This is the shape a
 * domain/plugin uses to keep a reference application's tool in place without
 * pretending it exists.
 */
export interface RibbonPendingItem extends RibbonItemBase {
  readonly command?: never;
  readonly standardView?: never;
  readonly viewPreset?: never;
  readonly action?: never;
  readonly pendingKey: string;
}

/**
 * One ribbon button.
 *
 * The four dispatch tokens are a union rather than optional fields: an item that
 * carried two of them would have no defined order, and the dispatch checks one
 * family at a time.
 */
export type RibbonItem =
  | RibbonCommandItem
  | RibbonStandardViewItem
  | RibbonSetViewItem
  | RibbonActionItem
  | RibbonPendingItem;

/** One panel of a tab. */
export interface RibbonPanel {
  readonly id: string;
  readonly titleKey: string;
  /** Literal title from a plugin contribution; wins over `titleKey`. */
  readonly title?: string;
  readonly items: readonly RibbonItem[];
  /** The panel's dialog-launcher arrow exists (disabled when it has no target). */
  readonly launcher?: boolean;
  /** The chrome-local selector the launcher arrow runs (see [`panelLauncherCommand`]). */
  readonly launcherCommand?: string;
  /** Small-item grid rows (column-major); the layout contract is data. */
  readonly rows?: number;
  /**
   * An explicit "not wired in this build" panel (ADR-0053 D5/D7).
   *
   * It keeps the reference panel's name and place and renders a note instead of
   * pretending the command exists. `items` is empty by contract; a pending panel
   * never carries a fake button.
   */
  readonly pending?: boolean;
}

/** One tab of the ribbon. */
export interface RibbonTab {
  readonly id: string;
  readonly labelKey: string;
  /** Literal trigger label from a plugin contribution; wins over `labelKey`. */
  readonly label?: string;
  readonly accent?: boolean;
  readonly panels: readonly RibbonPanel[];
}

/**
 * The display text of one item.
 *
 * A plugin contribution supplies literal text because it cannot add catalog
 * rows; every host-owned item keeps resolving through the catalog. The synthetic
 * `labelKey` a plugin ships is never displayed.
 */
export function itemLabel(t: Translator, item: { readonly label?: string; readonly labelKey: string }): string {
  return item.label ?? t(item.labelKey);
}

/** The display title of one panel (see [`itemLabel`]). */
export function panelTitle(t: Translator, panel: { readonly title?: string; readonly titleKey: string }): string {
  return panel.title ?? t(panel.titleKey);
}

/** The display label of one tab (see [`itemLabel`]). */
export function tabLabel(t: Translator, tab: { readonly label?: string; readonly labelKey: string }): string {
  return tab.label ?? t(tab.labelKey);
}

/** The refusal sentence of a pending item (see [`itemLabel`]). */
export function pendingLabel(
  t: Translator,
  item: { readonly pendingText?: string; readonly pendingKey?: string },
): string {
  return item.pendingText ?? t(item.pendingKey ?? '');
}

/**
 * The items a panel actually offers on the reported carrier.
 *
 * A desktop palette the carrier does not have must not be offered anywhere: the
 * status-bar chips are hidden, and so are these ribbon entries, because a button
 * that toggles a panel which is never rendered is a dead affordance. Only the
 * palette toggles are filtered; every other item is returned as the very same
 * array, so a carrier with the palettes (`native-below`, and any chrome state
 * without the flag) renders exactly the ribbon it always did.
 *
 * `sheetActions` is the one exemption (see [`SHEET_PALETTE_ACTIONS`]): palette
 * actions with a compact surface survive `palettesAvailable === false`.
 */
export function offeredItems(
  items: readonly RibbonItem[],
  palettesAvailable: boolean | undefined,
  sheetActions: readonly string[] = [],
): readonly RibbonItem[] {
  if (palettesAvailable !== false) return items;
  return items.filter(item => {
    if (typeof item.action === 'string' && sheetActions.includes(item.action)) return true;
    if (typeof item.action === 'string' && PALETTE_ACTIONS.includes(item.action)) return false;
    if (typeof item.command === 'string' && PALETTE_COMMANDS.includes(item.command)) return false;
    return true;
  });
}

/**
 * The panels a tab actually offers on the reported carrier.
 *
 * Filtering the items is not enough: the Home tab's "Layers" and "Properties"
 * panels hold nothing but palette toggles, so on a carrier without the palettes
 * they would render as empty titled boxes. A panel that has nothing left is
 * dropped whole, together with its title and launcher.
 *
 * `sheetActions` is passed through to [`offeredItems`], so a panel whose only
 * survivor is a sheet palette (the compact properties inspector) is kept.
 */
export function offeredPanels(
  panels: readonly RibbonPanel[],
  palettesAvailable: boolean | undefined,
  sheetActions: readonly string[] = [],
): readonly RibbonPanel[] {
  if (palettesAvailable !== false) return panels;
  return panels.filter(panel => panel.pending
    || offeredItems(panel.items, palettesAvailable, sheetActions).length > 0);
}

/**
 * The command a panel's dialog-launcher arrow runs, or `null`.
 *
 * The arrow in a panel's title row is the convention for "deeper settings of
 * this panel". A panel declares its target as data (`launcherCommand`), so the
 * renderer never has to know panel ids:
 *
 *   * `'LAYER'` / `'PROPERTIES'` — the panel's own palette, opened (and, for
 *     layers, focused) by the chrome-local selector of that name. Neither is a
 *     Rust command, so neither needs a document or a router entry;
 *   * no token (or `null`) — no dialog exists for that panel yet. The arrow stays
 *     visible but disabled with the catalog key `limits.ts` holds
 *     (`ribbon.panel_dialog_missing`), which says exactly that instead of
 *     claiming the command is unsupported.
 *
 * Anything that is not a non-empty string is "no target": an empty token would
 * otherwise resolve to a command name of `''` and quietly render a live arrow.
 */
export function panelLauncherCommand(panel: RibbonPanel | null | undefined): string | null {
  const command = panel?.launcherCommand ?? null;
  return typeof command === 'string' && command.length > 0 ? command : null;
}

// The AutoCAD tab order lives in `tabs.ts`; re-exported so the many readers that
// name the model keep one import. `tabs` -> `panel-items` -> `actions` is the
// only runtime edge, so this re-export does not create a cycle.
export { RIBBON_TABS, ribbonCommandIds, ribbonTab } from './tabs.ts';
