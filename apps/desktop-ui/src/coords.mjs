// Pure formatting of the status-bar coordinate readout.
//
// AutoCAD's readout shows the pointer in **drawing units**, not screen pixels:
// the host projects the very world point it draws the crosshair from
// (`cad_status.cursorWorld = [x, y, z] | null`), and this module only turns that
// point into text. It never derives a coordinate from a DOM pointer event, so a
// pixel position can never be shown under the "coordinates" label.
//
// Kept out of the component (and out of `metrics.mjs`, which owns the partition
// rect) so the formatting contract is unit-testable without a DOM.

/**
 * The placeholder for "no reliable world coordinate": AutoCAD's em-dash form.
 *
 * A session with a 3D view (or with the pointer outside the surface) has no
 * planar cursor at all. That is **not** the origin, so it must never render as
 * `0.0000, 0.0000`.
 */
export const COORDINATE_PLACEHOLDER = '\u2014, \u2014';

/** Decimal places of every printed coordinate (AutoCAD's `LUPREC`-style fixed). */
export const COORDINATE_DECIMALS = 4;

/**
 * One coordinate value, fixed to [`COORDINATE_DECIMALS`] places.
 *
 * `-0` (and any tiny negative that rounds to zero) prints as `0`, not `-0.0000`:
 * a signed zero is not a different drawing coordinate, and the sign is noise the
 * user would have to interpret.
 */
function formatCoordinate(value) {
  const text = value.toFixed(COORDINATE_DECIMALS);
  return Number(text) === 0 ? (0).toFixed(COORDINATE_DECIMALS) : text;
}

/**
 * The status-bar readout for a `cad_status` snapshot.
 *
 * Returns `{ text, hintKey }`:
 *
 * * a real `cursorWorld` → the world point as `X, Y` with `Z` appended **only
 *   when it is non-zero**, matching AutoCAD's habit of printing the working-plane
 *   coordinate for a 2D view (the Z is still there when the view genuinely has
 *   one);
 * * `null` (a 3D view, the pointer outside the surface, or a probe/unsupported
 *   snapshot) → the em-dash placeholder with the "no reliable world coordinate"
 *   hint key. The pixels of the DOM pointer are deliberately **not** a fallback:
 *   they are not drawing coordinates, so showing them under this label would be
 *   a lie.
 *
 * A malformed value (not a 3-element array of finite numbers) is treated exactly
 * like `null`: `parseCadStatus` rejects it before it reaches this function, so
 * this is only the honest rendering of an absent coordinate.
 */
export function formatWorldCursor(status) {
  const point = status?.cursorWorld;
  if (!Array.isArray(point) || point.length !== 3 || !point.every(Number.isFinite)) {
    return { text: COORDINATE_PLACEHOLDER, hintKey: 'status.coordinate_hint_none' };
  }
  const [x, y, z] = point;
  const zText = formatCoordinate(z);
  const plane = `${formatCoordinate(x)}, ${formatCoordinate(y)}`;
  // The Z is part of the readout only when it is a real non-zero elevation; a
  // plane coordinate prints as `X, Y`, exactly like AutoCAD's 2D readout.
  const text = Number(zText) === 0 ? plane : `${plane}, ${zText}`;
  return { text, hintKey: 'status.coordinate_hint_world' };
}
