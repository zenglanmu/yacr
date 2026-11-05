// `TCH_ELEVATION` / `TCH_ARROW` / `TCH_MULTILEADER` base-6 decoders.
//
// Plain (non-cipher) branches ported from MiniCAD `TchAnnotation.h` and
// `TchSymbol.h`. The real records are base 6 / AC1027 and consume the main
// stream exactly; anything else is an explicit unsupported layout, never a
// guessed success.
import { readSymbolBase6, textStream, consumeTexts, ClassText } from './base6.ts';
import type { SymbolBase6 } from './base6.ts';
import { reader } from './bit-reader.ts';
import type { BitReader, Point3 } from './bit-reader.ts';
import { add, bump } from './diagnostics.ts';
import { bulgeArc, formatNumber, text, upright } from './geometry.ts';
import type { Primitive, VendorRecord } from './types.ts';

const MAX_POLY_VERTICES = 4096;
const MAX_POINTS = 1e12;
const ASCII_HEIGHT = 0.85;
const WIDE_HEIGHT = 0.96454;

function isTruncated(error: unknown): boolean {
  return error instanceof Error && error.message === 'vendor.truncated';
}

function finite(value: number): boolean {
  return Number.isFinite(value);
}

function point(x: number, y: number, z = 0): Point3 {
  return { x, y, z };
}

function validPoint(p: Point3): boolean {
  return finite(p.x) && finite(p.y) && finite(p.z) && Math.abs(p.x) <= MAX_POINTS && Math.abs(p.y) <= MAX_POINTS;
}

