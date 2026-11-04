// `TCH_COLUMN`: base-6 outline, closed polyline and bulge arcs.
import { MAX_BULGE, MAX_POINT_ABS, reader, skipPropertyBag } from './bit-reader.ts';
import type { Point3 } from './bit-reader.ts';
import { bump } from './diagnostics.ts';
import type { ArcPrimitive, Primitive, VendorRecord } from './types.ts';

export function decodeColumn(record: VendorRecord, reasons: Map<string, number>): Primitive[] | null {
  const main = reader(record.main, record.mainBits, record.mainSkipBits);
  const text = reader(record.text, record.textBits, record.textSkipBits);

  const base = main.readByte();
  if (base !== 6) {
    bump(reasons, 'vendor.tch.column_unsupported_base');
    return null;
  }
  const scale = main.readBitDouble();
  main.readBitDouble(); // second value of ReadTchBase (ignored)
  const baseFlags = main.readByte();
  if ((baseFlags & 1) !== 0) main.readByte(); // handle count, unused
  if ((baseFlags & 2) !== 0) main.readByte(); // handle count, unused
  if (!Number.isFinite(scale) || scale <= 0) {
    bump(reasons, 'vendor.tch.column_header_mismatch');
    return null;
  }
  if (record.dwgVersion < 1021) {
    // Property-bag names are inline in `main` before R2007; that layout is not
    // verified here, so refuse instead of guessing the string encoding.
    bump(reasons, 'vendor.tch.column_text_stream_unsupported');
    return null;
  }
  skipPropertyBag(main, text, 0);

  const version = main.readByte();
  if (version < 1 || version > 6) {
    bump(reasons, 'vendor.tch.column_unsupported_version');
    return null;
  }
  const position = main.read3BitDouble();
  main.readByte(); // shape type
  const count = main.readBitShort();
  if (count < 2 || count > 4096) {
    bump(reasons, 'vendor.tch.column_vertex_count');
    return null;
  }
  const vertices: { x: number; y: number; bulge: number }[] = [];
  for (let index = 0; index < count; index += 1) {
    const x = main.readRawDouble();
    const y = main.readRawDouble();
    const bulge = main.readBitDouble();
    if (
      !Number.isFinite(x) ||
      !Number.isFinite(y) ||
      !Number.isFinite(bulge) ||
      Math.abs(x) > MAX_POINT_ABS ||
      Math.abs(y) > MAX_POINT_ABS ||
      Math.abs(bulge) > MAX_BULGE
    ) {
      bump(reasons, 'vendor.tch.column_geometry_invalid');
      return null;
    }
    vertices.push({ x, y, bulge });
  }
  const height = main.readBitDouble();
  main.readByte(); // opaque byte
  main.readByte(); // handle count (handles live in the handle stream)
  if (version >= 2) main.readByte(); // material
  if (version > 3) {
    const flags = main.readBitShort();
    if ((flags & 8) !== 0) {
      main.readRawDouble();
      main.readRawDouble();
      main.readBitDouble();
    }
  }
  if (version > 4) main.readByte();
  if (version > 5) main.readBitDouble();

  if (main.remaining !== 0) {
    bump(reasons, 'vendor.tch.column_layout_mismatch');
    return null;
  }
  if (
    !Number.isFinite(height) ||
    !Number.isFinite(position.x) ||
    !Number.isFinite(position.y) ||
    !Number.isFinite(position.z)
  ) {
    bump(reasons, 'vendor.tch.column_geometry_invalid');
    return null;
  }
  return columnPrimitives(vertices, position.z, reasons);
}

// ---------------------------------------------------------------------------
// TCH_OPENING (base version 6; only the wall cut is used)
// ---------------------------------------------------------------------------

function columnPrimitives(
  vertices: readonly { x: number; y: number; bulge: number }[],
  z: number,
  reasons: Map<string, number>,
): Primitive[] | null {
  const points: Point3[] = vertices.map(vertex => ({ x: vertex.x, y: vertex.y, z }));
  const firstArc = vertices.findIndex(vertex => vertex.bulge !== 0);
  if (firstArc < 0) {
    // Closed polyline: repeat the first point.
    return [{ kind: 0, points: [...points, { ...points[0] }] }];
  }

  const primitives: Primitive[] = [];
  let run: Point3[] = [];
  // Start right after an arc so straight runs never wrap across the loop seam.
  for (let step = 0; step < vertices.length; step += 1) {
    const index = (firstArc + 1 + step) % vertices.length;
    const next = (index + 1) % vertices.length;
    if (vertices[index].bulge === 0) {
      if (run.length === 0) run.push(points[index]);
      run.push(points[next]);
      continue;
    }
    if (run.length >= 2) primitives.push({ kind: 0, points: run });
    run = [];
    const arc = arcPrimitive(points[index], points[next], z, vertices[index].bulge);
    if (arc === null) {
      bump(reasons, 'vendor.tch.column_arc_invalid');
      return null;
    }
    primitives.push(arc);
  }
  if (run.length >= 2) primitives.push({ kind: 0, points: run });
  return primitives.length > 0 ? primitives : null;
}

/**
 * DWG bulge → world arc. Positive bulge is CCW; the centre sits on the left
 * normal of the chord at `radius * cos(theta/2) * sign(theta)`.
 */
function arcPrimitive(a: Point3, b: Point3, z: number, bulge: number): ArcPrimitive | null {
  const theta = 4 * Math.atan(bulge);
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const chord = Math.hypot(dx, dy);
  if (!Number.isFinite(theta) || chord <= 1e-12) return null;
  const sinHalf = Math.sin(Math.abs(theta) / 2);
  if (!(Math.abs(sinHalf) > 1e-12)) return null;
  const radius = chord / (2 * sinHalf);
  if (!Number.isFinite(radius) || radius <= 0 || radius > MAX_POINT_ABS) return null;
  const height = radius * Math.cos(theta / 2) * Math.sign(theta);
  const cx = (a.x + b.x) / 2 - (dy / chord) * height;
  const cy = (a.y + b.y) / 2 + (dx / chord) * height;
  const startAngle = Math.atan2(a.y - cy, a.x - cx);
  const sweep = theta;
  const values = [cx, cy, z, radius, startAngle, sweep];
  if (!values.every(Number.isFinite)) return null;
  return { kind: 2, cx, cy, cz: z, radius, startAngle, sweep };
}

// ---------------------------------------------------------------------------
// Result frame (ABI v3)
// ---------------------------------------------------------------------------

