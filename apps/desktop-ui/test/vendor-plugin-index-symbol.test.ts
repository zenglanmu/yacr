// Contract tests for the base-6 section / index-pointer / corner-window decoders.
//
// These exercise the plugin source directly (node type stripping), building
// synthetic class bodies with the shared records-frame writer. The decoders
// only return primitives; the caller owns `applied` bookkeeping.

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  decodeCornerWindow, decodeIndexPointer, decodeSection,
} from '../../../plugins/tch/src/index-symbol.ts';
import { BitWriter, makeRecord } from './vendor-plugin-fixtures.ts';

function plainBase(writer: BitWriter, scale: number): void {
  writer.byte(6);
  writer.bitDouble(scale);
  writer.bitDouble(0); // second base value
  writer.byte(0); // no base handles
  writer.bitLong(0); // empty property bag
}

function symbolBase(writer: BitWriter, scale: number, textHeight: number): void {
  plainBase(writer, scale);
  writer.byte(1); // symbol version
  writer.bitDouble(textHeight);
}

function writeText(writer: BitWriter, value: string): void {
  writer.bitShort(value.length);
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    writer.byte(code & 0xff);
    writer.byte((code >>> 8) & 0xff);
  }
}

function close(actual: number, expected: number, epsilon = 1e-6): void {
  assert.ok(
    Math.abs(actual - expected) <= epsilon,
    `expected ${actual} to be within ${epsilon} of ${expected}`,
  );
}

function closePoint(actual: { x: number; y: number }, expected: readonly [number, number]): void {
  close(actual.x, expected[0]);
  close(actual.y, expected[1]);
}

function sectionRecord(options: {
  version?: number;
  kind?: number;
  viewAngle?: number;
  endView?: number;
  indexFlags?: number;
  points: readonly (readonly [number, number])[];
  label?: string;
  label2?: string;
  scale?: number;
  textHeight?: number;
}): ReturnType<typeof makeRecord> {
  const version = options.version ?? 2;
  const main = new BitWriter();
  const text = new BitWriter();
  symbolBase(main, options.scale ?? 100, options.textHeight ?? 5);
  main.byte(version);
  main.byte(options.kind ?? 1);
  main.bitDouble(options.viewAngle ?? 0);
  main.byte(options.points.length);
  for (const [x, y] of options.points) {
    main.rawDouble(x);
    main.rawDouble(y);
  }
  writeText(text, options.label ?? 'A');
  for (let index = 0; index < 4; index += 1) main.rawDouble(0);
  if (version > 1) {
    writeText(text, options.label2 ?? '');
    for (let index = 0; index < 4; index += 1) main.rawDouble(0);
    main.bitDouble(0);
    main.bitShort(options.indexFlags ?? 0);
    main.bitDouble(0);
    main.bitDouble(options.endView ?? 0);
  }
  return makeRecord('TCH_SYMB_SECTION', 0x100n, main, { text });
}

function indexPointerRecord(options: {
  version?: number;
  kind?: number;
  point?: readonly [number, number];
  corner?: readonly [number, number];
  radius?: number;
  run?: number;
  mark?: number;
  list1?: number;
  list2?: number;
  number?: string;
  sheet?: string;
  above?: string;
  below?: string;
  scale?: number;
  textHeight?: number;
}): ReturnType<typeof makeRecord> {
  const main = new BitWriter();
  const text = new BitWriter();
  symbolBase(main, options.scale ?? 100, options.textHeight ?? 3.5);
  main.byte(options.version ?? 3);
  main.byte(options.kind ?? 0);
  const [px, py] = options.point ?? [0, 0];
  const [cx, cy] = options.corner ?? [100, 0];
  main.rawDouble(px);
  main.rawDouble(py);
  main.rawDouble(cx);
  main.rawDouble(cy);
  main.bitDouble(options.radius ?? 5);
  main.bitDouble(options.run ?? 10);
  main.bitDouble(options.mark ?? 20);
  const list1 = options.list1 ?? 0;
  const list2 = options.list2 ?? 0;
  main.byte(list1);
  main.byte(list2);
  main.bitDouble(0);
  main.bitDouble(0);
  main.byte(0);
  main.byte(0); // polyline closed flag
  main.bitShort(0); // no vertices
  main.bitDouble(0);
  writeText(text, options.number ?? '1');
  writeText(text, options.sheet ?? '2');
  writeText(text, options.above ?? 'A');
  writeText(text, options.below ?? 'B');
  for (let index = 0; index < list1 + list2 + 1; index += 1) writeText(text, '');
  return makeRecord('TCH_INDEXPOINTER', 0x200n, main, { text });
}

