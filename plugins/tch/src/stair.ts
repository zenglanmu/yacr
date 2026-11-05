// `TCH_LINESTAIR` / `TCH_RECTSTAIR` base-6 decoders and display builders.
//
// Plain (non-cipher) class bodies ported from MiniCAD `TchShape.h`
// (`DecodeLineStair` / `BuildLineStair` / `FlightBreak`) and `TchStair.h`
// (`DecodeRectStair` / `BuildRectStair`). Real records are base-6 / AC1027 and
// must consume the main stream exactly; every other class-body version or
// layout is an explicit unsupported layout, never a guessed success.
//
// Real `TCH_RECTSTAIR` records carry stair version 5, which differs from the
// MiniCAD version-4 layout by one trailing `BitShort` that `BuildRectStair`
// does not use. All 54 fixture records end at exactly that scalar, so it is
// consumed and ignored rather than rejecting otherwise displayable geometry.
import { readBase6 } from './base6.ts';
import { reader } from './bit-reader.ts';
import type { Point3 } from './bit-reader.ts';
import { add, bump } from './diagnostics.ts';
import { local, text } from './geometry.ts';
import type { Primitive, VendorRecord } from './types.ts';

const DEG = Math.PI / 180;
const MAX_POINT_ABS = 1e12;

function isTruncated(error: unknown): boolean {
  return error instanceof Error && error.message === 'vendor.truncated';
}

function finite(value: number): boolean {
  return Number.isFinite(value) && Math.abs(value) <= MAX_POINT_ABS;
}

function point(x: number, y: number): Point3 {
  return { x, y, z: 0 };
}

function validPoint(p: Point3): boolean {
  return finite(p.x) && finite(p.y) && finite(p.z);
}

/** `TchGraphics.Lines` as one kind-1 primitive of disjoint endpoint pairs. */
function linePairs(segments: readonly (readonly [Point3, Point3])[]): Primitive | null {
  const points: Point3[] = [];
  for (const [from, to] of segments) {
    if (!validPoint(from) || !validPoint(to)) continue;
    points.push(from, to);
  }
  return points.length >= 2 ? { kind: 1, points } : null;
}

/** MiniCAD `g.Arrows`: the ABI has no filled triangle, so a closed outline. */
function triangleOutline(a: Point3, b: Point3, tip: Point3, reasons: Map<string, number>): Primitive {
  add(reasons, 'vendor.tch.arrow_outline_only', 1);
  return { kind: 0, points: [a, b, tip, a] };
}

// ---------------------------------------------------------------------------
// TCH_LINESTAIR
// ---------------------------------------------------------------------------

interface LineStairData {
  steps: number;
  breakMode: number;
  breakStart: number;
  breakEnd: number;
  tread: number;
  width: number;
  x: number;
  y: number;
  rotation: number;
}

/** `Detail::BreakKind`: only the low nibble selects a break shape. */
function breakKind(mode: number): number {
  switch (mode & 15) {
    case 2: return 1;
    case 3: return 2;
    case 4: return 3;
    default: return 0;
  }
}

interface FlightBreak {
  kind: number;
  c: Point3;
  ux: number;
  uy: number;
  halfLength: number;
  from: number;
  to: number;
  lines: Point3[][];
  visible: Array<[Point3, Point3]>;
  uMin: number;
  uMax: number;
  inside: (p: Point3) => boolean;
  progress: (p: Point3) => number;
}

