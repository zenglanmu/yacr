// Contract tests for the vendor plugin frame (ABI v1).
//
// These run with `node --test` (type stripping), against the **plugin source**
// that is built into `public/plugins/tch.js`. They pin the binary header, the
// token/generation echo, the records-frame validator, the v4 primitive result
// frame, and the two real base-6 decoders (`TCH_WALL` / `TCH_COLUMN`).

import assert from 'node:assert/strict';
import test from 'node:test';

import { RECORDS_MAGIC, RESULT_MAGIC, abiVersion, decode } from '../../../plugins/tch/src/index.ts';


import {
  BitWriter, RECORDS_HEADER_BYTES, buildRecordsFrame, columnWriter, makeRecord,
  openingWriter, textWriter, wallWriter,
} from './vendor-plugin-fixtures.ts';

// ---------------------------------------------------------------------------
// ABI v1 result frame parser
// ---------------------------------------------------------------------------

interface ParsedPoint {
  x: number;
  y: number;
  z: number;
}

interface ParsedPrimitive {
  kind: number;
  points?: ParsedPoint[];
  cx?: number; cy?: number; cz?: number; radius?: number; startAngle?: number; sweep?: number;
  origin?: ParsedPoint; height?: number; rotation?: number; wrapWidth?: number;
  attachment?: number; text?: string; block?: bigint; scale?: ParsedPoint;
}

interface ParsedEntity {
  handle: bigint;
  primitives: ParsedPrimitive[];
}

interface ParsedResult {
  magic: number;
  abi: number;
  token: bigint;
  generation: bigint;
  status: number;
  applied: number;
  diagnostics: string[];
  entityCount: number;
  entities: ParsedEntity[];
}

function readResult(buffer: ArrayBuffer): ParsedResult {
  const view = new DataView(buffer);
  const result: ParsedResult = {
    magic: view.getUint32(0, true),
    abi: view.getUint32(4, true),
    token: view.getBigUint64(8, true),
    generation: view.getBigUint64(16, true),
    status: view.getUint32(24, true),
    applied: view.getUint32(28, true),
    diagnostics: [],
    entityCount: 0,
    entities: [],
  };
  let offset = 36;
  const diagnosticCount = view.getUint32(32, true);
  const decoder = new TextDecoder();
  for (let index = 0; index < diagnosticCount; index += 1) {
    const length = view.getUint16(offset, true);
    offset += 2;
    result.diagnostics.push(decoder.decode(new Uint8Array(buffer, offset, length)));
    offset += length;
  }
  result.entityCount = view.getUint32(offset, true);
  offset += 4;
  for (let index = 0; index < result.entityCount; index += 1) {
    const handle = view.getBigUint64(offset, true);
    offset += 8;
    const primitiveCount = view.getUint32(offset, true);
    offset += 4;
    const primitives: ParsedPrimitive[] = [];
    for (let primitiveIndex = 0; primitiveIndex < primitiveCount; primitiveIndex += 1) {
      const kind = view.getUint8(offset);
      offset += 1;
      if (kind === 0 || kind === 1) {
        const pointCount = view.getUint32(offset, true);
        offset += 4;
        const points: ParsedPoint[] = [];
        for (let pointIndex = 0; pointIndex < pointCount; pointIndex += 1) {
          points.push({
            x: view.getFloat64(offset, true),
            y: view.getFloat64(offset + 8, true),
            z: view.getFloat64(offset + 16, true),
          });
          offset += 24;
        }
        primitives.push({ kind, points });
      } else if (kind === 2) {
        primitives.push({
          kind,
          cx: view.getFloat64(offset, true),
          cy: view.getFloat64(offset + 8, true),
          cz: view.getFloat64(offset + 16, true),
          radius: view.getFloat64(offset + 24, true),
          startAngle: view.getFloat64(offset + 32, true),
          sweep: view.getFloat64(offset + 40, true),
        });
        offset += 48;
      } else if (kind === 3) {
        primitives.push({
          kind,
          cx: view.getFloat64(offset, true),
          cy: view.getFloat64(offset + 8, true),
          cz: view.getFloat64(offset + 16, true),
          radius: view.getFloat64(offset + 24, true),
        });
        offset += 32;
      } else if (kind === 4) {
        const origin = {
          x: view.getFloat64(offset, true),
          y: view.getFloat64(offset + 8, true),
          z: view.getFloat64(offset + 16, true),
        };
        const textLength = view.getUint16(offset + 49, true);
        primitives.push({
          kind,
          origin,
          height: view.getFloat64(offset + 24, true),
          rotation: view.getFloat64(offset + 32, true),
          wrapWidth: view.getFloat64(offset + 40, true),
          attachment: view.getUint8(offset + 48),
          text: decoder.decode(new Uint8Array(buffer, offset + 51, textLength)),
        });
        offset += 51 + textLength;
      } else if (kind === 5) {
        primitives.push({
          kind,
          block: view.getBigUint64(offset, true),
          origin: {
            x: view.getFloat64(offset + 8, true),
            y: view.getFloat64(offset + 16, true),
            z: view.getFloat64(offset + 24, true),
          },
          scale: {
            x: view.getFloat64(offset + 32, true),
            y: view.getFloat64(offset + 40, true),
            z: view.getFloat64(offset + 48, true),
          },
          rotation: view.getFloat64(offset + 56, true),
        });
        offset += 64;
      } else {
        throw new Error(`unexpected primitive kind ${kind}`);
      }
    }
    result.entities.push({ handle, primitives });
  }
  assert.equal(offset, buffer.byteLength, 'result frame must be consumed exactly');
  return result;
}

