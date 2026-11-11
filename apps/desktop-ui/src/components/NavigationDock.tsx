// Navigation dock: the right-edge column that carries the ViewCube above the
// icon rail.
//
// It is one flex child of `<Workspace />` with a fixed width (`--navrail-w`), so
// collapsing the rail changes what the column *shows*, never how much width the
// drawing gets: the CAD partition rect stays put and the host is not asked to
// resize its surface. Both children are ordinary docked DOM — this repo has no
// floating overlay across the native drawing surface (see `NavRail.tsx`).
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