/** Parse a symbol base, treating unsupported base layouts as a stable reason. */
function symbolBase(
  main: BitReader,
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

/** The builder's emitted run height: 0.85× ASCII, 0.96454× for wide runs. */
function runHeight(nominal: number, value: string): number {
  let wide = false;
  for (const char of value) {
    if (char.codePointAt(0)! >= 0x80) {
      wide = true;
      break;
    }
  }
  return (wide ? WIDE_HEIGHT : ASCII_HEIGHT) * nominal;
}

/** AddRuns strips `^U` toggles; the host has no superscript run model. */
function stripToggles(value: string): string {
  return value.split('^U').join('');
}

function hasVisibleText(value: string): boolean {
  return /\S/.test(value);
}

function runText(
  nominal: number,
  x: number,
  y: number,
  rotation: number,
  value: string,
  align: 0 | 1 | 2,
): Primitive | null {
  const clean = stripToggles(value);
  if (!hasVisibleText(clean)) return null;
  const height = runHeight(nominal, clean);
  if (!(height > 0) || !finite(x) || !finite(y) || !finite(rotation)) return null;
  return text(x, y, 0, height, rotation, clean, 7 + align);
}

/** MiniCAD `AddArrows`/`g.Arrows`: the ABI has no filled triangle, so outline. */
function triangleOutline(
  a: Point3,
  b: Point3,
  tip: Point3,
  reasons: Map<string, number>,
): Primitive {
  add(reasons, 'vendor.tch.arrow_outline_only', 1);
  return { kind: 0, points: [a, b, tip, a] };
}

/** `TchGraphics.Lines` as one kind-1 primitive of disjoint endpoint pairs. */
function linePairs(segments: readonly (readonly [Point3, Point3])[]): Primitive | null {
  const points: Point3[] = [];
  for (const [from, to] of segments) {
    if (!validPoint(from) || !validPoint(to)) continue;
    points.push(from, to);
  }
  return points.length >= 2 ? { kind: 1, points } : null;
}

/** A polyline path with per-vertex bulges: straight kind-0 runs + kind-2 arcs. */
function pathPrimitives(vertices: readonly { x: number; y: number; bulge: number }[]): Primitive[] {
  const out: Primitive[] = [];
  let run: Point3[] = [];
  const flush = (): void => {
    if (run.length >= 2) out.push({ kind: 0, points: run });
    run = [];
  };
  for (let index = 0; index + 1 < vertices.length; index += 1) {
    const from = vertices[index];
    const to = vertices[index + 1];
    const fromPoint = point(from.x, from.y);
    const toPoint = point(to.x, to.y);
    if (!validPoint(fromPoint) || !validPoint(toPoint)) continue;
    if (from.bulge !== 0) {
      flush();
      const arc = bulgeArc(fromPoint, toPoint, from.bulge);
      if (arc) out.push(arc);
      else run = [fromPoint, toPoint]; // degenerate bulge: keep the endpoint pair
    } else {
      if (run.length === 0) run.push(fromPoint);
      run.push(toPoint);
    }
  }
  flush();
  return out;
}

function readPolyline2d(main: BitReader): { closed: boolean; vertices: { x: number; y: number; bulge: number }[] } {
  const closed = main.readByte() !== 0;
  const count = main.readBitShort();
  if (count < 0 || count > MAX_POLY_VERTICES) throw new Error('vendor.tch.layout');
  const vertices: { x: number; y: number; bulge: number }[] = [];
  for (let index = 0; index < count; index += 1) {
    vertices.push({ x: main.readRawDouble(), y: main.readRawDouble(), bulge: main.readBitDouble() });
  }
  return { closed, vertices };
}

/** `strtod` whole-string check, matching MiniCAD's numeric-then-format path. */
function formatValue(value: string, decimals: number): string {
  if (!/^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?$/.test(value)) return value;
  return formatNumber(Number(value), decimals);
}

// ---------------------------------------------------------------------------
// TCH_ELEVATION
// ---------------------------------------------------------------------------

interface ElevationData {
  scale: number;
  textHeight: number;
  flags: number;
  x: number;
  y: number;
  lineLength: number;
  value: string;
}

function decodeElevationData(
  record: VendorRecord,
  reasons: Map<string, number>,
): ElevationData | null {
  const main = reader(record.main, record.mainBits, record.mainSkipBits);
  const base = symbolBase(main, reasons, 'vendor.tch.elevation_layout');
  if (!base) return null;
  const version = main.readByte();
  if (version < 1 || version > 4) {
    bump(reasons, 'vendor.tch.elevation_unsupported_version');
    return null;
  }
  const textReader = textStream(record);
  consumeTexts(textReader, base.texts);
  const classText = new ClassText(textReader);

  const flags = main.readByte();
  const x = main.readRawDouble();
  const y = main.readRawDouble();
  const value = classText.next();
  const lineLength = main.readBitDouble();
  if ((flags & 4) !== 0) main.readBitDouble();
  if ((flags & 8) !== 0) main.readBitDouble();
  if (version > 1) {
    main.readBitDouble();
    main.readByte();
    main.readRawDouble();
    main.readRawDouble();
    main.readRawDouble();
    main.readRawDouble();
    classText.next();
  }
  if (version > 3) main.readByte();

  if (main.remaining !== 0) {
    bump(reasons, 'vendor.tch.elevation_layout');
    return null;
  }
  if (!finite(x) || !finite(y) || !finite(lineLength) || !(base.scale > 0) || !(base.textHeight > 0)) {
    bump(reasons, 'vendor.tch.elevation_layout');
    return null;
  }
  return { scale: base.scale, textHeight: base.textHeight, flags, x, y, lineLength, value };
}

function buildElevation(data: ElevationData, reasons: Map<string, number>): Primitive[] {
  const primitives: Primitive[] = [];
  const scale = data.scale;
  const height = 3 * scale;
  const nominal = data.textHeight * scale;
  const up = (data.flags & 1) !== 0 ? -1 : 1;
  const side = (data.flags & 2) !== 0 ? -1 : 1;
  const y = data.y + up * height;
  const tip = point(data.x, data.y);
  const left = point(data.x - height, y);
  const right = point(data.x + height, y);
  if ((data.flags & 32) !== 0) {
    // MiniCAD pushes the triangle before the lines; geometry order is cosmetic.
    primitives.push(triangleOutline(tip, left, right, reasons));
  }
  const far = data.x + side * data.lineLength * scale;
  const lines = linePairs([
    [tip, left],
    [tip, right],
    [point(data.x - side * height, y), point(far, y)],
  ]);
  if (lines) primitives.push(lines);

  const value = formatValue(data.value, (data.flags & 16) !== 0 ? 2 : 3);
  if ((data.flags & 32) !== 0) {
    const item = runText(nominal, far + side * 1.35 * scale, data.y - 0.25 * scale, 0, value, side > 0 ? 0 : 2);
    if (item) primitives.push(item);
  } else {
    const baseline = up > 0 ? y + 0.3 * nominal : y - 1.3 * nominal;
    const item = runText(nominal, far - side * 0.3 * scale, baseline, 0, value, side > 0 ? 2 : 0);
    if (item) primitives.push(item);
  }
  return primitives;
}

export function decodeElevation(record: VendorRecord, reasons: Map<string, number>): Primitive[] | null {
  const data = decodeElevationData(record, reasons);
  return data ? buildElevation(data, reasons) : null;
}

// ---------------------------------------------------------------------------
// TCH_ARROW
// ---------------------------------------------------------------------------

interface ArrowVertex { x: number; y: number; bulge: number }

interface ArrowData {
  scale: number;
  textHeight: number;
  mode: number;
  vertices: ArrowVertex[];
  arrowLength: number;
  head: number;
  closed: boolean;
  hasText2: boolean;
  text1: string;
  text2: string;
}

function decodeArrowData(record: VendorRecord, reasons: Map<string, number>): ArrowData | null {
  const main = reader(record.main, record.mainBits, record.mainSkipBits);
  const base = symbolBase(main, reasons, 'vendor.tch.arrow_layout');
  if (!base) return null;
  const version = main.readByte();
  if (version < 4 || version > 5) {
    bump(reasons, 'vendor.tch.arrow_unsupported_version');
    return null;
  }
  const textReader = textStream(record);
  consumeTexts(textReader, base.texts);
  const classText = new ClassText(textReader);

  const mode = main.readByte();
  const text1 = classText.next();
  main.readBitDouble();
  main.readBitDouble();
  main.readBitDouble();
  const { closed, vertices } = readPolyline2d(main);
  const arrowLength = main.readBitDouble();
  const head = main.readByte();
  main.readBitDouble();
  const hasText2 = main.readBit() !== 0;
  const text2 = classText.next();
  main.readBitDouble();
  main.readBitDouble();
  main.readBitDouble();

  if (main.remaining !== 0) {
    bump(reasons, 'vendor.tch.arrow_layout');
    return null;
  }
  if (mode < 1 || mode > 3) {
    bump(reasons, 'vendor.tch.arrow_layout');
    return null;
  }
  if (head === 2 || head === 3) {
    bump(reasons, 'vendor.tch.arrow_layout');
    return null;
  }
  if (!finite(base.scale) || !finite(base.textHeight) || !finite(arrowLength)) {
    bump(reasons, 'vendor.tch.arrow_layout');
    return null;
  }
  if (!(base.scale > 0) || !(base.textHeight > 0) || arrowLength < 0) {
    bump(reasons, 'vendor.tch.arrow_layout');
    return null;
  }
  for (const vertex of vertices) {
    if (!finite(vertex.x) || !finite(vertex.y) || !finite(vertex.bulge) || Math.abs(vertex.bulge) > 1e6) {
      bump(reasons, 'vendor.tch.arrow_layout');
      return null;
    }
  }
  return {
    scale: base.scale,
    textHeight: base.textHeight,
    mode,
    vertices,
    arrowLength,
    head,
    closed,
    hasText2,
    text1,
    text2,
  };
}

function buildArrow(data: ArrowData, reasons: Map<string, number>): Primitive[] {
  const primitives: Primitive[] = [];
  const vertices = data.vertices;
  const count = vertices.length;
  // MiniCAD clears the vertices for closed/undersized polylines and draws nothing.
  if (count < 2 || data.closed) return primitives;
  const tip = point(vertices[count - 1].x, vertices[count - 1].y);
  const before = point(vertices[count - 2].x, vertices[count - 2].y);
  const last = Math.hypot(tip.x - before.x, tip.y - before.y);
  const head = data.head >= 4 ? 0 : Math.min(data.arrowLength * data.scale, last);
  const base = last > 0 && validPoint(tip) && validPoint(before)
    ? point(tip.x + (before.x - tip.x) / last * head, tip.y + (before.y - tip.y) / last * head)
    : tip;

  const path = vertices.slice(0, count - 1).map((vertex, index) => ({
    x: vertex.x,
    y: vertex.y,
    // MiniCAD drops the last original vertex's bulge and appends `base` flat.
    bulge: index + 2 < count ? vertex.bulge : 0,
  }));
  path.push({ x: base.x, y: base.y, bulge: 0 });
  if (head < last) primitives.push(...pathPrimitives(path));
  if (head > 0) {
    const px = -(tip.y - base.y) / 7.5;
    const py = (tip.x - base.x) / 7.5;
    primitives.push(triangleOutline(
      point(base.x + px, base.y + py),
      point(base.x - px, base.y - py),
      tip,
      reasons,
    ));
  }

  const nominal = data.textHeight * data.scale;
  if (data.mode === 1 && hasVisibleText(data.text1)) {
    const right = vertices[0].x >= vertices[1].x;
    const gap = 0.3 * nominal;
    const x = vertices[0].x + (right ? gap : -gap);
    const clean = stripToggles(data.text1);
    const height = WIDE_HEIGHT * nominal;
    if (height > 0 && finite(x) && finite(vertices[0].y)) {
      primitives.push(text(x, vertices[0].y, 0, height, 0, clean, right ? 4 : 6));
    }
  } else if (data.mode === 2) {
    const midX = (vertices[0].x + vertices[1].x) / 2;
    const midY = (vertices[0].y + vertices[1].y) / 2;
    const rotation = upright(Math.atan2(vertices[1].y - vertices[0].y, vertices[1].x - vertices[0].x)).rotation;
    const c = Math.cos(rotation);
    const s = Math.sin(rotation);
    const gap = 0.3 * nominal;
    const first = runText(nominal, midX - s * gap, midY + c * gap, rotation, data.text1, 1);
    if (first) primitives.push(first);
    const clean2 = stripToggles(data.text2);
    if (hasVisibleText(data.text2) && data.hasText2) {
      const drop = gap + runHeight(nominal, clean2);
      const second = runText(nominal, midX + s * drop, midY - c * drop, rotation, data.text2, 1);
      if (second) primitives.push(second);
    }
  } else if (data.mode === 3 && hasVisibleText(data.text1)) {
    const { rotation, flipped } = upright(Math.atan2(vertices[1].y - vertices[0].y, vertices[1].x - vertices[0].x));
    const c = Math.cos(rotation);
    const s = Math.sin(rotation);
    const gap = 0.3 * nominal;
    const item = runText(nominal, vertices[0].x - s * gap, vertices[0].y + c * gap, rotation, data.text1, flipped ? 2 : 0);
    if (item) primitives.push(item);
  }
  return primitives;
}

export function decodeArrow(record: VendorRecord, reasons: Map<string, number>): Primitive[] | null {
  const data = decodeArrowData(record, reasons);
  return data ? buildArrow(data, reasons) : null;
}

// ---------------------------------------------------------------------------
// TCH_MULTILEADER
// ---------------------------------------------------------------------------

interface MultiLeaderData {
  scale: number;
  textHeight: number;
  arrow: number;
  elbowX: number;
  elbowY: number;
  lineLength: number;
  side: number;
  arrowSize: number;
  targets: Point3[];
  upper: string;
  lower: string;
  gap: number;
  align: number;
}

function decodeMultiLeaderData(
  record: VendorRecord,
  reasons: Map<string, number>,
): MultiLeaderData | null {
  const main = reader(record.main, record.mainBits, record.mainSkipBits);
  const base = symbolBase(main, reasons, 'vendor.tch.multileader_layout');
  if (!base) return null;
  const version = main.readByte();
  if (version < 1 || version > 6) {
    bump(reasons, 'vendor.tch.multileader_unsupported_version');
    return null;
  }
  const textReader = textStream(record);
  consumeTexts(textReader, base.texts);
  const classText = new ClassText(textReader);

  const arrow = main.readByte();
  const elbowX = main.readRawDouble();
  const elbowY = main.readRawDouble();
  const lineLength = main.readBitDouble();
  const side = main.readByte();
  const upper = classText.next();
  const lower = classText.next();
  const count = main.readByte();
  const targets: Point3[] = [];
  for (let index = 0; index < count; index += 1) {
    targets.push(point(main.readRawDouble(), main.readRawDouble()));
  }
  let arrowSize = 3;
  if (version > 1) arrowSize = main.readBitDouble();
  let gap = -1000;
  let align = 3;
  if (version > 2) {
    gap = main.readBitDouble();
    align = main.readByte();
    main.readBitDouble();
  }
  if (version > 3) {
    main.readByte();
    main.readRawDouble();
    main.readRawDouble();
  }
  if (version > 4) {
    for (let index = 0; index < 6; index += 1) main.readBitDouble();
  }

  if (main.remaining !== 0) {
    bump(reasons, 'vendor.tch.multileader_layout');
    return null;
  }
  if (!finite(elbowX) || !finite(elbowY) || !finite(lineLength) || !finite(arrowSize)) {
    bump(reasons, 'vendor.tch.multileader_layout');
    return null;
  }
  if (!(base.scale > 0) || !(base.textHeight > 0)) {
    bump(reasons, 'vendor.tch.multileader_layout');
    return null;
  }
  for (const target of targets) {
    if (!validPoint(target)) {
      bump(reasons, 'vendor.tch.multileader_layout');
      return null;
    }
  }
  return {
    scale: base.scale,
    textHeight: base.textHeight,
    arrow,
    elbowX,
    elbowY,
    lineLength,
    side,
    arrowSize,
    targets,
    upper,
    lower,
    gap,
    align,
  };
}

function leaderEnd(
  primitives: Primitive[],
  kind: number,
  tip: Point3,
  toward: Point3,
  scale: number,
  size: number,
  reasons: Map<string, number>,
): void {
  if (kind === 1) {
    // MiniCAD draws a tiny thick "dot" path: two bulged segments around `tip`.
    const radius = 0.2 * scale;
    const dx = toward.x - tip.x;
    const dy = toward.y - tip.y;
    const distance = Math.hypot(dx, dy);
    const ux = distance > 0 ? dx / distance : 1;
    const uy = distance > 0 ? dy / distance : 0;
    const from = point(tip.x - ux * radius, tip.y - uy * radius);
    const to = point(tip.x + ux * radius, tip.y + uy * radius);
    const first = bulgeArc(from, to, 1);
    const second = bulgeArc(to, from, 1);
    if (first) primitives.push(first);
    if (second) primitives.push(second);
  } else if (kind === 2) {
    const dx = toward.x - tip.x;
    const dy = toward.y - tip.y;
    const distance = Math.hypot(dx, dy);
    if (!(distance > 0)) return;
    const ux = dx / distance;
    const uy = dy / distance;
    const length = size * scale;
    const width = 0.4 * scale;
    const base = point(tip.x + ux * length, tip.y + uy * length);
    primitives.push(triangleOutline(
      point(base.x - uy * width, base.y + ux * width),
      point(base.x + uy * width, base.y - ux * width),
      tip,
      reasons,
    ));
  }
}

function buildMultiLeader(data: MultiLeaderData, reasons: Map<string, number>): Primitive[] {
  const primitives: Primitive[] = [];
  const scale = data.scale;
  const nominal = data.textHeight * scale;
  const side = data.side === 0 ? 1 : -1;
  const elbow = point(data.elbowX, data.elbowY);
  const segments: (readonly [Point3, Point3])[] = [];
  for (const target of data.targets) {
    segments.push([target, elbow]);
  }
  const end = data.elbowX + side * data.lineLength * scale;
  segments.push([elbow, point(end, data.elbowY)]);
  const lines = linePairs(segments);
  if (lines) primitives.push(lines);
  for (const target of data.targets) {
    leaderEnd(primitives, data.arrow, target, elbow, scale, data.arrowSize, reasons);
  }

  const gap = data.gap >= 0 && data.gap < 10 ? data.gap : 0.3;
  const dir = end >= data.elbowX ? 1 : -1;
  const inset = 0.3 * nominal;
  let anchor = end - dir * inset;
  let column = dir > 0 ? 2 : 0;
  if (data.align === 1) {
    anchor = data.elbowX + dir * inset;
    column = dir > 0 ? 0 : 2;
  } else if (data.align === 2) {
    anchor = (data.elbowX + end) / 2;
    column = 1;
  }
  const upperY = data.elbowY + gap * nominal;
  const lowerY = data.elbowY - (1 + gap) * nominal;
  const upper = runText(nominal, anchor, upperY, 0, data.upper, column as 0 | 1 | 2);
  if (upper) primitives.push(upper);
  const lower = runText(nominal, anchor, lowerY, 0, data.lower, column as 0 | 1 | 2);
  if (lower) primitives.push(lower);
  return primitives;
}

export function decodeMultiLeader(record: VendorRecord, reasons: Map<string, number>): Primitive[] | null {
  const data = decodeMultiLeaderData(record, reasons);
  return data ? buildMultiLeader(data, reasons) : null;
}
