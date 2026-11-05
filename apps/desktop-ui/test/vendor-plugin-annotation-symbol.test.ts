// Contract tests for the TCH_DRAWINGINDEX / TCH_DRAWINGNAME / TCH_NORTHTHUMB
// base-6 decoders. Synthetic class bodies are built with the shared records
// fixtures; the decoders are exercised directly because they take a
// `VendorRecord` (the caller owns frame bookkeeping and `applied` counts).

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  decodeDrawingIndex,
  decodeDrawingName,
  decodeNorthThumb,
} from '../../../plugins/tch/src/annotation-symbol.ts';
import type { Primitive } from '../../../plugins/tch/src/types.ts';
import { BitWriter, makeRecord } from './vendor-plugin-fixtures.ts';

function base6(writer: BitWriter, scale: number): void {
  writer.byte(6);
  writer.bitDouble(scale);
  writer.bitDouble(0); // second base value
  writer.byte(0); // no extra base handles
  writer.bitLong(0); // empty property bag
}

function symbolBase(writer: BitWriter, scale: number, textHeight: number): void {
  base6(writer, scale);
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

function ofKind(primitives: readonly Primitive[], kind: number): Primitive[] {
  return primitives.filter(primitive => primitive.kind === kind);
}

function drawingIndexRecord(options: {
  version?: number;
  scale?: number;
  x?: number;
  y?: number;
  radius?: number;
  lineWidth?: number;
  number?: string;
  sheet?: string;
  scaleText?: string;
  scaleHeight?: number;
  showScale?: boolean;
}): ReturnType<typeof makeRecord> {
  const main = new BitWriter();
  const text = new BitWriter();
  symbolBase(main, options.scale ?? 100, 3.5);
  const version = options.version ?? 2;
  main.byte(version);
  main.rawDouble(options.x ?? 10);
  main.rawDouble(options.y ?? 20);
  main.bitDouble(options.radius ?? 7);
  main.bitDouble(options.lineWidth ?? 0.5);
  writeText(text, options.number ?? '');
  writeText(text, options.sheet ?? '');
  if (version > 1) {
    writeText(text, options.scaleText ?? '');
    main.bitDouble(options.scaleHeight ?? 3.5);
    main.bitDouble(0);
    main.byte(options.showScale ? 1 : 0);
    main.byte(0);
    main.bitDouble(0);
  }
  return makeRecord('TCH_DRAWINGINDEX', 0x41n, main, { text });
}

function drawingNameRecord(options: {
  version?: number;
  scale?: number;
  x?: number;
  y?: number;
  name?: string;
  scaleText?: string;
  nameHeight?: number;
  scaleHeight?: number;
  lineWidth?: number;
  gap?: number;
  showScale?: boolean;
  lineMode?: number;
  color?: number;
}): ReturnType<typeof makeRecord> {
  const main = new BitWriter();
  const text = new BitWriter();
  base6(main, options.scale ?? 100);
  const version = options.version ?? 1;
  main.bitShort(version);
  main.bitDouble(options.x ?? 100);
  main.bitDouble(options.y ?? 200);
  main.bitDouble(0);
  writeText(text, options.name ?? '');
  writeText(text, options.scaleText ?? '');
  main.bitDouble(0);
  main.bitDouble(options.nameHeight ?? 2.5);
  main.bitDouble(options.scaleHeight ?? 2);
  main.bitDouble(options.lineWidth ?? 0.5);
  main.bitDouble(options.gap ?? 0.6);
  main.pushBit(options.showScale ? 1 : 0);
  main.bitShort(options.lineMode ?? 1);
  main.bitShort(options.color ?? 7);
  if (version >= 2) {
    main.bitDouble(options.x ?? 100);
    main.bitDouble(options.y ?? 200);
    main.bitDouble(0);
  }
  return makeRecord('TCH_DRAWINGNAME', 0x42n, main, { text });
}

function northThumbRecord(options: {
  version?: number;
  scale?: number;
  x?: number;
  y?: number;
  radius?: number;
  rotation?: number;
  textHeight?: number;
  color?: number;
  text?: string;
}): ReturnType<typeof makeRecord> {
  const main = new BitWriter();
  const text = new BitWriter();
  base6(main, options.scale ?? 100);
  const version = options.version ?? 2;
  main.byte(version);
  main.rawDouble(options.x ?? 0);
  main.rawDouble(options.y ?? 0);
  main.bitDouble(options.radius ?? 2);
  main.bitDouble(options.rotation ?? 0);
  if (version > 1) {
    main.pushBit(0); // custom double code 0 (raw double)
    main.pushBit(0);
    main.rawDouble(options.textHeight ?? 1.5);
    main.bitShort(options.color ?? 7);
    main.rawDouble(0);
    main.rawDouble(0);
    writeText(text, options.text ?? 'N');
  }
  return makeRecord('TCH_NORTHTHUMB', 0x43n, main, { text });
}

test('drawing index decodes circle, diameter and number/sheet texts', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeDrawingIndex(
    drawingIndexRecord({ number: '3', sheet: 'A-101' }),
    reasons,
  );
  assert.ok(primitives);
  assert.equal(reasons.size, 0);

  assert.deepEqual(ofKind(primitives, 3), [{
    kind: 3,
    cx: 10,
    cy: 20,
    cz: 0,
    radius: 700,
  }]);
  assert.deepEqual(ofKind(primitives, 1), [{
    kind: 1,
    points: [
      { x: 710, y: 20, z: 0 },
      { x: -690, y: 20, z: 0 },
    ],
  }]);
  const texts = ofKind(primitives, 4);
  assert.deepEqual(texts, [
    {
      kind: 4,
      origin: { x: 10, y: 153, z: 0 }, // y + 0.19 r
      height: 364, // 0.26 * 2 r
      rotation: 0,
      wrapWidth: 0,
      attachment: 8,
      text: '3',
    },
    {
      kind: 4,
      origin: { x: 10, y: -477, z: 0 }, // y - 0.71 r
      height: 364,
      rotation: 0,
      wrapWidth: 0,
      attachment: 8,
      text: 'A-101',
    },
  ]);
});