function close(actual: number, expected: number): void {
  assert.ok(
    Math.abs(actual - expected) < 1e-9,
    `expected ${expected}, got ${actual}`,
  );
}

function closePoint(actual: ParsedPoint, expected: ParsedPoint): void {
  close(actual.x, expected.x);
  close(actual.y, expected.y);
  close(actual.z, expected.z);
}

function samePoint(a: ParsedPoint, b: ParsedPoint): boolean {
  return Math.abs(a.x - b.x) < 1e-9 && Math.abs(a.y - b.y) < 1e-9 && Math.abs(a.z - b.z) < 1e-9;
}

/** True when the segment list contains `a -> b` (either direction). */
function hasSegment(points: readonly ParsedPoint[], a: ParsedPoint, b: ParsedPoint): boolean {
  for (let index = 0; index + 1 < points.length; index += 2) {
    if (samePoint(points[index], a) && samePoint(points[index + 1], b)) return true;
    if (samePoint(points[index], b) && samePoint(points[index + 1], a)) return true;
  }
  return false;
}

function hasDiagnostic(result: ParsedResult, prefix: string): boolean {
  return result.diagnostics.some(message => message.startsWith(prefix));
}

// ---------------------------------------------------------------------------
// Frame-level tests
// ---------------------------------------------------------------------------

test('an empty v1 batch settles as unsupported without primitives', () => {
  const result = readResult(decode(buildRecordsFrame(7n, 7n, [])));
  assert.equal(result.magic, RESULT_MAGIC);
  assert.equal(result.abi, abiVersion);
  assert.equal(result.abi, 1);
  assert.equal(result.token, 7n);
  assert.equal(result.generation, 7n);
  assert.equal(result.status, 1);
  assert.equal(result.applied, 0);
  assert.equal(result.entityCount, 0);
  assert.deepEqual(result.diagnostics, ['vendor.decoder_no_supported_entities']);
});

test('a header that claims records but carries none is truncated', () => {
  const buffer = new ArrayBuffer(RECORDS_HEADER_BYTES);
  const view = new DataView(buffer);
  view.setUint32(0, RECORDS_MAGIC, true);
  view.setUint32(4, abiVersion, true);
  view.setBigUint64(8, 9n, true);
  view.setBigUint64(16, 9n, true);
  view.setUint32(24, 3, true);
  assert.throws(() => decode(buffer), /vendor\.truncated/);
});

test('a records frame cut inside a record is truncated, not unsupported', () => {
  const wall = makeRecord('TCH_WALL', 0x2cn, wallWriter({
    start: [0, 0, 0],
    end: [10, 0, 0],
    left: 1,
    right: 2,
    height: 3,
    joins: [1, 2, 3, 4],
  }));
  const full = buildRecordsFrame(12n, 12n, [wall]);
  assert.throws(() => decode(full.slice(0, full.byteLength - 3)), /vendor\.truncated/);
});

