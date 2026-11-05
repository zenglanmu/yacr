// Contract tests for `TCH_BLOCK_INSERT` decoding: the `AcDbBlockReference`
// prefix, its four scale encodings and the single derived kind-5 block
// primitive. Synthetic records mirror the real AC1027 layout, including the
// trailing TCH base-6 header that every fixture record carries.

import assert from 'node:assert/strict';
import test from 'node:test';

import { decodeBlockInsert } from '../../../plugins/tch/src/block-insert.ts';
import { reader } from '../../../plugins/tch/src/bit-reader.ts';
import { BitWriter, makeRecord } from './vendor-plugin-fixtures.ts';

// ---------------------------------------------------------------------------
// Local synthetic builders
// ---------------------------------------------------------------------------

function writeHandle(writer: BitWriter, value: bigint): void {
  const bytes: number[] = [];
  for (let temp = value; temp > 0n; temp >>= 8n) bytes.unshift(Number(temp & 0xffn));
  writer.byte(bytes.length);
  for (const byte of bytes) writer.byte(byte);
}

interface BlockInsertOptions {
  origin?: [number, number, number];
  scaleMode?: 0 | 1 | 2 | 3;
  x?: number;
  /** `null` inherits the mode default (x for mode 0, 1 for mode 1). */
  y?: number | null;
  z?: number | null;
  rotation?: number;
  normal?: [number, number, number];
  attributes?: boolean;
  owned?: number;
  version?: number;
  /** Real records append the TCH base-6 header; omit it to pin the prefix-only layout. */
  base6Tail?: boolean;
}

function writeScale(writer: BitWriter, fallback: number, value: number | null | undefined): void {
  if (value === null || value === undefined || value === fallback) {
    writer.pushBits(0, 2); // 00: keep the default
    return;
  }
  writer.pushBits(3, 2); // 11: full raw double
  writer.rawDouble(value);
}

/** `TchBase` v6 tail: version, drawing scale, second value, flags, empty bag. */
function writeBase6Tail(writer: BitWriter, scale = 150): void {
  writer.byte(6);
  writer.bitDouble(scale);
  writer.pushBits(2, 2); // second value 0.0
  writer.byte(1); // base flags: one handle-count byte follows
  writer.byte(0); // no base handles
  writer.pushBits(2, 2); // empty property bag
}

function blockInsertMain(options: BlockInsertOptions = {}): BitWriter {
  const main = new BitWriter();
  const origin = options.origin ?? [100, 200, 0];
  const x = options.x ?? 1;
  main.bitDouble(origin[0]);
  main.bitDouble(origin[1]);
  main.bitDouble(origin[2]);

  const mode = options.scaleMode ?? 3;
  main.pushBits(mode, 2);
  if (mode === 0) {
    main.rawDouble(x);
    writeScale(main, x, options.y);
    writeScale(main, x, options.z);
  } else if (mode === 1) {
    writeScale(main, 1, options.y);
    writeScale(main, 1, options.z);
  } else if (mode === 2) {
    main.rawDouble(x);
  }

  main.bitDouble(options.rotation ?? 0);
  const normal = options.normal ?? [0, 0, 1];
  main.bitDouble(normal[0]);
  main.bitDouble(normal[1]);
  main.bitDouble(normal[2]);
  main.pushBit(options.attributes ? 1 : 0);
  if (options.attributes) main.bitLong(options.owned ?? 0);
  main.byte(options.version ?? 4);
  main.byte(0); // mode: only 0 is reproducible
  if (options.base6Tail !== false) writeBase6Tail(main);
  return main;
}

function blockInsertRecord(
  options: BlockInsertOptions & { handle?: bigint; block?: bigint; dwgVersion?: number } = {},
) {
  const handle = options.handle ?? 0x600n;
  const handles = new BitWriter();
  writeHandle(handles, options.block ?? 0x3dd82n);
  const record = makeRecord('TCH_BLOCK_INSERT', handle, blockInsertMain(options), {
    handle: handles,
    dwgVersion: options.dwgVersion ?? 1027,
  });
  return { record, handle };
}

// ---------------------------------------------------------------------------
// Scale encodings
// ---------------------------------------------------------------------------

test('scale mode 0 inherits a non-unit X for Y and Z', () => {
  const reasons = new Map<string, number>();
  const { record } = blockInsertRecord({ scaleMode: 0, x: 2.5 });
  const primitives = decodeBlockInsert(record, reasons);
  assert.ok(primitives, 'record decodes');
  assert.equal(primitives.length, 1);
  const primitive = primitives[0];
  assert.equal(primitive.kind, 5);
  if (primitive.kind !== 5) return;
  assert.deepEqual(primitive.scale, { x: 2.5, y: 2.5, z: 2.5 });
  assert.equal(reasons.size, 0, 'a clean decode bumps no reason');
});

test('scale mode 2 applies one uniform value to every axis', () => {
  const reasons = new Map<string, number>();
  const { record } = blockInsertRecord({ scaleMode: 2, x: -1.5 });
  const primitives = decodeBlockInsert(record, reasons);
  assert.ok(primitives);
  const primitive = primitives[0];
  if (primitive.kind !== 5) return assert.fail('expected a block primitive');
  assert.deepEqual(primitive.scale, { x: -1.5, y: -1.5, z: -1.5 });
});

