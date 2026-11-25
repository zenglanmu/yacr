// Typed camera placement (`cad_set_view`) and the fitted BIM view presets.
//
// The host owns the camera. A preset is resolved there against the **active
// space bounds** (model or sheet), because the chrome never receives those
// bounds: `overview` and the four elevations become framed 3D perspective
// observations, `plan` becomes the fitted 2D top view. An absolute camera is
// also accepted by the wire command, but this module only composes the six
// presets the BIM View panel offers.
//
// Admission is never completion: the desktop queues the placement and applies it
// on the owner tick, `dom-canvas` applies it inline, and a real refusal (an empty
// drawing, a modal popup, a disabled session) arrives later through
// `cad_status.error`. Nothing here claims the camera moved.
import { NO_DOCUMENT_KEY } from '../commands/commands.ts';
import type { Invoke } from '../host/invoke.ts';
import type { Translator } from '../i18n.ts';

/** The one typed IPC name for a camera placement. */
export const SET_VIEW_COMMAND = 'cad_set_view';

/** The six fitted observations the BIM View panel exposes. */
export const SET_VIEW_PRESETS = Object.freeze([
  'overview', 'plan', 'front', 'back', 'left', 'right',
] as const);

export type SetViewPreset = typeof SET_VIEW_PRESETS[number];

/** Whether `value` is a preset this build knows. */
export function isSetViewPreset(value: unknown): value is SetViewPreset {
  return typeof value === 'string' && (SET_VIEW_PRESETS as readonly string[]).includes(value);
}

/** Whether `preset` is a 3D observation (everything but the 2D plan). */
export function setViewNeedsThreeD(preset: unknown): boolean {
  return preset !== 'plan';
}

/** The status slice the placement admission rules read. */
export interface SetViewContext {
  readonly status?: {
    readonly loading?: unknown;
    readonly hostProfile?: { readonly orbit?: unknown } | null;
  } | null;
  readonly documentOpen?: unknown;
}

/**
 * Whether this carrier declared that it cannot rotate the 3D camera.
 *
 * The same `hostProfile.orbit === false` declaration the ViewCube reads: the
 * `dom-canvas` wasm session is 2D-only. A 3D BIM view needs that camera, so the
 * five 3D presets follow the declaration; `plan` stays available because it is a
 * 2D observation. `null`/absent is "not declared" (the native hosts).
 */
function carrierRefusesThreeD(status: SetViewContext['status']): boolean {
  return status?.hostProfile?.orbit === false;
}

/**
 * Whether the chrome may send `preset` at all.
 *
 * A loaded status and an open drawing are required. A 3D preset additionally
 * requires a carrier that did not declare `orbit === false`; the 2D plan preset
 * does not.
 */
export function setViewAvailable(
  { status = null, documentOpen = false }: SetViewContext = {},
  preset?: unknown,
): boolean {
  if (!isSetViewPreset(preset)) return false;
  if (!(Boolean(status) && !status?.loading && Boolean(documentOpen))) return false;
  if (setViewNeedsThreeD(preset) && carrierRefusesThreeD(status)) return false;
  return true;
}

/**
 * Why `preset` cannot run here, in catalog words.
 *
 * The carrier declaration is checked first (a drawing *is* open on `dom-canvas`,
 * so "no drawing is open" would be a lie there), then the document state.
 */
export function setViewUnavailableReasonKey(
  { status = null, documentOpen = false }: SetViewContext = {},
  preset?: unknown,
): string {
  if (!status || status.loading) return 'command.unsupported';
  if (isSetViewPreset(preset) && setViewNeedsThreeD(preset) && carrierRefusesThreeD(status)) {
    return 'view.orbit_unavailable';
  }
  if (!documentOpen) return NO_DOCUMENT_KEY;
  return 'command.unsupported';
}

/** The outcome of one placement request: admission only, never completion. */
export type SetViewResult =
  | { readonly kind: 'accepted'; readonly preset: SetViewPreset }
  | { readonly kind: 'refused'; readonly preset: string; readonly error: string };

/** Whether an ack is the host's `{ accepted: true }` answer. */
function ackAccepted(ack: unknown): boolean {
  if (ack === null || typeof ack !== 'object') return false;
  return (ack as { readonly accepted?: unknown }).accepted === true;
}

/**
 * One `cad_set_view { preset }` request.
 *
 * An unknown local preset is refused before IPC (a programming error), and any
 * host refusal or rejected ack is returned verbatim so the banner can name it.
 */
export async function requestSetView(invoke: Invoke, preset: unknown): Promise<SetViewResult> {
  if (!isSetViewPreset(preset)) {
    return { kind: 'refused', preset: String(preset ?? ''), error: 'CAD_SET_VIEW_UNKNOWN_PRESET' };
  }
  try {
    const ack = await invoke(SET_VIEW_COMMAND, { preset });
    if (!ackAccepted(ack)) throw Error('CAD_SET_VIEW_NOT_ACCEPTED');
    return { kind: 'accepted', preset };
  } catch (error) {
    return { kind: 'refused', preset, error: String(error) };
  }
}

/** What [`submitSetView`] needs from the chrome. */
export interface SetViewSubmission {
  readonly invoke: Invoke;
  readonly preset: unknown;
  readonly t: Translator;
  /** The error banner; a refusal is reported through it. */
  readonly report?: (message: string) => void;
}

/**
 * Run one placement request and report a refusal through `report`.
 *
 * A successful admission reports nothing: the camera is not applied yet, and
 * claiming it here would be the fabricated completion this module avoids.
 */
export async function submitSetView({ invoke, preset, t, report }: SetViewSubmission): Promise<SetViewResult> {
  const result = await requestSetView(invoke, preset);
  if (result.kind === 'refused' && typeof report === 'function') {
    report(t('command.failed', { error: result.error }));
  }
  return result;
}
