// Contract tests for the base-6 annotation decoders.
//
// These exercise the plugin source directly (node type stripping), building
// synthetic `TCH_ELEVATION` / `TCH_ARROW` / `TCH_MULTILEADER` class bodies with
// the shared records-frame writer. The decoders only return primitives; the
// caller owns `applied` bookkeeping.

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  decodeArrow, decodeElevation, decodeMultiLeader,
} from '../../../plugins/tch/src/annotation.ts';
import type { Primitive } from '../../../plugins/tch/src/types.ts';
import { BitWriter, makeRecord } from './vendor-plugin-fixtures.ts';

function symbolBase(writer: BitWriter, scale: number, textHeight: number): void {
  writer.byte(6);
  writer.bitDouble(scale);
  writer.bitDouble(0); // second base value
  writer.byte(0); // no extra base handles
  writer.bitLong(0); // empty property bag
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

function elevationRecord(options: {
  scale?: number;
  textHeight?: number;
  flags?: number;
  x?: number;
  y?: number;
  value?: string;
  lineLength?: number;
}): ReturnType<typeof makeRecord> {
  const main = new BitWriter();
  const text = new BitWriter();
  symbolBase(main, options.scale ?? 100, options.textHeight ?? 3.5);
  main.byte(4); // elevation version
  main.byte(options.flags ?? 0);
  main.rawDouble(options.x ?? 0);
  main.rawDouble(options.y ?? 0);
  writeText(text, options.value ?? '2.00000');
  main.bitDouble(options.lineLength ?? 9.66);
  main.bitDouble(0); // version > 1 unused
  main.byte(0);
  main.rawDouble(0);
  main.rawDouble(0);
  main.rawDouble(0);
  main.rawDouble(0);
  writeText(text, '');
  main.byte(0); // version > 3
  return makeRecord('TCH_ELEVATION', 0x10n, main, { text });
}

function arrowRecord(options: {
  mode: number;
  vertices: readonly { x: number; y: number; bulge: number }[];
  arrowLength?: number;
  head?: number;
  text1?: string;
  text2?: string;
  hasText2?: boolean;
  closed?: boolean;
}): ReturnType<typeof makeRecord> {
  const main = new BitWriter();
  const text = new BitWriter();
  symbolBase(main, 100, 3.5);
  main.byte(5); // arrow version
  main.byte(options.mode);
  writeText(text, options.text1 ?? '');
  main.bitDouble(0);
  main.bitDouble(0);
  main.bitDouble(0);
  main.byte(options.closed ? 1 : 0);
  main.bitShort(options.vertices.length);
  for (const vertex of options.vertices) {
    main.rawDouble(vertex.x);
    main.rawDouble(vertex.y);
    main.bitDouble(vertex.bulge);
  }
  main.bitDouble(options.arrowLength ?? 3);
  main.byte(options.head ?? 1);
  main.bitDouble(0);
  main.pushBit(options.hasText2 ? 1 : 0);
  writeText(text, options.text2 ?? '');
  main.bitDouble(0);
  main.bitDouble(0);
  main.bitDouble(0);
  return makeRecord('TCH_ARROW', 0x20n, main, { text });
}

function multiLeaderRecord(options: {
  arrow?: number;
  elbow?: readonly [number, number];
  lineLength?: number;
  side?: number;
  targets: readonly (readonly [number, number])[];
  upper?: string;
  lower?: string;
  arrowSize?: number;
  gap?: number;
  align?: number;
}): ReturnType<typeof makeRecord> {
  const main = new BitWriter();
  const text = new BitWriter();
  symbolBase(main, 100, 3.5);
  main.byte(5); // multileader version
  main.byte(options.arrow ?? 2);
  main.rawDouble(options.elbow?.[0] ?? 100);
  main.rawDouble(options.elbow?.[1] ?? 200);
  main.bitDouble(options.lineLength ?? 10);
  main.byte(options.side ?? 0);
  writeText(text, options.upper ?? '');
  writeText(text, options.lower ?? '');
  main.byte(options.targets.length);
  for (const [x, y] of options.targets) {
    main.rawDouble(x);
    main.rawDouble(y);
  }
  main.bitDouble(options.arrowSize ?? 3);
  main.bitDouble(options.gap ?? -1000);
  main.byte(options.align ?? 3);
  main.bitDouble(0);
  main.byte(0);
  main.rawDouble(0);
  main.rawDouble(0);
  for (let index = 0; index < 6; index += 1) main.bitDouble(0);
  return makeRecord('TCH_MULTILEADER', 0x30n, main, { text });
}

function ofKind(primitives: readonly Primitive[], kind: number): Primitive[] {
  return primitives.filter(primitive => primitive.kind === kind);
}

test('elevation decodes tip lines, applied scale and baseline text', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeElevation(
    elevationRecord({ flags: 0, x: 10, y: 20, value: '2.01500', lineLength: 9.66 }),
    reasons,
  );
  assert.ok(primitives);

  const lines = ofKind(primitives, 1);
  assert.equal(lines.length, 1);
  assert.deepEqual(lines[0], {
    kind: 1,
    points: [
      { x: 10, y: 20, z: 0 },
      { x: -290, y: 320, z: 0 },
      { x: 10, y: 20, z: 0 },
      { x: 310, y: 320, z: 0 },
      { x: -290, y: 320, z: 0 },
      { x: 976, y: 320, z: 0 }, // x + lineLength * scale
    ],
  });

  const texts = ofKind(primitives, 4);
  assert.equal(texts.length, 1);
  assert.deepEqual(texts[0], {
    kind: 4,
    origin: { x: 946, y: 425, z: 0 }, // far - 0.3*scale, baseline + 0.3*nominal
    height: 297.5, // 0.85 * nominal (3.5 * 100)
    rotation: 0,
    wrapWidth: 0,
    attachment: 9, // AddRuns align 2 -> baseline right
    text: '2.015',
  });
  assert.equal(reasons.size, 0);
});

