// Contract tests for the base-6 dimension / axis-label / radius-dim decoders.
//
// They exercise the plugin source directly (node type stripping) and build
// synthetic class bodies with the shared records-frame writer. The decoders
// only return primitives; the caller owns `applied` bookkeeping.

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  decodeAxisLabel, decodeDimension2, decodeRadiusDim,
} from '../../../plugins/tch/src/dimension.ts';
import type { Primitive } from '../../../plugins/tch/src/types.ts';
import { BitWriter, makeRecord } from './vendor-plugin-fixtures.ts';

function base6(writer: BitWriter, scale: number): void {
  writer.byte(6);
  writer.bitDouble(scale);
  writer.bitDouble(0); // second base value
  writer.byte(0); // no base handles
  writer.bitLong(0); // empty property bag
}

function writeText(writer: BitWriter, value: string): void {
  writer.bitShort(value.length);
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    writer.byte(code & 0xff);
    writer.byte((code >>> 8) & 0xff);
  }
}

function dimensionRecord(options: {
  scale?: number;
  position?: readonly [number, number, number];
  angle?: number;
  offset?: number;
  spans: readonly number[];
  flags?: number;
  textPositions?: readonly { index: number; x: number; y: number }[];
  textOverrides?: readonly { index: number; value: string }[];
  arcRadius?: number;
  measuredRadius?: number;
  extensions?: readonly number[];
}): ReturnType<typeof makeRecord> {
  const main = new BitWriter();
  const text = new BitWriter();
  const flags = options.flags ?? 0;
  base6(main, options.scale ?? 100);
  main.byte(1); // native dimension version
  const [x, y, z] = options.position ?? [0, 0, 0];
  main.bitDouble(x);
  main.bitDouble(y);
  main.bitDouble(z);
  main.bitDouble(options.angle ?? 0);
  main.bitDouble(options.offset ?? 20);
  main.bitShort(options.spans.length);
  for (const span of options.spans) main.bitDouble(span);
  main.bitShort(flags);
  if ((flags & 2) !== 0) {
    const positions = options.textPositions ?? [];
    main.bitShort(positions.length);
    for (const position of positions) {
      main.bitShort(position.index);
      main.rawDouble(position.x);
      main.rawDouble(position.y);
    }
  }
  if ((flags & 4) !== 0) {
    const overrides = options.textOverrides ?? [];
    main.bitShort(overrides.length);
    for (const override of overrides) {
      main.bitShort(override.index);
      writeText(text, override.value);
    }
  }
  if ((flags & 0x10) !== 0) {
    main.bitDouble(options.arcRadius ?? 200);
    main.bitDouble(options.measuredRadius ?? 250);
  }
  if ((flags & 0x80) !== 0) {
    const extensions = options.extensions ?? [];
    main.bitShort(extensions.length);
    for (const value of extensions) main.bitDouble(value);
  }
  return makeRecord('TCH_DIMENSION2', 0x30n, main, { text });
}

function axisRecord(options: {
  scale?: number;
  mode?: number;
  start?: readonly [number, number];
  end?: readonly [number, number];
  bulge?: number;
  axes: readonly { flags?: number; label: string; spacing: number }[];
  startExtension?: number;
  endExtension?: number;
  radius?: number;
  showText?: boolean;
}): ReturnType<typeof makeRecord> {
  const main = new BitWriter();
  const text = new BitWriter();
  const version = 6;
  base6(main, options.scale ?? 100);
  main.byte(version);
  main.bitShort(options.mode ?? 0);
  const [x0, y0] = options.start ?? [0, 0];
  const [x1, y1] = options.end ?? [100, 0];
  main.rawDouble(x0);
  main.rawDouble(y0);
  main.rawDouble(x1);
  main.rawDouble(y1);
  main.bitDouble(options.bulge ?? 0);
  main.bitShort(options.axes.length);
  for (const axis of options.axes) {
    const flags = axis.flags ?? 0;
    main.bitShort(flags);
    writeText(text, axis.label);
    main.bitDouble(axis.spacing);
    if ((flags & 0x80) !== 0) main.bitDouble(0);
    if ((flags & 0x100) !== 0) main.bitDouble(0);
    if ((flags & 0x400) !== 0) main.bitDouble(0);
    if ((flags & 0x800) !== 0) main.bitDouble(0);
  }
  main.bitDouble(0); // unused trailing value
  main.bitDouble(options.startExtension ?? 5);
  main.bitDouble(options.endExtension ?? 5);
  main.bitDouble(options.radius ?? 4);
  main.bitShort(0); // version > 3 extra short
  main.bitShort(options.showText === false ? 0 : 1);
  return makeRecord('TCH_AXIS_LABEL', 0x31n, main, { text });
}

function radiusRecord(options: {
  scale?: number;
  flags?: number;
  leader?: number;
  center?: readonly [number, number, number];
  tip?: readonly [number, number];
  textOverride?: string;
  textPosition?: readonly [number, number];
}): ReturnType<typeof makeRecord> {
  const main = new BitWriter();
  const text = new BitWriter();
  const flags = options.flags ?? 0;
  base6(main, options.scale ?? 100);
  main.byte(1); // radius dimension version
  main.bitShort(flags);
  main.bitDouble(options.leader ?? 10);
  const [cx, cy, cz] = options.center ?? [0, 0, 0];
  main.bitDouble(cx);
  main.bitDouble(cy);
  main.bitDouble(cz);
  const [tx, ty] = options.tip ?? [50, 0];
  main.rawDouble(tx);
  main.rawDouble(ty);
  if ((flags & 4) !== 0) writeText(text, options.textOverride ?? '');
  if ((flags & 2) !== 0) {
    const [px, py] = options.textPosition ?? [0, 0];
    main.rawDouble(px);
    main.rawDouble(py);
  }
  return makeRecord('TCH_RADIUSDIM', 0x32n, main, { text });
}

