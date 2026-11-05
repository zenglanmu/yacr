// Synthetic records-frame and class-body builders for the vendor plugin tests.
import { RECORDS_MAGIC, abiVersion } from '../../../plugins/tch/src/index.ts';

export const RECORDS_HEADER_BYTES = 28;

// ---------------------------------------------------------------------------
// Synthetic bit writer (MSB-first, matching the plugin reader)
// ---------------------------------------------------------------------------

export class BitWriter {
  bits: number[] = [];

  pushBit(value: number): void {
    this.bits.push(value & 1);
  }

  pushBits(value: number, count: number): void {
    for (let shift = count - 1; shift >= 0; shift -= 1) {
      this.pushBit(value >>> shift);
    }
  }

  byte(value: number): void {
    this.pushBits(value & 0xff, 8);
  }

  bytes(values: ArrayLike<number>): void {
    for (let index = 0; index < values.length; index += 1) this.byte(values[index]);
  }

  rawShort(value: number): void {
    const raw = value & 0xffff;
    this.byte(raw & 0xff);
    this.byte((raw >>> 8) & 0xff);
  }

  rawLong(value: number): void {
    const raw = value >>> 0;
    this.byte(raw & 0xff);
    this.byte((raw >>> 8) & 0xff);
    this.byte((raw >>> 16) & 0xff);
    this.byte((raw >>> 24) & 0xff);
  }

  rawDouble(value: number): void {
    const buffer = new ArrayBuffer(8);
    new DataView(buffer).setFloat64(0, value, true);
    this.bytes(new Uint8Array(buffer));
  }

  bitShort(value: number): void {
    this.pushBits(0, 2);
    this.rawShort(value);
  }

  bitLong(value: number): void {
    this.pushBits(0, 2);
    this.rawLong(value);
  }

  bitDouble(value: number): void {
    this.pushBits(0, 2);
    this.rawDouble(value);
  }
}

function bitsToBytes(bits: readonly number[], bitCount: number): Uint8Array {
  const count = Math.min(bitCount, bits.length);
  const bytes = new Uint8Array(Math.ceil(count / 8));
  for (let index = 0; index < count; index += 1) {
    if (bits[index] === 1) bytes[index >>> 3] |= 0x80 >>> (index & 7);
  }
  return bytes;
}

// ---------------------------------------------------------------------------
// Synthetic records frame
// ---------------------------------------------------------------------------

export interface SyntheticRecord {
  className: string;
  handle: bigint;
  dwgVersion: number;
  source: number;
  main: Uint8Array;
  mainBits: number;
  text: Uint8Array;
  textBits: number;
  handleStream: Uint8Array;
  handleBits: number;
  mainSkipBits: number;
  textSkipBits: number;
  handleSkipBits: number;
  references: bigint[];
}

export interface RecordOptions {
  mainBits?: number;
  text?: BitWriter;
  handle?: BitWriter;
  dwgVersion?: number;
  mainSkipBits?: number;
  textSkipBits?: number;
  handleSkipBits?: number;
}

export function makeRecord(
  className: string,
  handle: bigint,
  main: BitWriter,
  options: RecordOptions = {},
): SyntheticRecord {
  const mainBits = options.mainBits ?? main.bits.length;
  const textBits = options.text ? options.text.bits.length : 0;
  const handleBits = options.handle ? options.handle.bits.length : 0;
  return {
    className,
    handle,
    dwgVersion: options.dwgVersion ?? 1027,
    source: 0,
    main: bitsToBytes(main.bits, mainBits),
    mainBits,
    text: options.text ? bitsToBytes(options.text.bits, textBits) : new Uint8Array(0),
    textBits,
    handleStream: options.handle ? bitsToBytes(options.handle.bits, handleBits) : new Uint8Array(0),
    handleBits,
    mainSkipBits: options.mainSkipBits ?? 0,
    textSkipBits: options.textSkipBits ?? 0,
    handleSkipBits: options.handleSkipBits ?? 0,
    references: [],
  };
}

