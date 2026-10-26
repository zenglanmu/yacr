// Pure helpers for reporting the CAD partition rect to the host.
//
// The host composites a native CAD surface at exactly this rect (CSS px), so
// the rect must be re-reported whenever the layout moves or resizes it: the
// host keeps its surface metrics in sync from these reports, and a stale rect
// makes `Camera::zoom_at` anchor the wheel zoom off the cursor.
//
// Keeping the math here (instead of inline in the input binding) makes the
// reporting contract unit-testable without a DOM.

/** The rect the host composites the CAD surface at. */
export interface PartitionRect {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
  readonly dpr: number;
}

/** A pointer position local to the partition, in CSS px. */
export interface LocalPoint {
  readonly x: number;
  readonly y: number;
}

/** A `<canvas>` backing-store size in integer physical pixels. */
export interface CanvasBackingSize {
  readonly width: number;
  readonly height: number;
}

/** The minimum a pointer event must expose (a `MouseEvent`/`PointerEvent`). */
export interface PointerPositionEvent {
  readonly clientX: number;
  readonly clientY: number;
}

/** The CAD partition rect, in CSS pixels relative to the webview viewport. */
export function rectFromElement(element: Element, dpr: number): PartitionRect {
  const rect = element.getBoundingClientRect();
  return { x: rect.left, y: rect.top, width: rect.width, height: rect.height, dpr };
}

/**
 * A stable identity for a rect. Two reports with the same key describe the same
 * partition, so the binding can drop the duplicate without re-reporting.
 */
export function metricsKey(rect: PartitionRect): string {
  return [rect.x, rect.y, rect.width, rect.height, rect.dpr].join('|');
}

/** Whether a rect is worth reporting (finite, strictly positive). */
export function isReportableRect(rect: PartitionRect): boolean {
  return [rect.x, rect.y, rect.width, rect.height, rect.dpr].every(Number.isFinite)
    && rect.width > 0
    && rect.height > 0
    && rect.dpr > 0;
}

/**
 * The `<canvas>` backing-store size for a partition rect, in integer **physical**
 * pixels (`CSS px × devicePixelRatio`).
 *
 * ADR 0024 D3: on a `dom-canvas` host the very same rect that drives the native
 * surface metrics also drives the in-DOM canvas, so both carriers are sized from
 * one source. The wasm presenter receives the canvas handle plus this physical
 * size and configures its own surface from it (`cad-render-wgpu::canvas`); the UI
 * never touches wgpu, and this helper stays pure (no DOM, no canvas mutation).
 *
 * Rounding policy: each axis is rounded to the **nearest** integer with
 * `Math.round` (uniform, not a directional floor), then clamped to at least `1`.
 * The clamp only matters for a positive rect whose physical size rounds to zero
 * (e.g. `0.3` CSS px at DPR 1): a zero dimension cannot be configured as a
 * surface, so reporting `1` is the honest smallest size instead of a silent `0`.
 *
 * Returns `null` for a degenerate rect (any non-finite value, or a non-positive
 * width, height or dpr). "No size" is not a size: the caller must not size or
 * present then, and must never substitute a placeholder size.
 */
export function canvasBackingSize(rect: PartitionRect | null | undefined): CanvasBackingSize | null {
  if (!rect) return null;
  const { width, height, dpr } = rect;
  if (![width, height, dpr].every(Number.isFinite)) return null;
  if (width <= 0 || height <= 0 || dpr <= 0) return null;
  return {
    width: Math.max(1, Math.round(width * dpr)),
    height: Math.max(1, Math.round(height * dpr)),
  };
}

/** Local CSS-pixel position of a pointer event inside the partition. */
export function pointerPosition(element: Element, event: PointerPositionEvent): LocalPoint {
  const rect = element.getBoundingClientRect();
  return { x: event.clientX - rect.left, y: event.clientY - rect.top };
}
