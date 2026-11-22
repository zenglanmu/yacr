// The orientation ball's 2D-canvas geometry: project the six standard-view
// directions onto an opaque sphere with the host camera basis, pick the cap
// under a point, and paint the ball.
//
// This module is **DOM-free**: it takes a `CanvasRenderingContext2D` (a plain
// value, so tests can pass a recorder), a [`SpherePalette`] of literal colours
// and a [`SphereDrawState`], and it never reads a CSS token or a host field
// itself. The component owns the element, the device-pixel transform, the clear
// and the theme read; this file owns only the maths.
//
// The ball is drawn in **CSS pixels inside a `sizePx × sizePx` box** and the
// caller must already have applied its device-pixel transform. Depth arrives from
// the basis (`cad/viewcube.ts`) — no camera is invented here. A projected sphere
// is orientation-independent in silhouette (it is a circle), so unlike the old
// cube it needs no perspective lens: the host's depth is used for culling and
// ordering only.
import type { StandardViewName } from './view3d.ts';
import type { Vec3, ViewCubeBasis } from './viewcube.ts';

/** One point in CSS px inside the ball's `sizePx × sizePx` box. */
export interface SpherePoint {
  readonly x: number;
  readonly y: number;
}

/** The six standard-view directions; also the DOM/`data-*` identity of each cap. */
export type SphereDirectionId = 'front' | 'back' | 'left' | 'right' | 'top' | 'bottom';

/**
 * One direction definition in **cube-local** coordinates.
 *
 * Local axes: `u = [1,0,0]`, `v = [0,1,0]`, `w = [0,0,1]`, where the local frame
 * is already the labelled one (`w` = front, `-v` = top; see `cad/viewcube.ts` for
 * the world mapping). Each cap sits where its outward normal meets the sphere.
 */
export interface SphereDirectionDefinition {
  readonly id: SphereDirectionId;
  readonly view: StandardViewName;
  /** Short upright glyph painted on the cap (ASCII; the catalog owns the name). */
  readonly short: string;
  /** Outward direction in cube-local coordinates. */
  readonly normal: Vec3;
}

const U: Vec3 = [1, 0, 0];
const V: Vec3 = [0, 1, 0];
const W: Vec3 = [0, 0, 1];
const NEG_U: Vec3 = [-1, 0, 0];
const NEG_V: Vec3 = [0, -1, 0];
const NEG_W: Vec3 = [0, 0, -1];

/**
 * The six standard-view directions in the same order as the standard-view panel.
 *
 * `short` is the painted glyph (`FRT`…), not the accessible name: the catalog's
 * `viewcube.face.*` keys are the button labels and titles.
 */
export const SPHERE_DIRECTIONS: readonly SphereDirectionDefinition[] = Object.freeze([
  { id: 'front', view: 'front', short: 'FRT', normal: W },
  { id: 'back', view: 'back', short: 'BCK', normal: NEG_W },
  { id: 'right', view: 'right', short: 'RGT', normal: U },
  { id: 'left', view: 'left', short: 'LFT', normal: NEG_U },
  { id: 'top', view: 'top', short: 'TOP', normal: NEG_V },
  { id: 'bottom', view: 'bottom', short: 'BOT', normal: V },
]);

/**
 * One projected cap: where its direction lands on the sphere, plus the depth that
 * decides whether it is on the front (painted/pickable) or the back (omitted).
 */
export interface SphereCap {
  readonly id: SphereDirectionId;
  /** The host standard-view key the cap sends. */
  readonly view: StandardViewName;
  readonly short: string;
  /** The cap's screen point in CSS px inside the box. */
  readonly center: SpherePoint;
  /** The painted/pickable disc radius, in CSS px. */
  readonly radius: number;
  /** The direction's depth toward the viewer (`+` is nearer). */
  readonly depth: number;
  /** Whether the cap is on the front hemisphere (above [`FRONT_DEPTH_EPSILON`]). */
  readonly visible: boolean;
}

/**
 * How much of the box the sphere's diameter fills. `0.46` leaves a 4 % margin on
 * each side for the Home button and the drop shadow while keeping the visible
 * diameter close to the box (≈ 103 px of a 112 px control).
 */
export const SPHERE_RADIUS_RATIO = 0.46;

/**
 * A cap needs at least this depth toward the viewer to be painted as a front cap.
 * Zero would include exactly edge-on directions (a sliver with no readable disc),
 * so the small positive threshold keeps the rim clean.
 */
export const FRONT_DEPTH_EPSILON = 0.08;

