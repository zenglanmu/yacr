// Navigation dock: the right-edge navigation control — the ViewCube, the one
// collapse/expand control directly under it, and the icon rail below that.
//
// The head is the part that is *always* present, in both rail states and on both
// layouts: the cube (the orientation cue) and the toggle (the one control that
// folds the rail away). Keeping the toggle in the head rather than as the rail's
// first item is what lets the compact layout collapse to the cube plus a single
// expand icon instead of an empty 56px strip, and it puts the control next to the
// thing it acts on.
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
import { useI18n } from '../context.ts';
import { useNavRailState } from '../layout/nav-rail-state.ts';
import { ViewCube } from './ViewCube.tsx';
import { NavRail } from './NavRail.tsx';

export function NavigationDock() {
  const { t } = useI18n();
  const { open, labelKey, toggle } = useNavRailState();
  const label = t(labelKey);
  return (
    <div className="navdock app-region" data-rail={open ? 'open' : 'closed'}>
      <div className="navdock__head">
        <ViewCube />
        {/* The rail is vertical, so up/down reads as "grows here" / "folds away"
            better than left/right would. */}
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
