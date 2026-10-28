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
//
// This module also owns the **drawing identity** of a `dom-canvas` session
// (`status.canvasSession`, relayed from the wasm session because the native shell
// holds no document state) and the deduplicating publisher of the session's own
// report. Both are pure, and both refuse to invent an identity.
//
// `shell` and `status` are host wire objects: they arrive typed `unknown` and are
// narrowed here, so nothing is claimed about a field the host did not send.

/** The carrier the host reported, as this chrome models it. */
export type SurfaceMode = 'native-below' | 'dom-canvas';

/** The host composites its own surface below this DOM tree. */
export const SURFACE_MODE_NATIVE_BELOW: SurfaceMode = 'native-below';

/** The WebView composes the pixels itself, inside a DOM `<canvas>`. */
export const SURFACE_MODE_DOM_CANVAS: SurfaceMode = 'dom-canvas';

/** One `dom-canvas` session's drawing identity, as relayed by the host. */
export interface CanvasSessionReport {
  /** The flattened, bounded drawing label (never empty). */
  readonly label: string;
  /** Reported renderer batch count, or `null` when the host sent no count. */
  readonly batches: number | null;
  /** Reported draw count, or `null` when the host sent no count. */
  readonly draws: number | null;
}

/**
 * The wire body of one report for the host's `canvas_session_status` command.
 *
 * Mutable because a count that is not a real count is *left out* (see
 * [`canvasSessionPayload`]), which is built by adding keys to the body.
 */
export interface CanvasSessionPayload {
  label: string;
  batches?: number;
  draws?: number;
}

/** A publisher that forwards only reports its predecessor did not already send. */
export interface CanvasSessionReporter {
  /** Publish `status` when it is a valid report that differs from the last one. */
  report(status: unknown): boolean;
}

