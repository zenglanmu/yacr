// The navigation rail's open state, lifted out of the rail so the one collapse
// control can sit in the dock's head, directly under the ViewCube.
//
// Two carriers of the same boolean, because the two layouts answer to different
// owners:
//
//   * **Desktop.** The host is the authority: `cad_status.navigation.navbarVisible`
//     is the host's own `NAVBAR` setting, so the toggle is a real `NAVBAR`
//     command and the chrome keeps no second copy of that setting. When the host
//     reports the navbar off, the rail stays closed and the toggle carries the
//     catalog's own "the navigation bar is off" sentence instead of pretending
//     the user collapsed it.
//   * **Compact (<= 1024 CSS px).** The desktop navbar flag describes the desktop
//     band, which this layout does not render, so the rail is local state. It
//     starts collapsed — the ViewCube alone is the compact orientation cue and
//     the card must not open onto a drawing a phone can barely see — and returns
//     to collapsed on a **breakpoint crossing** only. The reset lives in a
//     `useLayoutEffect` keyed on `shell.mobile`, so an ordinary render (a status
//     poll, a theme switch, a palette toggle) never closes a rail the user just
//     opened.
//
// The three sentences are returned as catalog **keys**, not translated text:
// this module holds state, the component holds the translator.
import { useLayoutEffect, useState } from 'react';
import { useShell } from '../context.ts';
import { navigationSettings } from '../cad/viewcube.ts';

/** The catalog keys the toggle can wear. */
export type NavRailLabelKey = 'navrail.collapse' | 'navrail.expand' | 'navrail.hidden';

/** The rail's open state and the one control that changes it. */
export interface NavRailState {
  /** Whether the rail's items are on screen (the dock's `data-rail` value). */
  readonly open: boolean;
  /** The toggle's accessible name and tooltip, as a catalog key. */
  readonly labelKey: NavRailLabelKey;
  /** Fold the rail away, or unfold it. */
  readonly toggle: () => void;
}

/**
 * The rail's open state for the current layout.
 *
 * Every hook runs before the layout branch, so the hook order is identical in
 * both layouts across a live breakpoint crossing.
 */
export function useNavRailState(): NavRailState {
  const shell = useShell();
  const mobile = shell.mobile;
  const [compactOpen, setCompactOpen] = useState(false);
  useLayoutEffect(() => {
    if (mobile) setCompactOpen(false);
  }, [mobile]);
  if (mobile) {
    return {
      open: compactOpen,
      labelKey: compactOpen ? 'navrail.collapse' : 'navrail.expand',
      toggle: () => setCompactOpen(value => !value),
    };
  }
  const navigation = navigationSettings(shell.status?.navigation);
  return {
    open: navigation.navbarVisible,
    labelKey: navigation.navbarVisible ? 'navrail.collapse' : 'navrail.hidden',
    toggle: () => shell.runCommand('NAVBAR'),
  };
}