test('scale mode 3 keeps the unit default on every axis', () => {
  const reasons = new Map<string, number>();
  const { record } = blockInsertRecord({ scaleMode: 3 });
  const primitives = decodeBlockInsert(record, reasons);
  assert.ok(primitives);
  const primitive = primitives[0];
  if (primitive.kind !== 5) return assert.fail('expected a block primitive');
  assert.deepEqual(primitive.scale, { x: 1, y: 1, z: 1 });
});

// ---------------------------------------------------------------------------
// Attributes and carried-through geometry
// ---------------------------------------------------------------------------

test('an attributed insert on AC1027 consumes the owned-count BitLong', () => {
  const reasons = new Map<string, number>();
  const { record } = blockInsertRecord({
    scaleMode: 2,
    x: 1,
    attributes: true,
    owned: 3,
    dwgVersion: 1027,
  });
  const primitives = decodeBlockInsert(record, reasons);
  assert.ok(primitives, 'the owned count is consumed, not read as the version byte');
  assert.equal(primitives.length, 1);
});

test('origin, rotation and block handle reach the derived primitive', () => {
  const reasons = new Map<string, number>();
  const { record } = blockInsertRecord({
    origin: [1234.5, -678.25, 12],
    scaleMode: 2,
    x: 0.5,
    rotation: Math.PI / 6,
    normal: [0, 0, 1],
    block: 0xabcden,
  });
  const primitives = decodeBlockInsert(record, reasons);
  assert.ok(primitives);
  const primitive = primitives[0];
  if (primitive.kind !== 5) return assert.fail('expected a block primitive');
  assert.equal(primitive.block, 0xabcden);
  assert.deepEqual(primitive.origin, { x: 1234.5, y: -678.25, z: 12 });
  assert.equal(primitive.rotation, Math.PI / 6);
  assert.deepEqual(primitive.scale, { x: 0.5, y: 0.5, z: 0.5 });
});

test('the trailing TCH base-6 header is optional', () => {
  const reasons = new Map<string, number>();
  const { record } = blockInsertRecord({ scaleMode: 3, base6Tail: false });
  assert.equal(record.mainBits, blockInsertMain({ scaleMode: 3, base6Tail: false }).bits.length);
  const primitives = decodeBlockInsert(record, reasons);
  assert.ok(primitives, 'a prefix-only record still decodes');
  assert.equal(primitives.length, 1);
});

// ---------------------------------------------------------------------------
// Shared `readBitDoubleWithDefault` helper (used by the scale walk)
// ---------------------------------------------------------------------------

function readerFor(writer: BitWriter) {
  const record = makeRecord('X', 0n, writer);
  return reader(record.main, record.mainBits, 0);
}

test('readBitDoubleWithDefault patches the default bytes in the documented order', () => {
  // 01: the next 4 bytes replace the default's low word (mantissa 52..33).
  const lowWord = new BitWriter();
  lowWord.pushBits(1, 2);
  lowWord.bytes([1, 0, 0, 0]);
  assert.equal(readerFor(lowWord).readBitDoubleWithDefault(1), 1 + Number.EPSILON);

  // 10: bytes 4 and 5 come first, then bytes 0..3; bytes 6/7 stay default.
  const highWord = new BitWriter();
  highWord.pushBits(2, 2);
  highWord.bytes([1, 0, 0, 0, 0, 0]);
  assert.equal(readerFor(highWord).readBitDoubleWithDefault(1), 1 + 2 ** -20);

  // 00 keeps the default; 11 is a full raw double.
  const passthrough = new BitWriter();
  passthrough.pushBits(0, 2);
  assert.equal(readerFor(passthrough).readBitDoubleWithDefault(2.5), 2.5);
  const raw = new BitWriter();
  raw.pushBits(3, 2);
  raw.rawDouble(3.5);
  assert.equal(readerFor(raw).readBitDoubleWithDefault(2.5), 3.5);
});

// ---------------------------------------------------------------------------
// Explicit rejections
// ---------------------------------------------------------------------------

test('a zero block handle is rejected as no_block', () => {
  const reasons = new Map<string, number>();
  const { record } = blockInsertRecord({ block: 0n });
  assert.equal(decodeBlockInsert(record, reasons), null);
  assert.equal(reasons.get('vendor.tch.block_insert_no_block'), 1);
});

test('an unsupported class version is rejected before the handle stream', () => {
  const reasons = new Map<string, number>();
  const { record } = blockInsertRecord({ version: 5, block: 0xabcden });
  assert.equal(decodeBlockInsert(record, reasons), null);
  assert.equal(reasons.get('vendor.tch.block_insert_unsupported_version'), 1);
  assert.equal(reasons.get('vendor.tch.block_insert_no_block'), undefined);
});

test('a DWG version outside AC1015..AC1032 is rejected as unsupported_version', () => {
  const reasons = new Map<string, number>();
  const { record } = blockInsertRecord({ dwgVersion: 1014, block: 0xabcden });
  assert.equal(decodeBlockInsert(record, reasons), null);
  assert.equal(reasons.get('vendor.tch.block_insert_unsupported_version'), 1);
});
