// Pure geometry for the floating palette windows.
//
// The five left-column palettes (properties, layers, diagnostics, references,
// fonts) are overlays inside `.workspace__canvas`. They default to the left edge
// of the drawing area but they never participate in its flex layout, so opening,
// moving, resizing or closing one leaves the `.native-input` rect untouched and
// the host is never asked to resize/re-present the CAD surface.
//
// This module owns the arithmetic only: default placement, cascade offsets, drag
// and resize clamping, and the header-hit predicate. It has no React and no DOM,
// so `test/palette-window.test.ts` can pin every edge without a browser. The
// pointer interaction and the per-window store live in
// `components/PaletteWindow.tsx`.

/** A palette window's placement inside the palette layer, in CSS px. */
export interface PaletteGeometry {
  readonly left: number;
  readonly top: number;
  readonly width: number;
}

/** The palette layer's measured content box (the drawing-area rect). */
export interface PaletteLayerSize {
  readonly width: number;
  readonly height: number;
}

/** The width a palette opens at on a narrow drawing: the old 300px dock width. */
export const PALETTE_DEFAULT_WIDTH = 300;

/** The widest the responsive default may grow to on a wide drawing. */
export const PALETTE_MAX_DEFAULT_WIDTH = 460;

/** The narrowest a palette may be resized to. */
export const PALETTE_MIN_WIDTH = 280;

/**
 * How much of a window must stay inside the layer while dragging. The header is
 * always kept fully inside vertically (`PALETTE_HEADER_HEIGHT`), so this is the
 * horizontal share that keeps a window grabbable even when it overhangs an edge.
 */
export const PALETTE_MIN_VISIBLE_WIDTH = 80;

/** The palette header's height; mirrors `.palette__head` in `style.css`. */
export const PALETTE_HEADER_HEIGHT = 26;

/** A newly opened window's cascade step, so two headers never coincide. */
export const PALETTE_CASCADE_STEP = 24;

/** One keyboard resize step (ArrowLeft/ArrowRight). */
export const PALETTE_RESIZE_STEP = 16;

/**
 * The base z-index for the windows. A window starts just below this (`- index`)
 * and every press lifts the pressed window above the whole layer.
 */
export const PALETTE_Z_BASE = 200;

/**
 * Elements a press on the palette header must not start a drag from. The
 * palette's own close/create buttons and every form control stay clickable; the
 * header text and empty chrome are the drag handle.
 */
export const PALETTE_DRAG_BLOCKER_SELECTOR =
  'button, a, input, select, textarea, [role="button"], [data-no-drag]';

/** The minimum a drag-target check needs from an `EventTarget` (testable). */
export interface DragTargetLike {
  closest?(selector: string): unknown;
}

/**
 * Whether a pointerdown target is header chrome (drag) rather than a control.
 *
 * Accepts any object with an optional `closest`, so a test can pin the rule
 * without a DOM. A target that cannot answer `closest` is never a drag handle:
 * failing closed keeps a clickable control from being swallowed by a drag.
 */
export function isDragHandleTarget(target: DragTargetLike | null | undefined): boolean {
  if (!target || typeof target.closest !== 'function') return false;
  return target.closest(PALETTE_DRAG_BLOCKER_SELECTOR) === null;
}

/** The vertical cascade offset for the `index`-th palette in the open set. */
export function cascadeOffset(index = 0): number {
  const slot = Number.isFinite(index) ? Math.max(0, Math.floor(index)) : 0;
  return slot * PALETTE_CASCADE_STEP;
}

/**
 * The width a palette opens at on a drawing `layerWidth` wide.
 *
 * A property row needs a readable label column *and* a value well, and the old
 * fixed 300px window was too narrow for both. The default therefore follows the
 * drawing area a little (32% of it), bounded below by the old 300px and above by
 * [`PALETTE_MAX_DEFAULT_WIDTH`]. A missing, zero or non-finite layer keeps the
 * fixed default instead of inventing a size.
 */
export function defaultPaletteWidth(layerWidth?: number): number {
  if (typeof layerWidth !== 'number' || !Number.isFinite(layerWidth) || layerWidth <= 0) {
    return PALETTE_DEFAULT_WIDTH;
  }
  const share = Math.round(layerWidth * 0.32);
  return Math.max(PALETTE_DEFAULT_WIDTH, Math.min(PALETTE_MAX_DEFAULT_WIDTH, share));
}

/** The default placement of the `index`-th palette: left edge, cascaded down. */
export function defaultPaletteGeometry(index = 0, layerWidth?: number): PaletteGeometry {
  return { left: 0, top: cascadeOffset(index), width: defaultPaletteWidth(layerWidth) };
}

/** Clamp a width to `[PALETTE_MIN_WIDTH, layerWidth]` (min wins on a tiny layer). */
export function clampWidth(width: number, layerWidth: number): number {
  const maximum = Number.isFinite(layerWidth)
    ? Math.max(PALETTE_MIN_WIDTH, Math.round(layerWidth))
    : PALETTE_DEFAULT_WIDTH;
  const requested = Number.isFinite(width) ? Math.round(width) : PALETTE_DEFAULT_WIDTH;
  return Math.max(PALETTE_MIN_WIDTH, Math.min(requested, maximum));
}

/**
 * Clamp a horizontal position so at least `PALETTE_MIN_VISIBLE_WIDTH` of the
 * window stays inside the layer (or the whole window, when it is narrower).
 */
export function clampLeft(left: number, width: number, layerWidth: number): number {
  const visible = Math.min(PALETTE_MIN_VISIBLE_WIDTH, width);
  const minimum = -(width - visible);
  const maximum = Math.max(minimum, Math.round(layerWidth) - visible);
  const requested = Number.isFinite(left) ? left : 0;
  return Math.round(Math.max(minimum, Math.min(requested, maximum)));
}

/** Clamp a vertical position so the whole header stays reachable. */
export function clampTop(top: number, layerHeight: number): number {
  const maximum = Math.max(0, Math.round(layerHeight) - PALETTE_HEADER_HEIGHT);
  const requested = Number.isFinite(top) ? top : 0;
  return Math.round(Math.max(0, Math.min(requested, maximum)));
}

/** Clamp a whole placement to the layer. Width first: the edges use it. */
export function clampGeometry(geometry: PaletteGeometry, layer: PaletteLayerSize): PaletteGeometry {
  const width = clampWidth(geometry.width, layer.width);
  return {
    left: clampLeft(geometry.left, width, layer.width),
    top: clampTop(geometry.top, layer.height),
    width,
  };
}

/** Move a placement by a pointer delta and clamp it to the layer. */
export function moveGeometry(
  start: PaletteGeometry,
  deltaX: number,
  deltaY: number,
  layer: PaletteLayerSize,
): PaletteGeometry {
  return clampGeometry(
    { left: start.left + deltaX, top: start.top + deltaY, width: start.width },
    layer,
  );
}

/** Resize a placement to a requested width and re-clamp its position. */
export function resizeGeometry(
  start: PaletteGeometry,
  width: number,
  layer: PaletteLayerSize,
): PaletteGeometry {
  return clampGeometry({ left: start.left, top: start.top, width }, layer);
}