export function buildRecordsFrame(
  token: bigint,
  generation: bigint,
  records: readonly SyntheticRecord[],
  abi = abiVersion,
): ArrayBuffer {
  const encoder = new TextEncoder();
  const classes = records.map(record => encoder.encode(record.className));
  let size = RECORDS_HEADER_BYTES;
  for (let index = 0; index < records.length; index += 1) {
    const record = records[index];
    size +=
      2 +
      classes[index].length +
      4 +
      1 +
      8 +
      (4 + 4 + record.main.length) +
      (4 + 4 + record.text.length) +
      (4 + 4 + record.handleStream.length) +
      12 +
      4 +
      record.references.length * 8;
  }

  const buffer = new ArrayBuffer(size);
  const view = new DataView(buffer);
  view.setUint32(0, RECORDS_MAGIC, true);
  view.setUint32(4, abi, true);
  view.setBigUint64(8, token, true);
  view.setBigUint64(16, generation, true);
  view.setUint32(24, records.length, true);

  let offset = RECORDS_HEADER_BYTES;
  records.forEach((record, index) => {
    const className = classes[index];
    view.setUint16(offset, className.length, true);
    offset += 2;
    new Uint8Array(buffer, offset, className.length).set(className);
    offset += className.length;
    view.setUint32(offset, record.dwgVersion, true);
    offset += 4;
    view.setUint8(offset, record.source);
    offset += 1;
    view.setBigUint64(offset, record.handle, true);
    offset += 8;
    for (const stream of [
      [record.mainBits, record.main],
      [record.textBits, record.text],
      [record.handleBits, record.handleStream],
    ] as const) {
      view.setUint32(offset, stream[0], true);
      offset += 4;
      view.setUint32(offset, stream[1].length, true);
      offset += 4;
      new Uint8Array(buffer, offset, stream[1].length).set(stream[1]);
      offset += stream[1].length;
    }
    view.setUint32(offset, record.mainSkipBits, true);
    offset += 4;
    view.setUint32(offset, record.textSkipBits, true);
    offset += 4;
    view.setUint32(offset, record.handleSkipBits, true);
    offset += 4;
    view.setUint32(offset, record.references.length, true);
    offset += 4;
    for (const handle of record.references) {
      view.setBigUint64(offset, handle, true);
      offset += 8;
    }
  });
  return buffer;
}

// ---------------------------------------------------------------------------
// Synthetic TCH_WALL / TCH_COLUMN class bodies
// ---------------------------------------------------------------------------

function protectedGeometry(options: {
  start: readonly [number, number, number];
  end: readonly [number, number, number];
  angle?: number;
  flags?: number;
  left: number;
  right: number;
  height: number;
}): Uint8Array {
  const buffer = new ArrayBuffer(84);
  const view = new DataView(buffer);
  view.setFloat64(0, options.start[0], true);
  view.setFloat64(8, options.start[1], true);
  view.setFloat64(16, options.start[2], true);
  view.setFloat64(24, options.end[0], true);
  view.setFloat64(32, options.end[1], true);
  view.setFloat64(40, options.end[2], true);
  view.setFloat64(48, options.angle ?? 0, true);
  view.setUint32(56, options.flags ?? 2, true);
  view.setFloat64(60, options.left, true);
  view.setFloat64(68, options.right, true);
  view.setFloat64(76, options.height, true);
  return new Uint8Array(buffer);
}

export function wallWriter(options: {
  start: readonly [number, number, number];
  end: readonly [number, number, number];
  angle?: number;
  flags?: number;
  left: number;
  right: number;
  height: number;
  mark?: number;
  key?: number;
  extra?: number;
  openings?: number;
  material?: number;
  patches?: number;
  insulation?: number;
  joins?: readonly [number, number, number, number];
  trailing?: number;
}): BitWriter {
  const writer = new BitWriter();
  writer.byte(6);
  writer.bitDouble(150);
  writer.bitShort(0);
  writer.byte(3);
  writer.pushBits(0, 2);
  writer.rawShort(512);

  const mark = options.mark ?? 9;
  const key = options.key ?? 0x5a;
  writer.byte(mark);
  writer.byte(key);
  for (const plain of protectedGeometry(options)) {
    const masked = (plain ^ key) & 0xff;
    writer.byte(mark === 7 ? ((masked << 4) | (masked >>> 4)) & 0xff : masked);
  }

  writer.byte(options.extra ?? 1);
  writer.bitShort(options.openings ?? 0);
  writer.byte(options.material ?? 10);
  writer.byte(options.patches ?? 0);
  writer.bitDouble(options.insulation ?? 80.080000001);
  for (const join of options.joins ?? [0, 0, 0, 12.5]) writer.bitDouble(join);
  writer.byte(0);
  writer.pushBits(2, 3);
  writer.byte(options.trailing ?? 0);
  return writer;
}

