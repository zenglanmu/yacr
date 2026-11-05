// `TCH_DIMENSION2` / `TCH_AXIS_LABEL` / `TCH_RADIUSDIM` base-6 decoders.
//
// Plain (non-cipher) branches ported from MiniCAD `TchDimension.h`. The real
// records are base 6 / AC1027 and must consume the main stream exactly; every
// other class-body version or layout is an explicit unsupported layout, never a
// guessed success. The ABI carries no dimension-style payload, so the builders
// use MiniCAD's documented default style and say so in diagnostics.
import { consumeTexts, readBase6, textStream, ClassText } from './base6.ts';
import type { TchBase6 } from './base6.ts';
import { reader } from './bit-reader.ts';
import type { BitReader, Point3 } from './bit-reader.ts';
import { add, bump } from './diagnostics.ts';
import { formatNumber, text, upright } from './geometry.ts';
import type { Primitive, VendorRecord } from './types.ts';

const MAX_POINTS = 1e12;
const MAX_SPANS = 4096;
const MAX_AXES = 4096;

/** MiniCAD's `TchDimStyle` defaults, in drawing units. */
const STYLE = {
  extOffset: 3,
  extExtension: 2.5,
  arrowSize: 1,
  textHeight: 3.5,
  gap: 1,
  decimals: 0,
  angularDecimals: 2,
  angularZeros: 0,
  textColor: 7,
} as const;

function isTruncated(error: unknown): boolean {
  return error instanceof Error && error.message === 'vendor.truncated';
}

function finite(value: number): boolean {
  return Number.isFinite(value) && Math.abs(value) < MAX_POINTS;
}

function point(x: number, y: number, z = 0): Point3 {
  return { x, y, z };
}

function validPoint(p: Point3): boolean {
  return finite(p.x) && finite(p.y) && finite(p.z);
}

/** `DecodeDimension` / `DecodeAxisLabel` / `DecodeRadiusDim` all share this header. */
function base6(
  main: BitReader,
  reasons: Map<string, number>,
  layoutReason: string,
): TchBase6 | null {
  try {
    return readBase6(main);
  } catch (error) {
    if (isTruncated(error)) throw error;
    bump(reasons, layoutReason);
    return null;
  }
}

/**
 * Run one plain decode, mapping layout failures to a stable reason. Truncated
 * streams still propagate so the caller can report a truncated frame.
 */
function decodePhase(
  reasons: Map<string, number>,
  layoutReason: string,
  run: () => void,
): boolean {
  try {
    run();
    return true;
  } catch (error) {
    if (isTruncated(error)) throw error;
    bump(reasons, layoutReason);
    return false;
  }
}

/** `Lines` / `Ticks` as one kind-1 primitive of disjoint endpoint pairs. */
function segmentPrimitive(segments: readonly (readonly [Point3, Point3])[]): Primitive | null {
  const points: Point3[] = [];
  for (const [from, to] of segments) {
    if (!validPoint(from) || !validPoint(to)) continue;
    points.push(from, to);
  }
  return points.length >= 2 ? { kind: 1, points } : null;
}

/** Filled `Arrows`/`Triangles`: the ABI has no filled shape, so closed outline. */
function triangle(a: Point3, b: Point3, tip: Point3, reasons: Map<string, number>): Primitive {
  add(reasons, 'vendor.tch.arrow_outline_only', 1);
  return { kind: 0, points: [a, b, tip, a] };
}

function circle(cx: number, cy: number, radius: number): Primitive {
  return { kind: 3, cx, cy, cz: 0, radius };
}

function addText(
  primitives: Primitive[],
  center: Point3,
  height: number,
  rotation: number,
  value: string,
  attachment: number,
): void {
  const clean = value.split('^U').join('');
  if (!(height > 0) || !finite(height) || !finite(rotation) || !validPoint(center)) return;
  primitives.push(text(center.x, center.y, center.z, height, rotation, clean, attachment));
}

/** MiniCAD `Detail::SuppressZeros`. */
function suppressZeros(value: string, zeros: number): string {
  let result = value;
  if ((zeros & 2) !== 0 && result.includes('.')) {
    while (result.endsWith('0')) result = result.slice(0, -1);
    if (result.endsWith('.')) result = result.slice(0, -1);
  }
  if ((zeros & 1) !== 0 && result.length > 1 && result[0] === '0' && result[1] === '.') {
    result = result.slice(1);
  }
  return result;
}

// ---------------------------------------------------------------------------
// TCH_DIMENSION2
// ---------------------------------------------------------------------------

