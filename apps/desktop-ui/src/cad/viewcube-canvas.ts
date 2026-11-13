// The ViewCube's 2D-canvas geometry: project the six faces of a unit cube with
// the host camera basis, pick the face under a point, and paint them.
//
// This module is **DOM-free**: it takes a `CanvasRenderingContext2D` (a plain
// value, so tests can pass a recorder), a [`CubePalette`] of literal colours and
// a `CubeDrawState`, and it never reads a CSS token or a host field itself. The
// component owns the element, the device-pixel transform, the clear and the
// theme read; this file owns only the maths.
//
// The cube is drawn in **CSS pixels inside a `sizePx × sizePx` box** and the
// caller must already have applied its device-pixel transform. Depth arrives from
// the basis (`cad/viewcube.ts`) — no camera is invented here. An orthographic
// host gets an exact parallel projection; a perspective host gets one mild,
// deterministic lens (see [`PERSPECTIVE_STRENGTH`]).
import type { StandardViewName } from './view3d.ts';
import type { Vec3, ViewCubeBasis } from './viewcube.ts';

/** One corner/centroid in CSS px inside the cube's `sizePx × sizePx` box. */
export interface CubePoint {
  readonly x: number;
  readonly y: number;
}

/** The four screen-space corners of one face, in a closed cycle. */
export type CubeQuadPoints = readonly [CubePoint, CubePoint, CubePoint, CubePoint];

/** One face that is actually facing the viewer. */
export interface CubeQuad {
  readonly id: CubeFaceId;
  /** The host standard-view key the face sends. */
  readonly view: StandardViewName;
  readonly points: CubeQuadPoints;
  /** Mean of the four corners (where the face label sits). */
  readonly centroid: CubePoint;
  /** The face centre's depth toward the viewer (`+` is nearer). */
  readonly depth: number;
}

/** The six cube faces; also the DOM/`data-*` identity of each one. */
export type CubeFaceId = 'front' | 'back' | 'left' | 'right' | 'top' | 'bottom';

/**
 * One face definition in **cube-local** coordinates.
 *
 * Local axes: `u = [1,0,0]`, `v = [0,1,0]`, `w = [0,0,1]`, where the cube-local
 * frame is already the labelled one (`w` = front, `-v` = top; see
 * `cad/viewcube.ts` for the world mapping).
 */
export interface CubeFaceDefinition {
  readonly id: CubeFaceId;
  readonly view: StandardViewName;
  /** Short upright glyph painted on the face (ASCII; the catalog owns the name). */
  readonly short: string;
  /** Outward face normal in cube-local coordinates. */
  readonly normal: Vec3;
  /** The face's two in-plane local axes, used to keep the label upright. */
  readonly plane: readonly [Vec3, Vec3];
  /** The four corners in cube-local coordinates (a closed cycle). */
  readonly corners: readonly [Vec3, Vec3, Vec3, Vec3];
}

const U: Vec3 = [1, 0, 0];
const V: Vec3 = [0, 1, 0];
const W: Vec3 = [0, 0, 1];
const NEG_U: Vec3 = [-1, 0, 0];
const NEG_V: Vec3 = [0, -1, 0];
const NEG_W: Vec3 = [0, 0, -1];

/**
 * The six faces in the same order as the standard-view panel.
 *
 * `short` is the painted glyph (`FRT`…), not the accessible name: the catalog's
 * `viewcube.face.*` keys are the button labels and titles.
 */
export const CUBE_FACES: readonly CubeFaceDefinition[] = Object.freeze([
  {
    id: 'front',
    view: 'front',
    short: 'FRT',
    normal: W,
    plane: [U, V],
    corners: [[-1, -1, 1], [1, -1, 1], [1, 1, 1], [-1, 1, 1]],
  },
  {
    id: 'back',
    view: 'back',
    short: 'BCK',
    normal: NEG_W,
    plane: [U, V],
    corners: [[1, -1, -1], [-1, -1, -1], [-1, 1, -1], [1, 1, -1]],
  },
  {
    id: 'right',
    view: 'right',
    short: 'RGT',
    normal: U,
    plane: [V, W],
    corners: [[1, -1, -1], [1, -1, 1], [1, 1, 1], [1, 1, -1]],
  },
  {
    id: 'left',
    view: 'left',
    short: 'LFT',
    normal: NEG_U,
    plane: [V, W],
    corners: [[-1, -1, 1], [-1, -1, -1], [-1, 1, -1], [-1, 1, 1]],
  },
  {
    id: 'top',
    view: 'top',
    short: 'TOP',
    normal: NEG_V,
    plane: [U, W],
    corners: [[-1, -1, -1], [1, -1, -1], [1, -1, 1], [-1, -1, 1]],
  },
  {
    id: 'bottom',
    view: 'bottom',
    short: 'BOT',
    normal: V,
    plane: [U, W],
    corners: [[-1, 1, 1], [1, 1, 1], [1, 1, -1], [-1, 1, -1]],
  },
]);

