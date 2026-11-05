// Vendor-decoding plugin entry (Tianzheng / `TCH_*`).
//
// This is the module the WebView loads. `scripts/build-vendor-plugin.sh` bundles
// it with rolldown into `plugins/tch/dist/index.js` and copies that single ES
// module to `apps/desktop-ui/public/plugins/tch.js`. The host only knows the
// `decode(ArrayBuffer) -> ArrayBuffer` shape registered on a stable global.
//
// ABI v1 keeps the v2 **records** frame unchanged and extends the **result**
// frame with derived primitives. Decoders live in `bit-reader.ts`, `wall.ts`,
// `column.ts`, `opening.ts` and `text.ts`; this file owns records parsing,
// two-pass dispatch, diagnostics and the result frame. Openings cut wall gaps
// and reference their imported 2D block for real door/window symbols; unknown
// layouts are reported, never guessed.

import { decodeArrow, decodeElevation, decodeMultiLeader } from './annotation.ts';
import { decodeDrawingIndex, decodeDrawingName, decodeNorthThumb } from './annotation-symbol.ts';
import { decodeBlockInsert } from './block-insert.ts';
import { decodeModi, decodeRupture } from './break-line.ts';
import { decodeColumn } from './column.ts';
import { bump } from './diagnostics.ts';
import { decodeAxisLabel, decodeDimension2, decodeRadiusDim } from './dimension.ts';
import { decodeCornerWindow, decodeIndexPointer, decodeSection } from './index-symbol.ts';
import { decodeOpening, openingPrimitives } from './opening.ts';
import type { OpeningCut, OpeningData } from './opening.ts';
import type { DecodedEntity, Primitive, VendorRecord } from './types.ts';
import { decodeLineStair, decodeRectStair } from './stair.ts';
import { decodeText } from './text.ts';
import { parseWall, wallPrimitives } from './wall.ts';
import type { WallData } from './wall.ts';

export const abiVersion = 1;

/** Records frame magic (`"YAND"`), little-endian. */
export const RECORDS_MAGIC = 0x59414e44;
/** Result frame magic (`"YANR"`), little-endian. */
export const RESULT_MAGIC = 0x59414e52;

const RECORDS_HEADER_BYTES = 28;
const RESULT_STATUS_OK = 0;
const RESULT_STATUS_UNSUPPORTED = 1;
const MAX_DIAGNOSTICS = 16;
const MAX_DIAGNOSTIC_BYTES = 512;
const MAX_COUNT_LENGTH = 400;
const MAX_RESULT_BYTES = 4 * 1024 * 1024;

/** The plugin surface the host reads from the global registration below. */
export interface VendorPlugin {
  readonly abiVersion: number;
  decode(records: ArrayBuffer): ArrayBuffer;
}

declare global {
  // eslint-disable-next-line no-var
  var __YACR_VENDOR_PLUGIN__: VendorPlugin | undefined;
}

/** The one export the host calls. */
export function decode(records: ArrayBuffer): ArrayBuffer {
  if (records.byteLength < RECORDS_HEADER_BYTES) {
    throw new Error('vendor.truncated');
  }
  const view = new DataView(records);
  if (view.getUint32(0, true) !== RECORDS_MAGIC) {
    throw new Error('vendor.bad_magic');
  }
  const abi = view.getUint32(4, true);
  if (abi !== abiVersion) {
    throw new Error(`vendor.unsupported_abi: ${abi}`);
  }
  const token = view.getBigUint64(8, true);
  const generation = view.getBigUint64(16, true);
  const recordCount = view.getUint32(24, true);
  const parsed = parseRecords(view, 28, recordCount);
  return decodeRecords(token, generation, parsed);
}

/** Read one u32 at `offset`, demanding that the frame contains it. */
function u32(view: DataView, offset: number): number {
  if (offset + 4 > view.byteLength) throw new Error('vendor.truncated');
  return view.getUint32(offset, true);
}

