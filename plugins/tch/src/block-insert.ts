// `TCH_BLOCK_INSERT`: the `AcDbBlockReference`-style prefix plus the Tianzheng
// version/mode tail. Emits the same kind-5 block primitive the door/window
// symbols use; the host resolves and expands the imported block definition.
// Mirror of MiniCAD `Tch::DecodeBlockInsert`.
import { MAX_POINT_ABS, reader } from './bit-reader.ts';
import { readBase6 } from './base6.ts';
import { bump } from './diagnostics.ts';
import type { Primitive, VendorRecord } from './types.ts';

const MIN_DWG_VERSION = 1015; // AC1015
const MAX_DWG_VERSION = 1032; // AC1032
const MAX_OWNED_ENTITIES = 100000;
const CLASS_VERSION_MAX = 4;
const NORMAL_EPSILON = 1e-12;

/**
 * Decode one block insert. Returns a single derived block primitive or `null`
 * for an unsupported version/layout; short reads throw `vendor.truncated`.
 */
export function decodeBlockInsert(
  record: VendorRecord,
  reasons: Map<string, number>,
): Primitive[] | null {
  if (record.dwgVersion < MIN_DWG_VERSION || record.dwgVersion > MAX_DWG_VERSION) {
    bump(reasons, 'vendor.tch.block_insert_unsupported_version');
    return null;
  }

  const main = reader(record.main, record.mainBits, record.mainSkipBits);
  const origin = main.read3BitDouble();

  // Scale packing: 2-bit mode selects which defaulting scheme applies. `x`
  // stays 1 for modes 1 and 3, so only the documented branches assign it.
  let x = 1;
  let y = 1;
  let z = 1;
  switch (main.readBits(2)) {
    case 0:
      x = main.readRawDouble();
      y = main.readBitDoubleWithDefault(x);
      z = main.readBitDoubleWithDefault(x);
      break;
    case 1:
      y = main.readBitDoubleWithDefault(1);
      z = main.readBitDoubleWithDefault(1);
      break;
    case 2:
      x = main.readRawDouble();
      y = x;
      z = x;
      break;
    default:
      break;
  }

  const rotation = main.readBitDouble();
  const normal = main.read3BitDouble();
  const attributes = main.readBit() !== 0;
  if (attributes && record.dwgVersion >= 1018) {
    const owned = main.readBitLong();
    if (owned < 0 || owned > MAX_OWNED_ENTITIES) {
      bump(reasons, 'vendor.tch.block_insert_layout');
      return null;
    }
  }
  const version = main.readByte();
  if (version > CLASS_VERSION_MAX) {
    bump(reasons, 'vendor.tch.block_insert_unsupported_version');
    return null;
  }
  // Mode 1 has database-dependent behavior; only mode 0 is reproducible.
  if (main.readByte() !== 0) {
    bump(reasons, 'vendor.tch.block_insert_layout');
    return null;
  }
  // Real AC1027 records append the TCH base-6 header after the inherited
  // block-reference prefix (220/220 in the fixture: base version 6, drawing
  // scale, empty property bag). MiniCAD only checks that the cursor stays
  // inside the stream; consuming the header keeps the exact-end check
  // meaningful instead of accepting arbitrary trailing bits.
  if (main.remaining > 0) {
    try {
      readBase6(main);
    } catch (error) {
      if (error instanceof Error && error.message === 'vendor.truncated') throw error;
      bump(reasons, 'vendor.tch.block_insert_layout');
      return null;
    }
  }
  if (main.remaining !== 0) {
    bump(reasons, 'vendor.tch.block_insert_layout');
    return null;
  }

  const handles = reader(record.handleStream, record.handleBits, record.handleSkipBits);
  const block = handles.readHandle(record.handle);
  if (block === 0n) {
    bump(reasons, 'vendor.tch.block_insert_no_block');
    return null;
  }

  const values = [
    origin.x, origin.y, origin.z,
    x, y, z, rotation,
    normal.x, normal.y, normal.z,
  ];
  if (
    !values.every(value => Number.isFinite(value) && Math.abs(value) <= MAX_POINT_ABS) ||
    x === 0 || y === 0 || z === 0 ||
    Math.hypot(normal.x, normal.y, normal.z) < NORMAL_EPSILON
  ) {
    // MiniCAD rejects these values outright; the layout is self-consistent but
    // unusable, so it maps onto the layout reason rather than inventing one.
    bump(reasons, 'vendor.tch.block_insert_layout');
    return null;
  }

  return [{ kind: 5, block, origin, scale: { x, y, z }, rotation }];
}