function cornerWindowRecord(options: {
  version?: number;
  flags?: number;
  point?: readonly [number, number, number];
  length1?: number;
  length2?: number;
  angle1?: number;
  angle2?: number;
  thickness?: number;
  thickness2?: number;
  labelX?: number;
  labelY?: number;
  labelHeight?: number;
  label?: string;
  scale?: number;
}): ReturnType<typeof makeRecord> {
  const version = options.version ?? 1;
  const flags = options.flags ?? 0;
  const main = new BitWriter();
  const text = new BitWriter();
  plainBase(main, options.scale ?? 100);
  main.byte(version);
  main.bitLong(flags);
  const [x, y, z] = options.point ?? [0, 0, 0];
  main.bitDouble(x);
  main.bitDouble(y);
  main.bitDouble(z);
  main.bitDouble(options.length1 ?? 3000);
  main.bitDouble(options.length2 ?? 2000);
  main.bitDouble(0);
  main.bitDouble(options.angle1 ?? 0);
  main.bitDouble(options.angle2 ?? Math.PI / 2);
  main.bitDouble(options.thickness ?? 200);
  main.bitDouble(options.thickness2 ?? -200);
  main.bitDouble(0);
  main.bitDouble(0);
  main.rawDouble(options.labelX ?? 10);
  main.rawDouble(options.labelY ?? 20);
  main.bitDouble(0);
  main.bitDouble(options.labelHeight ?? 3.5);
  if ((flags & 1) !== 0) {
    for (let index = 0; index < 4; index += 1) main.bitDouble(0);
  }
  main.bitShort(0);
  for (let index = 0; index < 3; index += 1) main.bitDouble(0);
  if ((flags & 4) !== 0) {
    main.bitShort(1);
    main.rawDouble(0);
    main.rawDouble(0);
    main.rawDouble(1);
    main.rawDouble(1);
    main.bitDouble(0);
  }
  if (version > 1) {
    main.bitShort(0);
    main.pushBit(0);
    for (let index = 0; index < 4; index += 1) main.bitDouble(0);
  }
  if (version > 2) main.bitDouble(0);
  if (version > 3 && (flags & 1) !== 0) main.bitDouble(0);
  writeText(text, options.label ?? 'W');
  return makeRecord('TCH_CORNER_WINDOW', 0x300n, main, { text });
}

// ---------------------------------------------------------------------------
// TCH_SYMB_SECTION
// ---------------------------------------------------------------------------

test('section decodes the classic cut line and both end labels', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeSection(
    sectionRecord({ points: [[0, 0], [100, 0]], label: 'A', label2: 'B' }),
    reasons,
  );
  assert.ok(primitives);
  assert.equal(primitives.length, 4);
  const [first, last, firstText, lastText] = primitives;
  assert.equal(first.kind, 0);
  assert.equal(last.kind, 0);
  if (first.kind !== 0 || last.kind !== 0) throw new Error('unreachable');
  // stroke = 10×scale, leg = 6×scale for a section (kind 1).
  assert.deepEqual(first.points.map(p => [p.x, p.y]), [[600, 0], [0, 0], [100, 0]]);
  assert.deepEqual(last.points.map(p => [p.x, p.y]), [[0, 0], [100, 0], [700, 0]]);
  for (const [item, x] of [[firstText, 1100], [lastText, 1200]] as const) {
    assert.equal(item.kind, 4);
    if (item.kind !== 4) throw new Error('unreachable');
    assert.equal(item.text, 'A');
    closePoint(item.origin, [x, 0]);
    close(item.height, 0.85 * 5 * 100);
    assert.equal(item.rotation, 0);
    assert.equal(item.attachment, 5);
  }
  assert.equal(reasons.get('vendor.tch.section_style_default'), 1);
});

test('section follows endViewAngle only when index flag 8 is set', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeSection(
    sectionRecord({
      points: [[0, 0], [100, 0]],
      viewAngle: 0,
      endView: Math.PI / 2,
      indexFlags: 8,
    }),
    reasons,
  );
  assert.ok(primitives);
  const last = primitives[1];
  assert.equal(last.kind, 0);
  if (last.kind !== 0) throw new Error('unreachable');
  // The end leg points along +Y instead of +X.
  closePoint(last.points[2], [100, 600]);
});

test('section version 1 uses the classic layout without end-view fields', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeSection(
    sectionRecord({ version: 1, points: [[0, 0], [100, 0]] }),
    reasons,
  );
  assert.ok(primitives);
  assert.equal(primitives.length, 4);
  assert.equal(reasons.size, 0);
});

test('section rejects an unsupported class-body version with one reason', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeSection(sectionRecord({ version: 3, points: [[0, 0], [10, 0]] }), reasons);
  assert.equal(primitives, null);
  assert.equal(reasons.get('vendor.tch.section_unsupported_version'), 1);
});

// ---------------------------------------------------------------------------
// TCH_INDEXPOINTER
// ---------------------------------------------------------------------------

