// `TCH_SYMB_SECTION` / `TCH_INDEXPOINTER` / `TCH_CORNER_WINDOW` base-6 decoders.
//
// Plain (non-cipher) branches ported from MiniCAD `TchSymbol.h` and
// `TchShape.h`. Every real fixture record starts with base 6, so
// `IsCipherObject` is false and only the commons layout is modelled. The
// classic section builder is used because the ABI carries no XData style
// switch; when a version-2 record could select the new style this is recorded
// with `vendor.tch.section_style_default` instead of guessing. Anything else
// unsupported is an explicit reason, never a guessed success.
import { consumeTexts, readBase6, readSymbolBase6, textStream, ClassText } from './base6.ts';
import type { SymbolBase6, TchBase6 } from './base6.ts';
import { reader } from './bit-reader.ts';
import type { BitReader, Point3 } from './bit-reader.ts';
import { add, bump } from './diagnostics.ts';
import { text } from './geometry.ts';
import type { Primitive, VendorRecord } from './types.ts';

const MAX_POINT_ABS = 1e12;
const MAX_SECTION_POINTS = 100000;
const MAX_VERTICES = 4096;
const MAX_SEGMENTS = 10000;
const ASCII_HEIGHT = 0.85;
const WIDE_HEIGHT = 0.96454;

const ATTACH_BOTTOM_CENTER = 8;
const ATTACH_MIDDLE_LEFT = 4;
const ATTACH_MIDDLE_RIGHT = 6;
const ATTACH_MIDDLE_CENTER = 5;

function isTruncated(error: unknown): boolean {
  return error instanceof Error && error.message === 'vendor.truncated';
}

function finite(value: number): boolean {
  return Number.isFinite(value);
}

function point(x: number, y: number): Point3 {
  return { x, y, z: 0 };
}

function validPoint(p: Point3): boolean {
  return (
    finite(p.x) &&
    finite(p.y) &&
    finite(p.z) &&
    Math.abs(p.x) <= MAX_POINT_ABS &&
    Math.abs(p.y) <= MAX_POINT_ABS
  );
}

function stripToggles(value: string): string {
  return value.split('^U').join('');
}

function hasWideChar(value: string): boolean {
  for (const char of value) {
    if (char.codePointAt(0)! >= 0x80) return true;
  }
  return false;
}

/** UTF-8 byte length, matching MiniCAD's `std::string::size()` checks. */
function byteLength(value: string): number {
  let length = 0;
  for (const char of value) {
    const code = char.codePointAt(0)!;
    length += code < 0x80 ? 1 : code < 0x800 ? 2 : code < 0x10000 ? 3 : 4;
  }
  return length;
}

/** `AddRuns`'s per-run height: 0.85× nominal for ASCII, 0.96454× for wide. */
function runHeight(nominal: number, value: string): number {
  return (hasWideChar(value) ? WIDE_HEIGHT : ASCII_HEIGHT) * nominal;
}

/** A finite text primitive, else `null` (empty text or invalid placement). */
function placedText(
  x: number,
  y: number,
  height: number,
  rotation: number,
  value: string,
  attachment: number,
  wrapWidth = 0,
): Primitive | null {
  const clean = stripToggles(value);
  if (clean.length === 0) return null;
  if (!(height > 0) || !finite(x) || !finite(y) || !finite(height) || !finite(rotation)) return null;
  return text(x, y, 0, height, rotation, clean, attachment, wrapWidth);
}

/** `AddRuns` text: `nominal` is the reference size, runs shrink to 0.85/0.96454. */
function runText(
  x: number,
  y: number,
  nominal: number,
  rotation: number,
  value: string,
  attachment: number,
): Primitive | null {
  const clean = stripToggles(value);
  if (clean.length === 0) return null;
  return placedText(x, y, runHeight(nominal, clean), rotation, clean, attachment);
}

/** `TchGraphics::Lines` as one kind-1 primitive of disjoint endpoint pairs. */
function linePairs(segments: readonly (readonly [Point3, Point3])[]): Primitive | null {
  const points: Point3[] = [];
  for (const [from, to] of segments) {
    if (!validPoint(from) || !validPoint(to)) continue;
    points.push(from, to);
  }
  return points.length >= 2 ? { kind: 1, points } : null;
}

