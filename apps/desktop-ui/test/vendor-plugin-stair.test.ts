// Contract tests for the base-6 stair decoders.
//
// These exercise the plugin source directly (node type stripping), building
// synthetic `TCH_LINESTAIR` / `TCH_RECTSTAIR` class bodies with the shared
// records-frame writer. The decoders only return primitives; the caller owns
// `applied` bookkeeping.

import assert from 'node:assert/strict';
import test from 'node:test';

import { decodeLineStair, decodeRectStair } from '../../../plugins/tch/src/stair.ts';
import type { Primitive } from '../../../plugins/tch/src/types.ts';
import { BitWriter, makeRecord } from './vendor-plugin-fixtures.ts';

/** Base-6 header with an empty property bag and no base handles. */
function base6(writer: BitWriter, scale = 100): void {
  writer.byte(6);
  writer.bitDouble(scale);
  writer.bitDouble(0); // second base value
  writer.byte(0); // no extra base handle counts
  writer.bitLong(0); // empty property bag
}

interface LineStairOptions {
  steps?: number;
  breakMode?: number;
  breakStart?: number;
  breakEnd?: number;
  tread?: number;
  width?: number;
  x?: number;
  y?: number;
  rotation?: number;
}

function lineStairRecord(options: LineStairOptions = {}): ReturnType<typeof makeRecord> {
  const main = new BitWriter();
  base6(main);
  main.byte(3); // flight version (reads break range and handle trailer)
  main.bitShort(options.steps ?? 4);
  main.bitDouble(150); // step height (unused by the plan builder)
  main.byte(options.breakMode ?? 1);
  main.bitDouble(options.breakStart ?? 0.4);
  main.bitDouble(options.breakEnd ?? 0.6);
  main.bitLong(0);
  main.bitDouble(0);
  main.bitDouble(0);
  main.bitDouble(0);
  main.byte(2); // line stair class version
  main.bitDouble(options.tread ?? 300);
  main.bitDouble(options.width ?? 1200);
  main.bitDouble(options.x ?? 0);
  main.bitDouble(options.y ?? 0);
  main.bitDouble(0); // z
  main.bitDouble(options.rotation ?? 0);
  main.bitDouble(0); // version > 1 unused
  return makeRecord('TCH_LINESTAIR', 0x50n, main);
}

interface RectStairOptions {
  version?: number;
  floor?: number;
  mirror?: number;
  steps1?: number;
  steps2?: number;
  scale?: number;
  tread?: number;
  flight?: number;
  landing?: number;
  total?: number;
  railWidth?: number;
  railOffset?: number;
  extTop?: number;
  extBot?: number;
  cutHeight?: number;
  cutAngle?: number;
  textHeight?: number;
  flags?: number;
}

function rectStairRecord(options: RectStairOptions = {}): ReturnType<typeof makeRecord> {
  const version = options.version ?? 4;
  const main = new BitWriter();
  base6(main, options.scale ?? 100);
  main.byte(2); // stair base version
  main.byte(version);
  main.bitDouble(0); // x
  main.bitDouble(0); // y
  main.bitDouble(0); // z
  main.bitDouble(0); // rotation
  main.bitDouble(150); // step height
  main.bitDouble(options.tread ?? 270);
  main.bitDouble(options.flight ?? 1230);
  main.bitDouble(options.landing ?? 1200);
  main.byte(options.steps1 ?? 10);
  main.byte(options.steps2 ?? 10);
  main.bitDouble(options.total ?? 2560);
  main.bitDouble(900); // rail height
  main.byte(options.floor ?? 1);
  main.byte(options.mirror ?? 0);
  main.byte(2);
  main.bitDouble(options.railWidth ?? 60);
  main.bitDouble(options.railOffset ?? 0);
  main.bitLong(options.flags ?? ((1 << 10) | (1 << 17) | (1 << 18)));
  main.bitDouble(0);
  main.bitDouble(0);
  main.bitDouble(0);
  main.bitDouble(0);
  main.bitDouble(0);
  main.byte(0);
  main.bitDouble(options.extTop ?? 60);
  main.bitDouble(options.extBot ?? 60);
  main.bitDouble(options.cutHeight ?? 675);
  main.bitDouble(options.cutAngle ?? Math.PI / 6);
  main.bitDouble(options.textHeight ?? 3.5);
  for (let index = 0; index < 4; index += 1) main.bitDouble(1200);
  if (version >= 5) main.bitShort(9); // real version-5 trailer
  return makeRecord('TCH_RECTSTAIR', 0x51n, main);
}