export interface ColumnVertex {
  x: number;
  y: number;
  bulge?: number;
}

export interface BagItem {
  name: string;
  type: number;
  value?: string;
  number?: number;
}

function writeText(writer: BitWriter, value: string): void {
  writer.bitShort(value.length);
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    writer.byte(code & 0xff);
    writer.byte((code >>> 8) & 0xff);
  }
}

function writeBag(writer: BitWriter, text: BitWriter, bag: readonly BagItem[]): void {
  writer.bitLong(bag.length);
  for (const item of bag) {
    writeText(text, item.name);
    writer.bitLong(item.type);
    if (item.type === 0x65) writeText(text, item.value ?? '');
    else if (item.type === 200) writer.bitDouble(item.number ?? 0);
    else throw new Error(`unsupported synthetic bag type ${item.type}`);
  }
}

export function columnWriter(options: {
  position: readonly [number, number, number];
  vertices: readonly ColumnVertex[];
  height?: number;
  material?: number;
  bag?: readonly BagItem[];
  text?: BitWriter;
}): BitWriter {
  const writer = new BitWriter();
  writer.byte(6);
  writer.bitDouble(150);
  writer.bitDouble(0);
  writer.byte(0); // base flags: no base handles
  writeBag(writer, options.text ?? new BitWriter(), options.bag ?? []);
  writer.byte(6); // column version
  writer.bitDouble(options.position[0]);
  writer.bitDouble(options.position[1]);
  writer.bitDouble(options.position[2]);
  writer.byte(0); // shape type
  writer.bitShort(options.vertices.length);
  for (const vertex of options.vertices) {
    writer.rawDouble(vertex.x);
    writer.rawDouble(vertex.y);
    writer.bitDouble(vertex.bulge ?? 0);
  }
  writer.bitDouble(options.height ?? 4200);
  writer.byte(0);
  writer.byte(0); // handle count
  writer.byte(options.material ?? 70);
  writer.bitShort(0); // version > 3 flags
  writer.byte(0); // version > 4
  writer.bitDouble(0); // version > 5
  return writer;
}

/** Absolute handle (code 0, minimal big-endian counter bytes). */
function writeHandle(writer: BitWriter, value: bigint): void {
  const bytes: number[] = [];
  for (let temp = value; temp > 0n; temp >>= 8n) bytes.unshift(Number(temp & 0xffn));
  if (bytes.length > 8) throw new Error('synthetic handle too long');
  writer.byte(bytes.length);
  for (const byte of bytes) writer.byte(byte);
}