test('elevation flags & 32 emits an explicit solid-triangle outline', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeElevation(
    elevationRecord({ flags: 32, x: 10, y: 20, value: '2.00000' }),
    reasons,
  );
  assert.ok(primitives);

  const outlines = ofKind(primitives, 0);
  assert.equal(outlines.length, 1);
  assert.deepEqual(outlines[0], {
    kind: 0,
    points: [
      { x: 10, y: 20, z: 0 },
      { x: -290, y: 320, z: 0 },
      { x: 310, y: 320, z: 0 },
      { x: 10, y: 20, z: 0 },
    ],
  });
  assert.equal(reasons.get('vendor.tch.arrow_outline_only'), 1);

  const texts = ofKind(primitives, 4);
  assert.equal(texts.length, 1);
  const textPrimitive = texts[0];
  assert.equal(textPrimitive.kind, 4);
  if (textPrimitive.kind === 4) {
    assert.deepEqual(textPrimitive.origin, { x: 1111, y: -5, z: 0 });
    assert.equal(textPrimitive.attachment, 7);
    assert.equal(textPrimitive.text, '2.000');
  }
});

test('elevation strips ^U toggles from the emitted value', () => {
  const primitives = decodeElevation(
    elevationRecord({ flags: 0, value: '^U1.000^U' }),
    new Map<string, number>(),
  );
  assert.ok(primitives);
  const texts = ofKind(primitives, 4);
  assert.equal(texts.length, 1);
  assert.equal(texts[0].kind === 4 ? texts[0].text : null, '1.000');
});