function ofKind(primitives: readonly Primitive[], kind: number): Primitive[] {
  return primitives.filter(primitive => primitive.kind === kind);
}

type Segment = readonly [readonly [number, number], readonly [number, number]];

/** Flatten the kind-1 segment primitive into plain pairs. */
function segmentsOf(primitives: readonly Primitive[]): Segment[] {
  const lines = primitives.find(primitive => primitive.kind === 1);
  if (!lines || lines.kind !== 1) return [];
  const segments: Segment[] = [];
  for (let index = 0; index + 1 < lines.points.length; index += 2) {
    segments.push([
      [lines.points[index].x, lines.points[index].y],
      [lines.points[index + 1].x, lines.points[index + 1].y],
    ]);
  }
  return segments;
}

function hasSegment(
  primitives: readonly Primitive[],
  from: readonly [number, number],
  to: readonly [number, number],
  tolerance = 1e-6,
): boolean {
  return segmentsOf(primitives).some(([a, b]) => (
    (Math.abs(a[0] - from[0]) <= tolerance && Math.abs(a[1] - from[1]) <= tolerance &&
      Math.abs(b[0] - to[0]) <= tolerance && Math.abs(b[1] - to[1]) <= tolerance) ||
    (Math.abs(a[0] - to[0]) <= tolerance && Math.abs(a[1] - to[1]) <= tolerance &&
      Math.abs(b[0] - from[0]) <= tolerance && Math.abs(b[1] - from[1]) <= tolerance)
  ));
}

function slantedSegments(primitives: readonly Primitive[]): Segment[] {
  return segmentsOf(primitives).filter(([a, b]) => (
    Math.abs(a[0] - b[0]) > 1e-6 && Math.abs(a[1] - b[1]) > 1e-6
  ));
}

// ---------------------------------------------------------------------------
// TCH_LINESTAIR
// ---------------------------------------------------------------------------

test('line stair decodes both flights straight outline and every tread', () => {
  const reasons = new Map<string, number>();
  const first = decodeLineStair(lineStairRecord({ steps: 4, tread: 300, width: 1200 }), reasons);
  assert.ok(first);

  // 2 side outlines + one tread line per step, all as disjoint endpoint pairs.
  assert.deepEqual(ofKind(first, 1), [{
    kind: 1,
    points: [
      { x: 0, y: 0, z: 0 }, { x: 0, y: 900, z: 0 },
      { x: 1200, y: 0, z: 0 }, { x: 1200, y: 900, z: 0 },
      { x: 0, y: 0, z: 0 }, { x: 1200, y: 0, z: 0 },
      { x: 0, y: 300, z: 0 }, { x: 1200, y: 300, z: 0 },
      { x: 0, y: 600, z: 0 }, { x: 1200, y: 600, z: 0 },
      { x: 0, y: 900, z: 0 }, { x: 1200, y: 900, z: 0 },
    ],
  }]);
  assert.equal(reasons.size, 0);

  const second = decodeLineStair(
    lineStairRecord({ steps: 5, tread: 270, width: 1000, x: 10, y: 20, rotation: Math.PI / 2 }),
    reasons,
  );
  assert.ok(second);
  const segments = segmentsOf(second);
  assert.equal(segments.length, 2 + 5);
  // At 90° local (x, y) maps to world (10 - y, 20 + x): the first tread (local
  // y = 0, x = 0..1000) becomes the world segment from (10, 20) to (10, 1020).
  assert.ok(hasSegment(second, [10, 20], [10, 1020]));
  assert.equal(reasons.size, 0);
});

