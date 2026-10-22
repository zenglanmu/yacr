// Ribbon overflow: the panel strip is one horizontal scroller inside a fixed
// 92px band, so a narrow window leaves panels past the right edge — and the
// overlay scrollbar the WebView draws when idle is invisible, which leaves a
// reader with a clipped label and no hint that anything is missing.
//
// This module is the **pure** part of the answer: given the strip's scroll
// metrics it says whether anything is hidden on either side, and how far one
// cue click should page. The renderer owns the DOM listener; the tests own this
// predicate, so the "cue only when there is overflow" contract is testable
// without a browser.
//
// `overflowing` is the honest question ("is anything hidden at all?"), `start`
// and `end` are the two directions a cue can page. A strip scrolled all the way
// to one end has no cue on that end, because clicking it would do nothing.

/**
 * Sub-pixel slack: scroll metrics are reported as integers but real layout is
 * fractional, so a strip with nothing hidden can still report `scrollWidth`
 * one pixel larger than `clientWidth`. Anything within this distance of a limit
 * counts as being **at** that limit.
 */
export const OVERFLOW_EPSILON = 1;

/**
 * Overflow state of a horizontal scroller.
 *
 * `{scrollWidth, clientWidth, scrollLeft}` is any object with those three
 * numbers: a DOM element satisfies it as-is, and a test passes a literal. A
 * missing or nonsensical value degrades to "nothing overflows", which is the
 * state that draws no cue.
 */
export function ribbonOverflow({ scrollWidth = 0, clientWidth = 0, scrollLeft = 0 } = {}) {
  const hidden = scrollWidth - clientWidth;
  if (!(hidden > OVERFLOW_EPSILON)) {
    return { overflowing: false, start: false, end: false };
  }
  // Negative `scrollLeft` appears on RTL / overscroll; clamp before comparing so
  // an overscrolled strip still shows the cue for the end it can reach.
  const offset = Math.min(Math.max(Number(scrollLeft) || 0, 0), hidden);
  return {
    overflowing: true,
    start: offset > OVERFLOW_EPSILON,
    end: offset < hidden - OVERFLOW_EPSILON,
  };
}

/**
 * How far one cue click pages the strip: most of a viewport, but never so small
 * a click looks broken on a narrow window and never so large the reader loses
 * their place. 80% keeps one panel of context on screen.
 */
export function ribbonScrollStep(clientWidth = 0) {
  const width = Number(clientWidth) || 0;
  if (width <= 0) return 0;
  return Math.max(120, Math.round(width * 0.8));
}

/**
 * The next scroll offset for a cue click, clamped to the strip's own limits.
 *
 * `direction` is `'start'` (page left) or `'end'` (page right); anything else
 * returns the current offset unchanged rather than guessing a direction.
 */
export function ribbonScrollTarget({ scrollWidth = 0, clientWidth = 0, scrollLeft = 0 } = {}, direction) {
  const hidden = Math.max(0, scrollWidth - clientWidth);
  const offset = Math.min(Math.max(Number(scrollLeft) || 0, 0), hidden);
  const step = ribbonScrollStep(clientWidth);
  if (direction === 'start') return Math.max(0, offset - step);
  if (direction === 'end') return Math.min(hidden, offset + step);
  return offset;
}

/**
 * Whether two overflow states say the same thing.
 *
 * The strip fires a scroll event for every frame of a smooth page; returning the
 * previous state object when nothing changed is what keeps that from re-rendering
 * every panel on each frame.
 */
export function sameOverflow(a, b) {
  return Boolean(a?.overflowing) === Boolean(b?.overflowing)
    && Boolean(a?.start) === Boolean(b?.start)
    && Boolean(a?.end) === Boolean(b?.end);
}

/**
 * Overflow of a strip that reserves space for its own cues.
 *
 * The measurement has to be taken against the **band**, not against the narrowed
 * strip: the reserve is a *consequence* of overflow, so comparing inside the
 * narrowed box latches the cue on — after the window grows back, `scrollWidth >
 * clientWidth` stays true precisely because `clientWidth` lost the reserve, and
 * no further event arrives to correct it. `bandWidth` is the width the strip
 * would have with no cue at all, which is the question the reader cares about
 * ("does the ribbon's content fit this window?").
 */
export function ribbonBandOverflow({ scrollWidth = 0, bandWidth = 0, scrollLeft = 0 } = {}) {
  return ribbonOverflow({ scrollWidth, clientWidth: bandWidth, scrollLeft });
}

/**
 * Band width the cue occupies on one side, in CSS pixels.
 *
 * This is the **only** place the number lives: the renderer writes it into the
 * band's `--ribbon-cue-start/end` custom properties, the stylesheet reads those
 * for the strip's margin and for the cue's own width, and the tests assert the
 * reserve against it. A cue is 30px wide so the 16px chevron keeps ~11px of
 * clear band on either side of it — the caret reads as a control instead of
 * touching the content it pages.
 */
export const RIBBON_CUE_WIDTH = 30;

/**
 * Widths reserved out of the scroll strip for the two cues.
 *
 * A reserve exists **only** for a side that really hides content, so a
 * maximised window reserves nothing and the strip keeps its full width (the
 * maximised ribbon stays byte-identical). The renderer applies these as the
 * strip's start/end margin, which is what moves the strip's clipping edge
 * inward: the last visible item then ends before the cue begins, and the cue
 * (chevron plus the short fade on the content side of it) never covers an item.
 */
export function ribbonCueReserve(state, cueWidth = RIBBON_CUE_WIDTH) {
  return {
    start: state?.start ? cueWidth : 0,
    end: state?.end ? cueWidth : 0,
  };
}

/**
 * Band geometry implied by an overflow state: the reserve, the strip's visible
 * content box and the two cue hit boxes, all in band coordinates.
 *
 * The layout contract the stylesheet implements, expressed as numbers so a test
 * can assert it without a browser: `strip` is where content clips, and each
 * `hit` box starts exactly at the strip edge on its side — no pixel of a hit box
 * overlaps the strip, so no live item is ever under the cue.
 */
export function ribbonCueLayout(metrics, state = ribbonOverflow(metrics), cueWidth = RIBBON_CUE_WIDTH) {
  const width = Math.max(0, Number(metrics?.clientWidth) || 0);
  const reserve = ribbonCueReserve(state, cueWidth);
  const stripLeft = reserve.start;
  const stripRight = Math.max(stripLeft, width - reserve.end);
  return {
    cueWidth,
    reserve,
    strip: { left: stripLeft, right: stripRight, width: stripRight - stripLeft },
    hit: {
      start: { left: 0, right: stripLeft },
      end: { left: stripRight, right: width },
    },
  };
}

