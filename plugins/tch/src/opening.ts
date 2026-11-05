// `TCH_OPENING`: base-6 fields, owning wall, 2D block, band-window segments and
// composite (group) parts.
import { MAX_POINT_ABS, reader, skipPropertyBag } from './bit-reader.ts';
import type { BitReader, Point3 } from './bit-reader.ts';
import { consumeTexts, readBase6, ClassText } from './base6.ts';
import { add, bump } from './diagnostics.ts';
import { bulgeArc } from './geometry.ts';
import type { Primitive } from './types.ts';
import type { WallData } from './wall.ts';
import type { VendorRecord } from './types.ts';

export interface OpeningCut { x: number; y: number; width: number }

export interface OpeningSegment {
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  bulge: number;
}

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
  label: string;
  labelHeight: number;
  labelX: number;
  labelY: number;
  flags: number;
  segments: OpeningSegment[];
  /** Composite (`kind === 8`) children; empty for every other kind. */
  parts: OpeningData[];
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
  const baseFlags = main.readByte();
  let handleCount = 0;
  if ((baseFlags & 1) !== 0) handleCount += main.readByte();
  if ((baseFlags & 2) !== 0) handleCount += main.readByte();
  if (!Number.isFinite(scale) || scale <= 0) {
    bump(reasons, 'vendor.tch.opening_header_mismatch');
    return null;
  }
  if (record.dwgVersion < 1021) {
    bump(reasons, 'vendor.tch.opening_text_stream_unsupported');
    return null;
  }
  const bag = skipPropertyBag(main, null, 0);
  handleCount += bag.handles;

  // Synthetic contract records may omit the text stream entirely; real TCH
  // records always carry one (AC1027). A missing stream means no strings.
  const text = record.textBits > 0
    ? reader(record.text, record.textBits, record.textSkipBits)
    : null;
  if (text) consumeTexts(text, bag.texts);
  const classText = text ? new ClassText(text) : null;

  const fields = readOpeningFields(main, classText, reasons);
  if (fields === null || !validOpening(fields, reasons)) return null;

  const partFields: Array<{ fields: OpeningFields; handles: number }> = [];
  if (fields.kind === 8) {
    if (main.readByte() !== 0) {
      bump(reasons, 'vendor.tch.opening_group_version');
      return null;
    }
    const count = main.readBitLong();
    if (count < 1 || count > 64) {
      bump(reasons, 'vendor.tch.opening_group_count');
      return null;
    }
    for (let index = 0; index < count; index += 1) {
      // Embedded AcDbEntity common fields: exactly one known layout.
      const headerStart = main.position;
      let header = false;
      for (const prefix of GROUP_PART_PREFIXES) {
        main.position = headerStart;
        header = prefix.every(bit => main.readBit() === bit);
        if (header) break;
      }
      if (!header) {
        main.position = headerStart;
        bump(reasons, 'vendor.tch.opening_group_header');
        return null;
      }
      const partBase = readBase6(main);
      if (text) consumeTexts(text, partBase.texts);
      const part = readOpeningFields(main, classText, reasons);
      if (part === null || !validOpening(part, reasons)) return null;
      if (part.kind !== 0 && part.kind !== 1) {
        bump(reasons, 'vendor.tch.opening_group_part_kind');
        return null;
      }
      partFields.push({ fields: part, handles: partBase.handles });
    }
  }

  if (main.remaining !== 0) {
    bump(reasons, 'vendor.tch.opening_layout_mismatch');
    return null;
  }

  // Handle stream order: base handles, 2D block, 3D block, owning wall, style,
  // then four more. Empty handles still occupy a slot.
  const handles = reader(record.handleStream, record.handleBits, record.handleSkipBits);
  for (let index = 0; index < handleCount; index += 1) handles.readHandle(record.handle);
  const block2d = handles.readHandle(record.handle);
  handles.readHandle(record.handle);
  const wall = handles.readHandle(record.handle);
  if (wall === 0n) {
    bump(reasons, 'vendor.tch.opening_no_wall');
    return null;
  }
  handles.readHandle(record.handle); // style
  for (let index = 0; index < 4; index += 1) handles.readHandle(record.handle);

  // Group parts: `4 + partBase.handleCount` slots, then the part's own handles.
  const parts: OpeningData[] = [];
  for (const part of partFields) {
    for (let index = 0; index < 4 + part.handles; index += 1) handles.readHandle(record.handle);
    const partBlock2d = handles.readHandle(record.handle);
    handles.readHandle(record.handle);
    const partWall = handles.readHandle(record.handle);
    handles.readHandle(record.handle); // style
    for (let index = 0; index < 4; index += 1) handles.readHandle(record.handle);
    if (partBlock2d === 0n) {
      bump(reasons, 'vendor.tch.opening_group_part_block_missing');
      return null;
    }
    parts.push(buildOpening(part.fields, partBlock2d, partWall, []));
  }

  return buildOpening(fields, block2d, wall, parts);
}