test('drawing index with an empty sheet centers one large text', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeDrawingIndex(drawingIndexRecord({ number: '3', sheet: '' }), reasons);
  assert.ok(primitives);
  assert.equal(reasons.size, 0);
  assert.equal(ofKind(primitives, 1).length, 0);
  const texts = ofKind(primitives, 4);
  assert.equal(texts.length, 1);
  const centered = texts[0];
  assert.equal(centered.kind, 4);
  if (centered.kind === 4) {
    assert.ok(Math.abs(centered.height - 770) < 1e-9); // h = 0.55 * 2 r
    assert.ok(Math.abs(centered.origin.y - -365) < 1e-9); // y - h/2
    assert.equal(centered.origin.x, 10);
    assert.equal(centered.attachment, 8);
    assert.equal(centered.text, '3');
  }
});

test('drawing index appends the optional scale text at the circle edge', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeDrawingIndex(drawingIndexRecord({
    number: '3',
    sheet: 'A-101',
    scaleText: '^U1:100',
    scaleHeight: 3.5,
    showScale: true,
  }), reasons);
  assert.ok(primitives);
  const texts = ofKind(primitives, 4);
  assert.equal(texts.length, 3);
  assert.deepEqual(texts[2], {
    kind: 4,
    origin: { x: 910, y: -680, z: 0 }, // x + r + 2*scale, y - r
    height: 350, // scaleHeight * scale
    rotation: 0,
    wrapWidth: 0,
    attachment: 7, // AddRuns align 0 -> left aligned
    text: '1:100',
  });
});

test('drawing index rejects an unsupported class-body version', () => {
  const reasons = new Map<string, number>();
  assert.equal(decodeDrawingIndex(drawingIndexRecord({ version: 3 }), reasons), null);
  assert.deepEqual([...reasons], [['vendor.tch.drawingindex_unsupported_version', 1]]);
});

