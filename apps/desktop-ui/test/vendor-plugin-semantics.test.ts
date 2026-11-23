// Contract tests for the `YANE` / `YANS` semantics frames (ADR 0030 D7/D8).
//
// Import: `extractSemantics(YAND) -> YANE` yields source parameters (not display
// primitives). Reopen: `decode(YANS) -> YANR` must rebuild the same primitives
// the records pass produced. Synthetic records only; no real DWG is required.

import assert from 'node:assert/strict';
import test from 'node:test';

import { decode, extractSemantics } from '../../../plugins/tch/src/index.ts';
import {
  SEMANTICS_MAGIC,
  SEMANTICS_REQUEST_MAGIC,
  decodeSemanticsFrame,
  encodeSemanticsFrame,
} from '../../../plugins/tch/src/semantics.ts';
import type { SemanticsFrameRow, SemanticsRow } from '../../../plugins/tch/src/semantics.ts';
import {
  BitWriter,
  buildRecordsFrame,
  makeRecord,
  openingWriter,
  textWriter,
  wallWriter,
} from './vendor-plugin-fixtures.ts';

// ---------------------------------------------------------------------------
// Frame helpers
// ---------------------------------------------------------------------------

interface ParsedRow {
  key: bigint;
  sourceHandle: bigint;
  row: SemanticsRow;
}

interface ParsedYane {
  magic: number;
  abi: number;
  token: bigint;
  generation: bigint;
  rows: ParsedRow[];
}

function parseYane(frame: ArrayBuffer): ParsedYane {
  const view = new DataView(frame);
  const magic = view.getUint32(0, true);
  const abi = view.getUint32(4, true);
  const token = view.getBigUint64(8, true);
  const generation = view.getBigUint64(16, true);
  const count = view.getUint32(24, true);
  const decoder = new TextDecoder();
  let offset = 28;
  const rows: ParsedRow[] = [];
  for (let index = 0; index < count; index += 1) {
    const key = view.getBigUint64(offset, true);
    const sourceHandle = view.getBigUint64(offset + 8, true);
    const length = view.getUint32(offset + 16, true);
    offset += 20;
    const text = decoder.decode(new Uint8Array(frame, offset, length));
    offset += length;
    rows.push({ key, sourceHandle, row: JSON.parse(text) as SemanticsRow });
  }
  assert.equal(offset, frame.byteLength, 'YANE frame has no trailing bytes');
  return { magic, abi, token, generation, rows };
}

interface ParsedResult {
  status: number;
  applied: number;
  diagnostics: string[];
  /** `entity_count` plus every entity group: primitives must match exactly. */
  tail: number[];
}

function parseResult(frame: ArrayBuffer): ParsedResult {
  const view = new DataView(frame);
  const status = view.getUint32(24, true);
  const applied = view.getUint32(28, true);
  const diagnosticCount = view.getUint32(32, true);
  const decoder = new TextDecoder();
  const diagnostics: string[] = [];
  let offset = 36;
  for (let index = 0; index < diagnosticCount; index += 1) {
    const length = view.getUint16(offset, true);
    diagnostics.push(decoder.decode(new Uint8Array(frame, offset + 2, length)));
    offset += 2 + length;
  }
  return {
    status,
    applied,
    diagnostics,
    tail: Array.from(new Uint8Array(frame, offset)),
  };
}

/** The plugin only encodes `YANE`; the host builds `YANS` with the same layout. */
function yansFrom(token: bigint, generation: bigint, rows: readonly ParsedRow[]): ArrayBuffer {
  const frames: SemanticsFrameRow[] = rows.map(row => ({
    key: row.key,
    sourceHandle: row.sourceHandle,
    row: row.row,
  }));
  return replaceUint32(
    encodeSemanticsFrame(token, generation, frames),
    0,
    SEMANTICS_REQUEST_MAGIC,
  );
}

function replaceUint32(frame: ArrayBuffer, offset: number, value: number): ArrayBuffer {
  const copy = frame.slice(0);
  new DataView(copy).setUint32(offset, value, true);
  return copy;
}

function appendByte(frame: ArrayBuffer): ArrayBuffer {
  const bytes = new Uint8Array(frame.byteLength + 1);
  bytes.set(new Uint8Array(frame));
  return bytes.buffer;
}

function writeHandle(writer: BitWriter, value: bigint): void {
  const bytes: number[] = [];
  for (let temp = value; temp > 0n; temp >>= 8n) bytes.unshift(Number(temp & 0xffn));
  writer.byte(bytes.length);
  for (const byte of bytes) writer.byte(byte);
}