test('trailing bytes after the last record are rejected', () => {
  const wall = makeRecord('TCH_WALL', 0x2cn, wallWriter({
    start: [0, 0, 0],
    end: [10, 0, 0],
    left: 1,
    right: 2,
    height: 3,
  }));
  const full = buildRecordsFrame(13n, 13n, [wall]);
  const extended = new Uint8Array(full.byteLength + 1);
  extended.set(new Uint8Array(full));
  assert.throws(() => decode(extended.buffer), /vendor\.trailing_bytes/);
});

test('a truncated or mis-magicked frame fails loudly', () => {
  assert.throws(() => decode(new ArrayBuffer(4)), /vendor\.truncated/);
  const wrong = buildRecordsFrame(1n, 1n, []);
  new DataView(wrong).setUint32(0, 0, true);
  assert.throws(() => decode(wrong), /vendor\.bad_magic/);
});

test('a records frame with a foreign ABI is refused', () => {
  const foreign = buildRecordsFrame(2n, 2n, [], 2);
  assert.throws(() => decode(foreign), /vendor\.unsupported_abi: 2/);
});

// ---------------------------------------------------------------------------
// TCH_WALL
// ---------------------------------------------------------------------------

test('a base-6 wall decodes to two side lines and two end caps (bit-aligned skip)', () => {
  const body = wallWriter({
    start: [0, 0, 0],
    end: [10, 0, 0],
    left: 1,
    right: 2,
    height: 3,
    joins: [1, 2, 3, 4],
  });
  // Prepend 3 common-data bits so the class body starts at a non-byte offset.
  const main = new BitWriter();
  main.pushBits(0b101, 3);
  for (const bit of body.bits) main.pushBit(bit);
  const wall = makeRecord('TCH_WALL', 0x3e868n, main, { mainSkipBits: 3 });

  const result = readResult(decode(buildRecordsFrame(21n, 21n, [wall])));
  assert.equal(result.status, 0);
  assert.equal(result.applied, 1);
  assert.equal(result.entityCount, 1);
  assert.equal(result.entities[0].handle, 0x3e868n);
  assert.equal(result.entities[0].primitives.length, 1);
  assert.ok(hasDiagnostic(result, 'vendor.tch.decoded: TCH_WALL=1'));

  const primitive = result.entities[0].primitives[0];
  assert.equal(primitive.kind, 1);
  const points = primitive.points ?? [];
  assert.equal(points.length, 8);

  const p0 = { x: 1, y: 1, z: 0 };
  const p1 = { x: 8, y: 1, z: 0 };
  const p2 = { x: 6, y: -2, z: 0 };
  const p3 = { x: 3, y: -2, z: 0 };
  closePoint(points[0], p0); // left side, start
  closePoint(points[1], p1); // left side, end
  closePoint(points[2], p3); // right side, start
  closePoint(points[3], p2); // right side, end
  closePoint(points[4], p0); // start cap
  closePoint(points[5], p3);
  closePoint(points[6], p1); // end cap
  closePoint(points[7], p2);
});

test('wall protection marks 7 and 8 decode like mark 9', () => {
  const options = {
    start: [0, 0, 0] as const,
    end: [10, 0, 0] as const,
    left: 1,
    right: 2,
    height: 3,
    joins: [1, 2, 3, 4] as const,
  };
  const expected = readResult(decode(buildRecordsFrame(26n, 26n, [
    makeRecord('TCH_WALL', 1n, wallWriter({ ...options, mark: 9 })),
  ]))).entities[0].primitives[0].points;
  for (const mark of [7, 8]) {
    const wall = makeRecord('TCH_WALL', BigInt(mark), wallWriter({ ...options, mark }));
    const result = readResult(decode(buildRecordsFrame(27n, 27n, [wall])));
    assert.equal(result.status, 0, `mark ${mark} must decode`);
    assert.deepEqual(result.entities[0].primitives[0].points, expected);
  }
});