/** Walk the v2 record list, validating every length before advancing. */
function parseRecords(view: DataView, start: number, count: number): VendorRecord[] {
  const records: VendorRecord[] = [];
  let offset = start;
  for (let index = 0; index < count; index += 1) {
    if (offset + 2 > view.byteLength) throw new Error('vendor.truncated');
    const classNameLength = view.getUint16(offset, true); offset += 2;
    if (offset + classNameLength > view.byteLength) throw new Error('vendor.truncated');
    const className = new TextDecoder().decode(new Uint8Array(view.buffer, view.byteOffset + offset, classNameLength));
    offset += classNameLength;
    if (offset + 13 > view.byteLength) throw new Error('vendor.truncated');
    const dwgVersion = view.getUint32(offset, true);
    offset += 5; // dwg_version u32 + source u8
    const handle = view.getBigUint64(offset, true); offset += 8;
    const main = readStream(view, offset); offset = main.offset;
    const text = readStream(view, offset); offset = text.offset;
    const handleStream = readStream(view, offset); offset = handleStream.offset;
    const mainSkipBits = u32(view, offset); offset += 4;
    const textSkipBits = u32(view, offset); offset += 4;
    const handleSkipBits = u32(view, offset); offset += 4;
    const referenceCount = u32(view, offset); offset += 4;
    if (offset + referenceCount * 8 > view.byteLength) throw new Error('vendor.truncated');
    offset += referenceCount * 8;
    if (mainSkipBits > main.bits || textSkipBits > text.bits || handleSkipBits > handleStream.bits) {
      throw new Error('vendor.truncated');
    }
    records.push({
      className, handle, dwgVersion,
      main: main.bytes, mainBits: main.bits, text: text.bytes, textBits: text.bits,
      handleStream: handleStream.bytes, handleBits: handleStream.bits,
      mainSkipBits, textSkipBits, handleSkipBits,
    });
  }
  if (offset !== view.byteLength) throw new Error('vendor.trailing_bytes');
  return records;
}

interface StreamSlice { bytes: Uint8Array; bits: number; offset: number }

/** Read `bits, byte_length, bytes` and return the next offset. */
function readStream(view: DataView, offset: number): StreamSlice {
  const bits = u32(view, offset);
  const byteLength = u32(view, offset + 4);
  offset += 8;
  if (offset + byteLength > view.byteLength) throw new Error('vendor.truncated');
  if (bits > byteLength * 8) throw new Error('vendor.truncated');
  const bytes = new Uint8Array(view.buffer, view.byteOffset + offset, byteLength);
  return { bytes, bits, offset: offset + byteLength };
}