/** The cap disc's share of the sphere radius (clamped to a readable range). */
const CAP_RADIUS_RATIO = 0.22;
const CAP_RADIUS_MIN = 4;
const CAP_RADIUS_MAX = 12;
/** Below this disc radius a cap is a rim dot, not a readable label target. */
const LABEL_MIN_RADIUS = 7;
/** Great-circle samples per coordinate plane; 72 keeps a 103 px arc smooth. */
const GRID_SAMPLES = 72;
/** The painted label's font range, in CSS px. */
const LABEL_MIN_FONT = 6;
const LABEL_MAX_FONT = 9;
/** Pointer slop around a cap disc and the sphere itself, in CSS px. */
const PICK_SLOP_PX = 3;
/** A click inside this share of the radius has no usable screen direction. */
const CENTRE_PICK_RATIO = 0.15;
/** The angular catchment around an axis: a cap is picked within ~30°. */
const PICK_COS_LIMIT = Math.cos((30 * Math.PI) / 180);

/** The six faces of the three coordinate planes, used for the grid arcs. */
const GRID_PLANES: readonly (readonly [Vec3, Vec3])[] = Object.freeze([
  [U, V],
  [V, W],
  [W, U],
]);

/** The colours the renderer needs, read from the CSS tokens by the caller. */
export interface SpherePalette {
  readonly surface: string;
  readonly highlight: string;
  readonly rim: string;
  readonly grid: string;
  readonly cap: string;
  readonly capEdge: string;
  readonly text: string;
  readonly textDisabled: string;
  readonly accent: string;
  readonly hover: string;
}

/** Hover/focus/disabled styling for one paint. */
export interface SphereDrawState {
  /** The cap under the pointer, or `null`/absent. */
  readonly hover?: SphereDirectionId | null;
  /** The cap whose hidden button has keyboard focus, or `null`/absent. */
  readonly focus?: SphereDirectionId | null;
  /** Caps this carrier/host refuses right now. */
  readonly disabled?: readonly SphereDirectionId[] | null;
  /** The UI font family the labels should use. */
  readonly fontFamily?: string | null;
}

