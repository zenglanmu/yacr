// Geometry helpers shared by the annotation / dimension / stair decoders.
import type { Point3 } from './bit-reader.ts';
import type { ArcPrimitive, TextPrimitive } from './types.ts';

/** A bulged polyline segment as a signed circular arc (host primitive kind 2). */
export function bulgeArc(from: Point3, to: Point3, bulge: number): ArcPrimitive | null {
  if (!Number.isFinite(bulge) || bulge === 0) return null;
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const length = Math.hypot(dx, dy);
  if (length < 1e-12) return null;
  const theta = 4 * Math.atan(bulge);
  const radius = length / 2 / Math.sin(theta / 2);
  const height = radius * Math.cos(theta / 2);
  const cx = (from.x + to.x) / 2 - (dy / length) * height;
  const cy = (from.y + to.y) / 2 + (dx / length) * height;
  const startAngle = Math.atan2(from.y - cy, from.x - cx);
  if (![cx, cy, radius, startAngle, theta].every(Number.isFinite)) return null;
  return {
    kind: 2,
    cx,
    cy,
    cz: from.z,
    radius: Math.abs(radius),
    startAngle,
    sweep: theta,
  };
}

/** One text primitive in world coordinates (`attachment` is MTEXT 1..9). */
export function text(
  x: number,
  y: number,
  z: number,
  height: number,
  rotation: number,
  value: string,
  attachment = 7,
  wrapWidth = 0,
): TextPrimitive {
  return { kind: 4, origin: { x, y, z }, height, rotation, wrapWidth, attachment, text: value };
}

/** `snprintf("%.*f")` for a non-negative decimal count, as the ported builders need. */
export function formatNumber(value: number, decimals: number): string {
  const places = Math.min(Math.max(Math.trunc(decimals), 0), 8);
  return value.toFixed(places);
}

/** MiniCAD's `Upright`: keep text readable, returning `true` when flipped. */
export function upright(rotation: number): { rotation: number; flipped: boolean } {
  const twoPi = Math.PI * 2;
  let angle = Math.atan2(Math.sin(rotation), Math.cos(rotation));
  if (angle > Math.PI / 2 + 1e-6 || angle <= -Math.PI / 2 + 1e-6) {
    angle = Math.atan2(Math.sin(angle + Math.PI), Math.cos(angle + Math.PI));
    return { rotation: angle, flipped: true };
  }
  return { rotation: angle, flipped: false };
}

/** Rotate a local `(x, y)` offset by `rotation` and add the origin. */
export function local(originX: number, originY: number, rotation: number, x: number, y: number): Point3 {
  const c = Math.cos(rotation);
  const s = Math.sin(rotation);
  return { x: originX + x * c - y * s, y: originY + x * s + y * c, z: 0 };
}
