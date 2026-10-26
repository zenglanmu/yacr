// The CAD shortcut menu: a pure model plus the two pure geometry/navigation
// helpers its overlay needs.
//
// Right-click inside the CAD partition is the most-used AutoCAD surface, so the
// menu is built from the same facts the rest of the chrome already holds
// (`selection`, the active `tool`, the command line's own history, the host's
// `canFit`/`documentLabel`) and every item dispatches through the one command
// funnel — no second router, no second history, no hidden state.
//
// Two rules shape the model:
//
//   * **Nothing is hidden.** Every section and every item is always present in
//     a fixed order with a stable id; an item that cannot run right now is
//     `enabled: false` and renders disabled (`aria-disabled`). A missing item
//     would read as "this CAD build has no cut", which is a different — and
//     false — claim.
//   * **A capturing tool is never overridden.** While a draw/measure tool owns
//     the picks (`tool != null`), the clipboard and destructive items are shown
//     but disabled: a right-click must not silently cut, copy, paste or erase
//     while the tool is collecting points. The tool's own confirm/cancel path
//     (Enter/Escape in the command line) is untouched, and there is deliberately
//     no "Cancel tool" item here — Escape already owns that.
//
// Everything here is DOM-free and deterministic (no clock, no randomness), so
// the menu contract is testable with plain Node.
import { selectionCount } from '../cad/selection-wire.ts';

// The count rule lives in `selection-wire.ts` beside the selection block it reads,
// and the command-admission rules (`commands.ts`) ask it the same question. It is
// re-exported here so the menu's callers keep a single import surface.
export { selectionCount };

/** The selection slice the shortcut menu reads (`cad_status.selection`). */
export interface ContextMenuSelection {
  readonly count?: unknown;
}

/** One actionable row of the shortcut menu. Always present, enabled or not. */
export interface ContextMenuItem {
  readonly id: string;
  readonly key: string;
  readonly command: string | null;
  readonly enabled: boolean;
}

export interface ContextMenuSection {
  readonly id: string;
  readonly items: ContextMenuItem[];
}

export interface ContextMenuModel {
  readonly sections: ContextMenuSection[];
}

export interface BuildContextMenuInput {
  readonly selection?: ContextMenuSelection | null;
  readonly tool?: object | null;
  readonly lastCommand?: string | null;
  readonly canFit?: boolean;
  readonly hasDocument?: boolean;
}

/** A viewport/anchor point in CSS px. */
export interface MenuPoint {
  readonly x: number;
  readonly y: number;
}

/** A box in CSS px. */
export interface MenuSize {
  readonly width: number;
  readonly height: number;
}

export interface PlaceContextMenuInput {
  readonly anchor?: Partial<MenuPoint> | null;
  readonly size?: Partial<MenuSize> | null;
  readonly viewport?: Partial<MenuSize> | null;
  readonly margin?: number;
}

export interface MenuPlacement {
  readonly left: number;
  readonly top: number;
}

/** Section ids, in rendering order. Stable: tests and CSS may key on them. */
export const CONTEXT_MENU_SECTIONS: readonly string[] = Object.freeze([
  'repeat',
  'clipboard',
  'selection',
  'view',
  'history',
]);

/** Gap kept between the menu and every window edge, in CSS px. */
export const CONTEXT_MENU_MARGIN = 8;

/** Normalize a command name the way the host does (trim + upper-case). */
function normalizeCommand(value: unknown): string {
  return String(value ?? '').trim().toUpperCase();
}

/**
 * Build the shortcut menu model.
 *
 * `hasSelection` goes through the shared `selectionCount` rule (imported above),
 * so the menu and the command-admission rules agree on what "a selection" is.
 *
 * @param {object|null} selection    `cad_status.selection` (`{count, ...}`)
 * @param {object|null} tool         `cad_status.tool`; non-null while a tool
 *                                   captures picks
 * @param {string|null} lastCommand  the command line's newest history entry
 * @param {boolean} canFit           `cad_status.canFit`
 * @param {boolean} hasDocument      the host holds a drawing
 * @returns {{sections: Array<{id: string, items: Array<{id: string, key: string, command: string|null, enabled: boolean}>}>}}
 */
