// The compact command bar's open state, and the one rule that overrides it.
//
// The bar carries the command field, the host's notice/banner line, the history
// sheet and the plugin echo. It is **hidden by default**: the drawing is what a
// phone is for, and a permanently docked bar costs a tenth of a landscape phone.
// Two things decide whether it is on screen, and only one of them is the user's:
//
//   * the floating trigger — a named toggle, exactly like the drawer's;
//   * a **must-see condition** (`CommandBarReveal`): the line is an answer field
//     (a tool's text step or a pending plugin prompt), the bar is the only surface
//     for a host failure/notice sentence, or it holds an open history sheet.
//
// `visible` is the render decision; `open` is the flag. Keeping them apart is what
// lets the bar come back by itself while a condition holds and leave again when it
// clears, without a second "was this forced?" state. The reveal is a **level**, not
// an edge: while a condition holds the bar stays revealed instead of being closed
// again on every render, so the user never watches it fight a prompt.
//
// Invariant on identity: an action that does not change the value returns the
// **same object**, so a redundant close never re-renders the tree.

/** What the bar's own flag is doing. No other field: the sheet lives in the shell. */
export interface MobileCommandBarState {
  readonly open: boolean;
}

/** The closed bar: also the initial state of every session. */
export const MOBILE_COMMAND_BAR_CLOSED: MobileCommandBarState = Object.freeze({ open: false });

/** The trigger's three actions. */
export type MobileCommandBarAction = 'toggle' | 'open' | 'close';

/** The next state for `action`. */
export function mobileCommandBarNext(state: MobileCommandBarState, action: MobileCommandBarAction): MobileCommandBarState {
  const open = action === 'toggle' ? !state.open : action === 'open';
  return open === state.open ? state : Object.freeze({ open });
}

/**
 * The conditions under which the bar must be readable, whether or not the user
 * asked for it. Each one is a fact the bar is the only surface for on this
 * layout; none of them is a style preference.
 */
export interface CommandBarReveal {
  /** A pending plugin prompt owns the line: the answer must be visible. */
  readonly promptMode: boolean;
  /** The active tool's terminal text step: the text must be visible. */
  readonly textMode: boolean;
  /** A host failure/notice sentence would be shown in the bar's banner. */
  readonly notice: boolean;
  /** The history sheet is open, and it opens upward from this bar. */
  readonly sheet: boolean;
}

/** Whether a must-see condition holds. */
export function commandBarMustShow(reveal: CommandBarReveal): boolean {
  return reveal.promptMode || reveal.textMode || reveal.notice || reveal.sheet;
}

/** Whether the bar is on screen: the user's choice, or a must-see condition. */
export function mobileCommandBarVisible(state: MobileCommandBarState, reveal: CommandBarReveal): boolean {
  return state.open || commandBarMustShow(reveal);
}
