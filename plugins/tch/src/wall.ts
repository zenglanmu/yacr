// `TCH_WALL`: base-6 protected geometry and opening-driven side/jamb cuts.
import { MAX_POINT_ABS, reader } from './bit-reader.ts';
import type { Point3 } from './bit-reader.ts';
import { add, bump } from './diagnostics.ts';
import type { OpeningCut } from './opening.ts';
import type { Primitive, VendorRecord } from './types.ts';

const WALL_GEOMETRY_BYTES = 84;

export interface WallData {
  start: Point3; end: Point3; left: number; right: number;
  joins: number[]; openings: number; length: number;
}

export function parseWall(record: VendorRecord, reasons: Map<string, number>): WallData | null {
  const main = reader(record.main, record.mainBits, record.mainSkipBits);

  const base = main.readByte();
  if (base !== 6) {
    bump(reasons, 'vendor.tch.wall_unsupported_base');
    return null;
  }
  const scale = main.readBitDouble();
  const headerShort = main.readBitShort();
  const headerByte = main.readByte();
  const headerBits = main.readBits(2);
  const headerRaw = main.readRawShort();
  const mark = main.readByte();
  const key = main.readByte();
  if (
    !Number.isFinite(scale) ||
    scale <= 0 ||
    headerShort !== 0 ||
    headerByte !== 3 ||
    headerBits !== 0 ||
    headerRaw !== 512 ||
    (mark !== 7 && mark !== 8 && mark !== 9)
  ) {
    bump(reasons, 'vendor.tch.wall_header_mismatch');
    return null;
  }

  const protectedBytes = new Uint8Array(WALL_GEOMETRY_BYTES);
  for (let index = 0; index < WALL_GEOMETRY_BYTES; index += 1) {
    protectedBytes[index] = main.readByte();
  }
  unprotect(protectedBytes, mark, key);

  const geometry = new DataView(protectedBytes.buffer);
  const start = geometryPoint(geometry, 0);
  const end = geometryPoint(geometry, 24);
  const angle = geometry.getFloat64(48, true);
  const flags = geometry.getUint32(56, true);
  const left = geometry.getFloat64(60, true);
  const right = geometry.getFloat64(68, true);
  const height = geometry.getFloat64(76, true);
  if (!wallGeometryValid(start, end, angle, left, right, height)) {
    bump(reasons, 'vendor.tch.wall_geometry_invalid');
    return null;
  }
  if (angle !== 0) {
    bump(reasons, 'vendor.tch.wall_arc_unsupported');
    return null;
  }
  if ((flags & 0x10) !== 0) {
    bump(reasons, 'vendor.tch.wall_profile_unsupported');
    return null;
  }

  main.readByte(); // extra (observed 1 on the real fixture)
  const openings = main.readBitShort();
  main.readByte(); // material
  const patches = main.readByte();
  const insulation = main.readBitDouble();
  const joins = [
    main.readBitDouble(),
    main.readBitDouble(),
    main.readBitDouble(),
    main.readBitDouble(),
  ];
  const signatureByte = main.readByte();
  const signatureBits = main.readBits(3);
  // Trailing bytes are padding (observed 0); the record must still end exactly.
  while (main.remaining >= 8) main.readByte();
  if (signatureByte !== 0 || signatureBits !== 2 || main.remaining !== 0) {
    bump(reasons, 'vendor.tch.wall_layout_mismatch');
    return null;
  }
  if (openings < 0 || openings > 1024 || patches > 1) {
    bump(reasons, 'vendor.tch.wall_layout_mismatch');
    return null;
  }
  if (!Number.isFinite(insulation) || !joins.every(Number.isFinite)) {
    bump(reasons, 'vendor.tch.wall_geometry_invalid');
    return null;
  }

  const length = Math.hypot(end.x - start.x, end.y - start.y);
  const joinLimit = length + 2 * (left + right);
  if (!joins.every(join => Math.abs(join) < joinLimit)) {
    bump(reasons, 'vendor.tch.wall_geometry_invalid');
    return null;
  }
  return { start, end, left, right, joins, openings, length };
}