test('drawing name decodes the centered name and single underline', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeDrawingName(
    drawingNameRecord({ name: 'AB', lineMode: 1 }),
    reasons,
  );
  assert.ok(primitives);
  assert.equal(reasons.get('vendor.tch.drawingname_text_width_estimate'), 1);

  // h1 = 2.5 * 100 = 250, advance = h1 per ASCII char => width 500.
  assert.deepEqual(ofKind(primitives, 4), [{
    kind: 4,
    origin: { x: -175, y: 325, z: 0 }, // left = 100 - (500 + 0.2*250)/2
    height: 250,
    rotation: 0,
    wrapWidth: 0,
    attachment: 4,
    text: 'AB',
  }]);
  assert.deepEqual(ofKind(primitives, 1), [{
    kind: 1,
    points: [
      { x: -175, y: 100, z: 0 }, // y - 2 * line, line = 0.5 * 100
      { x: 375, y: 100, z: 0 },
    ],
  }]);
});

test('drawing name mode 0 adds the second underline and the scale text', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeDrawingName(drawingNameRecord({
    version: 3,
    name: 'AB',
    scaleText: '1:50',
    showScale: true,
    lineMode: 0,
  }), reasons);
  assert.ok(primitives);
  assert.equal(reasons.get('vendor.tch.drawingname_text_width_estimate'), 1);

  const texts = ofKind(primitives, 4);
  assert.equal(texts.length, 2);
  assert.deepEqual(texts[1], {
    kind: 4,
    origin: { x: 5, y: 300, z: 0 }, // left + nameWidth + gap*h1, y + a2/2
    height: 200, // 2 * 100
    rotation: 0,
    wrapWidth: 0,
    attachment: 4,
    text: '1:50',
  });
  const underlines = ofKind(primitives, 1);
  assert.equal(underlines.length, 2);
  assert.deepEqual(underlines[0], {
    kind: 1,
    points: [
      { x: -645, y: 100, z: 0 },
      { x: 845, y: 100, z: 0 },
    ],
  });
  assert.deepEqual(underlines[1], {
    kind: 1,
    points: [
      { x: -645, y: -40, z: 0 }, // y - 1.4 * scale
      { x: 845, y: -40, z: 0 },
    ],
  });
});

test('drawing name rejects an unsupported class-body version', () => {
  const reasons = new Map<string, number>();
  assert.equal(decodeDrawingName(drawingNameRecord({ version: 4 }), reasons), null);
  assert.deepEqual([...reasons], [['vendor.tch.drawingname_unsupported_version', 1]]);
});

test('north thumb decodes the circle, outlined arrow and pointer text', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeNorthThumb(
    northThumbRecord({ text: 'N', textHeight: 1.5 }),
    reasons,
  );
  assert.ok(primitives);
  assert.equal(reasons.get('vendor.tch.arrow_outline_only'), 1);

  assert.deepEqual(ofKind(primitives, 3), [{
    kind: 3,
    cx: 0,
    cy: 0,
    cz: 0,
    radius: 200, // 2 * 100
  }]);
  // Filled triangles have no ABI primitive, so the arrow is its closed outline.
  assert.deepEqual(ofKind(primitives, 0), [{
    kind: 0,
    points: [
      { x: -200, y: 150, z: 0 },
      { x: -200, y: -150, z: 0 },
      { x: 200, y: 0, z: 0 },
      { x: -200, y: 150, z: 0 },
    ],
  }]);
  assert.deepEqual(ofKind(primitives, 4), [{
    kind: 4,
    origin: { x: 350, y: 0, z: 0 }, // x + ux*(r+h), y + uy*(r+h)
    height: 150, // textHeight * scale
    rotation: 0,
    wrapWidth: 0,
    attachment: 5,
    text: 'N',
  }]);
});

test('north thumb rejects an unsupported class-body version', () => {
  const reasons = new Map<string, number>();
  assert.equal(decodeNorthThumb(northThumbRecord({ version: 3 }), reasons), null);
  assert.deepEqual([...reasons], [['vendor.tch.norththumb_unsupported_version', 1]]);
});