// ---------------------------------------------------------------------------
// P0 class coverage
// ---------------------------------------------------------------------------

const WALL_HANDLE = 0x3e868n;
const WALL = {
  start: [0, 0, 0] as const,
  end: [10, 0, 0] as const,
  left: 1,
  right: 2,
  height: 3,
  joins: [1, 2, 3, 4] as const,
};

test('TCH_WALL extraction yields one YANE row of source parameters', () => {
  const record = makeRecord('TCH_WALL', WALL_HANDLE, wallWriter(WALL));
  const frame = buildRecordsFrame(7n, 9n, [record]);

  const parsed = parseYane(extractSemantics(frame));
  assert.equal(parsed.magic, SEMANTICS_MAGIC);
  assert.equal(parsed.abi, 1);
  assert.equal(parsed.token, 7n);
  assert.equal(parsed.generation, 9n);
  assert.equal(parsed.rows.length, 1);

  const [{ key, sourceHandle, row }] = parsed.rows;
  assert.equal(key, WALL_HANDLE, 'import key is the source handle');
  assert.equal(sourceHandle, WALL_HANDLE);
  assert.equal(row.kind, 'tch');
  assert.equal(row.class, 'TCH_WALL');
  assert.equal(row.schema, 1);
  assert.equal(row.status, 'ok');
  assert.deepEqual(row.diagnostics, []);
  assert.deepEqual(row.source, {
    handle: '3E868',
    dwg_version: 1027,
    decoder: 1,
    tch_base: 6,
  });
  assert.deepEqual(row.units, { scale: 150 });
  assert.deepEqual(row.attributes, []);
  assert.deepEqual(row.refs, []);
  assert.deepEqual(row.params, {
    start: [0, 0, 0],
    end: [10, 0, 0],
    angle: 0,
    left: 1,
    right: 2,
    height: 3,
    joins: [1, 2, 3, 4],
    material: 10,
    insulation: 80.080000001,
    openings: 0,
  });
});

test('decode(YANS) reproduces the decode(YAND) primitive result for a wall', () => {
  const record = makeRecord('TCH_WALL', WALL_HANDLE, wallWriter(WALL));
  const recordsFrame = buildRecordsFrame(11n, 12n, [record]);
  const rows = parseYane(extractSemantics(recordsFrame)).rows;

  const fromRecords = parseResult(decode(recordsFrame));
  const fromSemantics = parseResult(decode(yansFrom(11n, 12n, rows)));

  assert.equal(fromRecords.applied, 1);
  assert.equal(fromSemantics.applied, fromRecords.applied);
  assert.equal(fromSemantics.status, fromRecords.status);
  assert.deepEqual(fromSemantics.tail, fromRecords.tail, 'entity groups are byte-identical');
});

test('TCH_OPENING persists its refs and the wall cut is recomputed on reopen', () => {
  const OPENING_HANDLE = 0x600n;
  const BLOCK_2D = 0xab0n;
  const wall = makeRecord('TCH_WALL', WALL_HANDLE, wallWriter({ ...WALL, openings: 1 }));
  const handles = new BitWriter();
  const openingMain = openingWriter({
    position: [5, 0, 0],
    width: 2,
    wall: WALL_HANDLE,
    block2d: BLOCK_2D,
    handles,
  });
  const opening = makeRecord('TCH_OPENING', OPENING_HANDLE, openingMain, { handle: handles });

  const recordsFrame = buildRecordsFrame(21n, 22n, [wall, opening]);
  const rows = parseYane(extractSemantics(recordsFrame)).rows;
  assert.equal(rows.length, 2);

  const openingRow = rows[1].row;
  assert.equal(openingRow.class, 'TCH_OPENING');
  assert.equal(openingRow.status, 'ok');
  assert.deepEqual(openingRow.params, {
    position: [5, 0, 0],
    width: 2,
    thickness: 0.2,
    height: 2,
    angle: 0,
    mirror: 0,
    kind: 0,
    flags: 0,
    segments: [],
    label: '',
    labelHeight: 3.5,
  });
  assert.deepEqual(openingRow.refs, [
    { role: 'wall', handle: '3E868', kind: 'entity' },
    { role: 'block2d', handle: 'AB0', kind: 'block' },
  ]);
  assert.equal(openingRow.source.class_version, 5);

  const fromRecords = parseResult(decode(recordsFrame));
  const fromSemantics = parseResult(decode(yansFrom(21n, 22n, rows)));
  assert.ok(fromRecords.applied > 1, 'wall cut plus door symbol');
  assert.equal(fromSemantics.applied, fromRecords.applied);
  assert.deepEqual(
    fromSemantics.tail,
    fromRecords.tail,
    'the cut wall is rebuilt from the opening row, not persisted',
  );
});