/** Build the wall body, splitting both sides around merged opening intervals. */
export function wallPrimitives(
  data: WallData,
  openings: readonly OpeningCut[],
  reasons: Map<string, number>,
): Primitive[] {
  const { start, end, left, right, joins, length } = data;
  const dx = end.x - start.x;
  const dy = end.y - start.y;
  const ux = dx / length;
  const uy = dy / length;
  const nx = -uy;
  const ny = ux;
  const p0 = {
    x: start.x + nx * left + ux * joins[0],
    y: start.y + ny * left + uy * joins[0],
    z: start.z,
  };
  const p1 = {
    x: end.x + nx * left - ux * joins[1],
    y: end.y + ny * left - uy * joins[1],
    z: end.z,
  };
  const p2 = {
    x: end.x - nx * right - ux * joins[3],
    y: end.y - ny * right - uy * joins[3],
    z: end.z,
  };
  const p3 = {
    x: start.x - nx * right + ux * joins[2],
    y: start.y - ny * right + uy * joins[2],
    z: start.z,
  };

  const tol = 1e-6;
  const sidePoint = (a: Point3, b: Point3, from: number, to: number, t: number): Point3 => {
    if (to - from < 1e-9) return a;
    const k = (Math.min(Math.max(t, from), to) - from) / (to - from);
    return { x: a.x + (b.x - a.x) * k, y: a.y + (b.y - a.y) * k, z: a.z };
  };
  const leftFrom = joins[0];
  const leftTo = length - joins[1];
  const rightFrom = joins[2];
  const rightTo = length - joins[3];
  const leftAt = (t: number): Point3 => sidePoint(p0, p1, leftFrom, leftTo, t);
  const rightAt = (t: number): Point3 => sidePoint(p3, p2, rightFrom, rightTo, t);

  const intervals: Array<[number, number]> = [];
  let used = 0;
  for (const opening of openings) {
    const t = (opening.x - start.x) * ux + (opening.y - start.y) * uy;
    const lo = Math.max(0, t - opening.width / 2);
    const hi = Math.min(length, t + opening.width / 2);
    if (hi - lo > 1e-9) {
      intervals.push([lo, hi]);
      used += 1;
    }
  }
  intervals.sort((a, b) => a[0] - b[0]);
  const cuts: Array<[number, number]> = [];
  for (const interval of intervals) {
    const last = cuts[cuts.length - 1];
    if (last && interval[0] <= last[1] + 1e-9) last[1] = Math.max(last[1], interval[1]);
    else cuts.push([interval[0], interval[1]]);
  }
  if (used > 0) add(reasons, 'vendor.tch.opening_cuts', used);
  if (data.openings > 0 && used === 0) bump(reasons, 'vendor.tch.wall_openings_unmatched');

  const points: Point3[] = [];
  const addSide = (at: (t: number) => Point3, from: number, to: number): void => {
    if (to - from < 1e-9) return;
    let cursor = from;
    for (const [c0, c1] of cuts) {
      if (c1 <= cursor || c0 >= to) continue;
      if (c0 > cursor + tol) points.push(at(cursor), at(c0));
      cursor = Math.max(cursor, c1);
    }
    if (cursor < to - tol) points.push(at(cursor), at(to));
  };
  addSide(leftAt, leftFrom, leftTo);
  addSide(rightAt, rightFrom, rightTo);

  const jambFrom = Math.max(leftFrom, rightFrom);
  const jambTo = Math.min(leftTo, rightTo);
  for (const [c0, c1] of cuts) {
    for (const t of [c0, c1]) {
      if (t > jambFrom + tol && t < jambTo - tol) points.push(leftAt(t), rightAt(t));
    }
  }
  // Both end caps stay unconditional; connection handles are still not read.
  points.push(p0, p3, p1, p2);
  return [{ kind: 1, points }];
}

function unprotect(bytes: Uint8Array, mark: number, key: number): void {
  for (let index = 0; index < bytes.length; index += 1) {
    let value = bytes[index];
    if (mark === 7) value = ((value << 4) | (value >>> 4)) & 0xff;
    bytes[index] = value ^ key;
  }
}

function geometryPoint(view: DataView, offset: number): Point3 {
  return {
    x: view.getFloat64(offset, true),
    y: view.getFloat64(offset + 8, true),
    z: view.getFloat64(offset + 16, true),
  };
}

function wallGeometryValid(
  start: Point3,
  end: Point3,
  angle: number,
  left: number,
  right: number,
  height: number,
): boolean {
  const values = [start.x, start.y, start.z, end.x, end.y, end.z, angle, left, right, height];
  if (!values.every(value => Number.isFinite(value) && Math.abs(value) <= MAX_POINT_ABS)) {
    return false;
  }
  if (left < 0 || right < 0 || left + right <= 0 || height <= 0) return false;
  return Math.hypot(end.x - start.x, end.y - start.y) >= 1e-6;
}

// ---------------------------------------------------------------------------
// TCH_COLUMN (base version 6, plain `ReadTchBase` v5 layout)
// ---------------------------------------------------------------------------