function decodeRecords(
  token: bigint,
  generation: bigint,
  records: readonly VendorRecord[],
): ArrayBuffer {
  const decoded = new Map<string, number>();
  const unsupported = new Map<string, number>();
  const noDisplay = new Map<string, number>();
  const reasons = new Map<string, number>();
  const walls = new Map<VendorRecord, WallData>();
  const wallByHandle = new Map<bigint, WallData>();
  const wallHandles = new Set<bigint>();
  const decodedWallHandles = new Set<bigint>();
  const openings: Array<{ wall: bigint; cut: OpeningCut }> = [];
  const openingsByRecord = new Map<VendorRecord, OpeningData>();
  for (const record of records) {
    if (record.className === 'TCH_WALL') wallHandles.add(record.handle);
  }

  // Phase 1: parse walls and openings; walls are rebuilt after all cuts are known.
  for (const record of records) {
    if (record.className === 'TCH_WALL') {
      try {
        const data = parseWall(record, reasons);
        if (data) {
          walls.set(record, data);
          wallByHandle.set(record.handle, data);
          decodedWallHandles.add(record.handle);
        }
      } catch (error) {
        bump(reasons, failure('vendor.tch.wall', error));
      }
    } else if (record.className === 'TCH_OPENING') {
      try {
        const opening = decodeOpening(record, reasons);
        if (opening) {
          openings.push({ wall: opening.wall, cut: opening.cut });
          openingsByRecord.set(record, opening);
        } else {
          bump(unsupported, 'TCH_OPENING');
        }
      } catch (error) {
        bump(unsupported, 'TCH_OPENING');
        bump(reasons, failure('vendor.tch.opening', error));
      }
    }
  }

  const byWall = new Map<bigint, OpeningCut[]>();
  for (const opening of openings) {
    if (!wallHandles.has(opening.wall)) {
      bump(reasons, 'vendor.tch.opening_wall_missing');
      continue;
    }
    if (!decodedWallHandles.has(opening.wall)) {
      bump(reasons, 'vendor.tch.opening_wall_undecoded');
      continue;
    }
    const list = byWall.get(opening.wall);
    if (list) list.push(opening.cut);
    else byWall.set(opening.wall, [opening.cut]);
  }

  // Phase 2: emit entities in record order.
  const entities: DecodedEntity[] = [];
  for (const record of records) {
    try {
      let primitives: Primitive[] | null = null;
      if (record.className === 'TCH_WALL') {
        const data = walls.get(record);
        if (!data) {
          // Already explained by a `vendor.tch.wall_*` reason in phase 1; the
          // record still has no display, so it belongs to the refused bucket.
          bump(unsupported, 'TCH_WALL');
          continue;
        }
        primitives = wallPrimitives(data, byWall.get(record.handle) ?? [], reasons);
      } else if (record.className === 'TCH_COLUMN') {
        primitives = decodeColumn(record, reasons);
      } else if (record.className === 'TCH_TEXT' || record.className === 'TCH_MTEXT') {
        primitives = decodeText(record, reasons);
      } else if (record.className === 'TCH_OPENING') {
        const opening = openingsByRecord.get(record);
        if (!opening) continue;
        primitives = openingPrimitives(opening, wallByHandle, reasons);
      } else if (record.className === 'TCH_ELEVATION') {
        primitives = decodeElevation(record, reasons);
      } else if (record.className === 'TCH_ARROW') {
        primitives = decodeArrow(record, reasons);
      } else if (record.className === 'TCH_MULTILEADER') {
        primitives = decodeMultiLeader(record, reasons);
      } else if (record.className === 'TCH_DIMENSION2') {
        primitives = decodeDimension2(record, reasons);
      } else if (record.className === 'TCH_AXIS_LABEL') {
        primitives = decodeAxisLabel(record, reasons);
      } else if (record.className === 'TCH_RADIUSDIM') {
        primitives = decodeRadiusDim(record, reasons);
      } else if (record.className === 'TCH_LINESTAIR') {
        primitives = decodeLineStair(record, reasons);
      } else if (record.className === 'TCH_RECTSTAIR') {
        primitives = decodeRectStair(record, reasons);
      } else if (record.className === 'TCH_RUPTURE') {
        primitives = decodeRupture(record, reasons);
      } else if (record.className === 'TCH_MODI') {
        primitives = decodeModi(record, reasons);
      } else if (record.className === 'TCH_DRAWINGINDEX') {
        primitives = decodeDrawingIndex(record, reasons);
      } else if (record.className === 'TCH_DRAWINGNAME') {
        primitives = decodeDrawingName(record, reasons);
      } else if (record.className === 'TCH_NORTHTHUMB') {
        primitives = decodeNorthThumb(record, reasons);
      } else if (record.className === 'TCH_BLOCK_INSERT') {
        primitives = decodeBlockInsert(record, reasons);
      } else if (record.className === 'TCH_SYMB_SECTION') {
        primitives = decodeSection(record, reasons);
      } else if (record.className === 'TCH_INDEXPOINTER') {
        primitives = decodeIndexPointer(record, reasons);
      } else if (record.className === 'TCH_CORNER_WINDOW') {
        primitives = decodeCornerWindow(record, reasons);
      }
      // Every record lands in exactly one bucket: it produced display
      // primitives, it is valid but draws nothing (for example a revision-cloud
      // or arrow record with no usable vertices), or it is refused. The
      // coverage line below proves the three buckets add up to the record count.
      if (primitives === null) {
        bump(unsupported, record.className);
        continue;
      }
      if (primitives.length === 0) {
        bump(noDisplay, record.className);
        continue;
      }
      entities.push({ handle: record.handle, primitives });
      bump(decoded, record.className);
    } catch (error) {
      bump(reasons, failure(failurePrefix(record.className), error));
    }
  }

  const applied = entities.reduce((sum, entity) => sum + entity.primitives.length, 0);
  const status = entities.length > 0 ? RESULT_STATUS_OK : RESULT_STATUS_UNSUPPORTED;
  const diagnostics = buildDiagnostics(
    entities.length,
    records.length,
    decoded,
    unsupported,
    noDisplay,
    reasons,
  );
  return buildResult(token, generation, status, applied, entities, diagnostics);
}