test('TCH_TEXT params carry the world-space text fields', () => {
  const text = new BitWriter();
  const main = textWriter({
    className: 'TCH_TEXT',
    value: 'M0921',
    position: [1, 2, 0],
    height: 3.5,
    rotation: 0.25,
    attachment: 1,
    text,
  });
  const record = makeRecord('TCH_TEXT', 0x700n, main, { text });
  const recordsFrame = buildRecordsFrame(31n, 32n, [record]);
  const rows = parseYane(extractSemantics(recordsFrame)).rows;
  const row = rows[0].row;

  assert.equal(row.class, 'TCH_TEXT');
  assert.equal(row.status, 'ok');
  assert.deepEqual(row.units, { scale: 100 });
  assert.equal(row.source.class_version, 2);
  assert.deepEqual(row.params, {
    text: 'M0921',
    origin: [1, 2, 0],
    height: 350,
    rotation: 0.25,
    attachment: 1,
    wrapWidth: 0,
  });

  const fromRecords = parseResult(decode(recordsFrame));
  const fromSemantics = parseResult(decode(yansFrom(31n, 32n, rows)));
  assert.equal(fromSemantics.applied, fromRecords.applied);
  assert.deepEqual(fromSemantics.tail, fromRecords.tail);
});

test('TCH_BLOCK_INSERT persists origin/scale/rotation/normal and the block ref', () => {
  const BLOCK_HANDLE = 0x3dd82n;
  const main = new BitWriter();
  main.bitDouble(100);
  main.bitDouble(200);
  main.bitDouble(0);
  main.pushBits(2, 2); // uniform scale
  main.rawDouble(2.5);
  main.bitDouble(Math.PI / 6);
  main.bitDouble(0);
  main.bitDouble(0);
  main.bitDouble(1);
  main.pushBit(0); // no attributes
  main.byte(4); // class version
  main.byte(0); // mode 0
  main.byte(6); // base-6 tail
  main.bitDouble(150);
  main.pushBits(2, 2);
  main.byte(1);
  main.byte(0);
  main.pushBits(2, 2);
  const handles = new BitWriter();
  writeHandle(handles, BLOCK_HANDLE);
  const record = makeRecord('TCH_BLOCK_INSERT', 0x800n, main, { handle: handles });

  const recordsFrame = buildRecordsFrame(41n, 42n, [record]);
  const rows = parseYane(extractSemantics(recordsFrame)).rows;
  const row = rows[0].row;

  assert.equal(row.status, 'ok');
  assert.deepEqual(row.units, { scale: 150 });
  assert.deepEqual(row.params, {
    origin: [100, 200, 0],
    scale: [2.5, 2.5, 2.5],
    rotation: Math.PI / 6,
    normal: [0, 0, 1],
  });
  assert.deepEqual(row.refs, [{ role: 'block', handle: '3DD82', kind: 'block' }]);

  const fromRecords = parseResult(decode(recordsFrame));
  const fromSemantics = parseResult(decode(yansFrom(41n, 42n, rows)));
  assert.equal(fromSemantics.applied, fromRecords.applied);
  assert.deepEqual(fromSemantics.tail, fromRecords.tail);
});

test('classes outside P0 are unsupported with no fabricated params', () => {
  const record = makeRecord('TCH_COLUMN', 0x900n, new BitWriter());
  const rows = parseYane(extractSemantics(buildRecordsFrame(51n, 52n, [record]))).rows;
  const row = rows[0].row;

  assert.equal(row.class, 'TCH_COLUMN');
  assert.equal(row.status, 'unsupported');
  assert.deepEqual(row.diagnostics, ['vendor.tch.semantics_not_implemented']);
  assert.deepEqual(row.params, {});
  assert.deepEqual(row.refs, []);
  assert.deepEqual(row.attributes, []);
  assert.deepEqual(row.units, {});
  assert.equal(row.schema, 1);

  // An unsupported row renders nothing: no primitives, no `applied` claim.
  const result = parseResult(decode(yansFrom(51n, 52n, rows)));
  assert.equal(result.applied, 0);
  assert.equal(result.status, 1);
  assert.ok(result.diagnostics.some(code => code.startsWith('vendor.tch.semantics_not_implemented')));
});

