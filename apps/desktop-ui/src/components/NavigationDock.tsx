// Navigation dock: the right-edge navigation control — the compact layout's
// theme switch, the one collapse/expand control, and the icon rail below that.
//
// The orientation ball is **not** in this dock. It is its own floating overlay
// over the drawing area (`.viewball-layer` in `Workspace.tsx`), offset left of
// this 56px rail at the desktop drawing's top-right, and moved to the top-left
// on the compact layout so it never collides with this card at the top-right.
// The rail width (`--navrail-w`) and the dock's stacking level (`z-index: 5`)
// are unchanged; the ball paints at `6`, above the dock and below palettes.
//
// The head is the part that is *always* present, in both rail states and on both
// layouts: the compact layout's theme switch and the toggle (the one control that
// folds the rail away). Keeping the toggle in the head rather than as the rail's
// first item is what lets the compact layout collapse to the theme switch plus a
// single expand icon instead of an empty 56px strip, and it puts the control next
// to the thing it acts on. On the desktop the head therefore holds only the
// toggle.
//
// **The compact layout's theme switch is a head row, not a rail item.** The rail
// is the dock's scroller (it caps the card and scrolls inside, with no scrollbar
// on a phone), so anything that must be reachable without scrolling cannot live
// there — at 960x600 the five view commands already fill the visible part. The
// head is the fixed part, so the switch sits with the chevron; it is
// also why it is mounted here and not inside the rail's `tool bar|导航` landmark,
// whose items are CAD view tools rather than chrome appearance. Compact only: the
// desktop title row owns its one switch, so the desktop column never grows a
// second one.
//
// Head order is **theme -> chevron**: the chevron acts on the rail, so it stays
// the last control, immediately above the thing it expands; the theme switch (on
// compact) is chrome appearance, so it sits before the chevron and never reads as
// the rail's first tool. The head does not scroll or clip (`.navdock__head` is
// `flex: 0 0 auto`, no overflow), and the switch is one 44px row because every
// head row costs rail height on a short viewport.
//
// Desktop: an absolute, full-height 56px (`--navrail-w`) opaque column pinned to
// the right edge of the already-relative `<Workspace />`. It takes no flex space,
// so the CAD partition rect spans the whole row and the column is chrome painted
// over the drawing instead of width taken from it; collapsing the rail changes
// what the column *shows*, never how much drawing the user gets, and the host is
// never asked to resize its surface. The whole column, empty background
// included, is ordinary hit-testable DOM.
//
// Compact: the same element is a content-sized floating card at the drawing's
// top-right corner (see `mobile.css`), so a phone loses a card the width of one
// control instead of a full-height strip.
import React from 'react';
import { Icon } from '../icons.tsx';
import { useI18n, useShell } from '../context.ts';
import { useNavRailState } from '../layout/nav-rail-state.ts';
import { NavRail } from './NavRail.tsx';
import { ThemeToggle } from './ThemeToggle.tsx';

export function NavigationDock() {
  const { t } = useI18n();
  const shell = useShell();
  const { open, labelKey, toggle } = useNavRailState();
  const label = t(labelKey);
  return (
    <div className="navdock app-region" data-rail={open ? 'open' : 'closed'}>
      <div className="navdock__head">
        {shell.mobile ? (
          // Chrome appearance, reachable without scrolling, outside the rail's
          // `tool bar|导航` landmark. `ThemeToggle` remains the one owner of the
          // setting; this is only a mount point.
          <div className="navdock__theme">
            <ThemeToggle />
          </div>
        ) : null}
        {/* The rail is vertical, so up/down reads as "grows here" / "folds away"
            better than left/right would. Last in the head: it acts on the rail. */}
        <button
          type="button"
          className="navdock__toggle"
          aria-expanded={open}
          aria-label={label}
          title={label}
          onClick={toggle}
        >
          <Icon name={open ? 'chevronUp' : 'chevronDown'} size={16} />
        </button>
      </div>
      <NavRail open={open} />
    </div>
  );
}
