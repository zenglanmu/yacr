// ContextMenu: the CAD right-click shortcut menu (AutoCAD's most-used surface).
//
// One in-DOM overlay in this same React tree — no second Tauri window, no
// portal: the menu is `position: fixed`, and no ancestor of the shell creates a
// containing block, so it paints above the CAD partition rect on both carriers
// (`native-below` composites the surface *under* the webview, `dom-canvas` paints
// inside this document). It deliberately has **no backdrop**: a press anywhere
// else still lands on whatever is underneath (the CAD partition forwards it to
// the host exactly as before), which is also what dismisses the menu.
//
// Keyboard model (the reason the menu takes focus on open): Arrow keys move over
// the *enabled* items only, Home/End jump, Enter/Space run, Escape and Tab close
// and hand the keyboard back. The listener is installed in the capture phase so
// the menu owns Escape while it is open: the shell's own Escape (a tool's cancel)
// never fires for the same press. Pointer: hover highlights, click runs, a press
// outside / window blur / resize / scroll dismisses.
import React, { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { useI18n } from '../context.mjs';
import {
  edgeEnabledIndex,
  menuItems,
  nextEnabledIndex,
  placeContextMenu,
} from '../context-menu.mjs';

const ITEM_ID_PREFIX = 'cad-context-item-';

/**
 * @param menu      the pure model from `buildContextMenu`
 * @param anchor    `{x, y}` in viewport CSS px (the app converts the hook's
 *                  partition-relative point with the partition rect)
 * @param onRun     receives the item; the app closes the menu and dispatches
 * @param onDismiss receives `{restoreFocus}`: `true` when the keyboard left the
 *                  menu (Escape/Tab) and focus must go back to the drawing area
 */
export function ContextMenu({ menu, anchor, onRun, onDismiss }) {
  const { t } = useI18n();
  const rootRef = useRef(null);
  // `null` until the menu has been measured: the first paint is invisible, so
  // the menu never flashes at an unclamped position.
  const [placed, setPlaced] = useState(null);
  const [activeIndex, setActiveIndex] = useState(-1);
  const items = useMemo(() => menuItems(menu), [menu]);
  const indexById = useMemo(
    () => new Map(items.map((item, index) => [item.id, index])),
    [items],
  );

  // Measure once per open (and again if a locale/label change resizes it), then
  // place it inside the window with `placeContextMenu`'s flip-then-clamp rule.
  useLayoutEffect(() => {
    const node = rootRef.current;
    if (!node) return;
    const rect = node.getBoundingClientRect();
    setPlaced(placeContextMenu({
      anchor,
      size: { width: rect.width, height: rect.height },
      viewport: { width: window.innerWidth, height: window.innerHeight },
    }));
  }, [anchor, items, t]);

  // The menu owns the keyboard while it is open; the root is the focused node
  // (items are driven by `aria-activedescendant`, so focus never leaves it and
  // no item can be tabbed out of order).
  useEffect(() => {
    rootRef.current?.focus?.({ preventScroll: true });
  }, []);

  useEffect(() => {
    const onKeyDown = event => {
      // An IME composition owns its keys (Enter/Escape included).
      if (event.isComposing || event.keyCode === 229) return;
      const key = event.key;
      const close = () => {
        event.preventDefault();
        event.stopPropagation();
        onDismiss({ restoreFocus: true });
      };
      if (key === 'Escape' || key === 'Tab') {
        close();
        return;
      }
      if (key === 'ArrowDown' || key === 'ArrowUp') {
        event.preventDefault();
        event.stopPropagation();
        setActiveIndex(index => nextEnabledIndex(items, index, key === 'ArrowDown' ? 1 : -1));
        return;
      }
      if (key === 'Home' || key === 'End') {
        event.preventDefault();
        event.stopPropagation();
        setActiveIndex(edgeEnabledIndex(items, key === 'Home' ? 'first' : 'last'));
        return;
      }
      if (key === 'Enter' || key === ' ' || key === 'Spacebar') {
        event.preventDefault();
        event.stopPropagation();
        const item = items[activeIndex];
        if (item?.enabled === true) onRun(item);
      }
    };
    // Capture phase: this is what keeps the shell's Escape (tool cancel) from
    // also firing on the press that closes the menu.
    window.addEventListener('keydown', onKeyDown, true);
    return () => window.removeEventListener('keydown', onKeyDown, true);
  }, [items, activeIndex, onDismiss, onRun]);

  // Everything that moves the anchor or takes the window away closes the menu.
  // A press outside is not prevented: it reaches the partition (or the field)
  // underneath, so a drag or a click still does what it would have done.
  useEffect(() => {
    const dismiss = () => onDismiss({ restoreFocus: false });
    const contains = target => {
      const root = rootRef.current;
      return Boolean(root && target && typeof target === 'object' && root.contains(target));
    };
    const onPointerDown = event => { if (!contains(event.target)) dismiss(); };
    // A long menu scrolls inside itself (`max-height`), and that scroll must not
    // dismiss it; a scroll of anything else moves the page out from under it.
    const onScroll = event => { if (!contains(event.target)) dismiss(); };
    document.addEventListener('pointerdown', onPointerDown, true);
    window.addEventListener('blur', dismiss);
    window.addEventListener('resize', dismiss);
    window.addEventListener('scroll', onScroll, true);
    return () => {
      document.removeEventListener('pointerdown', onPointerDown, true);
      window.removeEventListener('blur', dismiss);
      window.removeEventListener('resize', dismiss);
      window.removeEventListener('scroll', onScroll, true);
    };
  }, [onDismiss]);

  const activeId = items[activeIndex]?.id ? `${ITEM_ID_PREFIX}${items[activeIndex].id}` : undefined;
  const sections = menu?.sections ?? [];

  return (
    <div
      ref={rootRef}
      className={['context-menu', placed ? '' : 'is-unplaced'].filter(Boolean).join(' ')}
      role="menu"
      tabIndex={-1}
      aria-activedescendant={activeId}
      style={{ left: `${placed?.left ?? 0}px`, top: `${placed?.top ?? 0}px` }}
      data-yacr-context-menu="cad"
    >
      {sections.map((section, sectionIndex) => (
        <React.Fragment key={section.id}>
          {sectionIndex > 0 ? <div className="context-menu__separator" role="separator" /> : null}
          {section.items.map(item => {
            const index = indexById.get(item.id) ?? -1;
            const disabled = item.enabled !== true;
            return (
              <button
                key={item.id}
                id={`${ITEM_ID_PREFIX}${item.id}`}
                type="button"
                role="menuitem"
                tabIndex={-1}
                className={[
                  'context-menu__item',
                  disabled ? 'is-disabled' : '',
                  index === activeIndex ? 'is-active' : '',
                ].filter(Boolean).join(' ')}
                aria-disabled={disabled ? 'true' : undefined}
                onMouseEnter={() => { if (!disabled) setActiveIndex(index); }}
                onClick={() => { if (!disabled) onRun(item); }}
              >
                <span className="context-menu__label">{t(item.key)}</span>
                {/* The repeat item names the command it would repeat: the name is
                    the machine identifier every other affordance already shows. */}
                {item.id === 'repeat.last' && item.command ? (
                  <span className="context-menu__command">{item.command}</span>
                ) : null}
              </button>
            );
          })}
        </React.Fragment>
      ))}
    </div>
  );
}