// ---------------------------------------------------------------------------
// Frame rejection paths
// ---------------------------------------------------------------------------

test('unknown magic is refused by both entry points', () => {
  const record = makeRecord('TCH_WALL', WALL_HANDLE, wallWriter(WALL));
  const recordsFrame = buildRecordsFrame(61n, 62n, [record]);
  const rows = parseYane(extractSemantics(recordsFrame)).rows;
  const yans = yansFrom(61n, 62n, rows);

  assert.throws(
    () => decode(replaceUint32(recordsFrame, 0, 0x59414e46)),
    /vendor\.bad_magic/,
    'YAND entry still uses the ABI v1 error',
  );
  assert.throws(
    () => decodeSemanticsFrame(recordsFrame),
    /vendor\.semantics\.bad_magic/,
    'a records frame is not a YANS frame',
  );
  assert.throws(
    () => decode(extractSemantics(recordsFrame)),
    /vendor\.bad_magic/,
    'a YANE frame is not a decode input',
  );
  assert.equal(parseResult(decode(yans)).applied, 1, 'the untouched frame still decodes');
});

test('ABI mismatch is refused with the semantics error', () => {
  const rows = parseYane(extractSemantics(
    buildRecordsFrame(63n, 64n, [makeRecord('TCH_WALL', WALL_HANDLE, wallWriter(WALL))]),
  )).rows;
  const yans = replaceUint32(yansFrom(63n, 64n, rows), 4, 2);
  assert.throws(() => decodeSemanticsFrame(yans), /vendor\.semantics\.unsupported_abi: 2/);
  assert.throws(() => decode(yans), /vendor\.semantics\.unsupported_abi: 2/);
});

test('token and generation mismatches are refused', () => {
  const rows = parseYane(extractSemantics(
    buildRecordsFrame(65n, 66n, [makeRecord('TCH_WALL', WALL_HANDLE, wallWriter(WALL))]),
  )).rows;
  const yans = yansFrom(65n, 66n, rows);

  decodeSemanticsFrame(yans, { token: 65n, generation: 66n });
  assert.throws(
    () => decodeSemanticsFrame(yans, { token: 1n }),
    /vendor\.semantics\.token_mismatch/,
  );
  assert.throws(
    () => decodeSemanticsFrame(yans, { generation: 1n }),
    /vendor\.semantics\.generation_mismatch/,
  );
});

test('a schema above the supported major is refused, never rendered empty', () => {
  const rows = parseYane(extractSemantics(
    buildRecordsFrame(67n, 68n, [makeRecord('TCH_WALL', WALL_HANDLE, wallWriter(WALL))]),
  )).rows;
  const tampered = rows.map(row => ({
    key: row.key,
    sourceHandle: row.sourceHandle,
    row: { ...row.row, schema: 2 },
  }));
  const yans = yansFrom(67n, 68n, tampered);

  assert.throws(() => decodeSemanticsFrame(yans), /vendor\.semantics\.unsupported_schema/);
  assert.throws(() => decode(yans), /vendor\.semantics\.unsupported_schema/);
});

test('truncation and trailing bytes are refused', () => {
  const rows = parseYane(extractSemantics(
    buildRecordsFrame(69n, 70n, [makeRecord('TCH_WALL', WALL_HANDLE, wallWriter(WALL))]),
  )).rows;
  const yans = yansFrom(69n, 70n, rows);

  assert.throws(
    () => decodeSemanticsFrame(yans.slice(0, yans.byteLength - 3)),
    /vendor\.semantics\.truncated/,
  );
  assert.throws(
    () => decodeSemanticsFrame(yans.slice(0, 12)),
    /vendor\.semantics\.truncated/,
  );
  assert.throws(
    () => decodeSemanticsFrame(appendByte(yans)),
    /vendor\.semantics\.trailing_bytes/,
  );
});

test('the records-frame header rules of extractSemantics match decode', () => {
  const record = makeRecord('TCH_WALL', WALL_HANDLE, wallWriter(WALL));
  const frame = buildRecordsFrame(71n, 72n, [record]);
  assert.throws(() => extractSemantics(frame.slice(0, 20)), /vendor\.truncated/);
  assert.throws(
    () => extractSemantics(replaceUint32(frame, 4, 7)),
    /vendor\.unsupported_abi: 7/,
  );
  assert.throws(
    () => extractSemantics(appendByte(frame)),
    /vendor\.trailing_bytes/,
  );
});
