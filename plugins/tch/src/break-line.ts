// `TCH_RUPTURE` / `TCH_MODI` base-6 decoders and display builders.
//
// Plain (non-cipher) class bodies ported from MiniCAD `TchElec.h`
// (`DecodeRupture` / `BuildRupture`) and `TchAnnotation.h`
// (`DecodeModi` / `BuildModi`). Real records start with base 6 / AC1027 and
// must consume the main stream exactly; every other class-body version or
// layout is an explicit unsupported layout, never a guessed success.
import { consumeTexts, readBase6, readSymbolBase6, textStream, ClassText } from './base6.ts';
import type { TchBase6, SymbolBase6 } from './base6.ts';
import { reader } from './bit-reader.ts';
import type { Point3 } from './bit-reader.ts';
import { bump } from './diagnostics.ts';
import { bulgeArc } from './geometry.ts';
import type { Primitive, VendorRecord } from './types.ts';

const MAX_POINT_ABS = 1e12;
const MAX_MODI_VERTICES = 100000;
const MAX_MODI_BULGE = 10;

function isTruncated(error: unknown): boolean {
  return error instanceof Error && error.message === 'vendor.truncated';
}

function finite(value: number): boolean {
  return Number.isFinite(value) && Math.abs(value) <= MAX_POINT_ABS;
}

function point(x: number, y: number): Point3 {
  return { x, y, z: 0 };
}

function validPoint(p: Point3): boolean {
  return finite(p.x) && finite(p.y) && finite(p.z);
}

/** `readBase6`, treating an unsupported base layout as one stable reason. */
function parseBase(main: ReturnType<typeof reader>, reasons: Map<string, number>, reason: string): TchBase6 | null {
  try {
    return readBase6(main);
  } catch (error) {
    if (isTruncated(error)) throw error;
    bump(reasons, reason);
    return null;
  }
}

/** `readSymbolBase6`, treating an unsupported symbol base layout as one reason. */
function parseSymbolBase(
  main: ReturnType<typeof reader>,
  reasons: Map<string, number>,
  reason: string,
): SymbolBase6 | null {
  try {
    return readSymbolBase6(main);
  } catch (error) {
    if (isTruncated(error)) throw error;
    bump(reasons, reason);
    return null;
  }
}

// ---------------------------------------------------------------------------
// TCH_RUPTURE
// ---------------------------------------------------------------------------

interface RuptureData {
  scale: number;
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  count: number;
}

function decodeRuptureData(record: VendorRecord, reasons: Map<string, number>): RuptureData | null {
  const main = reader(record.main, record.mainBits, record.mainSkipBits);
  const base = parseBase(main, reasons, 'vendor.tch.rupture_layout');
  if (!base) return null;

  const version = main.readByte();
  if (version < 1 || version > 2) {
    bump(reasons, 'vendor.tch.rupture_unsupported_version');
    return null;
  }
  // Version 2 carries an extra encoding key. The plain branch consumes it but
  // does not decrypt; keys >= 16 mark the encrypted variant.
  if (version === 2) {
    const key = main.readByte();
    if (key >= 16) {
      bump(reasons, 'vendor.tch.rupture_unsupported_version');
      return null;
    }
  }

  const x1 = main.readRawDouble();
  const y1 = main.readRawDouble();
  const x2 = main.readRawDouble();
  const y2 = main.readRawDouble();
  const count = main.readByte();

  if (main.remaining !== 0) {
    bump(reasons, 'vendor.tch.rupture_layout');
    return null;
  }
  const total = Math.hypot(x2 - x1, y2 - y1);
  if (
    !finite(base.scale) ||
    !(base.scale > 0) ||
    !finite(x1) || !finite(y1) || !finite(x2) || !finite(y2) ||
    !finite(total) ||
    total < 1e-9 ||
    count === 0
  ) {
    bump(reasons, 'vendor.tch.rupture_invalid');
    return null;
  }
  return { scale: base.scale, x1, y1, x2, y2, count };
}

