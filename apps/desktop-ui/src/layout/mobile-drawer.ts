// The mobile ribbon drawer's open/closed state, as a pure transition.
//
// The drawer is not a second window and not a route: it is one boolean in this
// tree, and the four ways it changes are named actions. Keeping the transition
// pure means the open/close contract ("running an item closes the drawer", "a
// toggle flips it", "closing twice is a no-op") is testable without React, and
// the component only renders from the returned state.
//
// Invariant on identity: an action that does not change the value returns the
// **same object**, so a redundant close (Escape after the drawer ran an item)
// never re-renders the tree.

/** What the drawer is doing. No other field: the tab lives in `shell.ribbonTab`. */
export interface MobileDrawerState {
  readonly open: boolean;
}

/** The closed drawer: also the initial state of every session. */
export const MOBILE_DRAWER_CLOSED: MobileDrawerState = Object.freeze({ open: false });

/**
 * The drawer actions.
 *
 *   * `toggle` — the top-bar button (open when closed, close when open);
 *   * `open` — an explicit open, used by anything that wants the drawer visible;
 *   * `close` — the close button, a backdrop tap and Escape. Running an item also
 *     ends here (the drawer closes so the drawing and the command line are
 *     visible again), which is why there is no separate "ran" action.
 */
export type MobileDrawerAction = 'toggle' | 'open' | 'close';

/** The next drawer state for `action`. */
export function mobileDrawerNext(state: MobileDrawerState, action: MobileDrawerAction): MobileDrawerState {
  const open = action === 'toggle' ? !state.open : action === 'open';
  return open === state.open ? state : Object.freeze({ open });
}