test('a wall declaring openings with no matching records stays continuous', () => {
  const wall = makeRecord('TCH_WALL', 0x3e869n, wallWriter({
    start: [0, 0, 0],
    end: [10, 0, 0],
    left: 1,
    right: 1,
    height: 3,
    joins: [0, 0, 0, 0],
    openings: 2,
  }));
  const result = readResult(decode(buildRecordsFrame(22n, 22n, [wall])));
  assert.equal(result.status, 0);
  assert.equal(result.applied, 1);
  assert.equal((result.entities[0].primitives[0].points ?? []).length, 8);
  assert.ok(hasDiagnostic(result, 'vendor.tch.wall_openings_unmatched: 1'));
  assert.ok(!hasDiagnostic(result, 'vendor.tch.opening_cuts'));
});

test('a decoded opening splits both wall sides and adds jamb segments', () => {
  const wallHandle = 0x3e868n;
  const wall = makeRecord('TCH_WALL', wallHandle, wallWriter({
    start: [0, 0, 0],
    end: [10, 0, 0],
    left: 1,
    right: 2,
    height: 3,
    joins: [0, 0, 0, 0],
    openings: 1,
  }));
  const handles = new BitWriter();
  const openingMain = openingWriter({ position: [5, 0, 0], width: 2, wall: wallHandle, handles });
  const opening = makeRecord('TCH_OPENING', 0x3e8afn, openingMain, { handle: handles });

  const result = readResult(decode(buildRecordsFrame(28n, 28n, [wall, opening])));
  assert.equal(result.status, 0);
  assert.equal(result.applied, 1); // openings are cuts, not entities
  assert.equal(result.entityCount, 1);
  const points = result.entities[0].primitives[0].points ?? [];
  assert.equal(points.length, 16); // 8 segment pairs

  const p0 = { x: 0, y: 1, z: 0 };
  const p1 = { x: 10, y: 1, z: 0 };
  const p2 = { x: 10, y: -2, z: 0 };
  const p3 = { x: 0, y: -2, z: 0 };
  const leftA = { x: 4, y: 1, z: 0 };
  const leftB = { x: 6, y: 1, z: 0 };
  const rightA = { x: 4, y: -2, z: 0 };
  const rightB = { x: 6, y: -2, z: 0 };

  assert.ok(hasSegment(points, p0, leftA), 'left side before the opening');
  assert.ok(hasSegment(points, leftB, p1), 'left side after the opening');
  assert.ok(hasSegment(points, p3, rightA), 'right side before the opening');
  assert.ok(hasSegment(points, rightB, p2), 'right side after the opening');
  assert.ok(hasSegment(points, leftA, rightA), 'jamb at the start edge');
  assert.ok(hasSegment(points, leftB, rightB), 'jamb at the end edge');
  assert.ok(hasSegment(points, p0, p3), 'start cap');
  assert.ok(hasSegment(points, p1, p2), 'end cap');

  assert.ok(hasDiagnostic(result, 'vendor.tch.opening_cuts: 1'));
  assert.ok(hasDiagnostic(result, 'vendor.tch.opening_block_missing: 1'));
  assert.ok(!hasDiagnostic(result, 'vendor.tch.wall_openings_unmatched'));
  assert.ok(!hasDiagnostic(result, 'vendor.tch.unsupported'));
});

test('an opening with a mismatched wall handle leaves the wall continuous', () => {
  const wall = makeRecord('TCH_WALL', 0x3e868n, wallWriter({
    start: [0, 0, 0],
    end: [10, 0, 0],
    left: 1,
    right: 2,
    height: 3,
    joins: [0, 0, 0, 0],
    openings: 1,
  }));
  const handles = new BitWriter();
  const openingMain = openingWriter({ position: [5, 0, 0], width: 2, wall: 0x999n, handles });
  const opening = makeRecord('TCH_OPENING', 0x3e8b0n, openingMain, { handle: handles });

  const result = readResult(decode(buildRecordsFrame(29n, 29n, [wall, opening])));
  assert.equal(result.status, 0);
  assert.equal(result.applied, 1);
  assert.equal((result.entities[0].primitives[0].points ?? []).length, 8);
  assert.ok(hasDiagnostic(result, 'vendor.tch.wall_openings_unmatched: 1'));
  assert.ok(hasDiagnostic(result, 'vendor.tch.opening_wall_missing: 1'));
  assert.ok(hasDiagnostic(result, 'vendor.tch.opening_block_missing: 1'));
  assert.ok(!hasDiagnostic(result, 'vendor.tch.unsupported'));
});

