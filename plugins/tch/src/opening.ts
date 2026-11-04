// `TCH_OPENING`: base-6 fields, owning wall, 2D block and symbol kind.
import { MAX_POINT_ABS, reader, skipPropertyBag } from './bit-reader.ts';
import type { Point3 } from './bit-reader.ts';
import { add, bump } from './diagnostics.ts';
import type { Primitive } from './types.ts';
import type { WallData } from './wall.ts';
import type { VendorRecord } from './types.ts';

export interface OpeningCut { x: number; y: number; width: number }

export interface OpeningData {
  wall: bigint;
  cut: OpeningCut;
  block2d: bigint;
  kind: number;
  mirror: number;
  angle: number;
  width: number;
  thickness: number;
  height: number;
  position: Point3;
  version: number;
}

export function decodeOpening(
  record: VendorRecord,
  reasons: Map<string, number>,
): OpeningData | null {
  const main = reader(record.main, record.mainBits, record.mainSkipBits);

  const base = main.readByte();
  if (base !== 6) {
    bump(reasons, 'vendor.tch.opening_unsupported_base');
    return null;
  }
  const scale = main.readBitDouble();
  main.readBitDouble(); // second value of ReadTchBase (ignored)
  const flags = main.readByte();
  let handleCount = 0;
  if ((flags & 1) !== 0) handleCount += main.readByte();
  if ((flags & 2) !== 0) handleCount += main.readByte();
  if (!Number.isFinite(scale) || scale <= 0) {
    bump(reasons, 'vendor.tch.opening_header_mismatch');
    return null;
  }
  if (record.dwgVersion < 1021) {
    bump(reasons, 'vendor.tch.opening_text_stream_unsupported');
    return null;
  }
  handleCount += skipPropertyBag(main, null, 0).handles;

  const version = main.readByte();
  if (version < 2 || version > 5) {
    bump(reasons, 'vendor.tch.opening_unsupported_version');
    return null;
  }
  const position = main.read3BitDouble();
  const width = main.readBitDouble();
  const thickness = main.readBitDouble();
  const height = main.readBitDouble();
  const angle = main.readBitDouble();
  const mirror = main.readByte();
  const kind = main.readByte();
  if (
    !Number.isFinite(position.x) ||
    !Number.isFinite(position.y) ||
    !Number.isFinite(position.z) ||
    !Number.isFinite(width) ||
    width <= 0 ||
    Math.abs(position.x) > MAX_POINT_ABS ||
    Math.abs(position.y) > MAX_POINT_ABS ||
    width > MAX_POINT_ABS
  ) {
    bump(reasons, 'vendor.tch.opening_invalid');
    return null;
  }

  // Base handles first, then 2D block, 3D block and the owning wall.
  const handles = reader(record.handleStream, record.handleBits, record.handleSkipBits);
  for (let index = 0; index < handleCount; index += 1) handles.readHandle(record.handle);
  const block2d = handles.readHandle(record.handle);
  handles.readHandle(record.handle);
  const wall = handles.readHandle(record.handle);
  if (wall === 0n) {
    bump(reasons, 'vendor.tch.opening_no_wall');
    return null;
  }
  return {
    wall,
    cut: { x: position.x, y: position.y, width },
    block2d,
    kind,
    mirror,
    angle,
    width,
    thickness,
    height,
    position,
    version,
  };
}

/** Emit the real 2D symbol (block insert) or hole jamb lines for one opening. */
export function openingPrimitives(
  opening: OpeningData,
  wall: WallData | undefined,
  reasons: Map<string, number>,
): Primitive[] {
  const { block2d, kind, mirror, angle, width, thickness, position } = opening;

  if (kind === 0 || kind === 1) {
    if (block2d === 0n) {
      bump(reasons, 'vendor.tch.opening_block_missing');
      return [];
    }
    let depth: number;
    if (kind === 0) depth = width;
    else if (wall) depth = wall.left + wall.right;
    else if (thickness > 0) depth = thickness;
    else {
      bump(reasons, 'vendor.tch.opening_window_depth_unknown');
      return [];
    }
    if (
      !Number.isFinite(depth) ||
      depth <= 0 ||
      !Number.isFinite(angle) ||
      !Number.isFinite(position.x) ||
      !Number.isFinite(position.y)
    ) {
      bump(reasons, 'vendor.tch.opening_invalid');
      return [];
    }
    const sx = mirror === 1 || mirror === 2 ? -width : width;
    const sy = mirror === 2 || mirror === 3 ? -depth : depth;
    add(reasons, 'vendor.tch.opening_symbols', 1);
    return [{
      kind: 5,
      block: block2d,
      origin: { x: position.x, y: position.y, z: 0 },
      scale: { x: sx, y: sy, z: width },
      rotation: angle,
    }];
  }

  if (kind === 2) {
    const ux = Math.cos(angle);
    const uy = Math.sin(angle);
    const nx = -uy;
    const ny = ux;
    const left = wall ? wall.left : thickness / 2;
    const right = wall ? wall.right : thickness / 2;
    const values = [ux, uy, nx, ny, left, right, width, position.x, position.y];
    if (!values.every(Number.isFinite)) {
      bump(reasons, 'vendor.tch.opening_invalid');
      return [];
    }
    const points: Point3[] = [];
    for (const t of [-width / 2, width / 2]) {
      points.push({
        x: position.x + ux * t + nx * left,
        y: position.y + uy * t + ny * left,
        z: 0,
      });
      points.push({
        x: position.x + ux * t - nx * right,
        y: position.y + uy * t - ny * right,
        z: 0,
      });
    }
    add(reasons, 'vendor.tch.opening_holes', 1);
    return [{ kind: 1, points }];
  }

  if (kind === 5) bump(reasons, 'vendor.tch.opening_symbol_band_unsupported');
  else if (kind === 8) bump(reasons, 'vendor.tch.opening_symbol_group_unsupported');
  else bump(reasons, 'vendor.tch.opening_kind_unsupported');
  return [];
}