interface DimensionData {
  scale: number;
  position: Point3;
  angle: number;
  offset: number;
  spans: number[];
  flags: number;
  arcRadius: number;
  measuredRadius: number;
  textOverrides: Map<number, string>;
  textPositions: Map<number, { x: number; y: number }>;
}

function isArc(data: DimensionData): boolean {
  return (data.flags & 0x10) !== 0;
}

function decodeDimensionData(
  record: VendorRecord,
  reasons: Map<string, number>,
): DimensionData | null {
  const main = reader(record.main, record.mainBits, record.mainSkipBits);
  let data: DimensionData | null = null;
  const ok = decodePhase(reasons, 'vendor.tch.dimension_layout', () => {
    const base = base6(main, reasons, 'vendor.tch.dimension_layout');
    if (!base) return;
    const version = main.readByte();
    if (version !== 1) {
      bump(reasons, 'vendor.tch.dimension_unsupported_version');
      return;
    }
    const textReader = textStream(record);
    consumeTexts(textReader, base.texts);
    const classText = new ClassText(textReader);

    const position = main.read3BitDouble();
    const angle = main.readBitDouble();
    const offset = main.readBitDouble();
    const spans: number[] = [];
    const spanCount = main.readBitShort();
    // Real records include zero-span dimensions whose layout is still exact;
    // the builder then emits only the station tick / extension geometry.
    if (spanCount < 0 || spanCount > MAX_SPANS) throw new Error('vendor.tch.layout');
    for (let index = 0; index < spanCount; index += 1) spans.push(main.readBitDouble());

    const flags = main.readBitShort();
    const textOverrides = new Map<number, string>();
    const textPositions = new Map<number, { x: number; y: number }>();
    if ((flags & 2) !== 0) {
      const count = main.readBitShort();
      if (count < 0 || count > MAX_SPANS) throw new Error('vendor.tch.layout');
      for (let index = 0; index < count; index += 1) {
        const span = main.readBitShort();
        const x = main.readRawDouble();
        const y = main.readRawDouble();
        textPositions.set(span, { x, y });
      }
    }
    if ((flags & 4) !== 0) {
      const count = main.readBitShort();
      if (count < 0 || count > MAX_SPANS) throw new Error('vendor.tch.layout');
      for (let index = 0; index < count; index += 1) {
        const span = main.readBitShort();
        textOverrides.set(span, classText.next());
      }
    }
    let arcRadius = 0;
    let measuredRadius = 0;
    if ((flags & 0x10) !== 0) {
      arcRadius = main.readBitDouble();
      measuredRadius = main.readBitDouble();
    }
    if ((flags & 0x80) !== 0) {
      const count = main.readBitShort();
      if (count < 0 || count > MAX_SPANS) throw new Error('vendor.tch.layout');
      for (let index = 0; index < count; index += 1) main.readBitDouble();
    }
    if (main.remaining !== 0) throw new Error('vendor.tch.layout');

    // CheckDimension
    const values = [
      position.x, position.y, position.z, angle, offset, base.scale, arcRadius, measuredRadius,
    ];
    if (!values.every(finite)) throw new Error('vendor.tch.layout');
    if (!(base.scale > 0)) throw new Error('vendor.tch.layout');
    if (!spans.every(finite)) throw new Error('vendor.tch.layout');
    if ((flags & 0x10) !== 0 && !(arcRadius > 0 && measuredRadius > 0)) {
      throw new Error('vendor.tch.layout');
    }

    data = {
      scale: base.scale,
      position,
      angle,
      offset,
      spans,
      flags,
      arcRadius,
      measuredRadius,
      textOverrides,
      textPositions,
    };
  });
  return ok ? data : null;
}