interface OpeningFields {
  version: number;
  position: Point3;
  width: number;
  thickness: number;
  height: number;
  angle: number;
  mirror: number;
  kind: number;
  label: string;
  labelX: number;
  labelY: number;
  flags: number;
  labelHeight: number;
  segments: OpeningSegment[];
}

// Embedded AcDbEntity common fields. MiniCAD records the 37-bit sample below;
// the real fixture also has one composite record (`0xbc731`) whose children
// carry a 50-bit extension in the middle of that header (identical on all
// three parts). Both observed layouts are accepted exactly; anything else is
// rejected rather than guessed at.
const GROUP_PART_PREFIXES = [
  '0010101101111100000000000001000011101',
  '001010000000000100100001000000000000000000000000000000001101111100000000000001000011100',
].map(bits => [...bits].map(bit => (bit === '1' ? 1 : 0)));

/** MiniCAD `Detail::ReadOpeningFields` (version 2..5). */
function readOpeningFields(
  main: BitReader,
  classText: ClassText | null,
  reasons: Map<string, number>,
): OpeningFields | null {
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
  main.readBitDouble(); // unused
  const label = classText ? classText.next() : '';
  const labelX = main.readBitDouble();
  const labelY = main.readBitDouble();
  main.readBitDouble();
  const flags = main.readBitShort();
  if ((flags & 1) !== 0 || kind === 10) {
    main.readBitDouble();
    main.readBitDouble();
  }
  if (kind === 4) {
    main.readByte();
    main.readBitDouble();
    main.readBitDouble();
    main.readBitDouble();
    if (version > 4) main.readBitDouble();
  } else if (kind === 6 || kind === 7) {
    main.readBitDouble();
    if (kind === 6) main.readBitDouble();
  }
  const labelHeight = main.readBitDouble();
  main.readBitDouble();
  main.readBitDouble();
  main.readBitDouble();
  const segmentCount = main.readBitShort();
  if (segmentCount < 0 || segmentCount > 1024) {
    bump(reasons, 'vendor.tch.opening_layout_mismatch');
    return null;
  }
  const segments: OpeningSegment[] = [];
  for (let index = 0; index < segmentCount; index += 1) {
    segments.push({
      x1: main.readRawDouble(),
      y1: main.readRawDouble(),
      x2: main.readRawDouble(),
      y2: main.readRawDouble(),
      bulge: main.readBitDouble(),
    });
  }
  if (version > 2) {
    main.readBitLong();
    main.readBitShort();
    main.readBit();
    if (classText) classText.next();
    for (let index = 0; index < 4; index += 1) main.readBitDouble();
  }
  if (version > 3) main.readBitDouble();
  return {
    version,
    position,
    width,
    thickness,
    height,
    angle,
    mirror,
    kind,
    label,
    labelX,
    labelY,
    flags,
    labelHeight,
    segments,
  };
}

function validOpening(fields: OpeningFields, reasons: Map<string, number>): boolean {
  const { position, width } = fields;
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
    return false;
  }
  return true;
}

function buildOpening(
  fields: OpeningFields,
  block2d: bigint,
  wall: bigint,
  parts: OpeningData[],
): OpeningData {
  return {
    wall,
    cut: { x: fields.position.x, y: fields.position.y, width: fields.width },
    block2d,
    kind: fields.kind,
    mirror: fields.mirror,
    angle: fields.angle,
    width: fields.width,
    thickness: fields.thickness,
    height: fields.height,
    position: fields.position,
    version: fields.version,
    label: fields.label,
    labelHeight: fields.labelHeight,
    labelX: fields.labelX,
    labelY: fields.labelY,
    flags: fields.flags,
    segments: fields.segments,
    parts,
  };
}

/** Emit the real 2D symbol(s) for one opening. */
export function openingPrimitives(
  opening: OpeningData,
  walls: ReadonlyMap<bigint, WallData>,
  reasons: Map<string, number>,
): Primitive[] {
  const { block2d, kind, mirror, angle, width, thickness, position } = opening;
  const wall = walls.get(opening.wall);

  if (kind === 8) {
    const primitives: Primitive[] = [];
    for (const part of opening.parts) {
      primitives.push(...openingPrimitives(part, walls, reasons));
    }
    add(reasons, 'vendor.tch.opening_group_parts', opening.parts.length);
    return primitives;
  }

  if (kind === 5) return bandWindowPrimitives(opening, wall, reasons);

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
    const block: Primitive = {
      kind: 5,
      block: block2d,
      origin: { x: position.x, y: position.y, z: 0 },
      scale: { x: sx, y: sy, z: width },
      rotation: angle,
    };
    add(reasons, 'vendor.tch.opening_symbols', 1);
    if (kind !== 0) return [block];
    return [block, ...doorThresholds(opening, wall, sy, reasons)];
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

  bump(reasons, 'vendor.tch.opening_kind_unsupported');
  return [];
}