/** The token each palette entry comes from (documented, not re-derived). */
const PALETTE_TOKENS: Readonly<Record<keyof SpherePalette, string>> = Object.freeze({
  surface: '--ball-surface',
  highlight: '--ball-highlight',
  rim: '--ball-rim',
  grid: '--ball-grid',
  cap: '--ball-cap',
  capEdge: '--line-soft',
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
 * shows the explicit "render unavailable" state, so the ball can never paint
 * with a stale or invented fill.
 */
export function readSpherePalette(get: (token: string) => string): SpherePalette | null {
  const entries = Object.entries(PALETTE_TOKENS) as [keyof SpherePalette, string][];
  const values: Partial<Record<keyof SpherePalette, string>> = {};
  for (const [key, token] of entries) {
    const value = get(token).trim();
    if (!value) return null;
    values[key] = value;
  }
  return values as SpherePalette;
}

/** A finite positive box size, or `0` for a box that cannot have coordinates. */
function boxSize(sizePx: number): number {
  return Number.isFinite(sizePx) && sizePx > 0 ? sizePx : 0;
}

/** The sphere radius in CSS px for a validated box. */
function sphereRadius(size: number): number {
  return SPHERE_RADIUS_RATIO * size;
}

/**
 * The cap disc radius, in CSS px.
 *
 * A cap on the sphere is seen face-on at the centre and edge-on at the rim, so
 * the disc shrinks with its depth (a simple foreshortening cue): the three
 * isometric caps stay comfortably labelled, while a cap just above the front
 * threshold becomes a small rim dot instead of a half-clipped label.
 */
function capRadius(radius: number, depth: number): number {
  const facing = 0.55 + 0.45 * Math.max(0, Math.min(1, depth));
  return Math.max(CAP_RADIUS_MIN, Math.min(CAP_RADIUS_MAX, radius * CAP_RADIUS_RATIO * facing));
}

/** The label font size for a cap disc, in whole CSS px. */
function capFont(radius: number): number {
  return Math.max(LABEL_MIN_FONT, Math.min(LABEL_MAX_FONT, Math.round(radius * 0.8)));
}

/** One direction projected into the box, with the depth toward the viewer. */
function projectDirection(
  normal: Vec3,
  axes: ViewCubeBasis['axes'],
  radius: number,
  centre: number,
): { readonly point: SpherePoint; readonly depth: number } {
  const [axisU, axisV, axisW] = axes;
  const screenX = normal[0] * axisU[0] + normal[1] * axisV[0] + normal[2] * axisW[0];
  const screenY = normal[0] * axisU[1] + normal[1] * axisV[1] + normal[2] * axisW[1];
  const depth = normal[0] * axisU[2] + normal[1] * axisV[2] + normal[2] * axisW[2];
  return {
    point: { x: centre + radius * screenX, y: centre + radius * screenY },
    depth,
  };
}

/**
 * Project the six standard-view caps, sorted back-to-front.
 *
 * A `depth <= FRONT_DEPTH_EPSILON` cap is still returned (with `visible: false`)
 * so callers can reason about the whole sphere without re-deriving the maths;
 * the painter and the picker only ever use the visible ones. An invalid box has
 * no coordinates to give and returns an empty list rather than NaN points.
 */
export function projectSphere(
  basis: ViewCubeBasis,
  sizePx: number,
): readonly SphereCap[] {
  const size = boxSize(sizePx);
  if (size === 0) return [];
  const radius = sphereRadius(size);
  const centre = size / 2;
  const caps: SphereCap[] = SPHERE_DIRECTIONS.map(direction => {
    const projected = projectDirection(direction.normal, basis.axes, radius, centre);
    return {
      id: direction.id,
      view: direction.view,
      short: direction.short,
      center: projected.point,
      radius: capRadius(radius, projected.depth),
      depth: projected.depth,
      visible: projected.depth > FRONT_DEPTH_EPSILON,
    };
  });
  // Back-to-front: a nearer cap paints over a farther one if two discs touch.
  caps.sort((left, right) => left.depth - right.depth);
  return caps;
}

/**
 * Project the three great circles (the coordinate planes) for the sphere cue.
 *
 * Each circle is sampled in cube-local space and projected with the same basis
 * as the caps; runs on the back hemisphere (`depth < 0`) are dropped, so the
 * returned polylines are exactly the front arcs that rotate with the camera.
 * Every point is within one radius of the box centre by construction.
 */
export function projectSphereGrid(
  basis: ViewCubeBasis,
  sizePx: number,
): readonly (readonly SpherePoint[])[] {
  const size = boxSize(sizePx);
  if (size === 0) return [];
  const radius = sphereRadius(size);
  const centre = size / 2;
  const lines: SpherePoint[][] = [];
  for (const [first, second] of GRID_PLANES) {
    let run: SpherePoint[] = [];
    const flush = (): void => {
      if (run.length >= 2) lines.push(run);
      run = [];
    };
    for (let index = 0; index <= GRID_SAMPLES; index += 1) {
      const angle = (index / GRID_SAMPLES) * Math.PI * 2;
      const cosine = Math.cos(angle);
      const sine = Math.sin(angle);
      const normal: Vec3 = [
        first[0] * cosine + second[0] * sine,
        first[1] * cosine + second[1] * sine,
        first[2] * cosine + second[2] * sine,
      ];
      const projected = projectDirection(normal, basis.axes, radius, centre);
      if (projected.depth >= 0) run.push(projected.point);
      else flush();
    }
    flush();
  }
  return lines;
}

/** The front cap with the greatest depth (the one nearest the viewer). */
function nearestViewer(front: readonly SphereCap[]): SphereCap | null {
  let best: SphereCap | null = null;
  for (const cap of front) {
    if (!best || cap.depth > best.depth) best = cap;
  }
  return best;
}

/**
 * The standard-view cap under `point`, or `null`.
 *
 * Only front caps are candidates. Inside a cap disc the nearest disc wins; near
 * the sphere's projected centre (where the screen direction is noise) the cap
 * nearest the viewer wins; elsewhere the angular nearest axis within ~30° wins,
 * so the gap between two caps is honestly nothing. Any point outside the sphere
 * — and any point between caps — returns `null`.
 */
export function pickSphere(
  basis: ViewCubeBasis,
  point: SpherePoint,
  sizePx: number,
): SphereDirectionId | null {
  const size = boxSize(sizePx);
  if (size === 0 || !Number.isFinite(point.x) || !Number.isFinite(point.y)) return null;
  const radius = sphereRadius(size);
  const centre = size / 2;
  const dx = point.x - centre;
  const dy = point.y - centre;
  const distance = Math.hypot(dx, dy);
  // The sphere itself is opaque and exact: a point outside its silhouette is
  // never a cap, no matter how close a rim disc gets.
  if (distance > radius) return null;
  const front = projectSphere(basis, size).filter(cap => cap.visible);
  if (front.length === 0) return null;

  // A disc under the pointer: the nearest cap centre is the one the user aimed at.
  let hit: SphereCap | null = null;
  let hitDistance = Number.POSITIVE_INFINITY;
  for (const cap of front) {
    const capDistance = Math.hypot(point.x - cap.center.x, point.y - cap.center.y);
    if (capDistance <= cap.radius + PICK_SLOP_PX && capDistance < hitDistance) {
      hit = cap;
      hitDistance = capDistance;
    }
  }
  if (hit) return hit.id;

  if (distance <= radius * CENTRE_PICK_RATIO) {
    return nearestViewer(front)?.id ?? null;
  }

  const unitX = dx / distance;
  const unitY = dy / distance;
  let best: SphereCap | null = null;
  for (const cap of front) {
    const offsetX = cap.center.x - centre;
    const offsetY = cap.center.y - centre;
    const offsetLength = Math.hypot(offsetX, offsetY);
    if (offsetLength < 1e-9) continue;
    const cosine = (unitX * offsetX + unitY * offsetY) / offsetLength;
    if (cosine + 1e-9 < PICK_COS_LIMIT) continue;
    if (!best || cap.depth > best.depth) best = cap;
  }
  return best?.id ?? null;
}

/**
 * Paint the projected ball.
 *
 * The body is an opaque radial gradient lit from the top-left, the great-circle
 * grid is faint, and each front cap is a small filled disc with its short upright
 * glyph. Hover/focus uses the accent edge and the hover fill; a disabled cap
 * keeps its fill but gets a dashed edge and disabled ink. The labels are drawn in
 * screen space with no rotation, so they can never be mirrored or upside-down.
 * The caller owns the clear and the device-pixel transform; this function draws
 * in CSS px.
 */
export function drawSphere(
  ctx: CanvasRenderingContext2D,
  basis: ViewCubeBasis,
  sizePx: number,
  palette: SpherePalette,
  state: SphereDrawState,
): void {
  const size = boxSize(sizePx);
  if (size === 0) return;
  const radius = sphereRadius(size);
  const centre = size / 2;
  const disabled = new Set<SphereDirectionId>(state.disabled ?? []);
  const hovered = state.hover ?? null;
  const focused = state.focus ?? null;
  const fontFamily = state.fontFamily && state.fontFamily.trim() ? state.fontFamily : 'sans-serif';
  const caps = projectSphere(basis, size);
  const grid = projectSphereGrid(basis, size);

  // The opaque body: the highlight sits up-left of centre, the base colour at
  // the silhouette, so the flat canvas reads as a lit sphere in both themes.
  ctx.save();
  ctx.beginPath();
  ctx.arc(centre, centre, radius, 0, Math.PI * 2);
  const gradient = ctx.createRadialGradient(
    centre - radius * 0.38,
    centre - radius * 0.42,
    radius * 0.12,
    centre,
    centre,
    radius * 1.02,
  );
  gradient.addColorStop(0, palette.highlight);
  gradient.addColorStop(1, palette.surface);
  ctx.fillStyle = gradient;
  ctx.fill();
  ctx.restore();

  // Grid and caps are clipped to the sphere: a label on the rim is cut by the
  // silhouette instead of spilling into the transparent corner of the box.
  ctx.save();
  ctx.beginPath();
  ctx.arc(centre, centre, radius, 0, Math.PI * 2);
  ctx.clip();

  ctx.save();
  ctx.globalAlpha = 0.55;
  ctx.strokeStyle = palette.grid;
  ctx.lineWidth = 1;
  for (const line of grid) {
    ctx.beginPath();
    ctx.moveTo(line[0].x, line[0].y);
    for (let index = 1; index < line.length; index += 1) {
      ctx.lineTo(line[index].x, line[index].y);
    }
    ctx.stroke();
  }
  ctx.restore();

  for (const cap of caps) {
    if (!cap.visible) continue;
    const highlighted = cap.id === hovered || cap.id === focused;
    const capDisabled = disabled.has(cap.id);
    ctx.save();
    ctx.beginPath();
    ctx.arc(cap.center.x, cap.center.y, cap.radius, 0, Math.PI * 2);
    ctx.fillStyle = highlighted ? palette.hover : palette.cap;
    ctx.fill();
    ctx.lineWidth = highlighted ? 1.5 : 1;
    ctx.strokeStyle = highlighted ? palette.accent : palette.capEdge;
    ctx.setLineDash(capDisabled ? [2, 2] : []);
    ctx.stroke();
    ctx.setLineDash([]);
    ctx.restore();

    // A rim dot is a direction cue, not a label target: drawing a 6 px glyph on
    // a half-clipped disc would read as noise.
    if (cap.radius < LABEL_MIN_RADIUS) continue;

    // Screen-space, upright text: no rotate/transform, so the glyph is never
    // mirrored and never upside-down regardless of the camera.
    ctx.save();
    ctx.font = `600 ${capFont(cap.radius)}px ${fontFamily}`;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillStyle = capDisabled ? palette.textDisabled : palette.text;
    ctx.fillText(cap.short, cap.center.x, cap.center.y);
    ctx.restore();
  }
  ctx.restore();

  // The silhouette edge is painted last (and unclipped) so it stays one crisp
  // circle even where a cap touches the rim.
  ctx.save();
  ctx.beginPath();
  ctx.arc(centre, centre, radius, 0, Math.PI * 2);
  ctx.strokeStyle = palette.rim;
  ctx.lineWidth = 1;
  ctx.stroke();
  ctx.restore();
}