function buildDimension(data: DimensionData, reasons: Map<string, number>): Primitive[] {
  const primitives: Primitive[] = [];
  const scale = data.scale;
  const exo = STYLE.extOffset * scale;
  const exe = STYLE.extExtension * scale;
  const asz = STYLE.arrowSize * scale;
  const ux = Math.cos(data.angle);
  const uy = Math.sin(data.angle);
  const nx = -uy;
  const ny = ux;

  if (!isArc(data)) {
    const off = data.offset * scale;
    const side = off < 0 ? -1 : 1;
    const stations = [0];
    for (const span of data.spans) stations.push(stations[stations.length - 1] + span);
    const at = (t: number, o: number): Point3 => point(
      data.position.x + ux * t + nx * o,
      data.position.y + uy * t + ny * o,
    );

    const lines: (readonly [Point3, Point3])[] = [];
    const ticks: (readonly [Point3, Point3])[] = [];
    for (const t of stations) {
      lines.push([at(t, side * exo), at(t, off + side * exe)]);
      const center = at(t, off);
      const hx = (ux + nx) * asz / 2;
      const hy = (uy + ny) * asz / 2;
      ticks.push([
        point(center.x + hx, center.y + hy),
        point(center.x - hx, center.y - hy),
      ]);
    }
    lines.push([at(0, off), at(stations[stations.length - 1], off)]);
    const linePrimitive = segmentPrimitive(lines);
    if (linePrimitive) primitives.push(linePrimitive);
    const tickPrimitive = segmentPrimitive(ticks);
    if (tickPrimitive) primitives.push(tickPrimitive);

    const { rotation, flipped } = upright(data.angle);
    const up = flipped ? -1 : 1;
    const lift = STYLE.textHeight * scale / 2 + STYLE.gap * scale;
    const height = STYLE.textHeight * 0.85 * scale;
    for (let index = 0; index < data.spans.length; index += 1) {
      const override = data.textOverrides.get(index);
      const value = override !== undefined
        ? override
        : formatNumber(Math.abs(data.spans[index]), STYLE.decimals);
      const fallback = at(
        (stations[index] + stations[index + 1]) / 2,
        off + up * lift,
      );
      const placed = data.textPositions.get(index);
      const center = placed ? point(placed.x, placed.y) : fallback;
      addText(primitives, center, height, rotation, value, 5);
    }
    return primitives;
  }

  // Arc dimension: the center is MeasuredRadius along the left normal of the
  // first measured point; spans are counter-clockwise central angles.
  const center = point(
    data.position.x + nx * data.measuredRadius,
    data.position.y + ny * data.measuredRadius,
  );
  const radius = data.arcRadius;
  const dir = radius < data.measuredRadius ? -1 : 1;
  const angles = [data.angle - Math.PI / 2];
  for (const span of data.spans) angles.push(angles[angles.length - 1] + span);
  const polar = (angle: number, r: number): Point3 => point(
    center.x + Math.cos(angle) * r,
    center.y + Math.sin(angle) * r,
  );

  const lines: (readonly [Point3, Point3])[] = [];
  for (const angle of angles) {
    lines.push([
      polar(angle, data.measuredRadius + dir * exo),
      polar(angle, radius + dir * exe),
    ]);
  }
  const linePrimitive = segmentPrimitive(lines);
  if (linePrimitive) primitives.push(linePrimitive);

  const delta = asz < 2 * radius ? 2 * Math.asin(asz / (2 * radius)) : 0;
  const height = STYLE.textHeight * 0.85 * scale;
  for (let index = 0; index < data.spans.length; index += 1) {
    const a0 = angles[index];
    const a1 = angles[index + 1];
    primitives.push({
      kind: 2,
      cx: center.x,
      cy: center.y,
      cz: 0,
      radius,
      startAngle: a0 + delta,
      sweep: (a1 - delta) - (a0 + delta),
    });
    for (const [tip, toward] of [[a0, a0 + delta], [a1, a1 - delta]] as const) {
      const t = polar(tip, radius);
      const b = polar(toward, radius);
      const length = Math.hypot(b.x - t.x, b.y - t.y);
      if (!(length > 0)) continue;
      const px = -(b.y - t.y) / length * asz / 6;
      const py = (b.x - t.x) / length * asz / 6;
      primitives.push(triangle(
        point(b.x + px, b.y + py),
        point(b.x - px, b.y - py),
        t,
        reasons,
      ));
    }
    const mid = (a0 + a1) / 2;
    const rotation = mid + Math.PI / 2;
    const { flipped } = upright(rotation);
    const up = flipped ? -1 : 1;
    const override = data.textOverrides.get(index);
    const value = override !== undefined
      ? override
      : suppressZeros(
        formatNumber(Math.abs(data.spans[index]) * 180 / Math.PI, STYLE.angularDecimals),
        STYLE.angularZeros,
      ) + '\u00b0';
    const placed = data.textPositions.get(index);
    const target = placed
      ? point(placed.x, placed.y)
      : polar(mid, radius - up * (height / 2 + STYLE.gap * scale));
    addText(primitives, target, height, rotation, value, 5);
  }
  return primitives;
}