test('index pointer kind 0 draws the point circle, leader, ring and notes', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeIndexPointer(indexPointerRecord({}), reasons);
  assert.ok(primitives);
  assert.deepEqual(primitives.map(p => p.kind), [3, 1, 3, 1, 4, 4, 4, 4]);

  const [pointCircle, leader, ring, diameter, ...texts] = primitives;
  assert.equal(pointCircle.kind, 3);
  if (pointCircle.kind !== 3) throw new Error('unreachable');
  closePoint({ x: pointCircle.cx, y: pointCircle.cy }, [0, 0]);
  close(pointCircle.radius, 10);

  assert.equal(leader.kind, 1);
  if (leader.kind !== 1) throw new Error('unreachable');
  assert.equal(leader.points.length, 4);
  closePoint(leader.points[0], [10, 0]);
  closePoint(leader.points[1], [100, 0]);
  closePoint(leader.points[2], [100, 0]);
  closePoint(leader.points[3], [1100, 0]);

  assert.equal(ring.kind, 3);
  if (ring.kind !== 3) throw new Error('unreachable');
  closePoint({ x: ring.cx, y: ring.cy }, [1600, 0]);
  close(ring.radius, 500);

  assert.equal(diameter.kind, 1);
  if (diameter.kind !== 1) throw new Error('unreachable');
  closePoint(diameter.points[0], [2100, 0]);
  closePoint(diameter.points[1], [1100, 0]);

  const [number, sheet, above, below] = texts;
  for (const [item, y, value] of [
    [number, 225, '1'],
    [sheet, -225, '2'],
  ] as const) {
    assert.equal(item.kind, 4);
    if (item.kind !== 4) throw new Error('unreachable');
    assert.equal(item.text, value);
    closePoint(item.origin, [1600, y]);
    close(item.height, 0.535 * 500);
    assert.equal(item.attachment, 5);
  }
  for (const [item, y, value] of [
    [above, 105 + 0.85 * 350 / 2, 'A'],
    [below, -(105 + 0.85 * 350 / 2), 'B'],
  ] as const) {
    assert.equal(item.kind, 4);
    if (item.kind !== 4) throw new Error('unreachable');
    assert.equal(item.text, value);
    closePoint(item.origin, [995, y]);
    close(item.height, 0.85 * 350);
    assert.equal(item.attachment, 6);
    assert.equal(item.wrapWidth, 1);
  }
  assert.equal(reasons.size, 0);
});

test('index pointer kind 1 draws the thick tick instead of a point circle', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeIndexPointer(indexPointerRecord({ kind: 1 }), reasons);
  assert.ok(primitives);
  const tick = primitives.find(p => p.kind === 0);
  assert.ok(tick);
  if (tick?.kind !== 0) throw new Error('unreachable');
  assert.deepEqual(tick.points.map(p => [p.x, p.y]), [[0, -150], [2000, -150]]);
  const circles = primitives.filter(p => p.kind === 3);
  assert.equal(circles.length, 1);
});

test('index pointer rejects an unsupported class-body version with one reason', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeIndexPointer(indexPointerRecord({ version: 5 }), reasons);
  assert.equal(primitives, null);
  assert.equal(reasons.get('vendor.tch.indexpointer_unsupported_version'), 1);
});

// ---------------------------------------------------------------------------
// TCH_CORNER_WINDOW
// ---------------------------------------------------------------------------

test('corner window builds four transverse lines, two sides and the label', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeCornerWindow(cornerWindowRecord({}), reasons);
  assert.ok(primitives);
  assert.deepEqual(primitives.map(p => p.kind), [0, 0, 0, 0, 1, 4]);

  const expected: readonly (readonly (readonly [number, number])[])[] = [
    [[3000, 0], [0, 0], [0, 2000]],
    [[3000, -200 / 3], [-200 / 3, -200 / 3], [-200 / 3, 2000]],
    [[3000, -400 / 3], [-400 / 3, -400 / 3], [-400 / 3, 2000]],
    [[3000, -200], [-200, -200], [-200, 2000]],
  ];
  for (let index = 0; index < 4; index += 1) {
    const path = primitives[index];
    assert.equal(path.kind, 0);
    if (path.kind !== 0) throw new Error('unreachable');
    assert.equal(path.points.length, 3);
    for (let vertex = 0; vertex < 3; vertex += 1) {
      closePoint(path.points[vertex], expected[index][vertex]);
    }
  }

  const sides = primitives[4];
  assert.equal(sides.kind, 1);
  if (sides.kind !== 1) throw new Error('unreachable');
  assert.equal(sides.points.length, 4);
  closePoint(sides.points[0], [3000, 0]);
  closePoint(sides.points[1], [3000, -200]);
  closePoint(sides.points[2], [0, 2000]);
  closePoint(sides.points[3], [-200, 2000]);

  const label = primitives[5];
  assert.equal(label.kind, 4);
  if (label.kind !== 4) throw new Error('unreachable');
  assert.equal(label.text, 'W');
  closePoint(label.origin, [10, 20 - 0.425 * 3.5 * 100]);
  close(label.height, 0.85 * 3.5 * 100);
  assert.equal(label.attachment, 8);
  assert.equal(reasons.size, 0);
});

test('corner window version 4 walks the optional flag blocks', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeCornerWindow(
    cornerWindowRecord({ version: 4, flags: 1 | 4 }),
    reasons,
  );
  assert.ok(primitives);
  assert.equal(primitives.length, 6);
  assert.equal(reasons.size, 0);
});

test('corner window rejects an unsupported class-body version with one reason', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeCornerWindow(cornerWindowRecord({ version: 5 }), reasons);
  assert.equal(primitives, null);
  assert.equal(reasons.get('vendor.tch.cornerwindow_unsupported_version'), 1);
});
