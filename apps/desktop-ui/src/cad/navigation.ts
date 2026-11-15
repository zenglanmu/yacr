// The `cad_status.navigation` block: its wire shape and its one reader.
//
// `cad-status.ts` owns the snapshot shape and calls this reader, so the status
// parser stays a list of one-line calls (the same reason `cad/host-profile.ts`
// owns the orbit capability instead of growing the parser).
//
// # One strict part, one lenient field
//
// The four display switches are strict: a partial block would hide a real host
// state, so a missing or wrongly typed switch is a contract violation.
//
// `orbitSensitivityPercent` is additive and deliberately lenient. An older host
// omits it, and a malformed or out-of-range value is read as 100 rather than
// failing the whole status: it only scales a 3D drag rate, so refusing the
// payload would cost the user the session over a comfort setting. The bounds
// are the same ones `cad/orbit.ts` applies to the rate itself, so the block and
// the gesture cannot disagree.
import {
  ORBIT_SENSITIVITY_DEFAULT_PERCENT,
  ORBIT_SENSITIVITY_MAX_PERCENT,
  ORBIT_SENSITIVITY_MIN_PERCENT,
} from './orbit.ts';

/**
 * The host's navigation-chrome switches (`cad_status.navigation`): AutoCAD's
 * `NAVVCUBEDISPLAY` bit code plus the two `DISPLAYVIEWCUBEIN2D/3D` registry
 * gates, the navbar visibility and the 3D orbit drag rate. Always host truth —
 * the chrome only hides, shows or scales from these values.
 */
export interface CadNavigationSettings {
  readonly navbarVisible: boolean;
  /** `0..=3`: 1 = 3D styles, 2 = 2D styles, 3 = both (AutoCAD bit code). */
  readonly viewCubeDisplay: number;
  readonly viewCubeIn2d: boolean;
  readonly viewCubeIn3d: boolean;
  /**
   * 3D orbit drag rate as a percentage of the base rate,
   * `25..=200` (100 = the base rate). One multiplier for mouse and touch alike.
   *
   * Optional because an older host does not publish it; the readers fall back to
   * 100 (`?? 100`). A value outside the range is read as 100 rather than
   * rejected — this is a comfort setting, and a bad number must never take the
   * whole status down.
   */
  readonly orbitSensitivityPercent?: number;
}

/** Whether a reported sensitivity is usable as-is (finite, `25..=200`). */
function validOrbitSensitivity(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value)
    && value >= ORBIT_SENSITIVITY_MIN_PERCENT
    && value <= ORBIT_SENSITIVITY_MAX_PERCENT;
}

/**
 * Parse the navigation block.
 *
 * A present block must carry the four switches (both real booleans and a 0..3
 * bit code). The orbit sensitivity keeps its documented fallback: absent stays
 * absent (the readers apply 100), and an unusable value is normalised to 100 in
 * a **copy** — a valid payload keeps its identity.
 */
export function parseCadNavigation(value: unknown): CadNavigationSettings {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw Error('CAD_STATUS_INVALID');
  }
  const navigation = value as Record<string, unknown>;
  const display = navigation.viewCubeDisplay;
  if (typeof navigation.navbarVisible !== 'boolean'
    || typeof navigation.viewCubeIn2d !== 'boolean'
    || typeof navigation.viewCubeIn3d !== 'boolean'
    || typeof display !== 'number' || !Number.isInteger(display)
    || display < 0 || display > 3) {
    throw Error('CAD_STATUS_INVALID');
  }
  const percent = navigation.orbitSensitivityPercent;
  if (percent === undefined || validOrbitSensitivity(percent)) {
    return value as CadNavigationSettings;
  }
  return {
    ...(value as CadNavigationSettings),
    orbitSensitivityPercent: ORBIT_SENSITIVITY_DEFAULT_PERCENT,
  };
}