/** A plain JSON object, or `null` for anything else (arrays included). */
function recordOf(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

/** The `status` object of a shell state, when the host sent one. */
function statusOf(shell: unknown): Record<string, unknown> | null {
  const outer = recordOf(shell);
  return outer === null ? null : recordOf(outer.status);
}

/**
 * The `surfaceMode` string the host reported, verbatim — or `undefined`.
 *
 * This is the raw wire value, not a decision: `"dom-canvas"` is the only spelling
 * a carrier may use, and an older host simply has no field. It exists as its own
 * function because two readers need the *reported* value rather than the
 * collapsed one: [`surfaceModeFromShell`] (which mode this chrome models) and
 * `host/runtime.ts`'s carrier state (who owns the CAD document). Both read it
 * here, so they can never disagree about what the host said.
 */
export function reportedSurfaceMode(shell: unknown): string | undefined {
  const value = statusOf(shell)?.surfaceMode;
  return typeof value === 'string' ? value : undefined;
}

/**
 * The carrier named by the host shell state.
 *
 * Only the exact reported string `"dom-canvas"` selects the in-DOM carrier.
 * Everything else — a missing `status`, a missing `surfaceMode` (an older host),
 * or an unrecognised value — resolves to `native-below`: a new mode must add its
 * own branch here (and its own carrier) rather than be inferred from a partial
 * match or a platform hint.
 */
export function surfaceModeFromShell(shell: unknown): SurfaceMode {
  return reportedSurfaceMode(shell) === SURFACE_MODE_DOM_CANVAS
    ? SURFACE_MODE_DOM_CANVAS
    : SURFACE_MODE_NATIVE_BELOW;
}

/**
 * Whether the docked palettes (layers, properties, diagnostics) exist on the
 * carrier the shell reported.
 *
 * `shell.palettesAvailable` is the one carrier flag: `false` only on a reported
 * `dom-canvas` host, which by design holds no host-side document state for the
 * palettes to project (AGENTS.md iron rule 8). This helper is the one reading of
 * that flag: every palette affordance — the ribbon entries, the status-bar chips,
 * the workspace panels and the nav-rail toggles — asks it, so a carrier without
 * the palettes hides all of them instead of leaving a button that toggles a panel
 * which is never rendered. An absent flag (older chrome state, no host answer)
 * keeps the palettes: "as today" is the default, never a guessed removal.
 */
export function palettesOffered(shell: unknown): boolean {
  return recordOf(shell)?.palettesAvailable !== false;
}

/**
 * The longest drawing label the chrome will display, in **code points**.
 *
 * A session names its own drawing, and nothing upstream is trusted to size it:
 * the identity slot of the window chrome is one line of fixed width, so the
 * helper that validates the label also bounds it. Counting code points (not
 * UTF-16 units) keeps a surrogate pair from being cut in half.
 */
export const CANVAS_SESSION_LABEL_MAX = 120;

/** C0/C1 control characters: never part of a one-line identity. */
const CONTROL_CHARS = /[\u0000-\u001f\u007f-\u009f]/g;

/** Runs of any JavaScript whitespace, including U+2028/U+2029 and NBSP. */
const WHITESPACE_RUN = /\s+/g;

/**
 * One reported drawing label, flattened and bounded — or `null`.
 *
 * The component that prints the label stays dumb: everything that could make a
 * label unusable for a single line happens here. Control characters and line
 * separators become one space (a newline between two words must not glue them
 * together), every whitespace run collapses to that one space, surrounding
 * space is dropped, and the result is cut at [`CANVAS_SESSION_LABEL_MAX`] code
 * points. A value that is not a string, or that is left empty (an empty string,
 * only control characters, only whitespace), is **no label**: the caller shows
 * no identity rather than a blank one.
 */
export function clampCanvasLabel(value: unknown): string | null {
  if (typeof value !== 'string') return null;
  const flat = value.replace(CONTROL_CHARS, ' ').replace(WHITESPACE_RUN, ' ').trim();
  if (flat.length === 0) return null;
  const points = Array.from(flat);
  return points.length <= CANVAS_SESSION_LABEL_MAX
    ? flat
    : points.slice(0, CANVAS_SESSION_LABEL_MAX).join('').trimEnd();
}

/**
 * A reported count (`batches`, `draws`) as a non-negative safe integer.
 *
 * These numbers are diagnostics, never a capability: a missing, negative,
 * fractional or non-numeric value becomes `null` instead of being rounded or
 * clamped into a plausible-looking figure.
 */
function canvasCount(value: unknown): number | null {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? value : null;
}

/**
 * One reported session object -> `{ label, batches, draws }`, or `null`.
 *
 * Shape is checked strictly: a non-object (including an array) is not a report.
 * `requireLive` additionally demands the session's own `ok: true`, which is what
 * "this session is drawing right now" means on the `dom-canvas` carrier — a
 * failed session is already reported through the transport's failure path, so it
 * must never also publish a document identity.
 */
function canvasSessionRecord(value: unknown, requireLive: boolean): CanvasSessionReport | null {
  const record = recordOf(value);
  if (record === null) return null;
  if (requireLive && record.ok !== true) return null;
  const label = clampCanvasLabel(record.label);
  if (label === null) return null;
  return { label, batches: canvasCount(record.batches), draws: canvasCount(record.draws) };
}

/**
 * The drawing identity the host relays for a `dom-canvas` session, or `null`.
 *
 * On that carrier the native shell deliberately holds no document state
 * (AGENTS.md iron rule 8), so `status.canvasSession` — the wasm session's own
 * report, relayed by the host — is the only honest source for "which drawing is
 * open". The descriptor is returned **only** when the host reported the
 * `dom-canvas` carrier, the field is an object, and it carries a usable label;
 * every other combination is `null`, and the chrome then keeps the start state it
 * has today. A `native-below` host (and every host older than the field) can
 * never produce a descriptor, so its rendering is unchanged.
 *
 * The label is already flattened and bounded by [`clampCanvasLabel`], so the
 * component that prints it does no string work of its own.
 */
export function canvasSessionFromShell(shell: unknown): CanvasSessionReport | null {
  if (surfaceModeFromShell(shell) !== SURFACE_MODE_DOM_CANVAS) return null;
  return canvasSessionRecord(statusOf(shell)?.canvasSession, false);
}

/**
 * The identity a **live** session publishes from its own status, or `null`.
 *
 * The wasm session answers every `start`/`resize`/`pan`/`zoom` with its status
 * JSON (`ok`, `label`, `batches`, `draws`, …). Only a healthy status that names
 * its drawing is a report: a failure goes through the transport's one failure
 * path instead, and a status with no identity has nothing to publish.
 */
export function canvasSessionReport(status: unknown): CanvasSessionReport | null {
  return canvasSessionRecord(status, true);
}

/** The comparison key of one report: two equal keys are the same report. */
export function canvasSessionKey(report: CanvasSessionReport): string {
  return `${report.label}\u0000${report.batches}\u0000${report.draws}`;
}

/**
 * The wire body of one report for the host's `canvas_session_status` command.
 *
 * The command takes `{ label, batches, draws }` and defaults a **missing** key to
 * its own honest zero; a `null` count would fail its integer deserialization
 * instead. So a count that was not a real count is left out, never sent as `null`
 * and never rounded into a fabricated figure. The label is always present — a
 * report without a usable label is never produced in the first place.
 */
export function canvasSessionPayload(report: CanvasSessionReport): CanvasSessionPayload {
  const body: CanvasSessionPayload = { label: report.label };
  if (report.batches !== null) body.batches = report.batches;
  if (report.draws !== null) body.draws = report.draws;
  return body;
}

/**
 * A deduplicating publisher for the session's own reports.
 *
 * `send(report)` is called for a valid report whose payload differs from the
 * **immediately preceding** one, so a navigation step that leaves the scene
 * unchanged (same label, same batch/draw counts) reaches the host once, not per
 * gesture. Nothing is sent for an invalid report, and `report` returns whether a
 * send happened — a fact tests and callers can assert instead of inferring it
 * from call counts.
 */
export function createCanvasSessionReporter(send: (report: CanvasSessionReport) => void): CanvasSessionReporter {
  if (typeof send !== 'function') throw new TypeError('createCanvasSessionReporter: send is required');
  let last: string | null = null;
  return {
    report(status: unknown): boolean {
      const next = canvasSessionReport(status);
      if (next === null) return false;
      const key = canvasSessionKey(next);
      if (key === last) return false;
      last = key;
      send(next);
      return true;
    },
  };
}