/** Port of `BuildRupture`: `count` identical six-point break polylines. */
function buildRupture(data: RuptureData): Primitive[] {
  const dx = data.x2 - data.x1;
  const dy = data.y2 - data.y1;
  const total = Math.hypot(dx, dy);
  const ux = dx / total;
  const uy = dy / total;
  const nx = -uy;
  const ny = ux;
  const scale = data.scale;
  const h = 2.1 * scale;
  const len = total / Math.max(data.count, 1);
  const primitives: Primitive[] = [];
  for (let index = 0; index < data.count; index += 1) {
    const x0 = data.x1 + ux * len * index;
    const y0 = data.y1 + uy * len * index;
    const at = (along: number, side: number): Point3 => point(
      x0 + ux * along + nx * side,
      y0 + uy * along + ny * side,
    );
    const a = len / 2 - 1.125 * scale;
    if (a < 0) {
      primitives.push({ kind: 0, points: [at(0, 0), at(len, 0)] });
      continue;
    }
    const peak = a + h / Math.tan(70 * Math.PI / 180);
    const valley = peak + 2 * h / Math.tan(80 * Math.PI / 180);
    primitives.push({
      kind: 0,
      points: [
        at(0, 0),
        at(a, 0),
        at(peak, h),
        at(valley, -h),
        at(len / 2 + 1.125 * scale, 0),
        at(len, 0),
      ],
    });
  }
  return primitives;
}

export function decodeRupture(record: VendorRecord, reasons: Map<string, number>): Primitive[] | null {
  const data = decodeRuptureData(record, reasons);
  return data ? buildRupture(data) : null;
}

// ---------------------------------------------------------------------------
// TCH_MODI
// ---------------------------------------------------------------------------

interface ModiData {
  closed: boolean;
  points: Point3[];
  bulge: number;
}

function decodeModiData(record: VendorRecord, reasons: Map<string, number>): ModiData | null {
  const main = reader(record.main, record.mainBits, record.mainSkipBits);
  const base = parseSymbolBase(main, reasons, 'vendor.tch.modi_layout');
  if (!base) return null;

  const text = textStream(record);
  consumeTexts(text, base.texts);
  const classText = new ClassText(text);

  const version = main.readByte();
  if (version !== 1) {
    bump(reasons, 'vendor.tch.modi_unsupported_version');
    return null;
  }
  const closed = main.readByte() !== 0;
  const count = main.readBitShort();
  if (count < 2 || count > MAX_MODI_VERTICES) {
    bump(reasons, 'vendor.tch.modi_layout');
    return null;
  }
  const points: Point3[] = [];
  for (let index = 0; index < count; index += 1) {
    const x = main.readRawDouble();
    const y = main.readRawDouble();
    main.readBitDouble();
    if (!finite(x) || !finite(y)) {
      bump(reasons, 'vendor.tch.modi_invalid');
      return null;
    }
    points.push(point(x, y));
  }
  main.readBitDouble();
  const bulge = main.readBitDouble();
  main.readByte();
  classText.next(); // 编号 is not drawn
  main.readBitDouble();
  main.readBitDouble();
  main.readBitDouble();

  if (main.remaining !== 0) {
    bump(reasons, 'vendor.tch.modi_layout');
    return null;
  }
  if (!finite(bulge) || Math.abs(bulge) > MAX_MODI_BULGE) {
    bump(reasons, 'vendor.tch.modi_invalid');
    return null;
  }
  return { closed, points, bulge };
}

/** A polyline path with per-vertex bulges: straight kind-0 runs + kind-2 arcs. */
function pathPrimitives(vertices: readonly { point: Point3; bulge: number }[]): Primitive[] {
  const primitives: Primitive[] = [];
  let run: Point3[] = [];
  const flush = (): void => {
    if (run.length >= 2) primitives.push({ kind: 0, points: run });
    run = [];
  };
  for (let index = 0; index + 1 < vertices.length; index += 1) {
    const from = vertices[index];
    const to = vertices[index + 1];
    if (!validPoint(from.point) || !validPoint(to.point)) continue;
    if (from.bulge !== 0) {
      flush();
      const arc = bulgeArc(from.point, to.point, from.bulge);
      if (arc) primitives.push(arc);
      else run = [from.point, to.point];
    } else {
      if (run.length === 0) run.push(from.point);
      run.push(to.point);
    }
  }
  flush();
  return primitives;
}

/** Port of `BuildModi`: one bulged path, closed by repeating the first vertex. */
function buildModi(data: ModiData): Primitive[] {
  const vertices = data.points.map(item => ({ point: item, bulge: data.bulge }));
  if (data.closed && data.points.length > 0) {
    vertices.push({ point: data.points[0], bulge: 0 });
  }
  if (vertices.length > 0) vertices[vertices.length - 1].bulge = 0;
  return pathPrimitives(vertices);
}

export function decodeModi(record: VendorRecord, reasons: Map<string, number>): Primitive[] | null {
  const data = decodeModiData(record, reasons);
  return data ? buildModi(data) : null;
}
