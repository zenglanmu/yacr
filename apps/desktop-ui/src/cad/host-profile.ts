// The one capability key of `cad_status.hostProfile` that has its own reader.
//
// `cad-status.ts` owns the snapshot shape and validates every key it is given;
// this module owns the **orbit** capability's rule, next to the module that acts
// on it (`cad/orbit.ts`), so the two cannot drift. Keeping it here also keeps the
// status parser a list of one-line calls instead of growing a second copy of the
// orbit story.
//
// # Semantics
//
// `hostProfile.orbit` answers one question: **can this carrier orbit at all?**
//
//   * `false` — an affirmative limitation. The `dom-canvas` wasm session has no
//     3D camera and would answer `unsupported-runtime: cad_orbit`; the chrome
//     disables the drag with `view.orbit_unavailable` instead of offering it.
//   * `true` — an explicit yes.
//   * `null` / absent — undeclared. An older host (and the desktop
//     `native-below` carriers) keeps the historic rule: orbit is allowed once a
//     3D drawing is open.
//
// Anything else (a string, a number, an object) is a contract violation. It is
// refused rather than coerced: `"false"` is truthy, and a truthy read would
// enable a gesture the carrier explicitly cannot honour.

/**
 * Validate `cad_status.hostProfile.orbit`.
 *
 * Throws the parser's own `CAD_STATUS_INVALID` for a present, non-boolean value.
 * `undefined` and `null` are the documented "undeclared" answers and pass.
 */
export function parseOrbitCapability(value: unknown): void {
  if (value === undefined || value === null) return;
  if (typeof value !== 'boolean') throw Error('CAD_STATUS_INVALID');
}