/** The painted glyph of one face. */
const SHORT_LABELS: Readonly<Record<CubeFaceId, string>> = Object.freeze(
  Object.fromEntries(CUBE_FACES.map(face => [face.id, face.short])) as Record<CubeFaceId, string>,
);

/**
 * The largest `|screen component|` any corner of a `[-1, 1]³` cube can reach for
 * an orthonormal basis — `√3`, the fully tilted (isometric-class) silhouette.
 *
 * The fitted radius divides by this, so a face-on cube is genuinely smaller than
 * the box: the silhouette grows as the cube turns (a real projection does that),
 * and no orientation can leave the box.
 */
export const MAX_CUBE_EXTENT = Math.sqrt(3);

/**
 * The perspective lens: the corner depth (normalised to `[-1, 1]` by
 * [`MAX_CUBE_EXTENT`]) lifts the near corner by this fraction and sinks the far
 * one; the both-ends divisor means the near-most corner never exceeds the
 * orthographic fit. `0.12` is deliberately mild — a depth cue, not a fisheye.
 */
export const PERSPECTIVE_STRENGTH = 0.12;

/** How much of the box the worst-case silhouette may use. */
const FIT = 0.98;
/** A face whose normal depth is not above this is edge-on (never painted). */
const BACKFACE_EPSILON = 1e-9;
/** Below this share of the box's short edge a face is too foreshortened to label. */
const LABEL_MIN_EDGE_RATIO = 0.16;
/** The painted label's font range, in CSS px. */
const LABEL_MIN_FONT = 6;
const LABEL_MAX_FONT = 10;
/** Roughly how many label widths (`FRT`) fit across the face. */
const LABEL_WIDTH_FACTOR = 2.4;

/** The colours the renderer needs, read from the CSS tokens by the caller. */
export interface CubePalette {
  readonly front: string;
  readonly back: string;
  readonly left: string;
  readonly right: string;
  readonly top: string;
  readonly bottom: string;
  readonly edge: string;
  readonly text: string;
  readonly textDisabled: string;
  readonly accent: string;
  readonly hover: string;
}

/** Hover/focus/disabled styling for one paint. */
export interface CubeDrawState {
  /** The box's CSS size (the same `sizePx` given to [`projectCube`]). */
  readonly sizePx: number;
  /** The face under the pointer, or `null`/absent. */
  readonly hover?: CubeFaceId | null;
  /** The face whose hidden button has keyboard focus, or `null`/absent. */
  readonly focus?: CubeFaceId | null;
  /** Faces this carrier/host refuses right now. */
  readonly disabled?: readonly CubeFaceId[] | null;
  /** The UI font family the labels should use. */
  readonly fontFamily?: string | null;
}

/** The token each palette entry comes from (documented, not re-derived). */
const PALETTE_TOKENS: Readonly<Record<keyof CubePalette, string>> = Object.freeze({
  front: '--cube-front',
  back: '--cube-back',
  left: '--cube-left',
  right: '--cube-right',
  top: '--cube-top',
  bottom: '--cube-bottom',
  edge: '--line-soft',
  text: '--text',
  textDisabled: '--text-disabled',
  accent: '--accent',
  hover: '--hover',
});

/**
 * Read the palette from a CSS-token reader (the component passes
 * `token => getComputedStyle(root).getPropertyValue(token)`).
 *
 * A missing/blank token returns `null` instead of a guessed colour: the caller
 * shows the explicit "render unavailable" state, so the cube can never paint
 * with a stale or invented fill.
 */
export function readCubePalette(get: (token: string) => string): CubePalette | null {
  const entries = Object.entries(PALETTE_TOKENS) as [keyof CubePalette, string][];
  const values: Partial<Record<keyof CubePalette, string>> = {};
  for (const [key, token] of entries) {
    const value = get(token).trim();
    if (!value) return null;
    values[key] = value;
  }
  return values as CubePalette;
}

/** One projected corner: CSS px inside the box plus its depth toward the viewer. */
function projectCorner(
  local: Vec3,
  axes: ViewCubeBasis['axes'],
  radius: number,
  centre: number,
  perspective: boolean,
): { readonly point: CubePoint; readonly depth: number } {
  const [axisU, axisV, axisW] = axes;
  const sx = local[0] * axisU[0] + local[1] * axisV[0] + local[2] * axisW[0];
  const sy = local[0] * axisU[1] + local[1] * axisV[1] + local[2] * axisW[1];
  const depth = local[0] * axisU[2] + local[1] * axisV[2] + local[2] * axisW[2];
  // Nearer corners grow, farther ones shrink; at depth 0 the lens is neutral
  // except for the both-ends divisor, which keeps the near-most corner at the
  // orthographic fit.
  const lens = perspective
    ? (1 + PERSPECTIVE_STRENGTH * (depth / MAX_CUBE_EXTENT)) / (1 + PERSPECTIVE_STRENGTH)
    : 1;
  return {
    point: { x: centre + radius * sx * lens, y: centre + radius * sy * lens },
    depth,
  };
}