test('an opening with a null wall handle is skipped as unsupported', () => {
  const handles = new BitWriter();
  const openingMain = openingWriter({ position: [5, 0, 0], width: 2, wall: 0n, handles });
  const opening = makeRecord('TCH_OPENING', 0x3e8b1n, openingMain, { handle: handles });
  const result = readResult(decode(buildRecordsFrame(30n, 30n, [opening])));
  assert.equal(result.status, 1);
  assert.equal(result.applied, 0);
  assert.ok(hasDiagnostic(result, 'vendor.tch.opening_no_wall: 1'));
  assert.ok(hasDiagnostic(result, 'vendor.tch.unsupported: TCH_OPENING=1'));
});

test('a door emits a kind-5 block insert with mirror and angle', () => {
  const handles = new BitWriter();
  const main = openingWriter({
    position: [10, 20, 0], width: 2, wall: 0x1n, handles,
    block2d: 0xABCDn, mirror: 0, kind: 0, angle: 0.5,
  });
  const opening = makeRecord('TCH_OPENING', 0x200n, main, { handle: handles });
  const result = readResult(decode(buildRecordsFrame(54n, 54n, [opening])));
  assert.equal(result.status, 0);
  assert.equal(result.applied, 1);
  const primitive = result.entities[0].primitives[0];
  assert.equal(primitive.kind, 5);
  assert.equal(primitive.block, 0xABCDn);
  closePoint(primitive.origin as ParsedPoint, { x: 10, y: 20, z: 0 });
  closePoint(primitive.scale as ParsedPoint, { x: 2, y: 2, z: 2 });
  close(primitive.rotation ?? 0, 0.5);
  assert.ok(hasDiagnostic(result, 'vendor.tch.opening_symbols: 1'));
});

test('a mirrored door flips both block scales', () => {
  const handles = new BitWriter();
  const main = openingWriter({
    position: [0, 0, 0], width: 2, wall: 0x1n, handles,
    block2d: 0xABCDn, mirror: 2, kind: 0,
  });
  const opening = makeRecord('TCH_OPENING', 0x201n, main, { handle: handles });
  const result = readResult(decode(buildRecordsFrame(55n, 55n, [opening])));
  const primitive = result.entities[0].primitives[0];
  closePoint(primitive.scale as ParsedPoint, { x: -2, y: -2, z: 2 });
});

test('a window uses the owning wall thickness for its block depth', () => {
  const wallHandle = 0x300n;
  const wall = makeRecord('TCH_WALL', wallHandle, wallWriter({
    start: [0, 0, 0], end: [10, 0, 0], left: 1, right: 2,
    height: 3, joins: [0, 0, 0, 0], openings: 1,
  }));
  const handles = new BitWriter();
  const main = openingWriter({
    position: [5, 0, 0], width: 2, thickness: 0.24, wall: wallHandle, handles,
    block2d: 0xABCDn, kind: 1,
  });
  const opening = makeRecord('TCH_OPENING', 0x301n, main, { handle: handles });
  const result = readResult(decode(buildRecordsFrame(56n, 56n, [wall, opening])));
  assert.equal(result.applied, 2);
  const entity = result.entities.find(item => item.handle === 0x301n);
  assert.ok(entity);
  closePoint(entity.primitives[0].scale as ParsedPoint, { x: 2, y: 3, z: 2 });
});

test('a window with no decoded wall falls back to its own thickness', () => {
  const handles = new BitWriter();
  const main = openingWriter({
    position: [0, 0, 0], width: 2, thickness: 0.24, wall: 0x999n, handles,
    block2d: 0xABCDn, kind: 1,
  });
  const opening = makeRecord('TCH_OPENING', 0x302n, main, { handle: handles });
  const result = readResult(decode(buildRecordsFrame(57n, 57n, [opening])));
  assert.equal(result.status, 0);
  assert.equal(result.applied, 1);
  closePoint(result.entities[0].primitives[0].scale as ParsedPoint, { x: 2, y: 0.24, z: 2 });
  assert.ok(hasDiagnostic(result, 'vendor.tch.opening_wall_missing: 1'));
});