export function decodeDimension2(
  record: VendorRecord,
  reasons: Map<string, number>,
): Primitive[] | null {
  const data = decodeDimensionData(record, reasons);
  if (!data) return null;
  add(reasons, 'vendor.tch.dimension_style_defaults', 1);
  return buildDimension(data, reasons);
}

// ---------------------------------------------------------------------------
// TCH_AXIS_LABEL
// ---------------------------------------------------------------------------

interface AxisLabelAxis {
  spacing: number;
  flags: number;
  startLeader: number;
  endLeader: number;
  startTrim: number;
  endTrim: number;
  label: string;
}

interface AxisLabelData {
  scale: number;
  start: Point3;
  end: Point3;
  axes: AxisLabelAxis[];
  mode: number;
  startExtension: number;
  endExtension: number;
  radius: number;
  showText: boolean;
}

/** `CheckAxisLabel` support limits: only the plain straight grid is ported. */
function axisSupported(data: AxisLabelData, bulge: number): boolean {
  if ((data.mode & 1) !== 0) return false;
  if ((data.mode & ~0xcc) !== 0) return false;
  if (Math.abs(bulge) >= 1e-6) return false;
  const values = [
    data.start.x, data.start.y, data.end.x, data.end.y,
    data.startExtension, data.endExtension, data.radius, data.scale,
  ];
  if (!values.every(finite)) return false;
  if (!(data.scale > 0) || data.radius < 0) return false;
  if (Math.hypot(data.end.x - data.start.x, data.end.y - data.start.y) < 1e-9) return false;
  for (const axis of data.axes) {
    const axisValues = [
      axis.spacing, axis.startLeader, axis.endLeader, axis.startTrim, axis.endTrim,
    ];
    if (!axisValues.every(finite)) return false;
  }
  return true;
}

function decodeAxisLabelData(
  record: VendorRecord,
  reasons: Map<string, number>,
): AxisLabelData | null {
  const main = reader(record.main, record.mainBits, record.mainSkipBits);
  let data: AxisLabelData | null = null;
  const ok = decodePhase(reasons, 'vendor.tch.axis_layout', () => {
    const base = base6(main, reasons, 'vendor.tch.axis_layout');
    if (!base) return;
    const version = main.readByte();
    if (version < 1 || version > 6) {
      bump(reasons, 'vendor.tch.axis_unsupported_version');
      return;
    }
    const textReader = textStream(record);
    consumeTexts(textReader, base.texts);
    const classText = new ClassText(textReader);
    const mode = version < 3 ? main.readByte() : main.readBitShort();

    const start = point(main.readRawDouble(), main.readRawDouble());
    const end = point(main.readRawDouble(), main.readRawDouble());
    const bulge = main.readBitDouble();
    const count = version === 6 ? main.readBitShort() : main.readByte();
    if (count < 1 || count > MAX_AXES) throw new Error('vendor.tch.layout');
    const axes: AxisLabelAxis[] = [];
    for (let index = 0; index < count; index += 1) {
      let flags = version < 3 ? main.readByte() : main.readBitShort();
      const label = classText.next();
      const spacing = main.readBitDouble();
      let startLeader = 0;
      let endLeader = 0;
      let startTrim = 0;
      let endTrim = 0;
      if (version > 2) {
        if ((flags & 0x80) !== 0) startLeader = main.readBitDouble();
        if ((flags & 0x100) !== 0) endLeader = main.readBitDouble();
      } else {
        flags &= ~0x180;
      }
      if (version > 4) {
        if ((flags & 0x400) !== 0) startTrim = main.readBitDouble();
        if ((flags & 0x800) !== 0) endTrim = main.readBitDouble();
      } else {
        flags &= ~0xc00;
      }
      axes.push({ spacing, flags, startLeader, endLeader, startTrim, endTrim, label });
    }
    main.readBitDouble(); // unused by BuildAxisLabel
    const startExtension = main.readBitDouble();
    const endExtension = version > 2 ? main.readBitDouble() : startExtension;
    const radius = main.readBitDouble();
    if (version > 3) main.readBitShort();
    let showText = true;
    if (version > 4) showText = (main.readBitShort() & 1) !== 0;
    if (main.remaining !== 0) throw new Error('vendor.tch.layout');

    const parsed: AxisLabelData = {
      scale: base.scale,
      start,
      end,
      axes,
      mode,
      startExtension,
      endExtension,
      radius,
      showText,
    };
    if (!axisSupported(parsed, bulge)) throw new Error('vendor.tch.layout');
    data = parsed;
  });
  return ok ? data : null;
}

