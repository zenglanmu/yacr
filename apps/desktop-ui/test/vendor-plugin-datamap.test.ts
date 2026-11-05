// Contract tests for the shared `TDBXDATAMAP_BLOCK_*` skip in class text
// streams. Real AC1027 records carry the Tianzheng xdata map between the base
// property-bag strings and the class's own labels; without the skip every
// decoder emitted the `TDBXDATAMAP_BLOCK_BEGIN_` sentinel as drawing text.

import assert from 'node:assert/strict';
import test from 'node:test';

import { decodeDrawingIndex } from '../../../plugins/tch/src/annotation-symbol.ts';
import { decodeMultiLeader } from '../../../plugins/tch/src/annotation.ts';
import { decodeText } from '../../../plugins/tch/src/text.ts';
import type { Primitive } from '../../../plugins/tch/src/types.ts';
import { BitWriter, makeRecord, textWriter } from './vendor-plugin-fixtures.ts';

const DATAMAP_BEGIN = 'TDBXDATAMAP_BLOCK_BEGIN_';
const DATAMAP_END = 'TDBXDATAMAP_BLOCK_END_';

function writeText(writer: BitWriter, value: string): void {
  writer.bitShort(value.length);
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    writer.byte(code & 0xff);
    writer.byte((code >>> 8) & 0xff);
  }
}

/** Base-6 header used by the symbol decoders. */
function symbolBase(main: BitWriter, scale: number, textHeight: number): void {
  main.byte(6);
  main.bitDouble(scale);
  main.bitDouble(0); // second base value
  main.byte(0); // no extra base handles
  main.bitLong(0); // empty property bag
  main.byte(1); // symbol version
  main.bitDouble(textHeight);
}

/** Append the real AC1027 shared block before the class's own strings. */
function writeDataMap(text: BitWriter): void {
  writeText(text, DATAMAP_BEGIN);
  writeText(text, 'Key');
  writeText(text, 'Value');
  writeText(text, DATAMAP_END);
}

function texts(primitives: readonly Primitive[] | null): string[] {
  if (!primitives) return [];
  return primitives
    .filter((primitive): primitive is Extract<Primitive, { kind: 4 }> => primitive.kind === 4)
    .map(primitive => primitive.text);
}

function textRecord(value: string, withDataMap: boolean): ReturnType<typeof makeRecord> {
  const text = new BitWriter();
  const main = textWriter({
    className: 'TCH_TEXT',
    value: withDataMap ? DATAMAP_BEGIN : value,
    text,
    position: [1, 2, 0],
    height: 2.5,
    rotation: 0,
    attachment: 5,
  });
  if (withDataMap) {
    writeText(text, 'Key');
    writeText(text, 'Value');
    writeText(text, DATAMAP_END);
    writeText(text, value);
  }
  return makeRecord('TCH_TEXT', 0x51n, main, { text });
}

function multiLeaderRecord(
  upper: string,
  lower: string,
  withDataMap: boolean,
): ReturnType<typeof makeRecord> {
  const main = new BitWriter();
  const text = new BitWriter();
  symbolBase(main, 100, 3.5);
  main.byte(1); // class-body version
  main.byte(0); // arrow kind
  main.rawDouble(10);
  main.rawDouble(20);
  main.bitDouble(5); // line length
  main.byte(0); // side
  main.byte(1); // one target
  main.rawDouble(1);
  main.rawDouble(2);
  if (withDataMap) writeDataMap(text);
  writeText(text, upper);
  writeText(text, lower);
  return makeRecord('TCH_MULTILEADER', 0x52n, main, { text });
}

function drawingIndexRecord(
  number: string,
  sheet: string,
  withDataMap: boolean,
): ReturnType<typeof makeRecord> {
  const main = new BitWriter();
  const text = new BitWriter();
  symbolBase(main, 100, 3.5);
  main.byte(1); // class-body version (no scale text)
  main.rawDouble(10);
  main.rawDouble(20);
  main.bitDouble(7); // radius
  main.bitDouble(0.5); // line width
  if (withDataMap) writeDataMap(text);
  writeText(text, number);
  writeText(text, sheet);
  return makeRecord('TCH_DRAWINGINDEX', 0x53n, main, { text });
}

test('TCH_TEXT skips the datamap and keeps the real value', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeText(textRecord('AC', true), reasons);
  assert.ok(primitives);
  assert.deepEqual(texts(primitives), ['AC']);
  assert.ok(!texts(primitives).some(value => value.includes('TDBXDATAMAP')));
  assert.equal(reasons.size, 0);
});

test('TCH_TEXT without a datamap still decodes unchanged', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeText(textRecord('label', false), reasons);
  assert.ok(primitives);
  assert.deepEqual(texts(primitives), ['label']);
  assert.equal(reasons.size, 0);
});

test('TCH_MULTILEADER reads both labels after the datamap', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeMultiLeader(multiLeaderRecord('EL. 3.500', 'NOTE', true), reasons);
  assert.ok(primitives);
  assert.deepEqual(texts(primitives), ['EL. 3.500', 'NOTE']);
  assert.equal(reasons.size, 0);
});

test('TCH_DRAWINGINDEX reads number and sheet after the datamap', () => {
  const reasons = new Map<string, number>();
  const primitives = decodeDrawingIndex(drawingIndexRecord('12', 'A-1', true), reasons);
  assert.ok(primitives);
  assert.deepEqual(texts(primitives), ['12', 'A-1']);
  assert.equal(reasons.size, 0);
});