test('arrow decodes a bulged segment to an arc, arrowhead outline and mode-2 texts', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeArrow(arrowRecord({
    mode: 2,
    vertices: [
      { x: 0, y: 0, bulge: 1 },
      { x: 1000, y: 0, bulge: 0 },
      { x: 2000, y: 0, bulge: 0 },
    ],
    arrowLength: 3,
    head: 1,
    text1: 'A',
    text2: 'B',
    hasText2: true,
  }), reasons);
  assert.ok(primitives);

  const arcs = ofKind(primitives, 2);
  assert.equal(arcs.length, 1);
  const arc = arcs[0];
  assert.equal(arc.kind, 2);
  if (arc.kind === 2) {
    assert.ok(Math.abs(arc.cx - 500) < 1e-9);
    assert.ok(Math.abs(arc.cy) < 1e-9);
    assert.ok(Math.abs(arc.radius - 500) < 1e-9);
    assert.ok(Math.abs(Math.abs(arc.sweep) - Math.PI) < 1e-9);
  }

  // Kind 0 carries the straight remainder of the path (stops at the arrowhead
  // base, 2000 - 300) followed by the explicit triangle outline.
  const polylines = ofKind(primitives, 0);
  assert.equal(polylines.length, 2);
  assert.deepEqual(polylines[0], {
    kind: 0,
    points: [
      { x: 1000, y: 0, z: 0 },
      { x: 1700, y: 0, z: 0 },
    ],
  });
  assert.deepEqual(polylines[1], {
    kind: 0,
    points: [
      { x: 1700, y: 40, z: 0 },
      { x: 1700, y: -40, z: 0 },
      { x: 2000, y: 0, z: 0 },
      { x: 1700, y: 40, z: 0 },
    ],
  });
  assert.equal(reasons.get('vendor.tch.arrow_outline_only'), 1);

  const texts = ofKind(primitives, 4);
  assert.equal(texts.length, 2);
  assert.deepEqual(texts[0], {
    kind: 4,
    origin: { x: 500, y: 105, z: 0 }, // midpoint + gap above the first segment
    height: 297.5,
    rotation: 0,
    wrapWidth: 0,
    attachment: 8, // AddRuns align 1 -> baseline center
    text: 'A',
  });
  assert.deepEqual(texts[1], {
    kind: 4,
    origin: { x: 500, y: -402.5, z: 0 }, // midpoint - gap - run height
    height: 297.5,
    rotation: 0,
    wrapWidth: 0,
    attachment: 8,
    text: 'B',
  });
});

test('arrow with fewer than two vertices decodes to no primitives', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeArrow(arrowRecord({ mode: 1, vertices: [] }), reasons);
  assert.deepEqual(primitives, []);
  assert.equal(reasons.size, 0);
});

test('multileader decodes leader lines, tail and upper/lower text', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeMultiLeader(multiLeaderRecord({
    arrow: 2,
    elbow: [100, 200],
    lineLength: 10,
    side: 0,
    targets: [[0, 0], [50, 50]],
    upper: 'UP',
    lower: 'DOWN',
    arrowSize: 3,
    gap: -1000,
    align: 3,
  }), reasons);
  assert.ok(primitives);

  const lines = ofKind(primitives, 1);
  assert.deepEqual(lines, [{
    kind: 1,
    points: [
      { x: 0, y: 0, z: 0 },
      { x: 100, y: 200, z: 0 },
      { x: 50, y: 50, z: 0 },
      { x: 100, y: 200, z: 0 },
      { x: 100, y: 200, z: 0 },
      { x: 1100, y: 200, z: 0 }, // elbow + lineLength * scale
    ],
  }]);

  // Arrow style 2 -> one outlined arrowhead per target.
  const outlines = ofKind(primitives, 0);
  assert.equal(outlines.length, 2);
  assert.equal(reasons.get('vendor.tch.arrow_outline_only'), 2);

  const texts = ofKind(primitives, 4);
  assert.equal(texts.length, 2);
  assert.deepEqual(texts[0], {
    kind: 4,
    origin: { x: 995, y: 305, z: 0 }, // end - 0.3*nominal, elbow + 0.3*nominal
    height: 297.5,
    rotation: 0,
    wrapWidth: 0,
    attachment: 9, // align 3 -> baseline right
    text: 'UP',
  });
  assert.deepEqual(texts[1], {
    kind: 4,
    origin: { x: 995, y: -255, z: 0 },
    height: 297.5,
    rotation: 0,
    wrapWidth: 0,
    attachment: 9,
    text: 'DOWN',
  });
});

test('unsupported class versions bump one stable reason and return null', () => {
  const main = new BitWriter();
  symbolBase(main, 100, 3.5);
  main.byte(9); // out of range elevation version
  const reasons = new Map<string, number>();
  const primitives = decodeElevation(makeRecord('TCH_ELEVATION', 0x40n, main), reasons);
  assert.equal(primitives, null);
  assert.equal(reasons.get('vendor.tch.elevation_unsupported_version'), 1);
});