/**
 * Project the visible faces of the cube, sorted back-to-front.
 *
 * `scale` multiplies the fitted radius (default `1`); it exists for tests and a
 * deliberate size tweak, and values outside a finite positive number fall back
 * to `1` rather than producing NaN points.
 */
export function projectCube(
  basis: ViewCubeBasis,
  sizePx: number,
  scale = 1,
): readonly CubeQuad[] {
  const size = Number.isFinite(sizePx) && sizePx > 0 ? sizePx : 0;
  if (size === 0) return [];
  const multiplier = Number.isFinite(scale) && scale > 0 ? scale : 1;
  const radius = ((size / 2) * FIT / MAX_CUBE_EXTENT) * multiplier;
  const centre = size / 2;
  const perspective = basis.projection === 'perspective';
  const [axisU, axisV, axisW] = basis.axes;
  const quads: CubeQuad[] = [];
  for (const face of CUBE_FACES) {
    // Backface culling: a face is visible only when its outward normal points
    // toward the viewer. An exactly edge-on face is not drawn at all.
    const normalDepth = face.normal[0] * axisU[2]
      + face.normal[1] * axisV[2]
      + face.normal[2] * axisW[2];
    if (!(normalDepth > BACKFACE_EPSILON)) continue;
    const corners = face.corners.map(corner => projectCorner(corner, basis.axes, radius, centre, perspective));
    const points = [
      corners[0].point,
      corners[1].point,
      corners[2].point,
      corners[3].point,
    ] as CubeQuadPoints;
    quads.push({
      id: face.id,
      view: face.view,
      points,
      centroid: {
        x: (points[0].x + points[1].x + points[2].x + points[3].x) / 4,
        y: (points[0].y + points[1].y + points[2].y + points[3].y) / 4,
      },
      depth: (corners[0].depth + corners[1].depth + corners[2].depth + corners[3].depth) / 4,
    });
  }
  // Back-to-front: a nearer face paints over a farther one at a shared edge.
  quads.sort((left, right) => left.depth - right.depth);
  return quads;
}

/** Whether `point` is inside the convex quad `corners` (even-odd crossing). */
function containsPoint(corners: CubeQuadPoints, point: CubePoint): boolean {
  let inside = false;
  for (let index = 0, previous = corners.length - 1; index < corners.length; previous = index, index += 1) {
    const a = corners[index];
    const b = corners[previous];
    const straddles = (a.y > point.y) !== (b.y > point.y);
    if (!straddles) continue;
    const crossingX = ((b.x - a.x) * (point.y - a.y)) / (b.y - a.y) + a.x;
    if (point.x < crossingX) inside = !inside;
  }
  return inside;
}

/**
 * The face under `point`, or `null`.
 *
 * Only the quads in `quads` are candidates — they are already the visible ones,
 * so a click can never select a face the user cannot see. At a shared edge the
 * nearer face (larger depth) wins.
 */
export function pickFace(quads: readonly CubeQuad[], point: CubePoint): CubeFaceId | null {
  let best: CubeQuad | null = null;
  for (const quad of quads) {
    if (!containsPoint(quad.points, point)) continue;
    if (!best || quad.depth > best.depth) best = quad;
  }
  return best ? best.id : null;
}

/** A short 2D vector helper for the label frame. */
function edgeVector(from: CubePoint, to: CubePoint): CubePoint {
  return { x: to.x - from.x, y: to.y - from.y };
}

/** Length of a 2D vector. */
function length(v: CubePoint): number {
  return Math.hypot(v.x, v.y);
}

/** The scale that makes `v` unit length, or `null` when it has no direction. */
function unit(v: CubePoint): CubePoint | null {
  const size = length(v);
  if (!Number.isFinite(size) || size < 1e-9) return null;
  return { x: v.x / size, y: v.y / size };
}

/**
 * The upright text frame of one face, or `null` when the face is too edge-on.
 *
 * The frame is derived from the **projected face axes** (the quad's own edges),
 * never from a fixed per-face rule: `up` is whichever in-plane direction points
 * most toward the top of the screen, and `right` is its perpendicular. That makes
 * an upside-down or mirrored label impossible — the label turns with the face,
 * and the determinant of the resulting transform is always `+1`. A face whose
 * projected short edge is below [`LABEL_MIN_EDGE_RATIO`] of the box gets no label
 * at all rather than a squashed one.
 */