/** One `TchGraphics::Path`: a straight kind-0 polyline (width is not modelled). */
function pathPrimitive(points: readonly Point3[]): Primitive | null {
  if (points.length < 2) return null;
  for (const p of points) {
    if (!validPoint(p)) return null;
  }
  return { kind: 0, points: points.map(p => ({ x: p.x, y: p.y, z: p.z })) };
}

function circlePrimitive(cx: number, cy: number, radius: number): Primitive | null {
  if (!finite(cx) || !finite(cy) || !finite(radius) || !(radius > 0)) return null;
  return { kind: 3, cx, cy, cz: 0, radius };
}

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

function base6(
  main: BitReader,
  reasons: Map<string, number>,
  reason: string,
): TchBase6 | null {
  try {
    return readBase6(main);
  } catch (error) {
    if (isTruncated(error)) throw error;
    bump(reasons, reason);
    return null;
  }
}

// ---------------------------------------------------------------------------
// TCH_SYMB_SECTION
// ---------------------------------------------------------------------------

interface SectionData {
  scale: number;
  textHeight: number;
  kind: number;
  viewAngle: number;
  endViewAngle: number;
  points: Point3[];
  label: string;
}

function decodeSectionData(
  record: VendorRecord,
  reasons: Map<string, number>,
): SectionData | null {
  const main = reader(record.main, record.mainBits, record.mainSkipBits);
  const base = symbolBase(main, reasons, 'vendor.tch.section_layout');
  if (!base) return null;
  const textReader = textStream(record);
  consumeTexts(textReader, base.texts);
  const classText = new ClassText(textReader);

  const version = main.readByte();
  if (version < 1 || version > 2) {
    bump(reasons, 'vendor.tch.section_unsupported_version');
    return null;
  }

  try {
    const kind = main.readByte();
    const viewAngle = main.readBitDouble();
    let endViewAngle = viewAngle;
    const count = main.readByte();
    if (count < 2 || count > MAX_SECTION_POINTS) {
      bump(reasons, 'vendor.tch.section_layout');
      return null;
    }
    const points: Point3[] = [];
    for (let index = 0; index < count; index += 1) {
      points.push(point(main.readRawDouble(), main.readRawDouble()));
    }
    const label = classText.next();
    for (let index = 0; index < 4; index += 1) main.readRawDouble();
    if (version > 1) {
      classText.next(); // index number (label2), classic builder does not draw it
      for (let index = 0; index < 4; index += 1) main.readRawDouble();
      main.readBitDouble();
      const indexFlags = main.readBitShort();
      main.readBitDouble();
      const endView = main.readBitDouble();
      if ((indexFlags & 8) !== 0) endViewAngle = endView;
    }
    if (main.remaining !== 0) {
      bump(reasons, 'vendor.tch.section_layout');
      return null;
    }
    if (kind !== 0 && kind !== 1) {
      bump(reasons, 'vendor.tch.section_layout');
      return null;
    }
    if (!finite(viewAngle) || !finite(endViewAngle) || !(base.scale > 0) || !(base.textHeight > 0)) {
      bump(reasons, 'vendor.tch.section_layout');
      return null;
    }
    if (!points.every(validPoint)) {
      bump(reasons, 'vendor.tch.section_layout');
      return null;
    }
    // The style switch lives in base XData, which the ABI does not expose.
    if (version > 1) add(reasons, 'vendor.tch.section_style_default', 1);
    return {
      scale: base.scale,
      textHeight: base.textHeight,
      kind,
      viewAngle,
      endViewAngle,
      points,
      label,
    };
  } catch (error) {
    if (isTruncated(error)) throw error;
    bump(reasons, 'vendor.tch.section_layout');
    return null;
  }
}

