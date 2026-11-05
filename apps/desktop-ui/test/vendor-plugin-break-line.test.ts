// Contract tests for the `TCH_RUPTURE` / `TCH_MODI` base-6 decoders.
//
// Synthetic records are built with the fixture `BitWriter` / `makeRecord`
// helpers. The real fixture records are all body version 1 (rupture) / 1
// (modi); unsupported versions must return `null` with one stable reason.

import assert from 'node:assert/strict';
import test from 'node:test';

import { decodeModi, decodeRupture } from '../../../plugins/tch/src/break-line.ts';
import type { VendorRecord } from '../../../plugins/tch/src/types.ts';
import { BitWriter, makeRecord } from './vendor-plugin-fixtures.ts';

function writeText(writer: BitWriter, value: string): void {
  writer.bitShort(value.length);
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    writer.byte(code & 0xff);
    writer.byte((code >>> 8) & 0xff);
  }
}

/** Base-6 header: scale, ignored second value, no base handles, empty bag. */
function base6(writer: BitWriter, scale: number): void {
  writer.byte(6);
  writer.bitDouble(scale);
  writer.bitDouble(0);
  writer.byte(0);
  writer.bitLong(0);
}

function ruptureRecord(options: {
  scale?: number;
  version?: number;
  key?: number;
  x1: number; y1: number; x2: number; y2: number;
  count: number;
}): VendorRecord {
  const writer = new BitWriter();
  base6(writer, options.scale ?? 1);
  writer.byte(options.version ?? 1);
  if ((options.version ?? 1) === 2) writer.byte(options.key ?? 0);
  writer.rawDouble(options.x1);
  writer.rawDouble(options.y1);
  writer.rawDouble(options.x2);
  writer.rawDouble(options.y2);
  writer.byte(options.count);
  return makeRecord('TCH_RUPTURE', 0x100n, writer) as VendorRecord;
}

function modiRecord(options: {
  scale?: number;
  textHeight?: number;
  version?: number;
  closed?: boolean;
  points: readonly { x: number; y: number }[];
  bulge: number;
}): VendorRecord {
  const text = new BitWriter();
  const writer = new BitWriter();
  base6(writer, options.scale ?? 100);
  // ReadTchSymbolBase: version 1 (no extra handles/values).
  writer.byte(1);
  writer.bitDouble(options.textHeight ?? 2.5);
  writer.byte(options.version ?? 1);
  writer.byte(options.closed ? 1 : 0);
  writer.bitShort(options.points.length);
  for (const point of options.points) {
    writer.rawDouble(point.x);
    writer.rawDouble(point.y);
    writer.bitDouble(0); // per-vertex bulge is read and ignored
  }
  writer.bitDouble(0); // arc length factor
  writer.bitDouble(options.bulge);
  writer.byte(0);
  writeText(text, '1'); // 编号 (not drawn)
  writer.bitDouble(0);
  writer.bitDouble(0);
  writer.bitDouble(0);
  return makeRecord('TCH_MODI', 0x200n, writer, { text }) as VendorRecord;
}

function near(actual: number, expected: number, message?: string): void {
  assert.ok(Math.abs(actual - expected) < 1e-9, message ?? `${actual} != ${expected}`);
}

const TAN70 = Math.tan(70 * Math.PI / 180);
const TAN80 = Math.tan(80 * Math.PI / 180);

test('TCH_RUPTURE: two repetitions on a 100-unit line emit two 6-point polylines', () => {
  const record = ruptureRecord({ scale: 1, x1: 0, y1: 0, x2: 100, y2: 0, count: 2 });
  const reasons = new Map<string, number>();
  const primitives = decodeRupture(record, reasons);
  assert.deepEqual([...reasons], []);
  assert.ok(primitives);
  assert.equal(primitives.length, 2);

  const h = 2.1;
  const len = 50;
  const a = len / 2 - 1.125;
  const peak = a + h / TAN70;
  const valley = peak + 2 * h / TAN80;
  for (const [index, primitive] of primitives.entries()) {
    assert.equal(primitive.kind, 0);
    assert.equal(primitive.points.length, 6);
    const base = index * len;
    near(primitive.points[0].x, base);
    near(primitive.points[0].y, 0);
    near(primitive.points[1].x, base + a);
    near(primitive.points[1].y, 0);
    near(primitive.points[2].x, base + peak);
    near(primitive.points[2].y, h);
    near(primitive.points[3].x, base + valley);
    near(primitive.points[3].y, -h);
    near(primitive.points[4].x, base + len / 2 + 1.125);
    near(primitive.points[4].y, 0);
    near(primitive.points[5].x, base + len);
    near(primitive.points[5].y, 0);
  }
});

