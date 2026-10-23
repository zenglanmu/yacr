// The host-reported pixel carrier for the CAD partition (ADR 0024 D1/D3).
//
// There are two **modelled** carriers and the React tree never forks by
// platform: the host reports a `surfaceMode` data flag and this module is the
// only place that reads it. `native-below` (Linux native Wayland, Windows) is
// today's compositing: the host puts its own wgpu surface under this WebView and
// `.native-input` paints nothing. `dom-canvas` (the staged X11/XWayland and
// browser path, **not implemented yet**) instead draws into a `<canvas>` inside
// this same tree, so no second native surface exists.
//
// The fallback policy is the same as every other optional host field the chrome
// reads (`shell.status?.crosshair`): a missing field is **not** a capability
// guess. `native-below` is the one carrier that exists in every shipped host,
// so an older host that reports nothing keeps exactly its current behaviour.

/** The host composites its own surface below this DOM tree. */
export const SURFACE_MODE_NATIVE_BELOW = 'native-below';

/** The WebView composes the pixels itself, inside a DOM `<canvas>`. */
export const SURFACE_MODE_DOM_CANVAS = 'dom-canvas';

/**
 * The carrier named by the host shell state.
 *
 * Only the exact reported string `"dom-canvas"` selects the in-DOM carrier.
 * Everything else — a missing `status`, a missing `surfaceMode` (an older host),
 * or an unrecognised value — resolves to `native-below`: a new mode must add its
 * own branch here (and its own carrier) rather than be inferred from a partial
 * match or a platform hint.
 */
export function surfaceModeFromShell(shell) {
  return shell?.status?.surfaceMode === SURFACE_MODE_DOM_CANVAS
    ? SURFACE_MODE_DOM_CANVAS
    : SURFACE_MODE_NATIVE_BELOW;
}