/** MiniCAD `BuildSection` (classic layout; no XData-style new builder). */
function buildSection(data: SectionData): Primitive[] {
  const primitives: Primitive[] = [];
  const {
    scale, textHeight, kind, viewAngle, endViewAngle, points, label,
  } = data;
  const stroke = 10 * scale;
  const leg = kind === 1 ? 6 * scale : 0;
  const nominal = textHeight * scale;
  const view = { x: Math.cos(viewAngle), y: Math.sin(viewAngle) };
  const endView = { x: Math.cos(endViewAngle), y: Math.sin(endViewAngle) };

  const toward = (from: Point3, to: Point3): Point3 => {
    const distance = Math.hypot(to.x - from.x, to.y - from.y);
    const t = distance > 0 ? Math.min(stroke, distance) / distance : 0;
    return point(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
  };

  const count = points.length;
  for (let index = 0; index < count; index += 1) {
    const p = points[index];
    const path: Point3[] = [];
    if (index === 0) {
      if (leg > 0) path.push(point(p.x + view.x * leg, p.y + view.y * leg));
      path.push(p);
      path.push(toward(p, points[1]));
    } else if (index + 1 === count) {
      path.push(toward(p, points[index - 1]));
      path.push(p);
      if (leg > 0) path.push(point(p.x + endView.x * leg, p.y + endView.y * leg));
    } else {
      path.push(toward(p, points[index - 1]));
      path.push(p);
      path.push(toward(p, points[index + 1]));
    }
    const primitive = pathPrimitive(path);
    if (primitive) primitives.push(primitive);
  }

  if (label.length > 0) {
    for (const [p, direction] of [
      [points[0], view],
      [points[count - 1], endView],
    ] as const) {
      const item = placedText(
        p.x + direction.x * (leg + nominal),
        p.y + direction.y * (leg + nominal),
        ASCII_HEIGHT * nominal,
        0,
        label,
        ATTACH_MIDDLE_CENTER,
      );
      if (item) primitives.push(item);
    }
  }
  return primitives;
}

export function decodeSection(
  record: VendorRecord,
  reasons: Map<string, number>,
): Primitive[] | null {
  const data = decodeSectionData(record, reasons);
  return data ? buildSection(data) : null;
}

// ---------------------------------------------------------------------------
// TCH_INDEXPOINTER
// ---------------------------------------------------------------------------

interface IndexPointerData {
  scale: number;
  textHeight: number;
  kind: number;
  pointX: number;
  pointY: number;
  cornerX: number;
  cornerY: number;
  radius: number;
  run: number;
  mark: number;
  number: string;
  sheet: string;
  above: string;
  below: string;
}

function decodeIndexPointerData(
  record: VendorRecord,
  reasons: Map<string, number>,
): IndexPointerData | null {
  const main = reader(record.main, record.mainBits, record.mainSkipBits);
  const base = symbolBase(main, reasons, 'vendor.tch.indexpointer_layout');
  if (!base) return null;

  const version = main.readByte();
  if (version < 3 || version > 4) {
    bump(reasons, 'vendor.tch.indexpointer_unsupported_version');
    return null;
  }

  try {
    const kind = main.readByte();
    if (kind !== 0 && kind !== 1) {
      bump(reasons, 'vendor.tch.indexpointer_layout');
      return null;
    }
    const pointX = main.readRawDouble();
    const pointY = main.readRawDouble();
    const cornerX = main.readRawDouble();
    const cornerY = main.readRawDouble();
    const radius = main.readBitDouble();
    const run = main.readBitDouble();
    const mark = main.readBitDouble();
    const list1 = main.readByte();
    const list2 = main.readByte();
    main.readBitDouble();
    main.readBitDouble();
    main.readByte();
    main.readByte(); // polyline closed flag
    const vertices = main.readBitShort();
    if (vertices < 0 || vertices > MAX_VERTICES) {
      bump(reasons, 'vendor.tch.indexpointer_layout');
      return null;
    }
    for (let index = 0; index < vertices; index += 1) {
      main.readRawDouble();
      main.readRawDouble();
      main.readBitDouble();
    }
    main.readBitDouble();
    if (main.remaining !== 0) {
      bump(reasons, 'vendor.tch.indexpointer_layout');
      return null;
    }

    const textReader = textStream(record);
    consumeTexts(textReader, base.texts);
    const classText = new ClassText(textReader);
    const number = classText.next();
    const sheet = classText.next();
    const above = classText.next();
    const below = classText.next();
    for (let index = 0; index < list1 + list2 + 1; index += 1) classText.next();

    const values = [pointX, pointY, cornerX, cornerY, radius, run, mark];
    if (!values.every(finite) || !(base.scale > 0) || !(base.textHeight > 0) || !(radius > 0)) {
      bump(reasons, 'vendor.tch.indexpointer_layout');
      return null;
    }
    return {
      scale: base.scale,
      textHeight: base.textHeight,
      kind,
      pointX,
      pointY,
      cornerX,
      cornerY,
      radius,
      run,
      mark,
      number,
      sheet,
      above,
      below,
    };
  } catch (error) {
    if (isTruncated(error)) throw error;
    bump(reasons, 'vendor.tch.indexpointer_layout');
    return null;
  }
}

/** MiniCAD `BuildIndexPointer`. */
function buildIndexPointer(data: IndexPointerData): Primitive[] {
  const primitives: Primitive[] = [];
  const {
    scale, textHeight, kind, pointX, pointY, cornerX, cornerY, radius, run, mark,
    number, sheet, above, below,
  } = data;
  const scaledRadius = radius * scale;
  const nominal = textHeight * scale;
  const dir = run >= 0 ? 1 : -1;
  const leader = point(pointX, pointY);
  const corner = point(cornerX, cornerY);
  const edge = point(corner.x + run * scale, corner.y);
  const center = point(edge.x + dir * scaledRadius, edge.y);
  const length = Math.hypot(corner.x - leader.x, corner.y - leader.y);
  const u = length > 0
    ? point((corner.x - leader.x) / length, (corner.y - leader.y) / length)
    : point(dir, 0);

  const segments: (readonly [Point3, Point3])[] = [];
  if (kind === 0) {
    const r = Math.abs(mark) / 2;
    const dot = circlePrimitive(leader.x, leader.y, r);
    if (dot) primitives.push(dot);
    if (length > r) segments.push([point(leader.x + u.x * r, leader.y + u.y * r), corner]);
  } else {
    if (length > 0) segments.push([leader, corner]);
    const side = mark < 0 ? 1.5 * scale : -1.5 * scale;
    const start = point(leader.x - u.y * side, leader.y + u.x * side);
    const end = point(start.x + u.x * Math.abs(mark) * scale, start.y + u.y * Math.abs(mark) * scale);
    const tick = pathPrimitive([start, end]);
    if (tick) primitives.push(tick);
  }
  if (run !== 0) segments.push([corner, edge]);
  const leaderLines = linePairs(segments);
  if (leaderLines) primitives.push(leaderLines);

  const ring = circlePrimitive(center.x, center.y, scaledRadius);
  if (ring) primitives.push(ring);
  const diameter = linePairs([[point(center.x + scaledRadius, center.y), point(center.x - scaledRadius, center.y)]]);
  if (diameter) primitives.push(diameter);

  const inner = (value: string, sign: number): void => {
    const clean = stripToggles(value);
    if (clean.length === 0) return;
    const height = (byteLength(clean) > 1 ? 0.46 : 0.535) * scaledRadius;
    const item = placedText(
      center.x,
      center.y + sign * 0.45 * scaledRadius,
      height,
      0,
      clean,
      ATTACH_MIDDLE_CENTER,
    );
    if (item) primitives.push(item);
  };
  inner(number, 1);
  inner(sheet, -1);

  const height = ASCII_HEIGHT * nominal;
  const gap = 0.3 * nominal;
  const noteX = edge.x - dir * gap;
  const attachment = dir > 0 ? ATTACH_MIDDLE_RIGHT : ATTACH_MIDDLE_LEFT;
  if (above.length > 0) {
    const item = placedText(noteX, edge.y + gap + height / 2, height, 0, above, attachment, 1);
    if (item) primitives.push(item);
  }
  if (below.length > 0) {
    const item = placedText(noteX, edge.y - gap - height / 2, height, 0, below, attachment, 1);
    if (item) primitives.push(item);
  }
  return primitives;
}

export function decodeIndexPointer(
  record: VendorRecord,
  reasons: Map<string, number>,
): Primitive[] | null {
  const data = decodeIndexPointerData(record, reasons);
  return data ? buildIndexPointer(data) : null;
}

// ---------------------------------------------------------------------------
// TCH_CORNER_WINDOW
// ---------------------------------------------------------------------------

interface CornerWindowData {
  scale: number;
  x: number;
  y: number;
  length1: number;
  length2: number;
  angle1: number;
  angle2: number;
  thickness: number;
  thickness2: number;
  labelX: number;
  labelY: number;
  labelHeight: number;
  label: string;
}

function decodeCornerWindowData(
  record: VendorRecord,
  reasons: Map<string, number>,
): CornerWindowData | null {
  const main = reader(record.main, record.mainBits, record.mainSkipBits);
  const base = base6(main, reasons, 'vendor.tch.cornerwindow_layout');
  if (!base) return null;

  const version = main.readByte();
  if (version < 1 || version > 4) {
    bump(reasons, 'vendor.tch.cornerwindow_unsupported_version');
    return null;
  }

  try {
    const flags = main.readBitLong();
    const origin = main.read3BitDouble();
    const length1 = main.readBitDouble();
    const length2 = main.readBitDouble();
    main.readBitDouble();
    const angle1 = main.readBitDouble();
    const angle2 = main.readBitDouble();
    const thickness = main.readBitDouble();
    const thickness2 = main.readBitDouble();
    main.readBitDouble();
    main.readBitDouble();
    const labelX = main.readRawDouble();
    const labelY = main.readRawDouble();
    main.readBitDouble();
    const labelHeight = main.readBitDouble();
    if ((flags & 1) !== 0) {
      for (let index = 0; index < 4; index += 1) main.readBitDouble();
    }
    main.readBitShort();
    for (let index = 0; index < 3; index += 1) main.readBitDouble();
    if ((flags & 4) !== 0) {
      const segments = main.readBitShort();
      if (segments < 0 || segments > MAX_SEGMENTS) {
        bump(reasons, 'vendor.tch.cornerwindow_layout');
        return null;
      }
      for (let index = 0; index < segments; index += 1) {
        main.readRawDouble();
        main.readRawDouble();
        main.readRawDouble();
        main.readRawDouble();
        main.readBitDouble();
      }
    }
    if (version > 1) {
      main.readBitShort();
      main.readBit();
      for (let index = 0; index < 4; index += 1) main.readBitDouble();
    }
    if (version > 2) main.readBitDouble();
    if (version > 3 && (flags & 1) !== 0) main.readBitDouble();
    if (main.remaining !== 0) {
      bump(reasons, 'vendor.tch.cornerwindow_layout');
      return null;
    }

    const textReader = textStream(record);
    consumeTexts(textReader, base.texts);
    const classText = new ClassText(textReader);
    const label = classText.next();

    const values = [
      origin.x, origin.y, length1, length2, angle1, angle2,
      thickness, thickness2, labelX, labelY, labelHeight,
    ];
    if (!values.every(finite) || !(base.scale > 0)) {
      bump(reasons, 'vendor.tch.cornerwindow_layout');
      return null;
    }
    return {
      scale: base.scale,
      x: origin.x,
      y: origin.y,
      length1,
      length2,
      angle1,
      angle2,
      thickness,
      thickness2,
      labelX,
      labelY,
      labelHeight,
      label,
    };
  } catch (error) {
    if (isTruncated(error)) throw error;
    bump(reasons, 'vendor.tch.cornerwindow_layout');
    return null;
  }
}

/** MiniCAD `BuildCornerWindow`. */
function buildCornerWindow(data: CornerWindowData): Primitive[] {
  const primitives: Primitive[] = [];
  const {
    scale, x, y, length1, length2, angle1, angle2, thickness, thickness2,
    labelX, labelY, labelHeight, label,
  } = data;
  const d1x = Math.cos(angle1);
  const d1y = Math.sin(angle1);
  const d2x = Math.cos(angle2);
  const d2y = Math.sin(angle2);
  const cross = d1x * d2y - d1y * d2x;

  const at = (fraction: number, which: 0 | 1 | 2): Point3 => {
    const a = thickness * fraction;
    const b = thickness2 * fraction;
    const ax = x + d1y * a;
    const ay = y - d1x * a;
    const bx = x + d2y * b;
    const by = y - d2x * b;
    if (which === 1) return point(ax + d1x * length1, ay + d1y * length1);
    if (which === 2) return point(bx + d2x * length2, by + d2y * length2);
    if (Math.abs(cross) < 1e-9) return point(ax, ay);
    const s = ((bx - ax) * d2y - (by - ay) * d2x) / cross;
    return point(ax + d1x * s, ay + d1y * s);
  };

  for (let k = 0; k <= 3; k += 1) {
    const fraction = k / 3;
    const transverse = pathPrimitive([at(fraction, 1), at(fraction, 0), at(fraction, 2)]);
    if (transverse) primitives.push(transverse);
  }
  const sides = linePairs([[at(0, 1), at(1, 1)], [at(0, 2), at(1, 2)]]);
  if (sides) primitives.push(sides);

  if (label.length > 0) {
    const nominal = labelHeight * scale;
    const item = runText(labelX, labelY - 0.425 * nominal, nominal, 0, label, ATTACH_BOTTOM_CENTER);
    if (item) primitives.push(item);
  }
  return primitives;
}

export function decodeCornerWindow(
  record: VendorRecord,
  reasons: Map<string, number>,
): Primitive[] | null {
  const data = decodeCornerWindowData(record, reasons);
  return data ? buildCornerWindow(data) : null;
}