/** Port of MiniCAD `Detail::FlightBreak` (constructor + `Clip`). */
function createFlightBreak(
  kind: number,
  a: Point3,
  b: Point3,
  inside: (p: Point3) => boolean,
  progress: (p: Point3) => number,
  from: number,
  to: number,
): FlightBreak {
  const c = point((a.x + b.x) / 2, (a.y + b.y) / 2);
  const length = Math.hypot(b.x - a.x, b.y - a.y);
  const ux = length > 1e-9 ? (b.x - a.x) / length : 1;
  const uy = length > 1e-9 ? (b.y - a.y) / length : 0;
  const breakState: FlightBreak = {
    kind,
    c,
    ux,
    uy,
    halfLength: length / 2,
    from: Math.min(from, to),
    to: Math.max(from, to),
    lines: [],
    visible: [],
    uMin: 0,
    uMax: 0,
    inside,
    progress,
  };

  const w = (u: number, n: number): Point3 => point(
    c.x + u * ux - n * uy,
    c.y + u * uy + n * ux,
  );
  const pb = { x: 135 * Math.cos(115 * DEG), y: 135 * Math.sin(115 * DEG) };
  const pc = { x: 135 * Math.cos(-75 * DEG), y: 135 * Math.sin(-75 * DEG) };
  const along = (p: { x: number; y: number }, n: number): Point3 => {
    const t = (n - p.y) / Math.sin(75 * DEG);
    return point(p.x + t * Math.cos(75 * DEG), n);
  };
  const h = length / 2 + 300;

  if (kind === 2) {
    const pa = along(pb, -50);
    const x2l = along(pb, 50);
    const pd = along(pc, 50);
    const x2r = along(pc, -50);
    breakState.lines = [
      [w(-h, 50), point(x2l.x, x2l.y)],
      [w(-h, -50), point(pa.x, pa.y), point(pb.x, pb.y), point(pc.x, pc.y), point(pd.x, pd.y), w(h, 50)],
      [point(x2r.x, x2r.y), w(h, -50)],
    ];
  } else if (kind === 1 || kind === 3) {
    const a0 = along(pb, 0);
    const d0 = along(pc, 0);
    breakState.lines = [[
      w(-h, 0), point(a0.x, a0.y), point(pb.x, pb.y),
      point(pc.x, pc.y), point(d0.x, d0.y), w(h, 0),
    ]];
  }

  clipFlightBreak(breakState);
  return breakState;
}

/** `FlightBreak::Clip`: keep only the portions inside the flight band. */
function clipFlightBreak(state: FlightBreak): void {
  const { c, ux, uy, inside } = state;
  const u = (p: Point3): number => (p.x - c.x) * ux + (p.y - c.y) * uy;
  const clip = (inPt: Point3, outPt: Point3, probe: (p: Point3) => boolean): Point3 => {
    let insidePoint = inPt;
    let outsidePoint = outPt;
    for (let index = 0; index < 50; index += 1) {
      const middle = point((insidePoint.x + outsidePoint.x) / 2, (insidePoint.y + outsidePoint.y) / 2);
      if (probe(middle)) insidePoint = middle;
      else outsidePoint = middle;
    }
    return insidePoint;
  };
  const h = state.halfLength;
  let first = true;
  for (const line of state.lines) {
    for (let index = 0; index + 1 < line.length; index += 1) {
      let a = line[index];
      let b = line[index + 1];
      if (Math.abs(u(a)) > h + 600 && Math.abs(u(b)) > h + 600) continue;
      const insideA = inside(a);
      const insideB = inside(b);
      if (!insideA && !insideB) continue;
      if (!insideA) a = clip(b, a, inside);
      if (!insideB) b = clip(a, b, inside);
      state.visible.push([a, b]);
      for (const value of [u(a), u(b)]) {
        state.uMin = first ? value : Math.min(state.uMin, value);
        state.uMax = first ? value : Math.max(state.uMax, value);
        first = false;
      }
    }
  }
}

/** `FlightBreak::Normal`. */
function breakNormal(state: FlightBreak, p: Point3): number {
  return -(p.x - state.c.x) * state.uy + (p.y - state.c.y) * state.ux;
}

/** `FlightBreak::Removed`: whether a segment midpoint is cut away. */
function breakRemoved(state: FlightBreak, p: Point3): boolean {
  const n = breakNormal(state, p);
  if (state.kind === 1) return n > 0 && state.progress(p) >= state.from - 1e-9;
  if (state.kind === 3) return n < 0 && state.progress(p) <= state.to + 1e-9;
  const value = (p.x - state.c.x) * state.ux + (p.y - state.c.y) * state.uy;
  if (value < state.uMin - 1e-6 || value > state.uMax + 1e-6) return false;
  return state.kind === 2 && Math.abs(n) < 50;
}

