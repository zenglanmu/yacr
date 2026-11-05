// `TCH_DRAWINGINDEX` / `TCH_DRAWINGNAME` / `TCH_NORTHTHUMB` base-6 decoders.
//
// Plain (non-cipher) branches ported from MiniCAD `TchAnnotation.h` and
// `TchSymbol.h`. Every real fixture record starts with base 6, so
// `IsCipherObject` is false and only the plain layout is modelled. Anything
// else is an explicit unsupported layout, never a guessed success.
import { consumeTexts, readBase6, readSymbolBase6, textStream, ClassText } from './base6.ts';
import type { TchBase6, SymbolBase6 } from './base6.ts';
import { reader } from './bit-reader.ts';
import type { BitReader, Point3 } from './bit-reader.ts';
import { add, bump } from './diagnostics.ts';
import { text } from './geometry.ts';
import type { Primitive, VendorRecord } from './types.ts';

/** MTEXT attachment for the C++ builders' centered/bottom-left defaults. */
const ATTACH_BOTTOM_CENTER = 8;
const ATTACH_MIDDLE_LEFT = 4;
const ATTACH_MIDDLE_CENTER = 5;
const ATTACH_BOTTOM_LEFT = 7;
const WIDE_HEIGHT = 0.96454;
const DEFAULT_NORTH_TEXT = '\u5317'; // `TchNorthThumb::Text` default (data, i18n: 北)

function isTruncated(error: unknown): boolean {
  return error instanceof Error && error.message === 'vendor.truncated';
}

function finite(value: number): boolean {
  return Number.isFinite(value);
}

function point(x: number, y: number): Point3 {
  return { x, y, z: 0 };
}

function stripToggles(value: string): string {
  return value.split('^U').join('');
}

function hasVisibleText(value: string): boolean {
  return /\S/.test(value);
}

function hasWideChar(value: string): boolean {
  for (const char of value) {
    if (char.codePointAt(0)! >= 0x80) return true;
  }
  return false;
}

