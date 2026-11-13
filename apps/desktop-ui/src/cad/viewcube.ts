// The ViewCube's host-derived facts: visibility (NAVVCUBEDISPLAY + the two
// 2D/3D gates), the AutoCAD defaults an older host implies, and the host camera
// basis in **screen space**, which is what the 2D-canvas cube is drawn from.
//
// Nothing here guesses a camera. `cad_status.view` is the host's own state; when
// it is absent/non-finite/degenerate the cube is rendered as an explicit
// "orientation unavailable" state instead of drawing from a made-up basis. This
// module is the **only** place the host basis becomes screen space, so the
// renderer (`cad/viewcube-canvas.ts`) and the component stay dumb consumers.
//
// # Derivation
//
// In host camera terms:
//
//   `z = normalize(eye - target)` — from the drawing toward the viewer;
//   `x = normalize(cross(up, z))` — the camera's right;
//   `y = cross(z, x)` — the camera's up.
//
// A world vector `a` therefore lands at `[dot(a, x), -dot(a, y), dot(a, z)]` in
// screen space: screen x grows right, screen y grows **down** (CSS/canvas), and
// depth grows **toward the viewer**. The cube is built in AutoCAD's world
// convention (Z up, front = −Y) while its own faces are labelled in a screen-local
// frame, so the fixed permutation below maps the two:
//
//   cube +X (right face)  = world +X,
//   cube +Y (bottom face) = world −Z,
//   cube +Z (front face)  = world −Y.
//
// Each local axis' image is the screen-space vector the renderer projects:
//
//   axisU (world +X) = [ x0, -y0,  z0]
//   axisV (world −Z) = [-x2,  y2, -z2]
//   axisW (world −Y) = [-x1,  y1, -z1]
//
// The canonical **front** view (`eye −Y`, `up +Z`) is therefore the identity
// basis, and TOP/RIGHT match the columns the old CSS `matrix3d` produced — the
// cube shows the host camera's real orientation, not a decorative spin.
import type { CadNavigationSettings, CadViewOrientation } from './cad-status.ts';

/** A finite 3D vector; the host's basis uses the same `[x, y, z]` shape. */
export type Vec3 = readonly [number, number, number];

/**
 * The host camera basis as the cube needs it: three cube-local axes projected
 * into screen space plus the host's own lens.
 *
 * Each axis is the image of one **cube-local** axis:
 * `axes[0]` = cube +X (right face), `axes[1]` = cube +Y (bottom face),
 * `axes[2]` = cube +Z (front face); every axis is
 * `[screenX (right, +), screenY (down, +), depth (toward the viewer, +)]`.
 */
export interface ViewCubeBasis {
  readonly axes: readonly [Vec3, Vec3, Vec3];
  readonly projection: 'orthographic' | 'perspective';
}

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

/** A basis component with `-0` folded to `0` (deep equality, not cosmetics). */
function axisNumber(value: number): number {
  return value === 0 ? 0 : value;
}

/**
 * The host camera basis in screen space, or `null` when there is none.
 *
 * `null` means the basis is absent, non-finite or degenerate; the caller renders
 * the unavailable state and never draws from a guessed orientation. The
 * projection is carried through as reported: an unknown value takes the
 * no-lens (orthographic) path rather than inventing a perspective.
 */
export function viewCubeBasis(view: CadViewOrientation | null | undefined): ViewCubeBasis | null {
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
  return {
    axes: [
      [axisNumber(x[0]), axisNumber(-y[0]), axisNumber(z[0])],
      [axisNumber(-x[2]), axisNumber(y[2]), axisNumber(-z[2])],
      [axisNumber(-x[1]), axisNumber(y[1]), axisNumber(-z[1])],
    ],
    projection: view.projection === 'perspective' ? 'perspective' : 'orthographic',
  };
}