test('TCH_RUPTURE: short spacing falls back to one straight segment', () => {
  const record = ruptureRecord({ scale: 100, x1: 0, y1: 0, x2: 10, y2: 0, count: 2 });
  const reasons = new Map<string, number>();
  const primitives = decodeRupture(record, reasons);
  assert.deepEqual([...reasons], []);
  assert.ok(primitives);
  assert.equal(primitives.length, 2);
  for (const [index, primitive] of primitives.entries()) {
    assert.equal(primitive.kind, 0);
    assert.equal(primitive.points.length, 2);
    near(primitive.points[0].x, index * 5);
    near(primitive.points[1].x, (index + 1) * 5);
  }
});

test('TCH_MODI: open 3-vertex cloud with bulge 0.5 emits two arcs', () => {
  const record = modiRecord({
    closed: false,
    points: [{ x: 0, y: 0 }, { x: 10, y: 0 }, { x: 10, y: 10 }],
    bulge: 0.5,
  });
  const reasons = new Map<string, number>();
  const primitives = decodeModi(record, reasons);
  assert.deepEqual([...reasons], []);
  assert.ok(primitives);
  assert.equal(primitives.length, 2);
  const sweep = 4 * Math.atan(0.5);
  for (const primitive of primitives) {
    assert.equal(primitive.kind, 2);
    assert.equal(primitive.kind === 2 ? primitive.sweep : 0, sweep);
    assert.equal(primitive.kind === 2 ? primitive.radius : 0, 10 / 2 / Math.sin(sweep / 2));
  }
});

test('TCH_MODI: zero bulge emits one kind-0 straight run', () => {
  const record = modiRecord({
    closed: false,
    points: [{ x: 0, y: 0 }, { x: 10, y: 0 }, { x: 10, y: 10 }],
    bulge: 0,
  });
  const reasons = new Map<string, number>();
  const primitives = decodeModi(record, reasons);
  assert.deepEqual([...reasons], []);
  assert.ok(primitives);
  assert.equal(primitives.length, 1);
  const [primitive] = primitives;
  assert.equal(primitive.kind, 0);
  assert.equal(primitive.points.length, 3);
  assert.deepEqual(primitive.points.map(p => [p.x, p.y]), [[0, 0], [10, 0], [10, 10]]);
});

test('TCH_MODI: closed flag appends the first point and zeroes the last bulge', () => {
  const straight = decodeModi(modiRecord({
    closed: true,
    points: [{ x: 0, y: 0 }, { x: 10, y: 0 }, { x: 10, y: 10 }],
    bulge: 0,
  }), new Map<string, number>());
  assert.ok(straight);
  assert.equal(straight.length, 1);
  assert.equal(straight[0].kind, 0);
  assert.equal(straight[0].points.length, 4, 'closing point is appended');
  assert.deepEqual(
    [straight[0].points[3].x, straight[0].points[3].y],
    [straight[0].points[0].x, straight[0].points[0].y],
  );

  const bulged = decodeModi(modiRecord({
    closed: true,
    points: [{ x: 0, y: 0 }, { x: 10, y: 0 }, { x: 10, y: 10 }],
    bulge: 0.5,
  }), new Map<string, number>());
  assert.ok(bulged);
  assert.equal(bulged.length, 3, 'closing segment is drawn as an arc');
  assert.ok(bulged.every(primitive => primitive.kind === 2));
});

test('TCH_RUPTURE: unsupported body version returns null with one reason', () => {
  const record = ruptureRecord({ x1: 0, y1: 0, x2: 100, y2: 0, count: 1, version: 3 });
  const reasons = new Map<string, number>();
  assert.equal(decodeRupture(record, reasons), null);
  assert.deepEqual([...reasons], [['vendor.tch.rupture_unsupported_version', 1]]);
});

test('TCH_MODI: unsupported body version returns null with one reason', () => {
  const record = modiRecord({
    version: 2,
    closed: true,
    points: [{ x: 0, y: 0 }, { x: 10, y: 0 }, { x: 10, y: 10 }],
    bulge: 0.5,
  });
  const reasons = new Map<string, number>();
  assert.equal(decodeModi(record, reasons), null);
  assert.deepEqual([...reasons], [['vendor.tch.modi_unsupported_version', 1]]);
});
