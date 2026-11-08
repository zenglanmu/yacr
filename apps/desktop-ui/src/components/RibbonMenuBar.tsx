// The collapsed ribbon as an AutoCAD-classic menu bar.
//
// Collapsing used to leave the tab strip standing over empty space: the "File"
// tab read as the selected tab of a band that was not there. This component
// replaces that row with the thing AutoCAD's simplified ribbon actually shows —
// a menu bar. The tab names become top-level menus, each menu is built from that
// tab's own `RIBBON_TABS` panels (through `ribbonMenuEntriesFor`), a panel with
// many items becomes a submenu, and an item that owns variants opens one more
// submenu. Nothing here is a second definition of the ribbon: the same model,
// the same carrier reduction (`offeredPanels`/`offeredItems`), and the same
// dispatch and refusal helpers the expanded band uses (`runItem`, `itemEnabled`,
// `unavailable` from `Ribbon.tsx`). A tool that is disabled in the band is
// disabled in the menu, worded by the same sentence, and running one reaches the
// host by the identical path.
//
// Shape, and why it is the same row as the tab strip:
//
//   * **One 26px row** (`--ribbon-bar-h`, shared with `.ribbon__tabs`). The host
//     samples the CAD partition rect from this chrome, so the collapsed and
//     expanded rows must measure the same; a shared token is what makes drift
//     impossible.
//   * **The dropdown is an in-DOM overlay**, exactly like `ContextMenu`: fixed
//     position, no portal and no backdrop, clamped/flipped inside the window by
//     the same `placeContextMenu` helper. It paints over the drawing (the ribbon
//     lifts its own stacking order while a menu is open) and changes no layout
//     height, so the partition rect is untouched by opening or closing it.
//
// Keyboard model: the menubar row is a `menubar`; ArrowDown opens a menu,
// ArrowLeft/Right walk the triggers while closed; inside a menu ArrowDown/Up
// move over the operable rows only, Home/End jump, Enter/Space run or open a
// submenu, ArrowRight opens a submenu, ArrowLeft closes the deepest level,
// Escape closes one level per press and finally returns focus to the trigger,
// and Tab closes everything. The listener is installed in the capture phase so
// the menu owns Escape while it is open (the shell's tool-cancel Escape never
// fires for the same press).
//
// Pointer model: click a trigger to open or close it, hover another trigger
// while a menu is open to switch menus (AutoCAD behaviour), hover a row to
// highlight it, click to run or open its submenu. Resting on a submenu parent
// opens its flyout after a short intent delay (`RIBBON_MENU_HOVER_OPEN_MS`), the
// Windows/AutoCAD convention; hovering any other row (a sibling leaf, a caption,
// another parent) retires the flyout that row did not own. A press outside, a
// window blur or a window resize dismisses without consuming the press.
import { Fragment, useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { Icon } from '../icons.tsx';
import type { IconName } from '../icons.tsx';
import { IconButton } from './primitives.tsx';
import { itemEnabled, runItem, unavailable } from './Ribbon.tsx';
import { useI18n, useShell } from '../context.ts';
import type { ShellValue } from '../context.ts';
import type { Translator } from '../i18n.ts';
import { RIBBON_TABS } from '../ribbon/ribbon-model.ts';
import type { RibbonItem } from '../ribbon/ribbon-model.ts';
import { ribbonMenuEntriesFor, panelStartsGroup, placeRibbonMenuLevel, ribbonMenuFlyoutAnchor, ribbonMenuMaxHeight, RIBBON_MENU_HOVER_OPEN_MS } from '../ribbon/ribbon-menu.ts';
import type { RibbonMenuEntry, RibbonMenuRow } from '../ribbon/ribbon-menu.ts';
import { edgeEnabledIndex, nextEnabledIndex } from '../commands/context-menu.ts';
import type { MenuPlacement } from '../commands/context-menu.ts';

/** One row rectangle the overlay hangs from (a trigger or a parent row). */
interface AnchorRect {
  readonly left: number;
  readonly right: number;
  readonly top: number;
  readonly bottom: number;
}

/** The open menu state: which tab, and the chain of open submenus. */
interface MenubarState {
  /** Index into `RIBBON_TABS`. */
  readonly tab: number;
  /** Anchors, one per open level: the trigger, then each opened row. */
  readonly anchors: readonly AnchorRect[];
  /** The row index that opened each deeper level (length = levels - 1). */
  readonly path: readonly number[];
  /** The active row index per level (length = levels). */
  readonly active: readonly number[];
}

/**
 * One rendered row of a menu level.
 *
 * `enabled` is **operable**, not "this command can run": a submenu parent whose
 * own command is refused stays reachable because its children are the point of
 * the row (the band keeps the caret live for the same reason). A leaf row is
 * operable exactly when `itemEnabled` admits it, so the arrow keys only ever
 * land on something the user can act on.
 */
interface RowView {
  /** Unique DOM id (the level is part of it, so `aria-activedescendant` is exact). */
  readonly id: string;
  readonly kind: 'caption' | 'item' | 'panel';
  /** The catalog key of the label (item label, panel title, or caption). */
  readonly labelKey: string;
  readonly item?: RibbonItem;
  /** Rows this row opens, when it is a submenu parent. */
  readonly children: readonly RibbonMenuRow[];
  readonly enabled: boolean;
  readonly hasSubmenu: boolean;
  /**
   * Whether a separator rule belongs above this row: a caption starts a group,
   * and so does the first row of a section whose caption was suppressed (see
   * `RibbonMenuEntry.caption`), so dropping a header never glues two groups
   * together. A `panel` row opens a group only when it follows an inline
   * section — consecutive submenu rows are one block (see `panelStartsGroup`).
   */
  readonly startsGroup: boolean;
}

/** The layout facts a menu level needs from its parent row. */
function rectOf(node: Element | null): AnchorRect {
  if (!node || typeof node.getBoundingClientRect !== 'function') {
    return { left: 0, right: 0, top: 0, bottom: 0 };
  }
  const rect = node.getBoundingClientRect();
  return { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom };
}

/** The row id for one position of one level. */
function rowId(depth: number, index: number): string {
  return `ribbon-menu-${depth}-${index}`;
}

/** An item as a row, with the item's own variants as its submenu. */
function itemView(shell: ShellValue, row: RibbonMenuRow, depth: number, index: number, startsGroup = false): RowView {
  const hasSubmenu = row.children.length > 0;
  return {
    id: rowId(depth, index),
    kind: 'item',
    labelKey: row.item.labelKey,
    item: row.item,
    children: row.children,
    // A submenu parent stays operable even when its own command is refused: the
    // refusal belongs to the one row, and its children keep their own gates.
    enabled: itemEnabled(shell, row.item) || hasSubmenu,
    hasSubmenu,
    startsGroup,
  };
}

/**
 * The rows of one menu level.
 *
 * A tab level interleaves inline panel sections (a caption, then their rows) and
 * submenu-parent rows for the big panels. Every deeper level is just item rows:
 * a panel's items, or an item's variants.
 */
function levelViews(shell: ShellValue, tabIndex: number, path: readonly number[]): RowView[][] {
  const tab = RIBBON_TABS[tabIndex];
  if (!tab) return [];
  const entries = ribbonMenuEntriesFor(tab, shell.palettesAvailable);
  const levels: RowView[][] = [tabViews(shell, entries, 0)];
  for (const index of path) {
    const view = levels[levels.length - 1][index];
    if (!view?.hasSubmenu) break;
    const depth = levels.length;
    levels.push(view.children.map((row, position) => itemView(shell, row, depth, position)));
  }
  return levels;
}

/**
 * The top-level rows of one tab's menu: sections inline, big panels as submenus.
 *
 * A run of sibling panels is one visual block — a list of submenus — so only a
 * panel that follows an inline section opens a new group (`panelStartsGroup`);
 * hair-lining each of them would box three ordinary rows into three sections.
 */
function tabViews(shell: ShellValue, entries: readonly RibbonMenuEntry[], depth: number): RowView[] {
  const views: RowView[] = [];
  entries.forEach((entry, entryIndex) => {
    if (entry.kind === 'panel') {
      views.push({
        id: rowId(depth, views.length),
        kind: 'panel',
        labelKey: entry.titleKey,
        children: entry.rows,
        enabled: entry.rows.length > 0,
        hasSubmenu: entry.rows.length > 0,
        startsGroup: panelStartsGroup(entries, entryIndex),
      });
      return;
    }
    // A caption suppressed by the projection (it would repeat the menu title,
    // or its first row already carries the panel's name) leaves its first row
    // to open the group instead, so the separator rule still separates this
    // section from the one before it.
    if (entry.caption) {
      views.push({
        id: rowId(depth, views.length),
        kind: 'caption',
        labelKey: entry.titleKey,
        children: [],
        enabled: false,
        hasSubmenu: false,
        startsGroup: true,
      });
    }
    entry.rows.forEach((row, position) => {
      const startsGroup = !entry.caption && position === 0 && views.length > 0;
      views.push(itemView(shell, row, depth, views.length, startsGroup));
    });
  });
  return views;
}

/** The tooltip of one row: its own limit, or the shared refusal sentence. */
function rowTitle(t: Translator, shell: ShellValue, view: RowView): string | undefined {
  const item = view.item;
  if (!item) return undefined;
  if (itemEnabled(shell, item)) return item.hintKey ? t(item.hintKey) : undefined;
  return unavailable(t, item, shell);
}

interface MenuSurfaceProps {
  readonly anchor: AnchorRect;
  readonly depth: number;
  readonly label: string;
  readonly activeId: string | undefined;
  readonly rowsKey: string;
  /** The inline height budget: the space below the anchor (see `ribbonMenuMaxHeight`). */
  readonly maxHeight: number;
  /** Only the deepest open menu holds the keyboard and the focused node. */
  readonly takeFocus: boolean;
  readonly children: ReactNode;
}

/**
 * One menu level: the positioned overlay.
 *
 * Measurement lives here (the menu must never flash at an unclamped position,
 * and its height budget is applied before that measurement so the placement can
 * never flip it upward), and the deepest level is the focused node — items are
 * driven by `aria-activedescendant`, so focus never leaves the menu and no row
 * can be tabbed out of order. `placeRibbonMenuLevel` owns all placement
 * arithmetic; this component only measures and applies its answer.
 */
function MenuSurface({ anchor, depth, label, activeId, rowsKey, maxHeight, takeFocus, children }: MenuSurfaceProps) {
  const rootRef = useRef<HTMLDivElement | null>(null);
  const [placed, setPlaced] = useState<MenuPlacement | null>(null);
  useLayoutEffect(() => {
    const node = rootRef.current;
    if (!node) return;
    const rect = node.getBoundingClientRect();
    const next = placeRibbonMenuLevel({
      anchor,
      depth,
      size: { width: rect.width, height: rect.height },
      viewport: { width: window.innerWidth, height: window.innerHeight },
    });
    setPlaced(previous => (previous && previous.left === next.left && previous.top === next.top ? previous : next));
  }, [anchor, depth, rowsKey]);
  useEffect(() => {
    if (takeFocus) rootRef.current?.focus?.({ preventScroll: true });
  }, [takeFocus]);
  return (
    <div
      ref={rootRef}
      className={['ribbon-menu', placed ? '' : 'is-unplaced'].filter(Boolean).join(' ')}
      role="menu"
      tabIndex={-1}
      aria-label={label}
      aria-activedescendant={activeId}
      style={{ left: `${placed?.left ?? 0}px`, top: `${placed?.top ?? 0}px`, maxHeight: `${maxHeight}px` }}
      data-yacr-ribbon-menu="collapsed"
      data-depth={depth}
      // Which side of the parent a flyout opened on. Its elevation is biased
      // away from that seam, so the shadow never darkens the parent's border or
      // padding; a flipped flyout gets the mirror image of the same shadow.
      // A top-level dropdown has no seam and keeps the symmetric one.
      data-side={depth > 0 && placed ? (placed.left >= anchor.right ? 'right' : 'left') : undefined}
    >
      {children}
    </div>
  );
}

export interface RibbonMenuBarProps {
  /** Whether a menu is open, so the ribbon can lift its own stacking order. */
  readonly onMenuOpenChange: (open: boolean) => void;
}

export function RibbonMenuBar({ onMenuOpenChange }: RibbonMenuBarProps) {
  const { t } = useI18n();
  const shell = useShell();
  const triggers = useRef<(HTMLButtonElement | null)[]>([]);
  const [state, setState] = useState<MenubarState | null>(null);

  const levels = state ? levelViews(shell, state.tab, state.path) : [];
  // The keyboard listener is registered once per open state change; this ref is
  // how it reads the current rows without listing them in the effect deps (they
  // are rebuilt on every render, and re-registering per render would be churn).
  const levelsRef = useRef<RowView[][]>(levels);
  levelsRef.current = levels;

  // The ribbon reads this to add its `is-menu-open` stacking class: the overlay
  // has to paint above the document tabs and the workspace, which are later
  // siblings in the flex column.
  const open = state !== null;
  useEffect(() => {
    onMenuOpenChange(open);
    return open ? () => onMenuOpenChange(false) : undefined;
  }, [open, onMenuOpenChange]);

  const focusTrigger = (tabIndex: number | null | undefined) => {
    if (tabIndex === null || tabIndex === undefined) return;
    triggers.current[tabIndex]?.focus?.({ preventScroll: true });
  };

  // The pending hover open. One timer for the whole menu bar: only the row the
  // pointer is resting on may own it, so entering any other row (or leaving one)
  // clears it before a new one is scheduled.
  const hoverTimer = useRef<number | null>(null);
  const cancelHover = () => {
    if (hoverTimer.current === null) return;
    window.clearTimeout(hoverTimer.current);
    hoverTimer.current = null;
  };
  // A delayed hover open must never fire after the component is gone.
  useEffect(() => () => {
    if (hoverTimer.current !== null) window.clearTimeout(hoverTimer.current);
    hoverTimer.current = null;
  }, []);

  const closeAll = (restoreFocus: boolean) => {
    cancelHover();
    const tabIndex = state?.tab ?? null;
    setState(null);
    if (restoreFocus) focusTrigger(tabIndex);
  };

  const openTab = (tabIndex: number, node: Element | null, activateFirst: boolean) => {
    cancelHover();
    const rows = tabViews(shell, ribbonMenuEntriesFor(RIBBON_TABS[tabIndex], shell.palettesAvailable), 0);
    setState({
      tab: tabIndex,
      anchors: [rectOf(node)],
      path: [],
      active: [activateFirst ? edgeEnabledIndex(rows, 'first') : -1],
    });
  };

  const setActive = (depth: number, index: number) => {
    setState(previous => (previous
      ? { ...previous, active: previous.active.map((value, level) => (level === depth ? index : value)) }
      : previous));
  };

  // A keyboard move must be able to land on a row the level's height budget has
  // pushed out of view: the Draw flyout is taller than a 700px window, and the
  // active row is driven by `aria-activedescendant`, so the browser never
  // scrolls it in by itself. The flag keeps this to keyboard moves — a hover
  // lands on a row the pointer can already see, and scrolling there would be a
  // surprise.
  const revealActive = useRef(false);
  const moveActive = (depth: number, index: number) => {
    revealActive.current = true;
    setActive(depth, index);
  };
  /**
   * Bring the active row of the deepest level into view.
   *
   * `block: 'nearest'` scrolls the nearest scrollable ancestor — the menu
   * surface — and nothing else when the row is already visible, so the page
   * never moves and the level never changes position. This runs after layout so
   * the new `aria-activedescendant` row exists at its final geometry.
   */
  useLayoutEffect(() => {
    if (!revealActive.current) return;
    revealActive.current = false;
    if (!state) return;
    const depth = state.path.length;
    const index = state.active[depth] ?? -1;
    const id = levelsRef.current[depth]?.[index]?.id;
    if (!id) return;
    const node = typeof document === 'undefined' ? null : document.getElementById(id);
    node?.scrollIntoView?.({ block: 'nearest' });
  }, [state]);

  const closeDeepest = () => {
    cancelHover();
    setState(previous => (previous && previous.path.length > 0
      ? {
        ...previous,
        anchors: previous.anchors.slice(0, -1),
        path: previous.path.slice(0, -1),
        active: previous.active.slice(0, -1),
      }
      : previous));
  };

  /**
   * Collapse every level deeper than `depth`.
   *
   * A flyout belongs to exactly one parent row. The moment the pointer rests on
   * anything else (a sibling leaf, a caption, another parent), that flyout is no
   * longer what the pointer is pointing at, so it must not linger: this is the
   * pointer half of "one open chain per menu".
   */
  const closeDeeper = (depth: number) => {
    cancelHover();
    setState(previous => (previous && previous.path.length > depth
      ? {
        ...previous,
        anchors: previous.anchors.slice(0, depth + 1),
        path: previous.path.slice(0, depth),
        active: previous.active.slice(0, depth + 1),
      }
      : previous));
  };

  const openSubmenu = (depth: number, index: number, activateFirst: boolean) => {
    // An explicit activation (click, Enter, ArrowRight) opens at once and owns
    // the decision: a hover open still pending for another row must not fire
    // afterwards and replace it.
    cancelHover();
    const view = levels[depth]?.[index];
    if (!view?.hasSubmenu) return;
    const rows = view.children.map((row, position) => itemView(shell, row, depth + 1, position));
    const rowNode = typeof document === 'undefined' ? null : document.getElementById(view.id);
    // A flyout abuts the **surface** it springs from, never the row inside it:
    // the row ends inside the parent's padding, so a row-anchored flyout would
    // cover the parent's border, its shadow and its bottom-right corner.
    const surfaceNode = rowNode?.closest?.('.ribbon-menu') ?? null;
    const anchor = ribbonMenuFlyoutAnchor(rectOf(rowNode), rectOf(surfaceNode));
    setState(previous => (previous
      ? {
        ...previous,
        anchors: [...previous.anchors.slice(0, depth + 1), anchor],
        path: [...previous.path.slice(0, depth), index],
        active: [...previous.active.slice(0, depth + 1), activateFirst ? edgeEnabledIndex(rows, 'first') : -1],
      }
      : previous));
  };

  /**
   * Open a submenu after the pointer has rested on its parent row.
   *
   * The timer is the whole point: crossing a list must not flash flyouts, so the
   * open waits [`RIBBON_MENU_HOVER_OPEN_MS`] and any other row entering, the row
   * leaving, or a close clears it first. Only submenu parents get a timer.
   */
  const scheduleSubmenu = (depth: number, index: number) => {
    cancelHover();
    hoverTimer.current = window.setTimeout(() => {
      hoverTimer.current = null;
      openSubmenu(depth, index, false);
    }, RIBBON_MENU_HOVER_OPEN_MS);
  };

  /**
   * The pointer half of one open chain: highlight, cancel any pending hover open,
   * collapse a flyout that belongs to a different row, and (for a submenu parent)
   * arm the intent timer.
   */
  const hoverRow = (depth: number, index: number) => {
    const view = levels[depth]?.[index];
    if (!view) return;
    const openParent = state?.path?.[depth];
    if (state && state.path.length > depth && openParent !== index) {
      // The open flyout belongs to another row: hovering this one retires it now
      // rather than after the intent delay.
      closeDeeper(depth);
    }
    cancelHover();
    if (view.enabled) setActive(depth, index);
    // A leaf row (or a refused one) never owns a timer; it only clears the one
    // the row before it may have armed.
    if (!view.enabled || !view.hasSubmenu) return;
    // Already open: nothing to arm, and re-opening would reset its active row.
    if (openParent === index) return;
    scheduleSubmenu(depth, index);
  };

  /**
   * Activate one row: open its submenu, or run the command and close the menu.
   * Both paths are the shared helpers; nothing here re-derives admission.
   */
  const activateRow = (depth: number, index: number, viaKeyboard: boolean) => {
    const view = levels[depth]?.[index];
    if (!view?.enabled) return;
    if (view.hasSubmenu) {
      openSubmenu(depth, index, viaKeyboard);
      return;
    }
    if (!view.item || !itemEnabled(shell, view.item)) return;
    runItem(shell, view.item);
    closeAll(true);
  };

  // Escape/arrow/Enter ownership while a menu is open. Capture phase, exactly
  // like `ContextMenu`: the shell's own Escape (tool cancel) must not also fire
  // on the press that closes a menu level.
  useEffect(() => {
    if (!state) return undefined;
    const onKeyDown = (event: KeyboardEvent) => {
      // An IME composition owns its keys (Enter/Escape included).
      if (event.isComposing || event.keyCode === 229) return;
      const depth = state.path.length;
      const rows = levelsRef.current[depth] ?? [];
      const activeIndex = state.active[depth] ?? -1;
      const consume = () => {
        event.preventDefault();
        event.stopPropagation();
      };
      switch (event.key) {
        case 'Escape':
          consume();
          if (depth > 0) closeDeepest();
          else closeAll(true);
          return;
        case 'Tab':
          consume();
          closeAll(true);
          return;
        case 'ArrowDown':
          consume();
          moveActive(depth, nextEnabledIndex(rows, activeIndex, 1));
          return;
        case 'ArrowUp':
          consume();
          moveActive(depth, nextEnabledIndex(rows, activeIndex, -1));
          return;
        case 'Home':
          consume();
          moveActive(depth, edgeEnabledIndex(rows, 'first'));
          return;
        case 'End':
          consume();
          moveActive(depth, edgeEnabledIndex(rows, 'last'));
          return;
        case 'ArrowRight':
          consume();
          if (rows[activeIndex]?.hasSubmenu) openSubmenu(depth, activeIndex, true);
          return;
        case 'ArrowLeft':
          consume();
          if (depth > 0) closeDeepest();
          return;
        case 'Enter':
        case ' ':
        case 'Spacebar':
          consume();
          activateRow(depth, activeIndex, true);
          return;
        default:
          return;
      }
    };
    window.addEventListener('keydown', onKeyDown, true);
    return () => window.removeEventListener('keydown', onKeyDown, true);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- the handlers read the current render's shell/levels through closures and `levelsRef`
  }, [state, shell, t]);

  // Dismissal: a press outside the menubar and its overlays, a window blur, or a
  // resize. The press is never prevented, so it still does whatever it would
  // have done underneath (the CAD partition forwards it as before).
  useEffect(() => {
    if (!open) return undefined;
    const contains = (target: EventTarget | null): boolean => {
      const element = target as Element | null;
      return Boolean(element?.closest?.('.ribbon__menubar') || element?.closest?.('.ribbon-menu'));
    };
    const onPointerDown = (event: PointerEvent) => { if (!contains(event.target)) { cancelHover(); setState(null); } };
    const dismiss = () => { cancelHover(); setState(null); };
    document.addEventListener('pointerdown', onPointerDown, true);
    window.addEventListener('blur', dismiss);
    window.addEventListener('resize', dismiss);
    return () => {
      document.removeEventListener('pointerdown', onPointerDown, true);
      window.removeEventListener('blur', dismiss);
      window.removeEventListener('resize', dismiss);
    };
  }, [open]);

  const activeTab = state ? RIBBON_TABS[state.tab] : undefined;

  return (
    <>
      <div className="ribbon__menubar" role="menubar" aria-label={t('ribbon.menubar_label')}>
        {RIBBON_TABS.map((entry, index) => {
          const expanded = state?.tab === index;
          return (
            <button
              key={entry.id}
              ref={node => { triggers.current[index] = node; }}
              type="button"
              role="menuitem"
              aria-haspopup="menu"
              aria-expanded={expanded}
              className="ribbon-menubar__item"
              onClick={event => {
                if (expanded) closeAll(false);
                else openTab(index, event.currentTarget, false);
              }}
              onPointerEnter={() => {
                // AutoCAD behaviour: once a menu is open, sliding along the bar
                // switches to that tab's menu without another click.
                if (state && state.tab !== index) openTab(index, triggers.current[index], false);
              }}
              onKeyDown={event => {
                if (event.key === 'ArrowDown') {
                  event.preventDefault();
                  openTab(index, event.currentTarget, true);
                  return;
                }
                if (event.key === 'ArrowRight' || event.key === 'ArrowLeft') {
                  event.preventDefault();
                  const step = event.key === 'ArrowRight' ? 1 : -1;
                  const next = (index + step + RIBBON_TABS.length) % RIBBON_TABS.length;
                  focusTrigger(next);
                }
              }}
            >
              {t(entry.labelKey)}
            </button>
          );
        })}
        <div className="ribbon-menubar__spacer" />
        <IconButton
          icon="chevronDown"
          size="sm"
          label={t('ribbon.expand')}
          onClick={() => {
            // Close first: the ribbon must not keep its menu stacking class once
            // the band comes back.
            cancelHover();
            setState(null);
            onMenuOpenChange(false);
            shell.setRibbonCollapsed(false);
          }}
        />
      </div>
      {state && activeTab ? levels.map((rows, depth) => {
        const activeIndex = state.active[depth] ?? -1;
        const parent = depth > 0 ? levels[depth - 1]?.[state.path[depth - 1]] : undefined;
        const label = parent ? t(parent.labelKey) : t(activeTab.labelKey);
        const anchor = state.anchors[depth] ?? state.anchors[0];
        // The level is capped to the space below its anchor, so placement always
        // opens downward; the CSS max-height stays as the no-JS fallback.
        const maxHeight = ribbonMenuMaxHeight({
          anchor,
          depth,
          viewport: { width: window.innerWidth, height: window.innerHeight },
        });
        return (
          <MenuSurface
            key={`${activeTab.id}-${depth}`}
            anchor={anchor}
            depth={depth}
            label={label}
            activeId={rows[activeIndex]?.id}
            rowsKey={rows.map(row => row.id).join(',')}
            maxHeight={maxHeight}
            takeFocus={depth === levels.length - 1}
          >
            {rows.map((view, index) => {
              // A group start gets a hairline above it, unless it is the first
              // row. Captions, panel submenus and the first row of a section whose
              // caption was suppressed all carry `startsGroup`.
              const separator = index > 0 && view.startsGroup
                ? <div className="ribbon-menu__separator" role="separator" />
                : null;
              if (view.kind === 'caption') {
                return (
                  <Fragment key={view.id}>
                    {separator}
                    <div
                      className="ribbon-menu__caption"
                      role="presentation"
                      onMouseEnter={() => closeDeeper(depth)}
                    >
                      {t(view.labelKey)}
                    </div>
                  </Fragment>
                );
              }
              const expanded = state.path[depth] === index;
              return (
                <Fragment key={view.id}>
                  {separator}
                  <button
                    id={view.id}
                    type="button"
                    role="menuitem"
                    tabIndex={-1}
                    className={[
                      'ribbon-menu__row',
                      view.enabled ? '' : 'is-disabled',
                      index === activeIndex ? 'is-active' : '',
                      view.kind === 'panel' ? 'is-panel' : '',
                    ].filter(Boolean).join(' ')}
                    aria-disabled={view.enabled ? undefined : 'true'}
                    aria-haspopup={view.hasSubmenu ? 'menu' : undefined}
                    aria-expanded={view.hasSubmenu ? expanded : undefined}
                    title={rowTitle(t, shell, view)}
                    onMouseEnter={() => hoverRow(depth, index)}
                    onMouseLeave={() => cancelHover()}
                    onClick={() => activateRow(depth, index, false)}
                  >
                    {view.item
                      ? <Icon name={view.item.icon as IconName} size={15} />
                      : <span className="ribbon-menu__glyph" aria-hidden="true" />}
                    <span className="ribbon-menu__label">{t(view.labelKey)}</span>
                    {view.item?.command ? <span className="ribbon-menu__command">{view.item.command}</span> : null}
                    <span className="ribbon-menu__chevron" aria-hidden="true">
                      {view.hasSubmenu ? <Icon name="chevronRight" size={13} /> : null}
                    </span>
                  </button>
                </Fragment>
              );
            })}
          </MenuSurface>
        );
      }) : null}
    </>
  );
}
