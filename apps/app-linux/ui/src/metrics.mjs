// Pure helpers for reporting the CAD partition rect to the host.
//
// The host composites a native CAD surface at exactly this rect (CSS px), so
// the rect must be re-reported whenever the layout moves or resizes it: the
// host keeps its surface metrics in sync from these reports, and a stale rect
// makes `Camera::zoom_at` anchor the wheel zoom off the cursor.
//
// Keeping the math here (instead of inline in the input binding) makes the
// reporting contract unit-testable without a DOM.

/** The CAD partition rect, in CSS pixels relative to the webview viewport. */
export function rectFromElement(element, dpr) {
  const rect = element.getBoundingClientRect();
  return { x: rect.left, y: rect.top, width: rect.width, height: rect.height, dpr };
}

/**
 * A stable identity for a rect. Two reports with the same key describe the same
 * partition, so the binding can drop the duplicate without re-reporting.
 */
export function metricsKey(rect) {
  return [rect.x, rect.y, rect.width, rect.height, rect.dpr].join('|');
}

/** Whether a rect is worth reporting (finite, strictly positive). */
export function isReportableRect(rect) {
  return [rect.x, rect.y, rect.width, rect.height, rect.dpr].every(Number.isFinite)
    && rect.width > 0
    && rect.height > 0
    && rect.dpr > 0;
}

/** Local CSS-pixel position of a pointer event inside the partition. */
export function pointerPosition(element, event) {
  const rect = element.getBoundingClientRect();
  return { x: event.clientX - rect.left, y: event.clientY - rect.top };
}