test('line stair draws the break symbol and clips the flight outline', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeLineStair(lineStairRecord({
    steps: 10,
    tread: 300,
    width: 1200,
    breakMode: 4, // BreakKind 3: single upper break
    breakStart: 0.4,
    breakEnd: 0.6,
  }), reasons);
  assert.ok(primitives);

  // The straight model has only axis-aligned segments; the break zigzag adds
  // slanted segments and removes the lower part of one flight edge.
  assert.ok(slantedSegments(primitives).length >= 3);
  assert.equal(hasSegment(primitives, [1200, 0], [1200, 2700]), false);
  assert.equal(reasons.size, 0);
});

// ---------------------------------------------------------------------------
// TCH_RECTSTAIR
// ---------------------------------------------------------------------------

test('rect stair decodes landing, flight, tread and rail lines with cut break', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeRectStair(rectStairRecord({ floor: 1 }), reasons);
  assert.ok(primitives);

  // Landing rectangle and both flight outer edges.
  assert.ok(hasSegment(primitives, [0, 0], [2560, 0]));
  assert.ok(hasSegment(primitives, [0, -1200], [2560, -1200]));
  assert.ok(hasSegment(primitives, [0, 0], [0, -1200]));
  assert.ok(hasSegment(primitives, [2560, 0], [2560, -1200]));
  const y0 = -1200 - 9 * 270; // -3630
  // The second flight is drawn whole; the first flight is cut by the break.
  assert.ok(hasSegment(primitives, [2560, y0], [2560, y0 + 9 * 270]));
  assert.ok(segmentsOf(primitives).some(([a, b]) => Math.abs(a[0]) < 1e-6 && Math.abs(b[0]) < 1e-6));

  // Mid-flight tread line at the start line, and the stair well bottom.
  assert.ok(hasSegment(primitives, [1230, y0], [1330, y0], 1e-6) ||
    hasSegment(primitives, [1170, y0], [1390, y0], 1e-6));
  const innerBot = Math.max(y0, y0 - 60 + 60);
  assert.ok(hasSegment(primitives, [1230, innerBot], [1330, innerBot], 1e-6));

  // Intermediate floor: linked rail rectangles at xl=1230, xr=1330.
  assert.ok(hasSegment(primitives, [1170, y0 - 60], [1170, -1140]));
  assert.ok(hasSegment(primitives, [1390, y0 - 60], [1390, -1140]));
  assert.ok(hasSegment(primitives, [1170, -1140], [1390, -1140]));

  // Double cut break adds slanted zigzag lines around yc = y0 + 1215.
  const slanted = slantedSegments(primitives);
  assert.ok(slanted.length >= 3);
  assert.ok(slanted.some(([a, b]) => Math.abs((a[1] + b[1]) / 2 + 2415) < 400));
  assert.equal(reasons.get('vendor.tch.arrow_outline_only'), 2);
});

test('rect stair emits 上/下 arrows, outline heads and rotated text when enabled', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeRectStair(rectStairRecord({ floor: 1 }), reasons);
  assert.ok(primitives);

  const texts = ofKind(primitives, 4);
  assert.equal(texts.length, 2);
  assert.deepEqual(texts.map(item => item.kind === 4 ? item.text : null).sort(), ['上', '下']);
  for (const item of texts) {
    assert.equal(item.kind, 4);
    if (item.kind !== 4) continue;
    assert.equal(item.attachment, 4);
    assert.equal(item.rotation, 0);
    assert.ok(Math.abs(item.height - 0.96454 * 3.5 * 100) < 1e-9);
  }

  const outlines = ofKind(primitives, 0).filter(item => (
    item.kind === 0 &&
    item.points.length === 4 &&
    item.points[0].x === item.points[3].x &&
    item.points[0].y === item.points[3].y
  ));
  assert.equal(outlines.length, 2);
  for (const outline of outlines) {
    assert.equal(outline.kind, 0);
    if (outline.kind !== 0) continue;
    assert.deepEqual(outline.points[0], outline.points[outline.points.length - 1]);
  }
  assert.equal(reasons.get('vendor.tch.arrow_outline_only'), 2);

  // The 下 arrow bends through the landing centre (local y = -L/2).
  const shafts = ofKind(primitives, 0).filter(item => item.kind === 0 && !(
    item.points.length === 4 &&
    item.points[0].x === item.points[3].x &&
    item.points[0].y === item.points[3].y
  ));
  assert.ok(shafts.some(item => item.kind === 0 && item.points.some(point => Math.abs(point.y + 600) < 1e-9)));
});