/** Stable diagnostic prefix for one class; unknown classes keep their name. */
function failurePrefix(className: string): string {
  switch (className) {
    case 'TCH_WALL': return 'vendor.tch.wall';
    case 'TCH_COLUMN': return 'vendor.tch.column';
    case 'TCH_OPENING': return 'vendor.tch.opening';
    case 'TCH_ELEVATION': return 'vendor.tch.elevation';
    case 'TCH_ARROW': return 'vendor.tch.arrow';
    case 'TCH_MULTILEADER': return 'vendor.tch.multileader';
    case 'TCH_DIMENSION2': return 'vendor.tch.dimension';
    case 'TCH_AXIS_LABEL': return 'vendor.tch.axis';
    case 'TCH_RADIUSDIM': return 'vendor.tch.radiusdim';
    case 'TCH_LINESTAIR': return 'vendor.tch.linestair';
    case 'TCH_RECTSTAIR': return 'vendor.tch.rectstair';
    case 'TCH_RUPTURE': return 'vendor.tch.rupture';
    case 'TCH_MODI': return 'vendor.tch.modi';
    case 'TCH_DRAWINGINDEX': return 'vendor.tch.drawingindex';
    case 'TCH_DRAWINGNAME': return 'vendor.tch.drawingname';
    case 'TCH_NORTHTHUMB': return 'vendor.tch.norththumb';
    case 'TCH_BLOCK_INSERT': return 'vendor.tch.block_insert';
    case 'TCH_SYMB_SECTION': return 'vendor.tch.section';
    case 'TCH_INDEXPOINTER': return 'vendor.tch.indexpointer';
    case 'TCH_CORNER_WINDOW': return 'vendor.tch.cornerwindow';
    default: return 'vendor.tch.text';
  }
}

function failure(prefix: string, error: unknown): string {
  return isTruncated(error) ? `${prefix}_truncated` : `${prefix}_layout`;
}

function isTruncated(error: unknown): boolean {
  return error instanceof Error && error.message === 'vendor.truncated';
}

function buildDiagnostics(
  entityCount: number,
  recordCount: number,
  decoded: Map<string, number>,
  unsupported: Map<string, number>,
  noDisplay: Map<string, number>,
  reasons: Map<string, number>,
): string[] {
  const messages: string[] = [];
  if (entityCount === 0) messages.push('vendor.decoder_no_supported_entities');
  if (decoded.size > 0) messages.push(`vendor.tch.decoded: ${formatCounts(decoded)}`);
  if (unsupported.size > 0) messages.push(`vendor.tch.unsupported: ${formatCounts(unsupported)}`);
  if (noDisplay.size > 0) messages.push(`vendor.tch.no_display: ${formatCounts(noDisplay)}`);
  const sum = (counts: Map<string, number>): number => {
    let total = 0;
    for (const value of counts.values()) total += value;
    return total;
  };
  const accounted = sum(decoded) + sum(unsupported) + sum(noDisplay);
  if (accounted !== recordCount) {
    messages.push(`vendor.tch.unaccounted: ${recordCount - accounted}`);
  }
  for (const reason of [...reasons.keys()].sort()) {
    messages.push(`${reason}: ${reasons.get(reason) ?? 0}`);
  }
  return messages.slice(0, MAX_DIAGNOSTICS);
}

function formatCounts(counts: Map<string, number>): string {
  const keys = [...counts.keys()].sort();
  let text = '';
  for (const key of keys) {
    const piece = `${text.length > 0 ? ',' : ''}${key}=${counts.get(key) ?? 0}`;
    if (text.length + piece.length > MAX_COUNT_LENGTH) {
      return `${text},...`;
    }
    text += piece;
  }
  return text;
}

export function buildResult(
  token: bigint,
  generation: bigint,
  status: number,
  applied: number,
  entities: readonly DecodedEntity[],
  diagnostics: readonly string[],
): ArrayBuffer {
  const encoded = diagnostics
    .slice(0, MAX_DIAGNOSTICS)
    .map(message => clampDiagnostic(message));
  let length = 36 + 4; // header + entity_count
  for (const bytes of encoded) length += 2 + bytes.byteLength;
  for (const entity of entities) {
    length += 8 + 4;
    for (const primitive of entity.primitives) length += primitiveBytes(primitive);
  }
  if (length > MAX_RESULT_BYTES) {
    throw new Error('vendor.result_too_large');
  }

  const buffer = new ArrayBuffer(length);
  const view = new DataView(buffer);
  view.setUint32(0, RESULT_MAGIC, true);
  view.setUint32(4, abiVersion, true);
  view.setBigUint64(8, token, true);
  view.setBigUint64(16, generation, true);
  view.setUint32(24, status, true);
  view.setUint32(28, applied, true);
  view.setUint32(32, encoded.length, true);
  let offset = 36;
  for (const bytes of encoded) {
    view.setUint16(offset, bytes.byteLength, true);
    offset += 2;
    new Uint8Array(buffer, offset, bytes.byteLength).set(bytes);
    offset += bytes.byteLength;
  }
  view.setUint32(offset, entities.length, true);
  offset += 4;
  for (const entity of entities) {
    view.setBigUint64(offset, entity.handle, true);
    offset += 8;
    view.setUint32(offset, entity.primitives.length, true);
    offset += 4;
    for (const primitive of entity.primitives) {
      offset = writePrimitive(view, offset, primitive);
    }
  }
  return buffer;
}

