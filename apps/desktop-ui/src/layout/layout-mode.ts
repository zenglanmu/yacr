// Width-based layout switch for the one shared chrome.
//
// The same bundle serves the desktop hosts (native Tauri windows), the Android
// WebView and the browser build, so the layout is a **function of the window
// width**, not of the platform: no host flag, no user setting, no second tree.
// The breakpoint is one exported constant because three places must agree on it
// — the `matchMedia` query the hook subscribes to, the `data-layout` attribute
// the stylesheet keys on, and the contract tests.
//
// CSS px, not device px: `matchMedia` evaluates the media query in CSS pixels,
// which is exactly the unit the stylesheet's breakpoints (`900px`, `640px`) use.
// A phone at DPR 3 reports 390 CSS px and is therefore `mobile`; the same phone
// **in landscape** reports ~914 CSS px on a 2400x1080 class screen and must get
// the compact chrome too — that is the width that used to fall to the desktop
// band and leave the drawing 153 px of a 412 px-tall window.
//
// **1024, and the rationale (product decision, recorded here because it is the
// one place the number exists):** the layout is chosen by **screen width only**,
// and 1024 CSS px is the last width that is still a phone or a tablet rather
// than a desktop window. Every phone/tablet in landscape reports <= 1024 (a
// 4:3 tablet in portrait is exactly 1024, and `max-width` is inclusive); the
// first desktop window width is 1025. Anything narrower gets the compact chrome
// whatever the platform, anything wider keeps the band, the dock and the status
// bar it has always had.

/**
 * Widest window (CSS px) that still gets the compact mobile layout.
 *
 * Phone portrait (390), phone landscape (~914) and every tablet in either
 * orientation (<= 1024) are `mobile`; 1025 px is the first `desktop` width.
 */
export const MOBILE_MAX_WIDTH = 1024;

/** The one media query the layout hook subscribes to. */
export const MOBILE_MEDIA_QUERY = `(max-width: ${MOBILE_MAX_WIDTH}px)`;

/**
 * The attribute carrying the resolved layout.
 *
 * It is written on the `.shell` root (so the shell's own rules can key on it) and
 * on `document.documentElement` (so overlays portalled outside the shell — the
 * popup layer — see the same value). One attribute name, one meaning.
 */
export const LAYOUT_ATTRIBUTE = 'data-layout';

/** The two layouts this bundle renders. */
export type LayoutMode = 'mobile' | 'desktop';

/**
 * The layout for a window width in CSS px.
 *
 * A missing or non-finite width answers `desktop`: that is the layout every
 * existing host already renders, so an unreadable measurement never invents the
 * compact chrome. The comparison is inclusive (`1024` is mobile), which is what
 * `max-width: 1024px` means — the constant and the query must not disagree.
 */
export function layoutForWidth(width: number | null | undefined): LayoutMode {
  if (typeof width !== 'number' || !Number.isFinite(width)) return 'desktop';
  return width <= MOBILE_MAX_WIDTH ? 'mobile' : 'desktop';
}

/**
 * The layout for a media-query match.
 *
 * `true` is the only value that means mobile: an engine without `matchMedia`
 * (or a listener that reports something unexpected) keeps the desktop layout it
 * has today.
 */
export function layoutFromMatches(matches: boolean | null | undefined): LayoutMode {
  return matches === true ? 'mobile' : 'desktop';
}
