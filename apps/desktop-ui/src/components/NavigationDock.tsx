// Navigation dock: the right-edge column that carries the ViewCube above the
// icon rail.
//
// It is an absolute overlay inside the already-relative `<Workspace />`, pinned
// to the row's right edge with a fixed width (`--navrail-w`). It takes no flex
// space, so the CAD partition rect spans the whole row (minus the start panel)
// and the 56px column is opaque chrome painted over the drawing instead of width
// taken from it. Collapsing the rail changes what the column *shows*, never how
// much drawing the user gets: the CAD partition rect stays put and the host is
// not asked to resize its surface. The whole column, empty background included,
// is ordinary hit-testable DOM (see `NavRail.tsx`).
import React from 'react';
import { ViewCube } from './ViewCube.tsx';
import { NavRail } from './NavRail.tsx';

export function NavigationDock() {
  return (
    <div className="navdock app-region">
      <ViewCube />
      <NavRail />
    </div>
  );
}