export function buildContextMenu({
  selection = null,
  tool = null,
  lastCommand = null,
  canFit = false,
  hasDocument = false,
}: BuildContextMenuInput = {}): ContextMenuModel {
  const hasSelection = selectionCount(selection) > 0;
  // `tool != null` is the capture fact: a tool object exists exactly while the
  // host is collecting picks for it.
  const capturing = tool != null;
  // Clipboard + destructive actions are blocked by a capture; they stay visible.
  const editable = hasSelection && !capturing;
  const repeat = normalizeCommand(lastCommand);
  // Fit needs both facts, exactly like the command-line admission rule.
  const canFitNow = hasDocument === true && canFit === true;

  return {
    sections: [
      {
        id: 'repeat',
        items: [
          {
            id: 'repeat.last',
            key: 'menu.repeat',
            // The command is carried even when disabled, so the overlay can show
            // *what* would repeat; a disabled item never dispatches it.
            command: repeat || null,
            enabled: repeat.length > 0,
          },
        ],
      },
      {
        id: 'clipboard',
        items: [
          { id: 'clipboard.cut', key: 'menu.clipboard.cut', command: 'CUTCLIP', enabled: editable },
          { id: 'clipboard.copy', key: 'menu.clipboard.copy', command: 'COPYCLIP', enabled: editable },
          { id: 'clipboard.paste', key: 'menu.clipboard.paste', command: 'PASTECLIP', enabled: !capturing },
        ],
      },
      {
        id: 'selection',
        items: [
          { id: 'selection.all', key: 'menu.selection.all', command: 'SELECTALL', enabled: true },
          { id: 'selection.erase', key: 'menu.selection.erase', command: 'ERASE', enabled: editable },
          // A chrome-local selector, like the command line's own intercept: the
          // run path resolves it to the docked palette, never to the router.
          { id: 'selection.properties', key: 'menu.selection.properties', command: 'PROPERTIES', enabled: true },
        ],
      },
      {
        id: 'view',
        items: [
          { id: 'view.fit', key: 'menu.view.fit', command: 'FIT', enabled: canFitNow },
          { id: 'view.pan', key: 'menu.view.pan', command: 'PAN', enabled: true },
        ],
      },
      {
        id: 'history',
        items: [
          { id: 'edit.undo', key: 'menu.edit.undo', command: 'UNDO', enabled: true },
          { id: 'edit.redo', key: 'menu.edit.redo', command: 'REDO', enabled: true },
        ],
      },
    ],
  };
}

/** Every item of the menu, section after section (the overlay's flat order). */
export function menuItems(menu: ContextMenuModel | null | undefined): ContextMenuItem[] {
  const sections = Array.isArray(menu?.sections) ? menu.sections : [];
  return sections.flatMap(section => (Array.isArray(section.items) ? section.items : []));
}

/**
 * The next **enabled** item in `direction` (+1/-1), wrapping at both ends.
 *
 * `from < 0` means "nothing active yet": Down starts at the first enabled item
 * and Up at the last. Disabled items are skipped, never focused, so the arrow
 * keys can only land on something the user can actually run. `-1` means no
 * enabled item exists at all.
 */
export function nextEnabledIndex(
  items: readonly ContextMenuItem[] | null | undefined,
  from: number,
  direction: number,
): number {
  const list = Array.isArray(items) ? items : [];
  const count = list.length;
  if (!count || !direction) return -1;
  const step = direction > 0 ? 1 : -1;
  let index = Number.isInteger(from) ? from : -1;
  for (let attempt = 0; attempt < count; attempt += 1) {
    index = index < 0
      ? (step > 0 ? 0 : count - 1)
      : ((index + step) % count + count) % count;
    if (list[index]?.enabled === true) return index;
  }
  return -1;
}

/** Home/End: the first (`'first'`) or last (`'last'`) enabled item, or `-1`. */
export function edgeEnabledIndex(
  items: readonly ContextMenuItem[] | null | undefined,
  edge: 'first' | 'last',
): number {
  return nextEnabledIndex(items, -1, edge === 'last' ? -1 : 1);
}

function clamp(value: number, min: number, max: number): number {
  if (!Number.isFinite(value)) return min;
  return Math.min(Math.max(value, min), Math.max(min, max));
}

/**
 * Where to paint the menu for an anchor point, in viewport CSS px.
 *
 * The menu opens down-right of the press. If that would cross the window's
 * right/bottom edge it **flips** to the other side of the point (a menu that
 * would otherwise cover the cursor), and the result is clamped so it always
 * stays fully inside the window with `margin` to spare. A menu larger than the
 * window is pinned to the top/left margin and scrolls internally instead.
 */
export function placeContextMenu({
  anchor,
  size,
  viewport,
  margin = CONTEXT_MENU_MARGIN,
}: PlaceContextMenuInput = {}): MenuPlacement {
  const width = Math.max(0, Number(size?.width) || 0);
  const height = Math.max(0, Number(size?.height) || 0);
  const viewportWidth = Math.max(0, Number(viewport?.width) || 0);
  const viewportHeight = Math.max(0, Number(viewport?.height) || 0);
  const anchorX = Number(anchor?.x);
  const anchorY = Number(anchor?.y);
  const maxLeft = Math.max(margin, viewportWidth - margin - width);
  const maxTop = Math.max(margin, viewportHeight - margin - height);
  const preferredLeft = anchorX + width > viewportWidth - margin ? anchorX - width : anchorX;
  const preferredTop = anchorY + height > viewportHeight - margin ? anchorY - height : anchorY;
  return {
    left: Math.round(clamp(preferredLeft, margin, maxLeft)),
    top: Math.round(clamp(preferredTop, margin, maxTop)),
  };
}