test('a hole emits jamb segments across both wall faces', () => {
  const wallHandle = 0x400n;
  const wall = makeRecord('TCH_WALL', wallHandle, wallWriter({
    start: [0, 0, 0], end: [10, 0, 0], left: 1, right: 2,
    height: 3, joins: [0, 0, 0, 0], openings: 1,
  }));
  const handles = new BitWriter();
  const main = openingWriter({
    position: [5, 0, 0], width: 2, wall: wallHandle, handles, kind: 2,
  });
  const opening = makeRecord('TCH_OPENING', 0x401n, main, { handle: handles });
  const result = readResult(decode(buildRecordsFrame(58n, 58n, [wall, opening])));
  assert.equal(result.applied, 2);
  const entity = result.entities.find(item => item.handle === 0x401n);
  assert.ok(entity);
  const primitive = entity.primitives[0];
  assert.equal(primitive.kind, 1);
  const points = primitive.points ?? [];
  assert.equal(points.length, 4);
  closePoint(points[0], { x: 4, y: 1, z: 0 });
  closePoint(points[1], { x: 4, y: -2, z: 0 });
  closePoint(points[2], { x: 6, y: 1, z: 0 });
  closePoint(points[3], { x: 6, y: -2, z: 0 });
  assert.ok(hasDiagnostic(result, 'vendor.tch.opening_holes: 1'));
});

test('a group opening without its part payload fails loudly', () => {
  const handles = new BitWriter();
  const main = openingWriter({
    position: [0, 0, 0], width: 2, wall: 0x999n, handles, kind: 8,
  });
  const opening = makeRecord('TCH_OPENING', 0x402n, main, { handle: handles });
  const result = readResult(decode(buildRecordsFrame(59n, 59n, [opening])));
  assert.equal(result.status, 1);
  assert.equal(result.applied, 0);
  assert.equal(result.entityCount, 0);
  assert.ok(hasDiagnostic(result, 'vendor.tch.opening_truncated: 1'));
  assert.ok(!result.entities.some(item => item.handle === 0x402n));
});

test('an arc wall is skipped with an explicit diagnostic', () => {
  const wall = makeRecord('TCH_WALL', 0x3e86an, wallWriter({
    start: [0, 0, 0],
    end: [10, 0, 0],
    angle: 0.5,
    left: 1,
    right: 1,
    height: 3,
  }));
  const result = readResult(decode(buildRecordsFrame(23n, 23n, [wall])));
  assert.equal(result.status, 1);
  assert.equal(result.applied, 0);
  assert.equal(result.entityCount, 0);
  assert.ok(hasDiagnostic(result, 'vendor.tch.wall_arc_unsupported: 1'));
  assert.ok(hasDiagnostic(result, 'vendor.decoder_no_supported_entities'));
});

test('a profile-polyline wall is skipped with an explicit diagnostic', () => {
  const wall = makeRecord('TCH_WALL', 0x3e86bn, wallWriter({
    start: [0, 0, 0],
    end: [10, 0, 0],
    flags: 0x12,
    left: 1,
    right: 1,
    height: 3,
  }));
  const result = readResult(decode(buildRecordsFrame(24n, 24n, [wall])));
  assert.equal(result.status, 1);
  assert.ok(hasDiagnostic(result, 'vendor.tch.wall_profile_unsupported: 1'));
});

test('a malformed wall is skipped, never emitted with guessed geometry', () => {
  const body = wallWriter({
    start: [0, 0, 0],
    end: [10, 0, 0],
    left: 1,
    right: 2,
    height: 3,
  });
  // Structurally valid frame, but the record claims its main stream ends in the
  // middle of the header (the reader must throw `vendor.truncated`).
  const wall = makeRecord('TCH_WALL', 0x3e86cn, body, { mainBits: 60 });
  const result = readResult(decode(buildRecordsFrame(25n, 25n, [wall])));
  assert.equal(result.status, 1);
  assert.equal(result.applied, 0);
  assert.equal(result.entityCount, 0);
  assert.ok(hasDiagnostic(result, 'vendor.tch.wall_truncated: 1'));
  assert.ok(hasDiagnostic(result, 'vendor.decoder_no_supported_entities'));
});

