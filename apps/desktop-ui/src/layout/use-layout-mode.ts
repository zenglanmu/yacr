// The layout hook: one `matchMedia` subscription, live for the document's life.
//
// Why a listener and not a resize handler: `matchMedia` reports the boundary
// crossing the breakpoint, so a drag-resize flips the layout exactly once per
// crossing instead of re-rendering on every pixel, and it keeps working when the
// window is resized by the window manager (maximise, half-tile) rather than by a
// pointer. A live flip must never reload the document: the CAD session, the
// command history and the open drawer all live in this same tree.
//
// The hook is also the writer of the layout attribute, because it is the single
// owner of the resolved value:
//
//   * `document.documentElement[data-layout]` — read by overlays that are
//     portalled outside the shell (the popup layer) and by the stylesheet's one
//     mobile guard;
//   * `App` writes the same value on `.shell` for the shell's own rules.
//
// `initialLayoutMode` is deliberately synchronous: the first paint must already
// have the right layout, or a phone would flash the desktop chrome before the
// effect runs.
import { useLayoutEffect, useState } from 'react';
import { LAYOUT_ATTRIBUTE, MOBILE_MEDIA_QUERY, layoutForWidth, layoutFromMatches } from './layout-mode.ts';
import type { LayoutMode } from './layout-mode.ts';

/** The media list for this document, or `null` where the engine has none. */
function mobileQuery(): MediaQueryList | null {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return null;
  return window.matchMedia(MOBILE_MEDIA_QUERY);
}

/**
 * The layout this document should start in.
 *
 * A `matchMedia` engine answers the same question the stylesheet's media query
 * asks; an engine without it falls back to the CSS-pixel width the window
 * reports, and a document without a window (a test harness) is `desktop` — the
 * layout every shipped host already renders.
 */
export function initialLayoutMode(): LayoutMode {
  const query = mobileQuery();
  if (query) return layoutFromMatches(query.matches);
  if (typeof window === 'undefined') return 'desktop';
  return layoutForWidth(window.innerWidth);
}

/**
 * The current layout, updated live on every breakpoint crossing.
 *
 * `useLayoutEffect`, not `useEffect`: the attribute the stylesheet keys on must
 * be on the document **before the first paint**, or a phone would paint one frame
 * of desktop chrome (the 300px dock, the nav rail) before the mobile rules apply.
 * There is no server render in this bundle, so the layout effect never warns.
 *
 * `addEventListener('change')` is the standard form; WebKitGTK versions that only
 * ship the deprecated `addListener` still work through that branch, so an older
 * engine degrades to "still updates" rather than "never updates".
 */
export function useLayoutMode(): LayoutMode {
  const [layout, setLayout] = useState<LayoutMode>(initialLayoutMode);

  useLayoutEffect(() => {
    const query = mobileQuery();
    const apply = (next: LayoutMode) => {
      setLayout(next);
      if (typeof document !== 'undefined') document.documentElement.setAttribute(LAYOUT_ATTRIBUTE, next);
    };
    // Re-assert on mount: the state initializer may have run before the document
    // existed (a test harness) and the attribute must always match the render.
    apply(layoutFromMatches(query?.matches));
    if (!query) return undefined;
    const onChange = (event: MediaQueryListEvent) => apply(layoutFromMatches(event.matches));
    if (typeof query.addEventListener === 'function') {
      query.addEventListener('change', onChange);
      return () => query.removeEventListener('change', onChange);
    }
    query.addListener(onChange);
    return () => query.removeListener(onChange);
  }, []);

  return layout;
}