function ofKind(primitives: readonly Primitive[], kind: number): Primitive[] {
  return primitives.filter(primitive => primitive.kind === kind);
}

test('linear dimension with two spans emits lines, ticks and two texts', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeDimension2(dimensionRecord({ spans: [100, 50] }), reasons);
  assert.ok(primitives);

  const lines = ofKind(primitives, 1);
  // 3 extension lines + 1 dimension line, then 3 ticks as the second segment set.
  assert.equal(lines.length, 2);
  assert.equal((lines[0] as { points: unknown[] }).points.length, 8);
  assert.equal((lines[1] as { points: unknown[] }).points.length, 6);

  const texts = ofKind(primitives, 4) as Extract<Primitive, { kind: 4 }>[];
  assert.deepEqual(texts.map(item => item.text), ['100', '50']);
  for (const item of texts) {
    assert.equal(item.height, 3.5 * 0.85 * 100);
    assert.equal(item.rotation, 0);
    assert.equal(item.attachment, 5);
  }
  assert.equal(reasons.get('vendor.tch.dimension_style_defaults'), 1);
});

test('override text wins and an explicit text position is honoured', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeDimension2(dimensionRecord({
    spans: [100],
    flags: 2 | 4,
    textOverrides: [{ index: 0, value: 'A^UB' }],
    textPositions: [{ index: 0, x: 7, y: 9 }],
  }), reasons);
  assert.ok(primitives);
  const texts = ofKind(primitives, 4) as Extract<Primitive, { kind: 4 }>[];
  assert.equal(texts.length, 1);
  assert.equal(texts[0].text, 'AB');
  assert.deepEqual(texts[0].origin, { x: 7, y: 9, z: 0 });
});

test('arc dimension emits a kind-2 radius arc and degree text', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeDimension2(dimensionRecord({
    spans: [Math.PI / 2],
    flags: 0x10,
    angle: Math.PI / 2,
    arcRadius: 200,
    measuredRadius: 250,
  }), reasons);
  assert.ok(primitives);

  const arcs = ofKind(primitives, 2) as Extract<Primitive, { kind: 2 }>[];
  assert.equal(arcs.length, 1);
  assert.equal(arcs[0].radius, 200);
  assert.ok(arcs[0].sweep > 0 && arcs[0].sweep < Math.PI / 2);

  const texts = ofKind(primitives, 4) as Extract<Primitive, { kind: 4 }>[];
  assert.equal(texts.length, 1);
  assert.equal(texts[0].text, '90.00\u00b0');
  // Two filled arrow heads become closed outlines.
  assert.equal(ofKind(primitives, 0).length, 2);
  assert.ok((reasons.get('vendor.tch.arrow_outline_only') ?? 0) >= 2);
});

test('axis label emits two circles and two labels, mode can hide one end', () => {
  const full = decodeAxisLabel(axisRecord({
    axes: [{ label: '1', spacing: 0 }],
  }), new Map());
  assert.ok(full);
  // Two sides × (one leader line) plus the label text pair.
  assert.equal(ofKind(full, 1).length, 1);
  assert.equal((ofKind(full, 1)[0] as { points: unknown[] }).points.length, 4);
  assert.equal(ofKind(full, 3).length, 2);
  assert.deepEqual(
    (ofKind(full, 4) as Extract<Primitive, { kind: 4 }>[]).map(item => item.text),
    ['1', '1'],
  );

  const hidden = decodeAxisLabel(axisRecord({
    mode: 0x04, // HideStart
    axes: [{ label: '2', spacing: 0 }],
  }), new Map());
  assert.ok(hidden);
  assert.equal(ofKind(hidden, 1).length, 1);
  assert.equal((ofKind(hidden, 1)[0] as { points: unknown[] }).points.length, 2);
  assert.equal(ofKind(hidden, 3).length, 1);
  assert.equal(ofKind(hidden, 4).length, 1);
});

test('radius dimension emits a leader line, arrow outline and R text', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeRadiusDim(radiusRecord({ leader: 10, tip: [50, 0] }), reasons);
  assert.ok(primitives);

  const lines = ofKind(primitives, 1) as Extract<Primitive, { kind: 1 }>[];
  assert.equal(lines.length, 1);
  assert.equal(lines[0].points.length, 2);
  const arrows = ofKind(primitives, 0) as Extract<Primitive, { kind: 0 }>[];
  assert.equal(arrows.length, 1);
  assert.equal(arrows[0].points.length, 4);
  assert.deepEqual(arrows[0].points[0], arrows[0].points[3]);

  const texts = ofKind(primitives, 4) as Extract<Primitive, { kind: 4 }>[];
  assert.equal(texts.length, 1);
  assert.equal(texts[0].text, 'R50');
  assert.equal(texts[0].height, 3.5 * 0.85 * 100);
  assert.equal(reasons.get('vendor.tch.dimension_style_defaults'), 1);
  assert.equal(reasons.get('vendor.tch.arrow_outline_only'), 1);
});

test('an unsupported class-body version reports one stable reason', () => {
  const main = new BitWriter();
  base6(main, 100);
  main.byte(2); // not version 1
  const record = makeRecord('TCH_DIMENSION2', 0x33n, main);
  const reasons = new Map<string, number>();
  assert.equal(decodeDimension2(record, reasons), null);
  assert.equal(reasons.get('vendor.tch.dimension_unsupported_version'), 1);
});
