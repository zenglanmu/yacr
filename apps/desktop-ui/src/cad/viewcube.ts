// The ViewCube's host-derived facts: visibility (NAVVCUBEDISPLAY + the two
// 2D/3D gates), the AutoCAD defaults an older host implies, and the CSS rotation
// that shows the host camera's real basis.
//
// Nothing here guesses a camera. `cad_status.view` is the host's own state; when
// it is absent/non-finite/degenerate the cube is rendered as an explicit
// "orientation unavailable" state instead of rotating from a made-up basis. The
// conversion below is the only place the host basis becomes a CSS matrix, so the
// component stays a renderer.
import type { CadNavigationSettings, CadViewOrientation } from './cad-status.ts';

/** A finite 3D vector; the host's basis uses the same `[x, y, z]` shape. */
export type Vec3 = readonly [number, number, number];

/**
 * AutoCAD's defaults for a host that predates `cad_status.navigation`: the
 * navbar is on and the cube shows in both 2D and 3D styles.
 */
export const DEFAULT_NAVIGATION: CadNavigationSettings = Object.freeze({
  navbarVisible: true,
  viewCubeDisplay: 3,
  viewCubeIn2d: true,
  viewCubeIn3d: true,
});

/**
 * The navigation switches the chrome acts on.
 *
 * `cad_status` validates the block when it is present, so this only fills in the
 * documented defaults for an **absent** block (an older host). A field that is
 * somehow missing from a present block keeps its default rather than becoming
 * `undefined`; the stylesheet and the rail must still agree.
 */
export function navigationSettings(
  navigation: CadNavigationSettings | null | undefined,
): CadNavigationSettings {
  if (!navigation) return DEFAULT_NAVIGATION;
  const display = navigation.viewCubeDisplay;
  return {
    navbarVisible: typeof navigation.navbarVisible === 'boolean'
      ? navigation.navbarVisible
      : DEFAULT_NAVIGATION.navbarVisible,
    viewCubeDisplay: Number.isInteger(display) && display >= 0 && display <= 3
      ? display
      : DEFAULT_NAVIGATION.viewCubeDisplay,
    viewCubeIn2d: typeof navigation.viewCubeIn2d === 'boolean'
      ? navigation.viewCubeIn2d
      : DEFAULT_NAVIGATION.viewCubeIn2d,
    viewCubeIn3d: typeof navigation.viewCubeIn3d === 'boolean'
      ? navigation.viewCubeIn3d
      : DEFAULT_NAVIGATION.viewCubeIn3d,
  };
}

/** The AutoCAD display class of a view: `3` for 3D styles, `2` for 2D styles. */
export function viewCubeClass(view: CadViewOrientation | null | undefined): 2 | 3 {
  return view?.mode === '3d' ? 3 : 2;
}

/**
 * Whether the host's display switches allow the cube for this view.
 *
 * The class bit follows the live view mode (`viewCubeDisplay` 1 = 3D styles,
 * 2 = 2D styles, 3 = both) and the per-class gate follows AutoCAD's
 * `DISPLAYVIEWCUBEIN2D/3D`. A `null` view keeps the 2D class exactly like the
 * host's own 2D default, so a host that reports "no camera" still shows the
 * explicit unavailable state when the 2D gate allows it — the null stays
 * visible rather than silently removing the control.
 */
export function viewCubeVisible(
  view: CadViewOrientation | null | undefined,
  navigation: CadNavigationSettings | null | undefined,
): boolean {
  const settings = navigationSettings(navigation);
  const viewClass = viewCubeClass(view);
  const bit = viewClass === 3 ? 1 : 2;
  if ((settings.viewCubeDisplay & bit) === 0) return false;
  return viewClass === 3 ? settings.viewCubeIn3d : settings.viewCubeIn2d;
}

/** `a - b`, component-wise. */
function subtract(a: Vec3, b: Vec3): Vec3 {
  return [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
}

/** The right-handed cross product `a × b`. */
function cross(a: Vec3, b: Vec3): Vec3 {
  return [
    a[1] * b[2] - a[2] * b[1],
    a[2] * b[0] - a[0] * b[2],
    a[0] * b[1] - a[1] * b[0],
  ];
}

/** Unit length, or `null` when the vector has no direction (never a guess). */
function normalize(v: Vec3): Vec3 | null {
  const length = Math.hypot(v[0], v[1], v[2]);
  if (!Number.isFinite(length) || length < 1e-9) return null;
  return [v[0] / length, v[1] / length, v[2] / length];
}

/** A CSS number with the host basis' noise trimmed and `-0` folded to `0`. */
function cssNumber(value: number): string {
  const rounded = Number(value.toFixed(6));
  return String(Object.is(rounded, -0) ? 0 : rounded);
}

/**
 * The `matrix3d(...)` that makes the cube show the host camera's basis.
 *
 * Constructed in the host's camera terms:
 *
 *   `z = normalize(eye - target)` — from the drawing toward the viewer;
 *   `x = normalize(cross(up, z))` — the camera's right;
 *   `y = cross(z, x)` — the camera's up.
 *
 * `[x; -y; z]` is the world→CSS view transform (CSS `+y` points down). The cube
 * is built in AutoCAD's world convention (**Z up**, front = −Y), while the CSS
 * faces are placed in a screen-local frame (front = +Z, top = −Y). The fixed
 * permutation `P` maps the host world axes onto those face labels:
 *
 *   cube +X = world +X (right), cube −Y = world +Z (top), cube +Z = world −Y
 *   (front), so the face that reaches the reader is the one whose label matches
 *   the host's real view direction (TOP view shows the T face, FRONT shows F).
 *
 * The result is the identity for the canonical **front** view (`eye −Y`,
 * `up +Z`), and a pure rotation (determinant +1) for every view. `null` means
 * the basis is absent or degenerate; the caller renders the unavailable state
 * and never animates from a guessed orientation.
 */
export function viewCubeMatrix(view: CadViewOrientation | null | undefined): string | null {
  if (!view) return null;
  const eye = view.eye;
  const target = view.target;
  const up = view.up;
  if (![eye, target, up].every(point => point.every(Number.isFinite))) return null;
  const z = normalize(subtract(eye, target));
  if (!z) return null;
  const x = normalize(cross(up, z));
  if (!x) return null;
  const y = cross(z, x);
  // Column-major `matrix3d`. Columns are `M = [x; -y; z] · P` with the world→face
  // permutation above: c0 = (x0,−y0,z0), c1 = (−x2,y2,−z2), c2 = (−x1,y1,−z1).
  const value = [
    x[0], -y[0], z[0], 0,
    -x[2], y[2], -z[2], 0,
    -x[1], y[1], -z[1], 0,
    0, 0, 0, 1,
  ].map(cssNumber);
  return `matrix3d(${value.join(', ')})`;
}
