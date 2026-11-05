// Contract tests for the remaining `TCH_OPENING` display: door thresholds,
// band windows and composite (group) openings.
//
// Synthetic records are built locally with the exported `BitWriter` so the test
// pins the full `ReadOpeningFields` walk instead of the short legacy
// `openingWriter` fixture (which stops after `kind`).

import assert from 'node:assert/strict';
import test from 'node:test';

import { decodeOpening, openingPrimitives } from '../../../plugins/tch/src/opening.ts';
import { parseWall } from '../../../plugins/tch/src/wall.ts';
import type { WallData } from '../../../plugins/tch/src/wall.ts';
import { BitWriter, makeRecord, wallWriter } from './vendor-plugin-fixtures.ts';

// ---------------------------------------------------------------------------
// Local synthetic builders
// ---------------------------------------------------------------------------

function writeText(writer: BitWriter, value: string): void {
  writer.bitShort(value.length);
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    writer.byte(code & 0xff);
    writer.byte((code >>> 8) & 0xff);
  }
}

function writeHandle(writer: BitWriter, value: bigint): void {
  const bytes: number[] = [];
  for (let temp = value; temp > 0n; temp >>= 8n) bytes.unshift(Number(temp & 0xffn));
  writer.byte(bytes.length);
  for (const byte of bytes) writer.byte(byte);
}

interface OpeningOptions {
  position: [number, number, number];
  width: number;
  height?: number;
  thickness?: number;
  angle?: number;
  mirror?: number;
  kind?: number;
  flags?: number;
  label?: string;
  labelHeight?: number;
  segments?: ReadonlyArray<{ x1: number; y1: number; x2: number; y2: number; bulge: number }>;
}

