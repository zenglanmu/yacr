// Pure geometry for the floating QuickCalc window.
//
// QuickCalc is a **non-modal** float, not a dialog: it is portalled to
// `document.body` and painted with `position: fixed`, so its coordinate space is
// the viewport, not the drawing-area rect the left-column palettes use. The
// drag/clamp arithmetic is still the shared palette arithmetic
// ([`palette-window.ts`]): a drag keeps the window inside the viewport, lets it
// overhang an edge only as far as `PALETTE_MIN_VISIBLE_WIDTH`, and never lets
// the header leave vertically.
//
// This module owns only what is QuickCalc's own: the two mode widths, the
// matching nominal heights used to anchor the first placement near the
// lower-right corner, and the thin wrappers that name the viewport as the layer.
// No React, no DOM, so `test/quickcalc-window.test.ts` can pin every edge.
import { clampGeometry, clampWidth, moveGeometry } from './palette-window.ts';
import type { PaletteGeometry, PaletteLayerSize } from './palette-window.ts';

/** Which keypad the window currently shows. */
export type QuickCalcMode = 'basic' | 'scientific';

/**
 * The basic card's width. It is the width the old modal used, and it keeps the
 * familiar 4-column numeric keypad at the same key module as before.
 */
export const QUICKCALC_BASIC_WIDTH = 384;

/**
 * The scientific card is exactly one key column wider than the basic one, so
 * the function strip above the keypad uses the *same* key module as the digits
 * instead of inventing a second rhythm.
 */
export const QUICKCALC_SCIENTIFIC_WIDTH = 473;

/**
 * The nominal card heights. They are only used to anchor the first placement
 * (`top = viewport - margin - height`); the real height is content-driven and
 * clamped by the viewport in CSS and by the inline `max-height`.
 */
export const QUICKCALC_BASIC_HEIGHT = 440;
export const QUICKCALC_SCIENTIFIC_HEIGHT = 552;

/**
 * The gap kept between the window and every viewport edge. It mirrors the CSS
 * `calc(100vw - 16px)` / `calc(100vh - 16px)` clamps, so the arithmetic and the
 * rendered card agree on how much room they are allowed.
 */
export const QUICKCALC_EDGE_MARGIN = 16;

/** The width a mode renders at, before the viewport is taken into account. */
export function quickCalcNominalWidth(mode: QuickCalcMode): number {
  return mode === 'scientific' ? QUICKCALC_SCIENTIFIC_WIDTH : QUICKCALC_BASIC_WIDTH;
}

/** The nominal height a mode opens at (see [`QUICKCALC_BASIC_HEIGHT`]). */
export function quickCalcNominalHeight(mode: QuickCalcMode): number {
  return mode === 'scientific' ? QUICKCALC_SCIENTIFIC_HEIGHT : QUICKCALC_BASIC_HEIGHT;
}

/**
 * The width to use for clamping on a viewport `viewportWidth` wide.
 *
 * It matches the stylesheet exactly: `min(nominal, 100vw - 16px)`, with the
 * shared 280px floor winning on a viewport too small for it (the card then
 * overhangs, but it stays usable instead of collapsing to nothing).
 */
export function quickCalcWidth(mode: QuickCalcMode, viewportWidth?: number): number {
  const nominal = quickCalcNominalWidth(mode);
  if (typeof viewportWidth !== 'number' || !Number.isFinite(viewportWidth) || viewportWidth <= 0) {
    return nominal;
  }
  const available = Math.round(viewportWidth) - QUICKCALC_EDGE_MARGIN;
  return clampWidth(Math.min(nominal, available), viewportWidth);
}

/**
 * A viewport measured in CSS px. A missing or non-finite side counts as `0`, so
 * a caller without a window still gets finite numbers rather than `NaN`
 * geometry that would paint the card at an undefined position.
 */
export function normalizeViewport(viewport: PaletteLayerSize): PaletteLayerSize {
  const width = viewport && Number.isFinite(viewport.width) ? Math.max(0, Math.round(viewport.width)) : 0;
  const height = viewport && Number.isFinite(viewport.height) ? Math.max(0, Math.round(viewport.height)) : 0;
  return { width, height };
}

/**
 * The first placement: near the lower-right corner of the viewport, never
 * off-screen.
 *
 * The left edge is measured from the right margin with the *clamped* width, so
 * a viewport barely wider than the card still leaves the promised 16px gutter
 * on both sides instead of pushing the card over the left edge. The top edge is
 * anchored by the mode's nominal height for the same reason; `clampGeometry`
 * then makes the final call (a short viewport pulls the card up so its header
 * stays reachable).
 */
export function defaultQuickCalcGeometry(
  viewport: PaletteLayerSize,
  width: number = QUICKCALC_BASIC_WIDTH,
  height: number = QUICKCALC_BASIC_HEIGHT,
): PaletteGeometry {
  const size = normalizeViewport(viewport);
  const clampedWidth = clampWidth(width, size.width);
  const requestedHeight = Number.isFinite(height) ? Math.max(0, Math.round(height)) : QUICKCALC_BASIC_HEIGHT;
  return clampGeometry(
    {
      left: size.width - QUICKCALC_EDGE_MARGIN - clampedWidth,
      top: size.height - QUICKCALC_EDGE_MARGIN - requestedHeight,
      width: clampedWidth,
    },
    size,
  );
}

/** Clamp a stored placement to the current viewport (a window resize re-runs it). */
export function clampQuickCalc(geometry: PaletteGeometry, viewport: PaletteLayerSize): PaletteGeometry {
  return clampGeometry(geometry, normalizeViewport(viewport));
}

/** Move a placement by a pointer delta and clamp it to the viewport. */
export function moveQuickCalc(
  start: PaletteGeometry,
  deltaX: number,
  deltaY: number,
  viewport: PaletteLayerSize,
): PaletteGeometry {
  return moveGeometry(start, deltaX, deltaY, normalizeViewport(viewport));
}
