// Which chrome surface a layout can actually show, as one pure table.
//
// The layout is a function of the window width alone (`layout-mode.ts`): the
// desktop band and the compact chrome are the **same React tree**, and the
// compact layout simply does not render some of the desktop regions. The palette
// windows are hidden by `mobile.css`, but the properties inspector is **not** one
// of the surfaces the compact layout loses: it is mounted again as its own bottom
// sheet (`MobilePropertiesSheet`), so `PROPERTIES`, its chord and its ribbon entry
// keep working there while every other docked palette does not.
//
// A global shortcut or a typed selector that names a missing region must not flip
// a flag that paints nothing. This module is the one place that decides what each
// entry point does in each layout, so the caller never has to repeat the question
// — and where no compact surface exists the answer is a refusal in the catalog's
// words (`DOCKED_PANEL_REFUSAL_KEY` plus the panel's own label), never a silent
// no-op.
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
  /** Toggle the properties inspector (its own window, or the compact sheet). */
  | 'toggle-properties'
  /** Toggle the docked command line. */
  | 'toggle-command-line'
  /** Toggle the history list (`mobile` renders it as the command bar's sheet). */
  | 'toggle-history'
  /** Not this layout's key: the shell takes no action for it. */
  | 'none';

/**
 * The catalog key for a desktop palette this layout does not render.
 *
 * `{panel}` is the panel's own catalog label (`layers.panel`), so one sentence
 * covers every palette the compact layout cannot show instead of one copy per
 * palette. The typed selectors still answer with it; no **shortcut** needs it any
 * more, because the one palette with a compact surface (properties) no longer
 * refuses.
 */
export const DOCKED_PANEL_REFUSAL_KEY = 'mobile.refusal.docked_panel';

/**
 * Whether `layout` renders the desktop palette windows (`Workspace`'s floating
 * layer).
 *
 * Only the desktop band does: the compact layout hides `.palette-layer` and never
 * re-opens it, which is also the rule `mobile-ribbon.ts` applies to the drawer's
 * palette toggles. The properties inspector is the documented exception — it is
 * re-mounted as a compact sheet, which is why `shortcutAction` below never
 * refuses it even though this predicate is `false` there.
 *
 * The palettes are not part of `.workspace__left` any more - that column belongs
 * to the start screen alone.
 */
export function dockedPaletteOpenable(layout: LayoutMode): boolean {
  return layout === 'desktop';
}

/**
 * The action `shortcut` has in `layout`.
 *
 * `properties` is layout-independent: the inspector exists in both layouts (as a
 * window on the desktop band, as a bottom sheet on the compact one), so the chord
 * toggles the one `propertiesOpen` flag everywhere.
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
  if (shortcut === 'properties') return 'toggle-properties';
  // The compact layout has no clean screen (the two bars are the whole chrome)
  // and no command-line toggle (its command bar is always visible). Neither key
  // is offered there, and neither may leave a state change behind. The shell
  // does not claim clean-screen on mobile; WebKit's Ctrl+0 zoom reset is a
  // separate suppression that applies in every layout (page zoom desyncs the
  // host composite rect), so `none` is not "the chord reaches the platform".
  if (layout !== 'desktop') return 'none';
  return shortcut === 'commandLine' ? 'toggle-command-line' : 'toggle-clean-screen';
}