/** `FlightBreak::AddLine`: split at the break symbol and drop removed parts. */
function addBreakLine(state: FlightBreak, out: Array<[Point3, Point3]>, a: Point3, b: Point3): void {
  if (state.kind === 0) {
    out.push([a, b]);
    return;
  }
  const params = [0, 1];
  for (const [p0, p1] of state.visible) {
    const dx = b.x - a.x;
    const dy = b.y - a.y;
    const ex = p1.x - p0.x;
    const ey = p1.y - p0.y;
    const denominator = dx * ey - dy * ex;
    if (Math.abs(denominator) < 1e-12) continue;
    const ox = p0.x - a.x;
    const oy = p0.y - a.y;
    const t = (ox * ey - oy * ex) / denominator;
    const u = (ox * dy - oy * dx) / denominator;
    if (t > 0 && t < 1 && u >= -1e-9 && u <= 1 + 1e-9) params.push(t);
  }
  params.sort((left, right) => left - right);
  const at = (t: number): Point3 => point(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
  for (let index = 0; index + 1 < params.length; index += 1) {
    if (params[index + 1] - params[index] <= 1e-9) continue;
    const middle = (params[index] + params[index + 1]) / 2;
    if (!breakRemoved(state, at(middle))) {
      out.push([at(params[index]), at(params[index + 1])]);
    }
  }
}

export function decodeLineStair(
  record: VendorRecord,
  reasons: Map<string, number>,
): Primitive[] | null {
  const main = reader(record.main, record.mainBits, record.mainSkipBits);
  let data: LineStairData | null = null;
  try {
    readBase6(main); // base-6 header (scale is not used by BuildLineStair)

    // Detail::ReadFlightBase
    const flightVersion = main.readByte();
    if (flightVersion < 1 || flightVersion > 3) {
      bump(reasons, 'vendor.tch.linestair_layout');
      return null;
    }
    const steps = main.readBitShort();
    main.readBitDouble(); // step height (not used by BuildLineStair)
    const breakMode = main.readByte();
    let breakStart = 0.4;
    let breakEnd = 0.6;
    if (flightVersion > 1) {
      breakStart = main.readBitDouble();
      breakEnd = main.readBitDouble();
    }
    if (flightVersion > 2) {
      main.readBitLong();
      main.readBitDouble();
      main.readBitDouble();
      main.readBitDouble();
    }

    const version = main.readByte();
    if (version < 1 || version > 2) {
      bump(reasons, 'vendor.tch.linestair_unsupported_version');
      return null;
    }
    const tread = main.readBitDouble();
    const width = main.readBitDouble();
    const x = main.readBitDouble();
    const y = main.readBitDouble();
    main.readBitDouble(); // z of the start point
    const rotation = main.readBitDouble();
    if (version > 1) main.readBitDouble();
    if (main.remaining !== 0) {
      bump(reasons, 'vendor.tch.linestair_layout');
      return null;
    }

    // Detail::CheckLineStair
    if (![tread, width, x, y, rotation, breakStart, breakEnd].every(finite)) {
      bump(reasons, 'vendor.tch.linestair_layout');
      return null;
    }
    if (steps < 2 || steps > 1000 || !(tread > 0) || !(width > 0)) {
      bump(reasons, 'vendor.tch.linestair_layout');
      return null;
    }
    data = { steps, breakMode, breakStart, breakEnd, tread, width, x, y, rotation };
  } catch (error) {
    if (isTruncated(error)) throw error;
    bump(reasons, 'vendor.tch.linestair_layout');
    return null;
  }
  return buildLineStair(data);
}

function buildLineStair(data: LineStairData): Primitive[] {
  const { steps, tread, width, x, y, rotation } = data;
  const length = (steps - 1) * tread;
  const at = (localX: number, localY: number): Point3 => local(x, y, rotation, localX, localY);
  const cosine = Math.cos(rotation);
  const sine = Math.sin(rotation);
  const inside = (p: Point3): boolean => {
    const dx = p.x - x;
    const dy = p.y - y;
    const localX = dx * cosine + dy * sine;
    return localX >= -1e-9 && localX <= width + 1e-9;
  };
  const progress = (p: Point3): number => {
    const dx = p.x - x;
    const dy = p.y - y;
    return (-dx * sine + dy * cosine) / length;
  };

  const state = createFlightBreak(
    breakKind(data.breakMode),
    at(0, data.breakStart * length),
    at(width, data.breakEnd * length),
    inside,
    progress,
    data.breakStart,
    data.breakEnd,
  );

  const segments: Array<[Point3, Point3]> = [];
  addBreakLine(state, segments, at(0, 0), at(0, length));
  addBreakLine(state, segments, at(width, 0), at(width, length));
  for (let index = 0; index < steps; index += 1) {
    addBreakLine(state, segments, at(0, index * tread), at(width, index * tread));
  }
  for (const [a, b] of state.visible) segments.push([a, b]);

  const primitive = linePairs(segments);
  return primitive ? [primitive] : [];
}

// ---------------------------------------------------------------------------
// TCH_RECTSTAIR
// ---------------------------------------------------------------------------

interface RectStairData {
  x: number;
  y: number;
  rotation: number;
  scale: number;
  stepHeight: number;
  treadWidth: number;
  flightWidth: number;
  landingWidth: number;
  totalWidth: number;
  steps1: number;
  steps2: number;
  floor: number;
  mirror: boolean;
  railWidth: number;
  railOffset: number;
  railExtTop: number;
  railExtBottom: number;
  cutHeight: number;
  cutAngle: number;
  textHeight: number;
  showArrows: boolean;
  linkedRails: boolean;
  showLanding: boolean;
}

/** Flags `FinishRectStair` refuses because the display would change. */
const UNSUPPORTED_RECT_FLAGS =
  (1 << 1) | (1 << 3) | (1 << 13) | (1 << 20) | (1 << 21) | 0xff000000;

/** Port of MiniCAD `Detail::FinishRectStair`; returns the stable failure name. */
function finishRectStair(
  data: RectStairData,
  flags: number,
  mirror: number,
): string | null {
  if ((flags & UNSUPPORTED_RECT_FLAGS) !== 0) return 'vendor.tch.rectstair_layout';
  if (data.floor < 0 || data.floor > 2) return 'vendor.tch.rectstair_layout';
  if (mirror !== 0 && mirror !== 1) return 'vendor.tch.rectstair_layout';
  data.mirror = mirror === 1;
  data.showArrows = ((flags >> 10) & 1) !== 0;
  data.linkedRails = ((flags >> 17) & 1) !== 0;
  data.showLanding = ((flags >> 18) & 1) !== 0;

  const values = [
    data.x, data.y, data.rotation, data.scale, data.stepHeight, data.treadWidth,
    data.flightWidth, data.landingWidth, data.totalWidth, data.railWidth, data.railOffset,
    data.railExtTop, data.railExtBottom, data.cutHeight, data.cutAngle, data.textHeight,
  ];
  if (!values.every(finite)) return 'vendor.tch.rectstair_layout';

  if (
    !(data.scale > 0) ||
    !(data.stepHeight > 0) ||
    !(data.treadWidth > 0) ||
    !(data.flightWidth > 0) ||
    data.landingWidth < 0 ||
    data.totalWidth < 2 * data.flightWidth - 1e-9 ||
    data.steps1 < 2 ||
    data.steps2 < 2 ||
    data.railWidth < 0 ||
    data.railOffset < 0 ||
    data.railWidth + data.railOffset > data.flightWidth ||
    !(data.textHeight > 0)
  ) {
    return 'vendor.tch.rectstair_layout';
  }

  if (data.floor !== 2) {
    // Real base-6 records may store the cut-line direction (for example
    // π + the acute slope) instead of the acute angle itself. Only
    // `tan(cutAngle)` drives the drawing, so validate its acute equivalent.
    const slope = Math.tan(data.cutAngle);
    const acute = Math.atan(slope);
    if (!Number.isFinite(slope) || !(acute > 0 && acute < Math.PI / 2)) {
      return 'vendor.tch.rectstair_layout';
    }
    const cut = (data.cutHeight / data.stepHeight) * data.treadWidth;
    if (cut < data.treadWidth / 2 - 1e-6 || cut > (data.steps1 - 1.5) * data.treadWidth + 1e-6) {
      return 'vendor.tch.rectstair_layout';
    }
  }
  return null;
}

export function decodeRectStair(
  record: VendorRecord,
  reasons: Map<string, number>,
): Primitive[] | null {
  const main = reader(record.main, record.mainBits, record.mainSkipBits);
  let data: RectStairData | null = null;
  try {
    const base = readBase6(main);
    const stairBase = main.readByte();
    if (stairBase < 1 || stairBase > 2) {
      bump(reasons, 'vendor.tch.rectstair_layout');
      return null;
    }
    // Real base-6 records use stair version 5: the version-4 layout plus one
    // trailing `BitShort` that `BuildRectStair` does not consume.
    const version = main.readByte();
    if (version !== 4 && version !== 5) {
      bump(reasons, 'vendor.tch.rectstair_unsupported_version');
      return null;
    }

    const x = main.readBitDouble();
    const y = main.readBitDouble();
    main.readBitDouble(); // z
    const rotation = main.readBitDouble();
    const stepHeight = main.readBitDouble();
    const treadWidth = main.readBitDouble();
    const flightWidth = main.readBitDouble();
    const landingWidth = main.readBitDouble();
    const steps1 = main.readByte();
    const steps2 = main.readByte();
    const totalWidth = main.readBitDouble();
    main.readBitDouble(); // rail height
    const floor = main.readByte();
    const mirror = main.readByte();
    main.readByte(); // observed 2; not used by BuildRectStair
    const railWidth = main.readBitDouble();
    const railOffset = main.readBitDouble();
    const flags = main.readBitLong() >>> 0;
    main.readBitDouble();
    main.readBitDouble();
    main.readBitDouble();
    main.readBitDouble(); // version >= 3: does not affect the plan
    main.readBitDouble();
    main.readByte();
    const railExtTop = main.readBitDouble();
    const railExtBottom = main.readBitDouble();
    const cutHeight = main.readBitDouble();
    const cutAngle = main.readBitDouble();
    const textHeight = main.readBitDouble();
    for (let index = 0; index < 4; index += 1) main.readBitDouble(); // platform lengths
    if (version >= 5) main.readBitShort(); // version-5 trailer (observed 9)
    if (main.remaining !== 0) {
      bump(reasons, 'vendor.tch.rectstair_layout');
      return null;
    }

    const parsed: RectStairData = {
      x, y, rotation, scale: base.scale, stepHeight, treadWidth, flightWidth, landingWidth,
      totalWidth, steps1, steps2, floor, mirror: false, railWidth, railOffset,
      railExtTop, railExtBottom, cutHeight, cutAngle, textHeight,
      showArrows: false, linkedRails: false, showLanding: false,
    };
    const failure = finishRectStair(parsed, flags, mirror);
    if (failure) {
      bump(reasons, failure);
      return null;
    }
    data = parsed;
  } catch (error) {
    if (isTruncated(error)) throw error;
    bump(reasons, 'vendor.tch.rectstair_layout');
    return null;
  }
  return buildRectStair(data, reasons);
}

/** `Detail::CrossParams`: segment/polyline intersection parameters. */
function crossParams(
  segment: readonly [Point3, Point3],
  line: readonly Point3[],
  params: number[],
  open: boolean,
): void {
  const dx = segment[1].x - segment[0].x;
  const dy = segment[1].y - segment[0].y;
  for (let index = 0; index + 1 < line.length; index += 1) {
    const ex = line[index + 1].x - line[index].x;
    const ey = line[index + 1].y - line[index].y;
    const denominator = dx * ey - dy * ex;
    if (Math.abs(denominator) < 1e-12) continue;
    const ox = line[index].x - segment[0].x;
    const oy = line[index].y - segment[0].y;
    const t = (ox * ey - oy * ex) / denominator;
    const u = (ox * dy - oy * dx) / denominator;
    const within = open ? (t > 0 && t < 1) : (t >= 0 && t <= 1);
    if (within && u >= 0 && u <= 1) params.push(t);
  }
}

function lerp(segment: readonly [Point3, Point3], t: number): Point3 {
  return point(
    segment[0].x + (segment[1].x - segment[0].x) * t,
    segment[0].y + (segment[1].y - segment[0].y) * t,
  );
}

function buildRectStair(data: RectStairData, reasons: Map<string, number>): Primitive[] {
  const T = data.treadWidth;
  const W = data.flightWidth;
  const L = data.landingWidth;
  const G = data.totalWidth;
  const rw = data.railWidth;
  const roff = data.railOffset;
  const extTop = data.railExtTop;
  const extBot = data.railExtBottom;
  const floor = data.floor;
  const linked = data.linkedRails;

  interface Flight { outer: number; inner: number; dir: number; steps: number; top: number }

  const f1: Flight = data.mirror
    ? { outer: G, inner: G - W, dir: -1, steps: data.steps1, top: 0 }
    : { outer: 0, inner: W, dir: 1, steps: data.steps1, top: 0 };
  const f2: Flight = data.mirror
    ? { outer: 0, inner: W, dir: 1, steps: data.steps2, top: 0 }
    : { outer: G, inner: G - W, dir: -1, steps: data.steps2, top: 0 };
  const y0 = -L - (Math.max(data.steps1, data.steps2) - 1) * T;
  f1.top = y0 + (f1.steps - 1) * T;
  f2.top = y0 + (f2.steps - 1) * T;
  const xc1 = (f1.outer + f1.inner) / 2;
  const xc2 = (f2.outer + f2.inner) / 2;

  // Cut center; the slope shrinks when the plain cut would leave the flight.
  const yc = y0 + (data.cutHeight / data.stepHeight) * T;
  const half = Math.min(
    (W / 2) * Math.tan(data.cutAngle),
    yc - (y0 + T / 2),
    f1.top - T / 2 - yc,
  );
  const angle = Math.atan2(2 * half, W);
  const ca = Math.cos(angle);
  const sa = Math.sin(angle);
  const cutLocal = (u: number, n: number): Point3 => point(
    xc1 + u * ca - n * sa,
    yc + u * sa + n * ca,
  );
  const yline = (n: number, x: number): number => yc + (x - xc1) * Math.tan(angle) + n / ca;
  const pb = { x: 135 * Math.cos(115 * DEG), y: 135 * Math.sin(115 * DEG) };
  const pc = { x: 135 * Math.cos(-75 * DEG), y: 135 * Math.sin(-75 * DEG) };
  const along = (p: { x: number; y: number }, n: number): Point3 => {
    const t = (n - p.y) / Math.sin(75 * DEG);
    return point(p.x + t * Math.cos(75 * DEG), n);
  };

  const railTop = -L + extTop;
  const yo = y0 - extBot;
  const yi = y0 - extBot + rw;
  const innerBot = floor === 2 || (floor === 1 && linked) ? Math.max(y0, yi) : y0;
  const innerTop = (top: number): number => linked && floor !== 0 ? Math.min(top, -L + extTop - rw) : top;
  const railBottom = (k: number): number => {
    if (floor === 0) return y0 - extBot + (linked ? rw / 2 : 0);
    if (floor === 2 && k === 0) return Math.max(y0, yi);
    return y0 - extBot;
  };
  const xl = Math.min(f1.inner, f2.inner);
  const xr = Math.max(f1.inner, f2.inner);
  const oL = xl - roff - rw;
  const oR = xr + roff + rw;
  const iL = xl - roff;
  const iR = xr + roff;
  const iTop = -L + extTop - rw;
  const o1 = data.mirror ? oR : oL;
  const i1 = data.mirror ? iR : iL;
  const o2 = data.mirror ? oL : oR;
  const i2 = data.mirror ? iL : iR;
  const gx1 = Math.min(f1.outer, o2);
  const gx2 = Math.max(f1.outer, o2);

  const spans = (flight: Flight, k: number, y: number): Array<[number, number]> => {
    if (railBottom(k) - 1e-9 <= y && y <= railTop + 1e-9) {
      const out: Array<[number, number]> = [
        [flight.outer, flight.inner - flight.dir * (roff + rw)],
      ];
      if (roff > 0 && innerBot - 1e-9 <= y && y <= innerTop(flight.top) + 1e-9) {
        out.push([flight.inner - flight.dir * roff, flight.inner]);
      }
      return out;
    }
    return [[flight.outer, flight.inner]];
  };

  interface FlightLine { k: number; segment: [Point3, Point3] }

  const flight: FlightLine[] = [];
  for (let k = 0; k < 2; k += 1) {
    if (floor === 0 && k === 1) continue;
    const current = k === 0 ? f1 : f2;
    for (let index = 0; index < current.steps; index += 1) {
      const y = y0 + index * T;
      for (const [a, b] of spans(current, k, y)) {
        if (
          floor === 2 && yo < y && y < yi - 1e-9 &&
          gx1 - 1e-9 <= Math.min(a, b) && Math.max(a, b) <= gx2 + 1e-9
        ) {
          continue;
        }
        flight.push({ k, segment: [point(a, y), point(b, y)] });
      }
    }
    flight.push({ k, segment: [point(current.outer, y0), point(current.outer, current.top)] });
    flight.push({ k, segment: [point(current.inner, innerBot), point(current.inner, innerTop(current.top))] });
  }

  const segments: Array<[Point3, Point3]> = [];
  const line = (a: Point3, b: Point3): void => {
    segments.push([a, b]);
  };

  const lo1 = Math.min(f1.outer, f1.inner);
  const hi1 = Math.max(f1.outer, f1.inner);
  if (floor === 1) {
    // Intermediate floor: double break lines 100 apart; cut the treads between.
    const pa = along(pb, -50);
    const x2l = along(pb, 50);
    const pd = along(pc, 50);
    const x2r = along(pc, -50);
    const breaks: Point3[][] = [
      [point(lo1 - 1, yline(50, lo1 - 1)), cutLocal(x2l.x, x2l.y)],
      [
        point(lo1 - 1, yline(-50, lo1 - 1)), cutLocal(pa.x, pa.y),
        cutLocal(pb.x, pb.y), cutLocal(pc.x, pc.y), cutLocal(pd.x, pd.y),
        point(hi1 + 1, yline(50, hi1 + 1)),
      ],
      [cutLocal(x2r.x, x2r.y), point(hi1 + 1, yline(-50, hi1 + 1))],
    ];
    const inBand = (p: Point3): boolean => Math.abs(-(p.x - xc1) * sa + (p.y - yc) * ca) < 50;
    for (const { k, segment } of flight) {
      if (k !== 0) {
        line(segment[0], segment[1]);
        continue;
      }
      const params: number[] = [];
      for (const polyline of breaks) crossParams(segment, polyline, params, false);
      if (inBand(segment[0])) params.push(0);
      if (inBand(segment[1])) params.push(1);
      if (params.length === 0) {
        line(segment[0], segment[1]);
        continue;
      }
      const first = Math.min(...params);
      const last = Math.max(...params);
      if (first > 1e-9) line(segment[0], lerp(segment, first));
      if (last < 1 - 1e-9) line(lerp(segment, last), segment[1]);
    }
    line(point(lo1, yline(50, lo1)), cutLocal(x2l.x, x2l.y));
    line(point(lo1, yline(-50, lo1)), cutLocal(pa.x, pa.y));
    line(cutLocal(pa.x, pa.y), cutLocal(pb.x, pb.y));
    line(cutLocal(pb.x, pb.y), cutLocal(pc.x, pc.y));
    line(cutLocal(pc.x, pc.y), cutLocal(pd.x, pd.y));
    line(cutLocal(pd.x, pd.y), point(hi1, yline(50, hi1)));
    line(cutLocal(x2r.x, x2r.y), point(hi1, yline(-50, hi1)));
  } else if (floor === 0) {
    // Ground floor: single break line; use even-odd inside the kept polygon.
    const a0 = along(pb, 0);
    const d0 = along(pc, 0);
    const brk: Point3[] = [
      point(hi1, yline(0, hi1)), cutLocal(d0.x, d0.y), cutLocal(pc.x, pc.y),
      cutLocal(pb.x, pb.y), cutLocal(a0.x, a0.y), point(lo1, yline(0, lo1)),
    ];
    const poly: Point3[] = [
      point(hi1 + 1, y0 - 1e6),
      point(hi1 + 1, yline(0, hi1 + 1)),
      ...brk.slice(1, brk.length - 1),
      point(lo1 - 1, yline(0, lo1 - 1)),
      point(lo1 - 1, y0 - 1e6),
    ];
    const ring = [...poly, poly[0]];
    const inside = (p: Point3): boolean => {
      let contains = false;
      for (let index = 0; index < poly.length; index += 1) {
        const a = poly[index];
        const b = poly[(index + 1) % poly.length];
        if (
          (a.y > p.y) !== (b.y > p.y) &&
          p.x < a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y)
        ) {
          contains = !contains;
        }
      }
      return contains;
    };
    const yb = yo + (linked ? rw / 2 : 0);
    flight.push({ k: 0, segment: [point(o1, yb), point(o1, railTop)] });
    flight.push({ k: 0, segment: [point(i1, yb), point(i1, railTop)] });
    flight.push({ k: 0, segment: [point(o1, yb), point(i1, yb)] });
    flight.push({ k: 0, segment: [point(o1, railTop), point(i1, railTop)] });
    for (const { segment } of flight) {
      const params = [0, 1];
      crossParams(segment, ring, params, true);
      params.sort((left, right) => left - right);
      for (let index = 0; index + 1 < params.length; index += 1) {
        if (params[index + 1] - params[index] <= 1e-9) continue;
        if (inside(lerp(segment, (params[index] + params[index + 1]) / 2))) {
          line(lerp(segment, params[index]), lerp(segment, params[index + 1]));
        }
      }
    }
    for (let index = 0; index + 1 < brk.length; index += 1) line(brk[index], brk[index + 1]);
  } else {
    for (const { segment } of flight) line(segment[0], segment[1]);
  }
  if (floor !== 0) line(point(f1.inner, innerBot), point(f2.inner, innerBot)); // stair well bottom

  // Rails.
  const rect = (x1: number, x2: number, ya: number, yb: number): void => {
    line(point(x1, ya), point(x1, yb));
    line(point(x1, yb), point(x2, yb));
    line(point(x2, yb), point(x2, ya));
    line(point(x2, ya), point(x1, ya));
  };
  if (floor === 1) {
    if (linked) {
      rect(oL, oR, yo, railTop);
      rect(iL, iR, yo + rw, iTop);
    } else {
      rect(oL, iL, yo, railTop);
      rect(iR, oR, yo, railTop);
    }
  } else if (floor === 0) {
    // BuildRectStair draws no separate rail rectangle on the ground floor.
  } else if (!linked) {
    const y1b = Math.max(y0, yi);
    rect(o1, i1, y1b, railTop);
    line(point(i2, railTop), point(o2, railTop));
    line(point(o2, railTop), point(o2, yo));
    line(point(o2, yo), point(f1.outer, yo));
    line(point(i2, railTop), point(i2, yi));
    line(point(i2, yi), point(f1.outer, yi));
    line(point(f1.outer, yo), point(f1.outer, yi));
  } else {
    // Top floor: the second-flight rail extends over the first-flight start.
    line(point(o1, yi), point(o1, railTop));
    line(point(oL, railTop), point(oR, railTop));
    line(point(o2, railTop), point(o2, yo));
    line(point(o2, yo), point(f1.outer, yo));
    line(point(i1, yi), point(i1, iTop));
    line(point(iL, iTop), point(iR, iTop));
    line(point(i2, iTop), point(i2, yi));
    line(point(i2, yi), point(f1.outer, yi));
    line(point(o1, yi), point(i1, yi));
    line(point(f1.outer, yo), point(f1.outer, yi));
  }

  // Landing; uneven flights fold the short flight's inner edge into an L.
  if (floor !== 0 && data.showLanding) {
    line(point(0, 0), point(G, 0));
    if (f1.steps === f2.steps) {
      line(point(0, -L), point(G, -L));
      line(point(0, 0), point(0, -L));
      line(point(G, 0), point(G, -L));
    } else {
      const short = f1.steps < f2.steps ? f1 : f2;
      const long = f1.steps < f2.steps ? f2 : f1;
      line(point(short.outer, 0), point(short.outer, short.top));
      for (const [a, b] of spans(short, short === f1 ? 0 : 1, short.top)) {
        line(point(a, short.top), point(b, short.top));
      }
      line(point(short.inner, short.top), point(short.inner, -L));
      line(point(short.inner, -L), point(long.outer, -L));
      line(point(long.outer, 0), point(long.outer, -L));
    }
  }

  // Arrows: start half a tread outside the start line, head 3×scale long.
  interface Arrow { points: Point3[]; value: string }
  const arrows: Arrow[] = [];
  const head = 3 * data.scale;
  if (data.showArrows) {
    if (floor !== 2) {
      const tip = yc - T / 2;
      arrows.push({
        points: [point(xc1, y0 - T / 2), point(xc1, tip - head), point(xc1, tip)],
        value: '\u4e0a', // 上
      });
    }
    if (floor !== 0) {
      const tip = floor === 1 ? yc + T / 2 : y0 + T / 2;
      arrows.push({
        points: [
          point(xc2, y0 - T / 2), point(xc2, -L / 2), point(xc1, -L / 2),
          point(xc1, tip + head), point(xc1, tip),
        ],
        value: '\u4e0b', // 下
      });
    }
  }

  const cr = Math.cos(data.rotation);
  const sr = Math.sin(data.rotation);
  const world = (q: Point3): Point3 => point(
    data.x + q.x * cr - q.y * sr,
    data.y + q.x * sr + q.y * cr,
  );

  const primitives: Primitive[] = [];
  const lines = linePairs(segments.map(([a, b]) => [world(a), world(b)] as const));
  if (lines) primitives.push(lines);

  const nominal = data.textHeight * data.scale;
  const height = 0.96454 * nominal;
  const dx = -sr;
  const dy = cr;
  for (const arrow of arrows) {
    const shaft = arrow.points.slice(0, -1).map(world);
    if (shaft.length >= 2) primitives.push({ kind: 0, points: shaft });

    const base = arrow.points[arrow.points.length - 2];
    const tip = arrow.points[arrow.points.length - 1];
    const length = Math.hypot(tip.x - base.x, tip.y - base.y);
    if (!(length > 0)) continue;
    const nx = -(tip.y - base.y) / length * 0.4 * data.scale;
    const ny = (tip.x - base.x) / length * 0.4 * data.scale;
    primitives.push(triangleOutline(
      world(point(base.x + nx, base.y + ny)),
      world(point(base.x - nx, base.y - ny)),
      world(tip),
      reasons,
    ));

    // Text sits behind the arrow start, as in MiniCAD's world-space placement.
    const center = world(point(arrow.points[0].x, arrow.points[0].y - 0.3 * nominal));
    const width = (arrow.value === '\u4e0a' ? 0.617648 : 0.562501) * height;
    let insetX: number;
    let insetY: number;
    if (Math.abs(dy) >= Math.abs(dx)) {
      insetX = center.x - width / 2;
      insetY = dy > 0 ? center.y - nominal : center.y;
    } else {
      insetX = dx > 0 ? center.x - width : center.x;
      insetY = center.y - nominal / 2;
    }
    if (!(height > 0) || !finite(insetX) || !finite(insetY)) continue;
    primitives.push(text(insetX, insetY + height / 2, 0, height, 0, arrow.value, 4));
  }
  return primitives;
}
