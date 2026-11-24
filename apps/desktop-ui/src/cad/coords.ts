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
export const COORDINATE_PLACEHOLDER: string = '\u2014, \u2014';

import { formatLength, unitsFormat } from './units-format.ts';
import type { CadUnits, UnitsFormat } from './units-format.ts';

/** Decimal places of the legacy readout when the host reports no `units` block. */
export const COORDINATE_DECIMALS = 4;

/** The status slice the readout reads: `cad_status.cursorWorld` + `units`. */
export interface WorldCursorStatus {
  readonly cursorWorld?: unknown;
  /** The document's stored `UNITS` block; absent keeps the legacy format. */
  readonly units?: CadUnits | null;
}

/** The rendered readout plus the catalog key of its hint. */
export interface WorldCursorReadout {
  readonly text: string;
  readonly hintKey: 'status.coordinate_hint_none' | 'status.coordinate_hint_world';
}

/**
 * One coordinate value in the document's stored length format and precision.
 *
 * A non-finite ordinate cannot be a drawing coordinate; the caller already
 * rejects the whole point before this runs, so the legacy fallback here is only
 * a defensive, honest `null` path.
 */
function formatCoordinate(value: number, units: UnitsFormat): string {
  return formatLength(value, units) ?? (0).toFixed(COORDINATE_DECIMALS);
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
export function formatWorldCursor(status: WorldCursorStatus | null | undefined): WorldCursorReadout {
  const point = status?.cursorWorld;
  if (!Array.isArray(point) || point.length !== 3 || !point.every(Number.isFinite)) {
    return { text: COORDINATE_PLACEHOLDER, hintKey: 'status.coordinate_hint_none' };
  }
  const [x, y, z] = point as number[];
  const format = unitsFormat(status?.units);
  const zero = formatCoordinate(0, format);
  const zText = formatCoordinate(z, format);
  const plane = `${formatCoordinate(x, format)}, ${formatCoordinate(y, format)}`;
  // The Z is part of the readout only when it is a real non-zero elevation; a
  // plane coordinate prints as `X, Y`, exactly like AutoCAD's 2D readout.
  const text = zText === zero ? plane : `${plane}, ${zText}`;
  return { text, hintKey: 'status.coordinate_hint_world' };
}