/** MiniCAD `ConvertOpening` door threshold lines (`flags & 0x600`). */
function doorThresholds(
  opening: OpeningData,
  wall: WallData | undefined,
  sy: number,
  reasons: Map<string, number>,
): Primitive[] {
  const { flags, angle, width, position } = opening;
  if (!wall || (flags & 0x600) === 0) return [];
  const wl = Math.hypot(wall.end.x - wall.start.x, wall.end.y - wall.start.y);
  if (!(wl > 0)) {
    bump(reasons, 'vendor.tch.opening_invalid');
    return [];
  }
  const ux = Math.cos(angle);
  const uy = Math.sin(angle);
  const wlx = -(wall.end.y - wall.start.y) / wl;
  const wly = (wall.end.x - wall.start.x) / wl;
  const half = width / 2;
  const primitives: Primitive[] = [];
  for (const [flag, leaf] of [[0x200, 1], [0x400, -1]] as const) {
    if ((flags & flag) === 0) continue;
    const side = (sy > 0 ? 1 : -1) * leaf;
    const ox = -uy * side;
    const oy = ux * side;
    const face = ox * wlx + oy * wly > 0 ? wall.left : wall.right;
    const cx = position.x + ox * face;
    const cy = position.y + oy * face;
    const points = [
      { x: cx - ux * half, y: cy - uy * half, z: 0 },
      { x: cx + ux * half, y: cy + uy * half, z: 0 },
    ];
    if (!points.every(point => Number.isFinite(point.x) && Number.isFinite(point.y))) {
      bump(reasons, 'vendor.tch.opening_invalid');
      continue;
    }
    primitives.push({ kind: 1, points });
    add(reasons, 'vendor.tch.opening_thresholds', 1);
  }
  return primitives;
}

/** MiniCAD `TchShape::BuildBandWindow`: four offsets per segment. */
function bandWindowPrimitives(
  opening: OpeningData,
  wall: WallData | undefined,
  reasons: Map<string, number>,
): Primitive[] {
  if (!wall) {
    bump(reasons, 'vendor.tch.opening_band_wall_missing');
    return [];
  }
  const wallAngle = Math.atan2(wall.end.y - wall.start.y, wall.end.x - wall.start.x);
  const nx = -Math.sin(wallAngle);
  const ny = Math.cos(wallAngle);
  const lines: Point3[] = [];
  const arcs: Primitive[] = [];

  const offsets = (a: Point3, b: Point3, bulge: number): void => {
    for (let k = 0; k <= 3; k += 1) {
      const t = -wall.right + (wall.left + wall.right) * k / 3;
      if (Math.abs(bulge) < 1e-12) {
        lines.push(
          { x: a.x + nx * t, y: a.y + ny * t, z: 0 },
          { x: b.x + nx * t, y: b.y + ny * t, z: 0 },
        );
        continue;
      }
      const arc = bulgeArc(a, b, bulge);
      if (!arc) continue;
      const radius = arc.radius - (bulge > 0 ? t : -t);
      if (radius <= 0) continue;
      if (!Number.isFinite(radius)) {
        bump(reasons, 'vendor.tch.opening_invalid');
        continue;
      }
      arcs.push({ ...arc, radius });
    }
  };

  if (opening.segments.length === 0) {
    const ux = Math.cos(opening.angle) * opening.width / 2;
    const uy = Math.sin(opening.angle) * opening.width / 2;
    offsets(
      { x: opening.position.x - ux, y: opening.position.y - uy, z: 0 },
      { x: opening.position.x + ux, y: opening.position.y + uy, z: 0 },
      0,
    );
  } else {
    for (const segment of opening.segments) {
      offsets(
        { x: segment.x1, y: segment.y1, z: 0 },
        { x: segment.x2, y: segment.y2, z: 0 },
        segment.bulge,
      );
    }
  }

  const primitives: Primitive[] = [];
  if (lines.length > 0) primitives.push({ kind: 1, points: lines });
  primitives.push(...arcs);
  add(reasons, 'vendor.tch.opening_band_symbols', 1);
  return primitives;
}