test('rect stair honours ShowArrows=false, ShowLanding=false and floor modes', () => {
  // Intermediate floor without arrows or landing: only the flight/rail geometry.
  const reasons = new Map<string, number>();
  const hidden = decodeRectStair(rectStairRecord({ floor: 1, flags: 1 << 17 }), reasons);
  assert.ok(hidden);
  assert.equal(ofKind(hidden, 4).length, 0);
  assert.equal(ofKind(hidden, 0).length, 0);
  assert.equal(hasSegment(hidden, [0, 0], [2560, 0]), false);
  assert.equal(reasons.size, 0);

  // Ground floor: second flight is absent, only the 上 arrow is shown.
  const ground = decodeRectStair(rectStairRecord({ floor: 0 }), new Map<string, number>());
  assert.ok(ground);
  assert.equal(hasSegment(ground, [0, 0], [2560, 0]), false);
  assert.equal(segmentsOf(ground).some(([a, b]) => Math.abs(a[0] - 2560) < 1e-6 && Math.abs(b[0] - 2560) < 1e-6), false);
  assert.ok(segmentsOf(ground).some(([a, b]) => Math.abs(a[0]) < 1e-6 && Math.abs(b[0]) < 1e-6));
  const groundTexts = ofKind(ground, 4);
  assert.equal(groundTexts.length, 1);
  assert.equal(groundTexts[0].kind === 4 ? groundTexts[0].text : null, '上');

  // Top floor: no cut break, no 上 arrow, only the 下 arrow.
  const top = decodeRectStair(rectStairRecord({ floor: 2 }), new Map<string, number>());
  assert.ok(top);
  assert.equal(slantedSegments(top).length, 0);
  const topTexts = ofKind(top, 4);
  assert.equal(topTexts.length, 1);
  assert.equal(topTexts[0].kind === 4 ? topTexts[0].text : null, '下');
  assert.ok(hasSegment(top, [0, 0], [2560, 0]));
});

test('rect stair accepts the real version-5 trailing scalar', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeRectStair(rectStairRecord({ version: 5, floor: 1, flags: 1 << 17 }), reasons);
  assert.ok(primitives);
  assert.ok(segmentsOf(primitives).length > 0);
  assert.equal(reasons.size, 0);
});

test('unsupported stair class versions bump one stable reason and return null', () => {
  const lineMain = new BitWriter();
  base6(lineMain);
  lineMain.byte(3);
  lineMain.bitShort(4);
  lineMain.bitDouble(150);
  lineMain.byte(1);
  lineMain.bitDouble(0.4);
  lineMain.bitDouble(0.6);
  lineMain.bitLong(0);
  lineMain.bitDouble(0);
  lineMain.bitDouble(0);
  lineMain.bitDouble(0);
  lineMain.byte(9); // out-of-range line stair version
  const lineReasons = new Map<string, number>();
  assert.equal(
    decodeLineStair(makeRecord('TCH_LINESTAIR', 0x52n, lineMain), lineReasons),
    null,
  );
  assert.equal(lineReasons.get('vendor.tch.linestair_unsupported_version'), 1);

  const rectReasons = new Map<string, number>();
  assert.equal(
    decodeRectStair(rectStairRecord({ version: 6 }), rectReasons),
    null,
  );
  assert.equal(rectReasons.get('vendor.tch.rectstair_unsupported_version'), 1);
});