function buildAxisLabel(data: AxisLabelData): Primitive[] {
  const primitives: Primitive[] = [];
  const scale = data.scale;
  const length = Math.hypot(data.end.x - data.start.x, data.end.y - data.start.y);
  const dx = (data.end.x - data.start.x) / length;
  const dy = (data.end.y - data.start.y) / length;
  const rx = dy;
  const ry = -dx;
  const radius = data.radius * scale;
  const diagonal = 3 * Math.PI / 4;
  const lineSegments: (readonly [Point3, Point3])[] = [];
  let offset = 0;

  for (let index = 0; index < data.axes.length; index += 1) {
    const axis = data.axes[index];
    const here = offset;
    offset += axis.spacing;
    if (index === 0 && (data.mode & 0x40) !== 0) continue;
    let p0 = point(data.start.x + rx * here, data.start.y + ry * here);
    let p1 = point(data.end.x + rx * here, data.end.y + ry * here);
    if ((axis.flags & 0x400) !== 0 && axis.startTrim < length + 1e-6) {
      p0 = point(p0.x + dx * axis.startTrim, p0.y + dy * axis.startTrim);
    }
    if ((axis.flags & 0x800) !== 0 && axis.endTrim < length + 1e-6) {
      p1 = point(p1.x - dx * axis.endTrim, p1.y - dy * axis.endTrim);
    }

    const sides = [
      {
        base: p0, inX: dx, inY: dy,
        hidden: (data.mode & 0x04) !== 0 || (axis.flags & 0x02) !== 0,
        leader: (axis.flags & 0x80) !== 0 ? axis.startLeader * scale : data.startExtension * scale,
        plus: 0x08, minus: 0x10,
      },
      {
        base: p1, inX: -dx, inY: -dy,
        hidden: (data.mode & 0x08) !== 0 || (axis.flags & 0x04) !== 0,
        leader: (axis.flags & 0x100) !== 0 ? axis.endLeader * scale : data.endExtension * scale,
        plus: 0x20, minus: 0x40,
      },
    ];

    for (const side of sides) {
      if (side.hidden) continue;
      const q = point(
        side.base.x - side.inX * side.leader,
        side.base.y - side.inY * side.leader,
      );
      if (Math.abs(side.leader) > 1e-9) lineSegments.push([side.base, q]);
      let circleCenter = point(q.x - side.inX * radius, q.y - side.inY * radius);
      if ((axis.flags & (side.plus | side.minus)) !== 0) {
        const angle = Math.atan2(side.inY, side.inX)
          + ((axis.flags & side.plus) !== 0 ? diagonal : -diagonal);
        const ux = Math.cos(angle);
        const uy = Math.sin(angle);
        circleCenter = point(
          q.x + ux * Math.SQRT2 * radius,
          q.y + uy * Math.SQRT2 * radius,
        );
        if (data.startExtension > 1e-6) {
          lineSegments.push([
            point(circleCenter.x - ux * radius, circleCenter.y - uy * radius),
            q,
          ]);
        }
      }
      primitives.push(circle(circleCenter.x, circleCenter.y, radius));
      if (!data.showText || axis.label.length === 0) continue;

      const slash = axis.label.indexOf('/');
      if (slash !== -1) {
        lineSegments.push([
          point(circleCenter.x - radius, circleCenter.y),
          point(circleCenter.x + radius, circleCenter.y),
        ]);
        addText(
          primitives,
          point(circleCenter.x, circleCenter.y + 0.457 * radius),
          0.62 * radius,
          0,
          axis.label.slice(0, slash),
          5,
        );
        addText(
          primitives,
          point(circleCenter.x, circleCenter.y - 0.457 * radius),
          0.62 * radius,
          0,
          axis.label.slice(slash + 1),
          5,
        );
      } else {
        // MiniCAD counts UTF-8 lead bytes (continuation bytes excluded).
        const chars = [...new TextEncoder().encode(axis.label)]
          .filter(byte => (byte & 0xc0) !== 0x80).length;
        const height = chars <= 1 ? 1.24 * radius : 1.94 * radius / chars;
        addText(primitives, circleCenter, height, 0, axis.label, 5);
      }
    }
  }

  const linePrimitive = segmentPrimitive(lineSegments);
  if (linePrimitive) primitives.unshift(linePrimitive);
  return primitives;
}

export function decodeAxisLabel(
  record: VendorRecord,
  reasons: Map<string, number>,
): Primitive[] | null {
  const data = decodeAxisLabelData(record, reasons);
  if (!data) return null;
  return buildAxisLabel(data);
}