function labelFrame(
  quad: CubeQuad,
  sizePx: number,
): { readonly up: CubePoint; readonly right: CubePoint; readonly fontPx: number } | null {
  const first = unit(edgeVector(quad.points[0], quad.points[1]));
  const second = unit(edgeVector(quad.points[0], quad.points[3]));
  if (!first || !second) return null;
  const firstLength = length(edgeVector(quad.points[0], quad.points[1]));
  const secondLength = length(edgeVector(quad.points[0], quad.points[3]));
  if (Math.min(firstLength, secondLength) < sizePx * LABEL_MIN_EDGE_RATIO) return null;
  // Prefer the direction that points most toward the top of the screen; ties
  // (a face-on face) keep the first axis so the label never flickers between
  // two equivalent frames.
  const candidates: readonly CubePoint[] = [
    first,
    { x: -first.x, y: -first.y },
    second,
    { x: -second.x, y: -second.y },
  ];
  let up = candidates[0];
  for (const candidate of candidates) {
    if (candidate.y < up.y - 1e-9) up = candidate;
  }
  const right = { x: -up.y, y: up.x };
  // The label runs along `right`; its available width is the edge it aligns
  // with (the other in-plane axis).
  const alongFirst = (Math.abs(up.x - first.x) < 1e-6 && Math.abs(up.y - first.y) < 1e-6)
    || (Math.abs(up.x + first.x) < 1e-6 && Math.abs(up.y + first.y) < 1e-6);
  const widthAxis = alongFirst ? secondLength : firstLength;
  const fontPx = Math.floor(Math.min(
    LABEL_MAX_FONT,
    Math.min(firstLength, secondLength) * 0.5,
    widthAxis / LABEL_WIDTH_FACTOR,
  ));
  if (fontPx < LABEL_MIN_FONT) return null;
  return { up, right, fontPx };
}

/** Paint one face label with the upright frame from [`labelFrame`]. */
function drawLabel(
  ctx: CanvasRenderingContext2D,
  quad: CubeQuad,
  color: string,
  fontFamily: string,
  sizePx: number,
): void {
  const frame = labelFrame(quad, sizePx);
  if (!frame) return;
  ctx.save();
  ctx.translate(quad.centroid.x, quad.centroid.y);
  // Local text x → `right`, local text y (down) → `-up`; determinant +1, so the
  // glyph can never be mirrored.
  ctx.transform(frame.right.x, frame.right.y, -frame.up.x, -frame.up.y, 0, 0);
  ctx.font = `700 ${frame.fontPx}px ${fontFamily}`;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  ctx.fillStyle = color;
  ctx.fillText(SHORT_LABELS[quad.id], 0, 0);
  ctx.restore();
}

/**
 * Paint the projected cube.
 *
 * Fills follow the face's light step (the `--cube-*` token), edges are 1 CSS px
 * (accent and slightly heavier for hover/focus), a disabled face keeps its fill
 * but gets a dashed edge and disabled ink, and every readable face carries its
 * short upright glyph. The caller owns the clear and the device-pixel transform;
 * this function draws in CSS px.
 */
export function drawCube(
  ctx: CanvasRenderingContext2D,
  quads: readonly CubeQuad[],
  palette: CubePalette,
  state: CubeDrawState,
): void {
  const disabled = new Set<CubeFaceId>(state.disabled ?? []);
  const hovered = state.hover ?? null;
  const focused = state.focus ?? null;
  const fontFamily = state.fontFamily && state.fontFamily.trim() ? state.fontFamily : 'sans-serif';
  for (const quad of quads) {
    const highlighted = quad.id === hovered || quad.id === focused;
    const faceDisabled = disabled.has(quad.id);
    ctx.save();
    ctx.beginPath();
    ctx.moveTo(quad.points[0].x, quad.points[0].y);
    for (let index = 1; index < quad.points.length; index += 1) {
      ctx.lineTo(quad.points[index].x, quad.points[index].y);
    }
    ctx.closePath();
    ctx.fillStyle = quad.id === hovered ? palette.hover : palette[quad.id];
    ctx.fill();
    ctx.lineWidth = highlighted ? 1.5 : 1;
    ctx.strokeStyle = highlighted ? palette.accent : palette.edge;
    ctx.setLineDash(faceDisabled ? [2, 2] : []);
    ctx.stroke();
    ctx.setLineDash([]);
    ctx.restore();
    drawLabel(ctx, quad, faceDisabled ? palette.textDisabled : palette.text, fontFamily, state.sizePx);
  }
}