/** `ReadOpeningFields` (version 5), writing text slots in stream order. */
function writeOpeningFields(writer: BitWriter, text: BitWriter, options: OpeningOptions): void {
  const version = 5;
  writer.byte(version);
  writer.bitDouble(options.position[0]);
  writer.bitDouble(options.position[1]);
  writer.bitDouble(options.position[2]);
  writer.bitDouble(options.width);
  writer.bitDouble(options.thickness ?? 0.2);
  writer.bitDouble(options.height ?? 2);
  writer.bitDouble(options.angle ?? 0);
  writer.byte(options.mirror ?? 0);
  writer.byte(options.kind ?? 0);
  writer.bitDouble(0); // unused
  writeText(text, options.label ?? '');
  writer.bitDouble(0); // label x
  writer.bitDouble(0); // label y
  writer.bitDouble(0);
  const flags = options.flags ?? 0;
  writer.bitShort(flags);
  if ((flags & 1) !== 0) {
    writer.bitDouble(0);
    writer.bitDouble(0);
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
  writer.bitLong(0); // display flags
  writer.bitShort(0);
  writer.pushBit(0);
  writeText(text, ''); // version > 2 display string
  for (let index = 0; index < 4; index += 1) writer.bitDouble(0);
  writer.bitDouble(0); // version > 3
}

function writeBase6(writer: BitWriter, scale = 100): void {
  writer.byte(6);
  writer.bitDouble(scale);
  writer.bitDouble(0);
  writer.byte(0); // no base handles
  writer.bitLong(0); // empty property bag
}

function writeMainHandles(handles: BitWriter, wall: bigint, block2d: bigint): void {
  writeHandle(handles, block2d);
  writeHandle(handles, 0n); // 3D block
  writeHandle(handles, wall);
  writeHandle(handles, 0n); // style
  for (let index = 0; index < 4; index += 1) writeHandle(handles, 0n);
}

const GROUP_PART_PREFIX = '0010101101111100000000000001000011101';

function writeGroupPart(
  main: BitWriter,
  text: BitWriter,
  options: OpeningOptions,
): void {
  for (const bit of GROUP_PART_PREFIX) main.pushBit(bit === '1' ? 1 : 0);
  writeBase6(main, 130);
  writeOpeningFields(main, text, options);
}

function writePartHandles(
  handles: BitWriter,
  wall: bigint,
  block2d: bigint,
): void {
  for (let index = 0; index < 4; index += 1) writeHandle(handles, 0n);
  writeHandle(handles, block2d);
  writeHandle(handles, 0n);
  writeHandle(handles, wall);
  writeHandle(handles, 0n);
  for (let index = 0; index < 4; index += 1) writeHandle(handles, 0n);
}

function buildWall(
  reasons: Map<string, number>,
  handle: bigint,
  left: number,
  right: number,
): WallData {
  const record = makeRecord('TCH_WALL', handle, wallWriter({
    start: [0, 0, 0],
    end: [10, 0, 0],
    left,
    right,
    height: 3,
    joins: [0, 0, 0, 0],
    openings: 1,
  }));
  const wall = parseWall(record, reasons);
  assert.ok(wall, 'synthetic wall decodes');
  return wall;
}

// ---------------------------------------------------------------------------
// Door thresholds
// ---------------------------------------------------------------------------

test('a door with both leaf flags emits threshold segments on both wall faces', () => {
  const wallHandle = 0x500n;
  const reasons = new Map<string, number>();
  const wall = buildWall(reasons, wallHandle, 1, 2);
  const text = new BitWriter();
  const main = new BitWriter();
  const handles = new BitWriter();
  writeBase6(main);
  writeOpeningFields(main, text, {
    position: [5, 0, 0],
    width: 2,
    kind: 0,
    flags: 0x600,
    label: 'M1',
  });
  writeMainHandles(handles, wallHandle, 0xABCDn);
  const record = makeRecord('TCH_OPENING', 0x600n, main, { text, handle: handles });

  const opening = decodeOpening(record, reasons);
  assert.ok(opening);
  assert.equal(opening.label, 'M1');
  const primitives = openingPrimitives(opening, new Map([[wallHandle, wall]]), reasons);
  assert.equal(primitives.length, 3, 'block insert plus two threshold segments');
  assert.equal(primitives[0].kind, 5);
  const left = primitives[1];
  const right = primitives[2];
  assert.equal(left.kind, 1);
  assert.equal(right.kind, 1);
  if (left.kind !== 1 || right.kind !== 1) return;
  // Wall runs +x; left face is +y (wall.left = 1), right face is -y (wall.right = 2).
  assert.deepEqual(left.points.map(point => [point.x, point.y]), [[4, 1], [6, 1]]);
  assert.deepEqual(right.points.map(point => [point.x, point.y]), [[4, -2], [6, -2]]);
  assert.equal(reasons.get('vendor.tch.opening_thresholds'), 2);
  assert.equal(reasons.get('vendor.tch.opening_symbols'), 1);
});

test('a single leaf flag emits only its own threshold segment', () => {
  const wallHandle = 0x501n;
  const reasons = new Map<string, number>();
  const wall = buildWall(reasons, wallHandle, 1, 2);
  const text = new BitWriter();
  const main = new BitWriter();
  const handles = new BitWriter();
  writeBase6(main);
  writeOpeningFields(main, text, {
    position: [5, 0, 0], width: 2, kind: 0, flags: 0x200,
  });
  writeMainHandles(handles, wallHandle, 0xABCDn);
  const record = makeRecord('TCH_OPENING', 0x601n, main, { text, handle: handles });
  const opening = decodeOpening(record, reasons);
  assert.ok(opening);
  const primitives = openingPrimitives(opening, new Map([[wallHandle, wall]]), reasons);
  assert.equal(primitives.length, 2);
  assert.equal(primitives[1].kind, 1);
  if (primitives[1].kind !== 1) return;
  assert.deepEqual(primitives[1].points.map(point => [point.x, point.y]), [[4, 1], [6, 1]]);
  assert.equal(reasons.get('vendor.tch.opening_thresholds'), 1);
});

// ---------------------------------------------------------------------------
// Band windows
// ---------------------------------------------------------------------------

test('a band window offsets every segment and keeps concentric arcs', () => {
  const wallHandle = 0x502n;
  const reasons = new Map<string, number>();
  const wall = buildWall(reasons, wallHandle, 0.5, 0.5);
  const text = new BitWriter();
  const main = new BitWriter();
  const handles = new BitWriter();
  writeBase6(main);
  writeOpeningFields(main, text, {
    position: [5, 0, 0],
    width: 2,
    kind: 5,
    flags: 0,
    segments: [
      { x1: 4, y1: 0, x2: 5, y2: 0, bulge: 0 },
      { x1: 5, y1: 0, x2: 6, y2: 0, bulge: Math.tan(Math.PI / 8) },
    ],
  });
  writeMainHandles(handles, wallHandle, 0n);
  const record = makeRecord('TCH_OPENING', 0x602n, main, { text, handle: handles });

  const opening = decodeOpening(record, reasons);
  assert.ok(opening);
  assert.equal(opening.kind, 5);
  assert.equal(opening.segments.length, 2);
  const primitives = openingPrimitives(opening, new Map([[wallHandle, wall]]), reasons);
  assert.equal(primitives.length, 5, 'one straight line primitive plus four concentric arcs');

  const lines = primitives[0];
  assert.equal(lines.kind, 1);
  if (lines.kind !== 1) return;
  assert.equal(lines.points.length, 8, 'four offsets for the straight segment');
  // n = (0, 1); t = -0.5, -1/6, 1/6, 0.5.
  assert.deepEqual(lines.points.slice(0, 2).map(point => [point.x, point.y]), [[4, -0.5], [5, -0.5]]);
  assert.deepEqual(lines.points.slice(6, 8).map(point => [point.x, point.y]), [[4, 0.5], [5, 0.5]]);

  // A positive bulge adjusts the radius by -t and keeps the same sweep.
  const arcs = primitives.slice(1);
  assert.equal(arcs.length, 4);
  for (const arc of arcs) {
    assert.equal(arc.kind, 2);
    if (arc.kind !== 2) continue;
    assert.ok(arc.radius > 0);
    assert.ok(Math.abs(arc.sweep - Math.PI / 2) < 1e-9);
    assert.ok(Math.abs(arc.cx - 5.5) < 1e-9);
    assert.ok(Math.abs(arc.cy - 0.5) < 1e-9);
  }
  assert.equal(reasons.get('vendor.tch.opening_band_symbols'), 1);
});

test('a band window without its owning wall is rejected explicitly', () => {
  const reasons = new Map<string, number>();
  const text = new BitWriter();
  const main = new BitWriter();
  const handles = new BitWriter();
  writeBase6(main);
  writeOpeningFields(main, text, { position: [5, 0, 0], width: 2, kind: 5 });
  writeMainHandles(handles, 0x999n, 0n);
  const record = makeRecord('TCH_OPENING', 0x603n, main, { text, handle: handles });
  const opening = decodeOpening(record, reasons);
  assert.ok(opening);
  assert.deepEqual(openingPrimitives(opening, new Map(), reasons), []);
  assert.equal(reasons.get('vendor.tch.opening_band_wall_missing'), 1);
});

// ---------------------------------------------------------------------------
// Group openings
// ---------------------------------------------------------------------------

test('a group opening emits one block insert per child part', () => {
  const wallHandle = 0x503n;
  const reasons = new Map<string, number>();
  const wall = buildWall(reasons, wallHandle, 1, 2);
  const text = new BitWriter();
  const main = new BitWriter();
  const handles = new BitWriter();
  writeBase6(main);
  writeOpeningFields(main, text, {
    position: [5, 0, 0], width: 4, kind: 8, flags: 0, label: 'ZU1',
  });
  main.byte(0); // group version
  main.bitLong(2); // part count
  writeGroupPart(main, text, {
    position: [4, 0, 0], width: 1.5, kind: 0, flags: 0, label: '',
  });
  writeGroupPart(main, text, {
    position: [6, 0, 0], width: 1.5, kind: 1, flags: 0, label: '',
  });
  // Handle order: main opening (no 2D block), then each part behind four slots.
  writeMainHandles(handles, wallHandle, 0n);
  writePartHandles(handles, wallHandle, 0xD001n);
  writePartHandles(handles, wallHandle, 0xD002n);

  const record = makeRecord('TCH_OPENING', 0x604n, main, { text, handle: handles });
  const opening = decodeOpening(record, reasons);
  assert.ok(opening);
  assert.equal(opening.kind, 8);
  assert.equal(opening.parts.length, 2);
  assert.equal(opening.parts[0].kind, 0);
  assert.equal(opening.parts[1].kind, 1);
  assert.equal(opening.parts[0].wall, wallHandle);
  assert.equal(opening.parts[1].block2d, 0xD002n);
  assert.equal(opening.label, 'ZU1');

  const primitives = openingPrimitives(opening, new Map([[wallHandle, wall]]), reasons);
  assert.equal(primitives.length, 2);
  assert.equal(primitives[0].kind, 5);
  assert.equal(primitives[1].kind, 5);
  if (primitives[0].kind !== 5 || primitives[1].kind !== 5) return;
  assert.equal(primitives[0].block, 0xD001n);
  assert.equal(primitives[1].block, 0xD002n);
  assert.equal(reasons.get('vendor.tch.opening_group_parts'), 2);
  assert.equal(reasons.get('vendor.tch.opening_symbols'), 2);
  assert.equal(reasons.get('vendor.tch.opening_thresholds'), undefined);
});
