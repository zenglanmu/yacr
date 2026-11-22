// The shortcut runner: one closure that turns a chrome chord into exactly one
// surface effect (or "not mine"), using the pure per-layout table in
// `layout-surfaces.ts`.
//
// It lives beside the table rather than inside `App.tsx` because the shell is at
// the file-size ceiling and because the "which layout owns this chord" question
// must stay answerable without a DOM: the runner only receives setters, so a test
// can drive every branch with plain functions.
//
// There is no refusal branch here any more. The one palette that used to be
// refused on the compact layout (properties) now has a compact surface — its own
// bottom sheet — so every chord this runner can see names a surface that exists
// in the layout it runs in. A missing *docked* palette is still refused, but by
// the typed selectors (`commands/local-selectors.ts`), which is where the user
// can reach one.
import { shortcutAction } from './layout-surfaces.ts';
import type { LayoutShortcut } from './layout-surfaces.ts';
import type { LayoutMode } from './layout-mode.ts';

/** A boolean state setter, narrowed to the updater form the runner needs. */
export type BooleanDispatch = (update: (value: boolean) => boolean) => void;

/** The chrome state the runner is allowed to touch, one setter per surface. */
export interface ShortcutTargets {
  readonly setCleanScreen: BooleanDispatch;
  readonly setPropertiesOpen: BooleanDispatch;
  readonly setCommandLineOpen: BooleanDispatch;
  readonly setHistoryOpen: BooleanDispatch;
}

/**
 * Build the per-render keyboard dispatcher.
 *
 * `false` means "the shell takes no action for this chord in this layout" — the
 * caller then leaves the event alone. It never means the platform will act:
 * WebKit's Ctrl+0 zoom reset is suppressed separately in every layout.
 */
export function createShortcutRunner(
  layout: LayoutMode,
  textStep: boolean,
  targets: ShortcutTargets,
): (shortcut: LayoutShortcut) => boolean {
  return (shortcut: LayoutShortcut): boolean => {
    const action = shortcutAction(layout, shortcut, { textStep });
    if (action === 'none') return false;
    if (action === 'toggle-clean-screen') targets.setCleanScreen(value => !value);
    else if (action === 'toggle-properties') targets.setPropertiesOpen(value => !value);
    else if (action === 'toggle-command-line') targets.setCommandLineOpen(value => !value);
    else targets.setHistoryOpen(value => !value);
    return true;
  };
}