// ---------------------------------------------------------------------------
// TCH_COLUMN
// ---------------------------------------------------------------------------

test('a base-6 column with four vertices decodes to a closed polyline', () => {
  const column = makeRecord('TCH_COLUMN', 0x57a50n, columnWriter({
    position: [100.5, -200.25, 5],
    vertices: [
      { x: 0, y: 0 },
      { x: 10, y: 0 },
      { x: 10, y: 10 },
      { x: 0, y: 10 },
    ],
  }));
  const result = readResult(decode(buildRecordsFrame(31n, 31n, [column])));
  assert.equal(result.status, 0);
  assert.equal(result.applied, 1);
  assert.equal(result.entityCount, 1);
  assert.equal(result.entities[0].handle, 0x57a50n);
  assert.ok(hasDiagnostic(result, 'vendor.tch.decoded: TCH_COLUMN=1'));

  const primitive = result.entities[0].primitives[0];
  assert.equal(primitive.kind, 0);
  const points = primitive.points ?? [];
  assert.equal(points.length, 5);
  closePoint(points[0], { x: 0, y: 0, z: 5 });
  closePoint(points[1], { x: 10, y: 0, z: 5 });
  closePoint(points[2], { x: 10, y: 10, z: 5 });
  closePoint(points[3], { x: 0, y: 10, z: 5 });
  closePoint(points[4], points[0]); // closed loop repeats the first point
});

test('a bulged column edge becomes an arc, not a dropped segment', () => {
  const column = makeRecord('TCH_COLUMN', 0x57a51n, columnWriter({
    position: [0, 0, 2],
    vertices: [
      { x: 0, y: 0, bulge: 1 },
      { x: 10, y: 0 },
      { x: 10, y: 10 },
      { x: 0, y: 10 },
    ],
  }));
  const result = readResult(decode(buildRecordsFrame(32n, 32n, [column])));
  assert.equal(result.status, 0);
  assert.equal(result.applied, 2);
  const kinds = result.entities[0].primitives.map(primitive => primitive.kind).sort();
  assert.deepEqual(kinds, [0, 2]);

  const arc = result.entities[0].primitives.find(primitive => primitive.kind === 2);
  assert.ok(arc);
  close(arc.cz ?? Number.NaN, 2);
  close(arc.radius ?? Number.NaN, 5);
  close(arc.cx ?? Number.NaN, 5);
  close(arc.cy ?? Number.NaN, 0);
  // atan2 has ±π at the branch cut; both describe the same semicircle.
  close(Math.abs(arc.startAngle ?? Number.NaN), Math.PI);
  close(arc.sweep ?? Number.NaN, Math.PI);

  const polyline = result.entities[0].primitives.find(primitive => primitive.kind === 0);
  assert.ok(polyline);
  assert.equal((polyline.points ?? []).length, 4);
});

test('a column property bag consumes text names/values and still decodes', () => {
  const text = new BitWriter();
  const main = columnWriter({
    position: [1, 2, 3],
    vertices: [
      { x: 0, y: 0 },
      { x: 4, y: 0 },
      { x: 4, y: 4 },
    ],
    text,
    bag: [
      { name: 'TCH_NAME', type: 0x65, value: 'hello' },
      { name: 'TCH_NUMBER', type: 200, number: 1.5 },
    ],
  });
  const column = makeRecord('TCH_COLUMN', 0x57a52n, main, { text });
  const result = readResult(decode(buildRecordsFrame(33n, 33n, [column])));
  assert.equal(result.status, 0);
  assert.equal(result.applied, 1);
  assert.equal(result.entities[0].primitives[0].kind, 0);
});

