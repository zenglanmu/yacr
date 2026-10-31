// Which chrome surface a layout can actually show, as one pure table.
//
// The layout is a function of the window width alone (`layout-mode.ts`): the
// desktop band and the compact chrome are the **same React tree**, and the
// compact layout simply does not render some of the desktop regions. The docked
// properties/layers palettes are hidden by `mobile.css`, the mobile top bar and
// command bar are unconditional (so there is no clean screen and no command-line
// toggle), and the history list is the mobile command bar's own sheet.
//
// A global shortcut or a typed selector that names a missing region must not
// flip a flag that paints nothing. This module is the one place that decides
// what each entry point does in each layout, so the caller never has to repeat
// the question — and when no compact surface exists the answer is a refusal in
// the catalog's words (`DOCKED_PANEL_REFUSAL_KEY` plus the panel's own label),
// never a silent no-op.
//
// Pure data + functions (no React, no DOM, no catalog), so the contract is
// testable without a window.
import type { LayoutMode } from './layout-mode.ts';

/** The four chrome shortcuts the shell routes per layout. */
export type LayoutShortcut = 'cleanScreen' | 'properties' | 'commandLine' | 'history';

/** What the shell should do for one shortcut in the running layout. */
export type ShortcutAction =
  /** Flip the clean-screen state (desktop chrome only). */
  | 'toggle-clean-screen'
  /** Toggle the docked properties palette. */
  | 'toggle-properties'
  /** Toggle the docked command line. */
  | 'toggle-command-line'
  /** Toggle the history list (`mobile` renders it as the command bar's sheet). */
  | 'toggle-history'
  /** No surface exists here: refuse in words. */
  | 'refuse-docked-panel'
  /** Not this layout's key: the shell takes no action for it. */
  | 'none';

/**
 * The catalog key for a docked palette this layout does not render.
 *
 * `{panel}` is the panel's own catalog label (`properties.panel` /
 * `layers.panel`), so one sentence covers both palettes instead of two copies of
 * the same fact.
 */
export const DOCKED_PANEL_REFUSAL_KEY = 'mobile.refusal.docked_panel';

/**
 * Whether `layout` renders the docked palettes (`Workspace`'s left column).
 *
 * Only the desktop band does: the compact layout hides `.workspace__left` and
 * never re-opens it, which is also the rule `mobile-ribbon.ts` applies to the
 * drawer's palette toggles.
 */
export function dockedPaletteOpenable(layout: LayoutMode): boolean {
  return layout === 'desktop';
}

/**
 * The action `shortcut` has in `layout`.
 *
 * `textStep` is read for the mobile history sheet only: while the command bar is
 * a tool's text prompt it hides its history (its own toggle is gone too), so
 * flipping the flag there would paint nothing either. On the desktop band the
 * history panel is rendered in both modes, so the key keeps working.
 */
export function shortcutAction(
  layout: LayoutMode,
  shortcut: LayoutShortcut,
  { textStep = false }: { readonly textStep?: boolean } = {},
): ShortcutAction {
  if (shortcut === 'history') {
    return layout === 'mobile' && textStep ? 'none' : 'toggle-history';
  }
  if (shortcut === 'properties') {
    return dockedPaletteOpenable(layout) ? 'toggle-properties' : 'refuse-docked-panel';
  }
  // The compact layout has no clean screen (the two bars are the whole chrome)
  // and no command-line toggle (its command bar is always visible). Neither key
  // is offered there, and neither may leave a state change behind. The shell
  // does not claim clean-screen on mobile; WebKit's Ctrl+0 zoom reset is a
  // separate suppression that applies in every layout (page zoom desyncs the
  // host composite rect), so `none` is not "the chord reaches the platform".
  if (layout !== 'desktop') return 'none';
  return shortcut === 'commandLine' ? 'toggle-command-line' : 'toggle-clean-screen';
}