// ---------------------------------------------------------------------------
// TCH_RADIUSDIM
// ---------------------------------------------------------------------------

interface RadiusDimData {
  scale: number;
  flags: number;
  leader: number;
  center: Point3;
  tipX: number;
  tipY: number;
  textOverride: string;
  hasTextPosition: boolean;
  textX: number;
  textY: number;
}

function decodeRadiusDimData(
  record: VendorRecord,
  reasons: Map<string, number>,
): RadiusDimData | null {
  const main = reader(record.main, record.mainBits, record.mainSkipBits);
  let data: RadiusDimData | null = null;
  const ok = decodePhase(reasons, 'vendor.tch.radiusdim_layout', () => {
    const base = base6(main, reasons, 'vendor.tch.radiusdim_layout');
    if (!base) return;
    const version = main.readByte();
    if (version !== 1) {
      bump(reasons, 'vendor.tch.radiusdim_unsupported_version');
      return;
    }
    const textReader = textStream(record);
    consumeTexts(textReader, base.texts);
    const classText = new ClassText(textReader);

    const flags = main.readBitShort();
    const leader = main.readBitDouble();
    const center = main.read3BitDouble();
    const tipX = main.readRawDouble();
    const tipY = main.readRawDouble();
    let textOverride = '';
    if ((flags & 4) !== 0) textOverride = classText.next();
    let hasTextPosition = false;
    let textX = 0;
    let textY = 0;
    if ((flags & 2) !== 0) {
      hasTextPosition = true;
      textX = main.readRawDouble();
      textY = main.readRawDouble();
    }
    if (main.remaining !== 0) throw new Error('vendor.tch.layout');

    // CheckRadiusDim
    const values = [center.x, center.y, center.z, tipX, tipY, leader, base.scale, textX, textY];
    if (!values.every(finite)) throw new Error('vendor.tch.layout');
    if (!(base.scale > 0) || Math.hypot(tipX - center.x, tipY - center.y) < 1e-9) {
      throw new Error('vendor.tch.layout');
    }

    data = {
      scale: base.scale,
      flags,
      leader,
      center,
      tipX,
      tipY,
      textOverride,
      hasTextPosition,
      textX,
      textY,
    };
  });
  return ok ? data : null;
}

function buildRadiusDim(data: RadiusDimData, reasons: Map<string, number>): Primitive[] {
  const primitives: Primitive[] = [];
  const scale = data.scale;
  const radius = Math.hypot(data.tipX - data.center.x, data.tipY - data.center.y);
  let ux = (data.tipX - data.center.x) / radius;
  let uy = (data.tipY - data.center.y) / radius;
  if (data.leader > 0) {
    ux = -ux;
    uy = -uy;
  }
  const length = Math.abs(data.leader) * scale;
  const tip = point(data.tipX, data.tipY);
  const at = (t: number, o: number): Point3 => point(
    tip.x + ux * t - uy * o,
    tip.y + uy * t + ux * o,
  );
  const arrow = Math.min(2 * STYLE.arrowSize * scale, length);
  if (length > arrow && validPoint(at(arrow, 0)) && validPoint(at(length, 0))) {
    primitives.push({ kind: 1, points: [at(arrow, 0), at(length, 0)] });
  }
  if (arrow > 0) {
    primitives.push(triangle(at(arrow, 0.165 * arrow), at(arrow, -0.165 * arrow), tip, reasons));
  }

  const value = (data.flags & 4) !== 0
    ? data.textOverride
    : `R${formatNumber(radius, STYLE.decimals)}`;
  const { rotation, flipped } = upright(Math.atan2(uy, ux));
  const height = STYLE.textHeight * 0.85 * scale;
  if (data.hasTextPosition) {
    addText(primitives, point(data.textX, data.textY), height, rotation, value, 5);
  } else {
    const lift = (STYLE.textHeight / 2 + STYLE.gap) * scale;
    addText(
      primitives,
      at(length - STYLE.gap * scale, flipped ? -lift : lift),
      height,
      rotation,
      value,
      flipped ? 4 : 6,
    );
  }
  return primitives;
}

export function decodeRadiusDim(
  record: VendorRecord,
  reasons: Map<string, number>,
): Primitive[] | null {
  const data = decodeRadiusDimData(record, reasons);
  if (!data) return null;
  add(reasons, 'vendor.tch.dimension_style_defaults', 1);
  return buildRadiusDim(data, reasons);
}