test('a base-6 TCH_TEXT decodes to a kind-4 text primitive', () => {
  const text = new BitWriter();
  const main = textWriter({
    className: 'TCH_TEXT',
    value: 'Hello',
    position: [10, 20, 30],
    height: 3,
    rotation: 0.5,
    attachment: 7,
    scale: 2,
    text,
    // Base property-bag strings are counted, not read, during the base walk.
    bag: [{ name: 'TCH_NAME', type: 0x65, value: 'ignored' }],
  });
  const record = makeRecord('TCH_TEXT', 0x100n, main, { text });
  const result = readResult(decode(buildRecordsFrame(51n, 51n, [record])));
  assert.equal(result.status, 0);
  assert.equal(result.applied, 1);
  assert.equal(result.entities[0].handle, 0x100n);
  const primitive = result.entities[0].primitives[0];
  assert.equal(primitive.kind, 4);
  closePoint(primitive.origin as ParsedPoint, { x: 10, y: 20, z: 30 });
  close(primitive.height ?? 0, 6);
  close(primitive.rotation ?? 0, 0.5);
  close(primitive.wrapWidth ?? 1, 0);
  assert.equal(primitive.attachment, 7);
  assert.equal(primitive.text, 'Hello');
  assert.ok(hasDiagnostic(result, 'vendor.tch.decoded: TCH_TEXT=1'));
});

test('a base-6 TCH_MTEXT scales height and wrap width', () => {
  const text = new BitWriter();
  const main = textWriter({
    className: 'TCH_MTEXT',
    value: 'Line1\\PLine2',
    position: [1, 2, 3],
    height: 2.5,
    rotation: 0,
    attachment: 1,
    scale: 2,
    text,
    boxWidth: 10,
  });
  const record = makeRecord('TCH_MTEXT', 0x101n, main, { text });
  const result = readResult(decode(buildRecordsFrame(52n, 52n, [record])));
  assert.equal(result.status, 0);
  assert.equal(result.applied, 1);
  const primitive = result.entities[0].primitives[0];
  assert.equal(primitive.kind, 4);
  assert.equal(primitive.text, 'Line1\\PLine2');
  close(primitive.height ?? 0, 5);
  close(primitive.wrapWidth ?? 0, 20);
  assert.equal(primitive.attachment, 1);
});

test('TCH_MTEXTNEW is counted unsupported', () => {
  const record = makeRecord('TCH_MTEXTNEW', 0x102n, new BitWriter());
  record.main = new Uint8Array([0x06]);
  record.mainBits = 8;
  const result = readResult(decode(buildRecordsFrame(53n, 53n, [record])));
  assert.equal(result.status, 1);
  assert.equal(result.applied, 0);
  assert.ok(hasDiagnostic(result, 'vendor.tch.unsupported: TCH_MTEXTNEW=1'));
});

// ---------------------------------------------------------------------------
// Unsupported classes / aggregation
// ---------------------------------------------------------------------------

test('an unsupported class is counted in diagnostics and never applied', () => {
  const record = makeRecord('TCH_DIMENSION', 0x3e8afn, new BitWriter());
  record.main = new Uint8Array([0x06]);
  record.mainBits = 8;
  const result = readResult(decode(buildRecordsFrame(41n, 41n, [record])));
  assert.equal(result.status, 1);
  assert.equal(result.applied, 0);
  assert.equal(result.entityCount, 0);
  assert.ok(hasDiagnostic(result, 'vendor.tch.unsupported: TCH_DIMENSION=1'));
  assert.ok(hasDiagnostic(result, 'vendor.decoder_no_supported_entities'));
});

test('decoded and unsupported counts aggregate in one result frame', () => {
  const wall = makeRecord('TCH_WALL', 0x3e868n, wallWriter({
    start: [0, 0, 0],
    end: [10, 0, 0],
    left: 1,
    right: 2,
    height: 3,
  }));
  const record = makeRecord('TCH_DIMENSION', 0x3e8afn, new BitWriter());
  record.main = new Uint8Array([0x06]);
  record.mainBits = 8;
  const result = readResult(decode(buildRecordsFrame(42n, 42n, [wall, record])));
  assert.equal(result.status, 0);
  assert.equal(result.applied, 1);
  assert.equal(result.entityCount, 1);
  assert.equal(result.entities[0].handle, 0x3e868n);
  assert.ok(hasDiagnostic(result, 'vendor.tch.decoded: TCH_WALL=1'));
  assert.ok(hasDiagnostic(result, 'vendor.tch.unsupported: TCH_DIMENSION=1'));
  assert.ok(!hasDiagnostic(result, 'vendor.decoder_no_supported_entities'));
});