export function openingWriter(options: {
  position: readonly [number, number, number];
  width: number;
  wall: bigint;
  thickness?: number;
  height?: number;
  angle?: number;
  bag?: readonly BagItem[];
  text?: BitWriter;
  baseHandles?: readonly bigint[];
  block2d?: bigint;
  block3d?: bigint;
  handles?: BitWriter;
  mirror?: number;
  kind?: number;
  label?: string;
  labelX?: number;
  labelY?: number;
  labelHeight?: number;
  flags?: number;
  segments?: readonly { x1: number; y1: number; x2: number; y2: number; bulge: number }[];
  bodyVersion?: number;
}): BitWriter {
  const writer = new BitWriter();
  const baseHandles = options.baseHandles ?? [];
  writer.byte(6);
  writer.bitDouble(150);
  writer.bitDouble(0);
  writer.byte(baseHandles.length > 0 ? 1 : 0);
  if (baseHandles.length > 0) writer.byte(baseHandles.length);
  const text = options.text ?? new BitWriter();
  writeBag(writer, text, options.bag ?? []);
  const version = options.bodyVersion ?? 5;
  writer.byte(version);
  writer.bitDouble(options.position[0]);
  writer.bitDouble(options.position[1]);
  writer.bitDouble(options.position[2]);
  writer.bitDouble(options.width);
  writer.bitDouble(options.thickness ?? 0.2);
  writer.bitDouble(options.height ?? 2);
  writer.bitDouble(options.angle ?? 0);
  const mirror = options.mirror ?? 0;
  const kind = options.kind ?? 0;
  writer.byte(mirror);
  writer.byte(kind);
  writer.bitDouble(0); // unused value after kind
  if (options.text) writeText(text, options.label ?? '');
  writer.bitDouble(options.labelX ?? 0);
  writer.bitDouble(options.labelY ?? 0);
  writer.bitDouble(0);
  const flags = options.flags ?? 0;
  writer.bitShort(flags);
  if ((flags & 1) !== 0 || kind === 10) {
    writer.bitDouble(0);
    writer.bitDouble(0);
  }
  if (kind === 4) {
    writer.byte(0);
    writer.bitDouble(0);
    writer.bitDouble(0);
    writer.bitDouble(0);
    if (version > 4) writer.bitDouble(0);
  } else if (kind === 6 || kind === 7) {
    writer.bitDouble(0);
    if (kind === 6) writer.bitDouble(0);
  }
  writer.bitDouble(options.labelHeight ?? 3.5);
  writer.bitDouble(0);
  writer.bitDouble(0);
  writer.bitDouble(0);
  const segments = options.segments ?? [];
  writer.bitShort(segments.length);
  for (const segment of segments) {
    writer.rawDouble(segment.x1);
    writer.rawDouble(segment.y1);
    writer.rawDouble(segment.x2);
    writer.rawDouble(segment.y2);
    writer.bitDouble(segment.bulge);
  }
  writer.bitLong(0);
  writer.bitShort(0);
  writer.pushBit(0);
  if (options.text) writeText(text, '');
  for (let index = 0; index < 4; index += 1) writer.bitDouble(0);
  writer.bitDouble(0);
  const handles = options.handles ?? new BitWriter();
  for (const handle of baseHandles) writeHandle(handles, handle);
  writeHandle(handles, options.block2d ?? 0n);
  writeHandle(handles, options.block3d ?? 0n);
  writeHandle(handles, options.wall);
  // style + four trailing slots (empty handles still occupy a slot)
  for (let index = 0; index < 5; index += 1) writeHandle(handles, 0n);
  return writer;
}

export function textWriter(options: {
  className: 'TCH_TEXT' | 'TCH_MTEXT';
  value: string;
  position: readonly [number, number, number];
  height: number;
  rotation: number;
  attachment: number;
  text: BitWriter;
  scale?: number;
  version?: number;
  bag?: readonly BagItem[];
  mversion?: number;
  encryption?: number;
  boxWidth?: number;
  lineSpacing?: number;
}): BitWriter {
  const writer = new BitWriter();
  const version = options.version ?? 2;
  writer.byte(6);
  writer.bitDouble(options.scale ?? 100);
  writer.bitDouble(0);
  writer.byte(0); // base flags: no base handles
  writeBag(writer, options.text, options.bag ?? []);
  writeText(options.text, options.value);
  writer.byte(version);
  writer.bitDouble(options.position[0]);
  writer.bitDouble(options.position[1]);
  writer.bitDouble(options.position[2]);
  writer.bitDouble(options.height);
  writer.bitDouble(options.rotation);
  writer.byte(options.attachment);
  if (version >= 2) writer.byte(0);
  if (options.className === 'TCH_MTEXT') {
    const mversion = options.mversion ?? 2;
    writer.byte(mversion);
    if (mversion > 1) writer.byte(options.encryption ?? 0);
    writer.bitDouble(options.boxWidth ?? 0);
    writer.bitDouble(options.lineSpacing ?? 0);
  }
  return writer;
}