function encodeText(value: string): Uint8Array {
  return new TextEncoder().encode(value);
}

function primitiveBytes(primitive: Primitive): number {
  switch (primitive.kind) {
    case 0:
    case 1:
      return 1 + 4 + primitive.points.length * 24;
    case 2:
      return 1 + 48;
    case 3:
      return 1 + 32;
    case 4:
      return 52 + encodeText(primitive.text).byteLength;
    case 5:
      return 65;
  }
}

function writePrimitive(view: DataView, offset: number, primitive: Primitive): number {
  view.setUint8(offset, primitive.kind);
  offset += 1;
  switch (primitive.kind) {
    case 0:
    case 1: {
      view.setUint32(offset, primitive.points.length, true);
      offset += 4;
      for (const point of primitive.points) {
        view.setFloat64(offset, point.x, true);
        view.setFloat64(offset + 8, point.y, true);
        view.setFloat64(offset + 16, point.z, true);
        offset += 24;
      }
      return offset;
    }
    case 2: {
      view.setFloat64(offset, primitive.cx, true);
      view.setFloat64(offset + 8, primitive.cy, true);
      view.setFloat64(offset + 16, primitive.cz, true);
      view.setFloat64(offset + 24, primitive.radius, true);
      view.setFloat64(offset + 32, primitive.startAngle, true);
      view.setFloat64(offset + 40, primitive.sweep, true);
      return offset + 48;
    }
    case 3: {
      view.setFloat64(offset, primitive.cx, true);
      view.setFloat64(offset + 8, primitive.cy, true);
      view.setFloat64(offset + 16, primitive.cz, true);
      view.setFloat64(offset + 24, primitive.radius, true);
      return offset + 32;
    }
    case 4: {
      view.setFloat64(offset, primitive.origin.x, true);
      view.setFloat64(offset + 8, primitive.origin.y, true);
      view.setFloat64(offset + 16, primitive.origin.z, true);
      view.setFloat64(offset + 24, primitive.height, true);
      view.setFloat64(offset + 32, primitive.rotation, true);
      view.setFloat64(offset + 40, primitive.wrapWidth, true);
      view.setUint8(offset + 48, primitive.attachment);
      const bytes = encodeText(primitive.text);
      view.setUint16(offset + 49, bytes.byteLength, true);
      new Uint8Array(view.buffer, view.byteOffset + offset + 51, bytes.byteLength).set(bytes);
      return offset + 51 + bytes.byteLength;
    }
    case 5: {
      view.setBigUint64(offset, primitive.block, true);
      view.setFloat64(offset + 8, primitive.origin.x, true);
      view.setFloat64(offset + 16, primitive.origin.y, true);
      view.setFloat64(offset + 24, primitive.origin.z, true);
      view.setFloat64(offset + 32, primitive.scale.x, true);
      view.setFloat64(offset + 40, primitive.scale.y, true);
      view.setFloat64(offset + 48, primitive.scale.z, true);
      view.setFloat64(offset + 56, primitive.rotation, true);
      return offset + 64;
    }
  }
}

function clampDiagnostic(message: string): Uint8Array {
  const encoder = new TextEncoder();
  let bytes = encoder.encode(message);
  if (bytes.byteLength <= MAX_DIAGNOSTIC_BYTES) return bytes;
  let end = message.length;
  while (end > 0) {
    end -= 1;
    bytes = encoder.encode(message.slice(0, end));
    if (bytes.byteLength <= MAX_DIAGNOSTIC_BYTES) return bytes;
  }
  return new Uint8Array(0);
}

// The host loads this file with a `<script type="module">` (the same pattern as
// the canvas bootstrap): a module script cannot hand its exports to the page, so
// it registers the one plugin surface on a stable global. The host validates the
// shape before calling it.
globalThis.__YACR_VENDOR_PLUGIN__ = { abiVersion, decode };