/** Unicode scalar count, matching MiniCAD's UTF-8 code-point iteration. */
function charCount(value: string): number {
  return [...value].length;
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

/** Parse a plain base-6 header, also mapping base failures to `reason`. */
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
// TCH_DRAWINGINDEX
// ---------------------------------------------------------------------------

interface DrawingIndexData {
  scale: number;
  x: number;
  y: number;
  radius: number;
  lineWidth: number;
  number: string;
  sheet: string;
  scaleText: string;
  scaleHeight: number;
  showScale: boolean;
}

function decodeDrawingIndexData(
  record: VendorRecord,
  reasons: Map<string, number>,
): DrawingIndexData | null {
  const main = reader(record.main, record.mainBits, record.mainSkipBits);
  const base = symbolBase(main, reasons, 'vendor.tch.drawingindex_layout');
  if (!base) return null;
  const version = main.readByte();
  if (version < 1 || version > 2) {
    bump(reasons, 'vendor.tch.drawingindex_unsupported_version');
    return null;
  }
  const textReader = textStream(record);
  consumeTexts(textReader, base.texts);
  const classText = new ClassText(textReader);

  const x = main.readRawDouble();
  const y = main.readRawDouble();
  const radius = main.readBitDouble();
  const lineWidth = main.readBitDouble();
  const number = classText.next();
  const sheet = classText.next();
  let scaleText = '';
  let scaleHeight = 3.5; // `TchDrawingIndex::ScaleHeight` default for version 1
  let showScale = false;
  if (version > 1) {
    scaleText = classText.next();
    scaleHeight = main.readBitDouble();
    main.readBitDouble();
    showScale = main.readByte() !== 0;
    main.readByte();
    main.readBitDouble();
  }

  if (main.remaining !== 0) {
    bump(reasons, 'vendor.tch.drawingindex_layout');
    return null;
  }
  if (
    !finite(x) ||
    !finite(y) ||
    !finite(radius) ||
    !(radius > 0) ||
    !finite(lineWidth) ||
    !(base.scale > 0) ||
    !finite(radius * base.scale) ||
    !(radius * base.scale > 0) ||
    !finite(scaleHeight) ||
    !(scaleHeight > 0) ||
    !finite(scaleHeight * base.scale)
  ) {
    bump(reasons, 'vendor.tch.drawingindex_layout');
    return null;
  }
  return {
    scale: base.scale,
    x,
    y,
    radius,
    lineWidth,
    number,
    sheet,
    scaleText,
    scaleHeight,
    showScale,
  };
}

function buildDrawingIndex(data: DrawingIndexData): Primitive[] {
  const primitives: Primitive[] = [];
  const scale = data.scale;
  const r = data.radius * scale;
  primitives.push({ kind: 3, cx: data.x, cy: data.y, cz: 0, radius: r });

  if (data.sheet === '') {
    const height = 0.55 * 2 * r;
    const value = stripToggles(data.number);
    if (height > 0 && hasVisibleText(value)) {
      primitives.push(text(data.x, data.y - height / 2, 0, height, 0, value, ATTACH_BOTTOM_CENTER));
    }
  } else {
    primitives.push({
      kind: 1,
      points: [point(data.x + r, data.y), point(data.x - r, data.y)],
    });
    const height = 0.26 * 2 * r;
    const number = stripToggles(data.number);
    const sheet = stripToggles(data.sheet);
    if (height > 0 && hasVisibleText(number)) {
      primitives.push(text(data.x, data.y + 0.19 * r, 0, height, 0, number, ATTACH_BOTTOM_CENTER));
    }
    if (height > 0 && hasVisibleText(sheet)) {
      primitives.push(text(data.x, data.y - 0.71 * r, 0, height, 0, sheet, ATTACH_BOTTOM_CENTER));
    }
  }

  if (data.showScale && data.scaleText !== '') {
    const height = data.scaleHeight * scale;
    const value = stripToggles(data.scaleText);
    if (height > 0 && hasVisibleText(value)) {
      // `AddRuns(..., align 0)` -> left aligned, x at the circle's right + 2×scale.
      primitives.push(text(
        data.x + r + 2 * scale,
        data.y - r,
        0,
        height,
        0,
        value,
        ATTACH_BOTTOM_LEFT,
      ));
    }
  }
  return primitives;
}

export function decodeDrawingIndex(
  record: VendorRecord,
  reasons: Map<string, number>,
): Primitive[] | null {
  const data = decodeDrawingIndexData(record, reasons);
  return data ? buildDrawingIndex(data) : null;
}

// ---------------------------------------------------------------------------
// TCH_DRAWINGNAME
// ---------------------------------------------------------------------------

interface DrawingNameData {
  scale: number;
  x: number;
  y: number;
  name: string;
  scaleText: string;
  nameHeight: number;
  scaleHeight: number;
  lineWidth: number;
  gap: number;
  showScale: boolean;
  lineMode: number;
  color: number;
}

function decodeDrawingNameData(
  record: VendorRecord,
  reasons: Map<string, number>,
): DrawingNameData | null {
  const main = reader(record.main, record.mainBits, record.mainSkipBits);
  const base = base6(main, reasons, 'vendor.tch.drawingname_layout');
  if (!base) return null;
  const textReader = textStream(record);
  consumeTexts(textReader, base.texts);
  const classText = new ClassText(textReader);

  const version = main.readBitShort();
  if (version < 1 || version > 3) {
    bump(reasons, 'vendor.tch.drawingname_unsupported_version');
    return null;
  }
  let x = main.readBitDouble();
  let y = main.readBitDouble();
  main.readBitDouble();
  const name = classText.next();
  const scaleText = classText.next();
  main.readBitDouble();
  const nameHeight = main.readBitDouble();
  const scaleHeight = main.readBitDouble();
  const lineWidth = main.readBitDouble();
  const gap = main.readBitDouble();
  const showScale = main.readBit() !== 0;
  const lineMode = main.readBitShort();
  const color = main.readBitShort();
  if (version >= 2) {
    x = main.readBitDouble();
    y = main.readBitDouble();
    main.readBitDouble();
  }

  if (main.remaining !== 0) {
    bump(reasons, 'vendor.tch.drawingname_layout');
    return null;
  }
  if (lineMode < 0 || lineMode > 2) {
    bump(reasons, 'vendor.tch.drawingname_layout');
    return null;
  }
  if (
    !finite(base.scale) ||
    !finite(x) ||
    !finite(y) ||
    !finite(nameHeight) ||
    !finite(scaleHeight) ||
    !finite(lineWidth) ||
    !finite(gap) ||
    !(base.scale > 0) ||
    !(nameHeight > 0) ||
    !finite(nameHeight * base.scale) ||
    !finite(scaleHeight * base.scale) ||
    !finite(lineWidth * base.scale) ||
    color < 0 ||
    color > 256
  ) {
    bump(reasons, 'vendor.tch.drawingname_layout');
    return null;
  }
  return {
    scale: base.scale,
    x,
    y,
    name,
    scaleText,
    nameHeight,
    scaleHeight,
    lineWidth,
    gap,
    showScale,
    lineMode,
    color,
  };
}

/**
 * Width estimate for a non-TrueType string, mirroring the C++ builder's
 * deterministic rule: `HasWideChar ? 0.96454 * h : h` per character. The real
 * `BuildDrawingName` takes an extent callback, which this plugin has no
 * text-metrics dependency for, so the caller records the approximation.
 */
function estimateAdvance(value: string, height: number): number {
  const advance = hasWideChar(value) ? WIDE_HEIGHT * height : height;
  return advance * charCount(value);
}

function buildDrawingName(data: DrawingNameData, reasons: Map<string, number>): Primitive[] {
  const primitives: Primitive[] = [];
  const s = data.scale;
  const h1 = data.nameHeight * s;
  const h2 = data.scaleHeight * s;
  const line = data.lineWidth * s;
  const a1 = hasWideChar(data.name) ? WIDE_HEIGHT * h1 : h1;
  const a2 = hasWideChar(data.scaleText) ? WIDE_HEIGHT * h2 : h2;
  const scale = data.showScale && data.scaleText !== '';
  const name = stripToggles(data.name);
  const scaleText = stripToggles(data.scaleText);
  const w1 = estimateAdvance(name, h1);
  const w2 = scale ? estimateAdvance(scaleText, h2) : 0;
  const total = scale
    ? w1 + data.gap * h1 + w2 + (data.lineMode === 0 ? 0.2 * h2 : 0)
    : w1 + 0.2 * h1;
  const left = data.x - total / 2;

  if (hasVisibleText(name) && a1 > 0) {
    primitives.push(text(left, data.y + a1 / 2, 0, a1, 0, name, ATTACH_MIDDLE_LEFT));
    add(reasons, 'vendor.tch.drawingname_text_width_estimate', 1);
  }
  if (scale && hasVisibleText(scaleText) && a2 > 0) {
    primitives.push(text(
      left + w1 + data.gap * h1,
      data.y + a2 / 2,
      0,
      a2,
      0,
      scaleText,
      ATTACH_MIDDLE_LEFT,
    ));
  }

  // `extent(...).Bottom` is 0 for the deterministic non-TrueType approximation.
  const y = data.y - 2 * line;
  const length = data.lineMode === 0 && scale
    ? w1 + data.gap * h1 + w2 + 0.2 * h2
    : w1 + 0.2 * h1;
  primitives.push({ kind: 1, points: [point(left, y), point(left + length, y)] });
  if (data.lineMode === 0) {
    const y2 = y - 1.4 * s;
    primitives.push({ kind: 1, points: [point(left, y2), point(left + length, y2)] });
  }
  return primitives;
}

export function decodeDrawingName(
  record: VendorRecord,
  reasons: Map<string, number>,
): Primitive[] | null {
  const data = decodeDrawingNameData(record, reasons);
  return data ? buildDrawingName(data, reasons) : null;
}

// ---------------------------------------------------------------------------
// TCH_NORTHTHUMB
// ---------------------------------------------------------------------------

interface NorthThumbData {
  scale: number;
  x: number;
  y: number;
  radius: number;
  rotation: number;
  textHeight: number;
  color: number;
  text: string;
}

function decodeNorthThumbData(
  record: VendorRecord,
  reasons: Map<string, number>,
): NorthThumbData | null {
  const main = reader(record.main, record.mainBits, record.mainSkipBits);
  const base = base6(main, reasons, 'vendor.tch.norththumb_layout');
  if (!base) return null;
  const version = main.readByte();
  if (version < 1 || version > 2) {
    bump(reasons, 'vendor.tch.norththumb_unsupported_version');
    return null;
  }
  const x = main.readRawDouble();
  const y = main.readRawDouble();
  const radius = main.readBitDouble();
  const rotation = main.readBitDouble();
  let textHeight = 5; // `TchNorthThumb::TextHeight` default for version 1
  let color = 7;
  let value = DEFAULT_NORTH_TEXT;
  if (version > 1) {
    const code = (main.readBit() ? 2 : 0) + (main.readBit() ? 1 : 0);
    textHeight = code === 0 ? main.readRawDouble() : code === 1 ? 1.0 : 0.0;
    color = main.readBitShort();
    main.readRawDouble();
    main.readRawDouble();
    const textReader = textStream(record);
    consumeTexts(textReader, base.texts);
    value = new ClassText(textReader).next();
  }

  if (main.remaining !== 0) {
    bump(reasons, 'vendor.tch.norththumb_layout');
    return null;
  }
  if (
    !finite(x) ||
    !finite(y) ||
    !finite(radius) ||
    !(radius > 0) ||
    !finite(rotation) ||
    !(base.scale > 0) ||
    !finite(radius * base.scale) ||
    !(radius * base.scale > 0) ||
    !finite(textHeight) ||
    textHeight < 0 ||
    !finite(textHeight * base.scale)
  ) {
    bump(reasons, 'vendor.tch.norththumb_layout');
    return null;
  }
  return {
    scale: base.scale,
    x,
    y,
    radius,
    rotation,
    textHeight,
    color,
    text: value,
  };
}

function buildNorthThumb(data: NorthThumbData, reasons: Map<string, number>): Primitive[] {
  const primitives: Primitive[] = [];
  const s = data.scale;
  const r = data.radius * s;
  const ux = Math.cos(data.rotation);
  const uy = Math.sin(data.rotation);
  const w = 1.5 * s;
  primitives.push({ kind: 3, cx: data.x, cy: data.y, cz: 0, radius: r });

  const tail = point(data.x - ux * r, data.y - uy * r);
  const tip = point(data.x + ux * r, data.y + uy * r);
  // The ABI has no filled triangle: emit the closed outline and say so.
  add(reasons, 'vendor.tch.arrow_outline_only', 1);
  primitives.push({
    kind: 0,
    points: [
      point(tail.x - uy * w, tail.y + ux * w),
      point(tail.x + uy * w, tail.y - ux * w),
      tip,
      point(tail.x - uy * w, tail.y + ux * w),
    ],
  });

  const value = stripToggles(data.text);
  const height = data.textHeight * s * (hasWideChar(value) ? WIDE_HEIGHT : 1.0);
  if (hasVisibleText(value) && height > 0) {
    primitives.push(text(
      data.x + ux * (r + height),
      data.y + uy * (r + height),
      0,
      height,
      0,
      value,
      ATTACH_MIDDLE_CENTER,
    ));
  }
  return primitives;
}

export function decodeNorthThumb(
  record: VendorRecord,
  reasons: Map<string, number>,
): Primitive[] | null {
  const data = decodeNorthThumbData(record, reasons);
  return data ? buildNorthThumb(data, reasons) : null;
}
