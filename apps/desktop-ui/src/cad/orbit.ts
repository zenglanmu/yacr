// Orbit: the free-drag camera gesture, its host admission, and the coalescer that
// turns a burst of `pointermove` events into at most one IPC per animation frame.
//
// `cad_orbit { yaw, pitch }` is the typed channel (radians, finite). The reply
// `{accepted: true}` is **admission only**: the host applies the rotation on its
// own tick, and a real refusal (a 2D view, a modal popup, a full queue) arrives
// later through `cad_status.error`. Nothing in this module may claim a camera
// moved.
//
// # The gesture mirrors the host
//
// `crates/cad-host/src/input/pointer.rs` maps a drag delta to radians with
// `ORBIT_RADIANS_PER_PIXEL = 0.008`, `yaw = -dx * K`, `pitch = +dy * K` (yaw about
// world +Z, pitch about the view's right axis). The same constant and signs live
// here so a drag on the ViewCube and a drag on the native surface rotate the same
// way. Non-finite input yields `(0, 0)` instead of poisoning a camera.
//
// # Carrier capability: `cad_status.hostProfile.orbit`
//
// `false` means the carrier declares it cannot orbit at all (the 2D-only
// `dom-canvas` wasm session); `null`/absent is "undeclared" and keeps the
// desktop rule (allowed once a 3D drawing is open). `true` is an explicit yes.
// The toolbar can therefore disable the gesture for the same reason the six
// faces are disabled, instead of letting the click reach a transport that will
// throw `unsupported-runtime: cad_orbit`.
import { NO_DOCUMENT_KEY } from '../commands/commands.ts';
import type { Translator } from '../i18n.ts';
import type { Invoke } from './view3d.ts';

/**
 * Radians of orbit per CSS pixel of drag — the host's own constant
 * (`cad-host/src/input/pointer.rs`). The gesture must feel identical on both
 * surfaces, so this is a mirror, not a second tuning knob.
 */
export const ORBIT_RADIANS_PER_PIXEL = 0.008;

/** A refused request whose angles were not finite; never sent. */
export const CAD_ORBIT_NOT_FINITE = 'CAD_ORBIT_NOT_FINITE';
/** A refused request the host did not admit (`{accepted:false}` or no object). */
export const CAD_ORBIT_NOT_ACCEPTED = 'CAD_ORBIT_NOT_ACCEPTED';
/** A missing result (the caller dropped the answer) is a named refusal, not a success. */
export const CAD_ORBIT_NO_RESULT = 'CAD_ORBIT_NO_RESULT';

/** The carrier declared it cannot orbit at all (`hostProfile.orbit === false`). */
export const ORBIT_UNAVAILABLE_KEY = 'view.orbit_unavailable';
/**
 * Orbit needs a 3D view; the host is in 2D.
 *
 * This reuses the catalog's existing `view.orbit.needs_3d` sentence (the ribbon's
 * disabled Orbit item shows the same line) rather than adding a second wording of
 * the same state.
 */
export const ORBIT_NEEDS_3D_KEY = 'view.orbit.needs_3d';

/**
 * The shell/status fields the orbit admission rules read.
 *
 * Structural and permissive, like the standard-view context: before the first
 * `cad_status` answer the field may be absent, and an older host may omit the
 * whole `hostProfile` block. `view` is the **live** orientation when the caller
 * has it (the ViewCube reads the per-frame volatile store); `status.view` is only
 * the last full snapshot and may lag a 2D/3D switch.
 */
export interface OrbitContext {
  readonly status?: {
    readonly loading?: unknown;
    readonly view?: { readonly mode?: unknown } | null;
    readonly hostProfile?: { readonly orbit?: unknown } | null;
  } | null;
  readonly documentOpen?: unknown;
  /** The live orientation, overriding `status.view` when present. */
  readonly view?: { readonly mode?: unknown } | null;
}

/** Whether the carrier declared `hostProfile.orbit === false`. */
function carrierRefusesOrbit(status: OrbitContext['status']): boolean {
  // Only a real `false` is a declaration; `null`/absent/`true` all keep the
  // desktop rule. `cad_status` has already rejected any other type.
  return status?.hostProfile?.orbit === false;
}

/** The live (or last-known) view is a 3D one. */
function is3d(view: { readonly mode?: unknown } | null | undefined): boolean {
  return view?.mode === '3d';
}

/**
 * The catalog key for why free orbit cannot start right now.
 *
 * Order: no loaded status answers the generic key (the host has not said
 * anything yet); a declared carrier refusal is a carrier limitation (a drawing
 * *is* open on `dom-canvas`, so "no drawing is open" would be a lie); no drawing
 * is the catalog's existing sentence; a 2D view needs 3D. A ready session still
 * returns `command.unsupported`, mirroring the standard-view helper.
 */
export function orbitUnavailableReasonKey(
  { status = null, documentOpen = false, view }: OrbitContext = {},
): string {
  if (!status || status.loading) return 'command.unsupported';
  if (carrierRefusesOrbit(status)) return ORBIT_UNAVAILABLE_KEY;
  if (!documentOpen) return NO_DOCUMENT_KEY;
  if (!is3d(view ?? status.view)) return ORBIT_NEEDS_3D_KEY;
  return 'command.unsupported';
}

/**
 * Whether the chrome may start a drag-orbit at all.
 *
 * Exactly the host support check: a loaded status, an open drawing, a 3D view and
 * no carrier refusal. A pending command is not a reason to block — the gesture
 * has its own coalescer.
 */
export function orbitAvailable({ status = null, documentOpen = false, view }: OrbitContext = {}): boolean {
  if (!(Boolean(status) && !status?.loading && Boolean(documentOpen))) return false;
  if (carrierRefusesOrbit(status)) return false;
  return is3d(view ?? status?.view);
}

/**
 * The outcome of one `cad_orbit` request: admission only, never completion.
 */
export type OrbitResult =
  | { readonly kind: 'accepted' }
  | { readonly kind: 'refused'; readonly error: string };

/** Whether an ack is the host's `{ accepted: true }` answer. */
function ackAccepted(ack: unknown): boolean {
  if (ack === null || typeof ack !== 'object') return false;
  return (ack as { readonly accepted?: unknown }).accepted === true;
}

/**
 * One `cad_orbit { yaw, pitch }` request.
 *
 * Non-finite angles are refused **before** IPC with [`CAD_ORBIT_NOT_FINITE`]: they
 * are a programming error (the coalescer drops them), and sending them could
 * poison the host camera. A rejected invoke keeps the host's own reason verbatim
 * so the banner names it.
 */
export async function requestOrbit(invoke: Invoke, yaw: number, pitch: number): Promise<OrbitResult> {
  if (!Number.isFinite(yaw) || !Number.isFinite(pitch)) {
    return { kind: 'refused', error: CAD_ORBIT_NOT_FINITE };
  }
  try {
    const ack = await invoke('cad_orbit', { yaw, pitch });
    if (!ackAccepted(ack)) throw Error(CAD_ORBIT_NOT_ACCEPTED);
    return { kind: 'accepted' };
  } catch (error) {
    return { kind: 'refused', error: String(error) };
  }
}

/**
 * The banner line for a request that was not admitted, or `''` when it was.
 *
 * A refusal keeps the host's own reason verbatim and goes through the shared
 * `command.failed` line, exactly like a refused standard view.
 */
export function orbitFailure(result: OrbitResult | null | undefined, t: Translator): string {
  if (result && result.kind === 'accepted') return '';
  return t('command.failed', { error: result?.error ?? CAD_ORBIT_NO_RESULT });
}

/** What [`submitOrbit`] needs from the chrome. */
export interface OrbitSubmission {
  readonly invoke: Invoke;
  readonly yaw: number;
  readonly pitch: number;
  readonly t: Translator;
  /** The error banner; a refusal is reported through it. */
  readonly report?: (message: string) => void;
}

/**
 * Run one request and report its failure through `report` (the error banner).
 *
 * An admission reports nothing — the camera has not moved yet. The banner is
 * **not** cleared first, either: a drag emits one request per animation frame and
 * clearing on each would erase an unrelated error the user just triggered. A
 * refusal is reported once (the coalescer then stops the gesture).
 */
export async function submitOrbit({ invoke, yaw, pitch, t, report }: OrbitSubmission): Promise<OrbitResult> {
  const result = await requestOrbit(invoke, yaw, pitch);
  const failure = orbitFailure(result, t);
  if (failure && typeof report === 'function') report(failure);
  return result;
}

/** The radians a drag from `from` to `to` should orbit by (host signs). */
export function orbitDelta(
  from: readonly [number, number],
  to: readonly [number, number],
): { readonly yaw: number; readonly pitch: number } {
  const dx = to[0] - from[0];
  const dy = to[1] - from[1];
  if (!Number.isFinite(dx) || !Number.isFinite(dy)) return { yaw: 0, pitch: 0 };
  // `dx === 0` keeps the result a real `0`, never `-0`: the value is compared
  // (and serialised) downstream, and `-0` is noise from `-dx * K`.
  return {
    yaw: dx === 0 ? 0 : -dx * ORBIT_RADIANS_PER_PIXEL,
    pitch: dy === 0 ? 0 : dy * ORBIT_RADIANS_PER_PIXEL,
  };
}

/** Schedule one coalescing frame (injectable so tests need no real rAF). */
export type OrbitSchedule = (callback: () => void) => number;
/** Cancel a scheduled frame. */
export type OrbitCancel = (handle: number) => void;

/** What [`OrbitDrag`] needs: one send, and a scheduler (real rAF by default). */
export interface OrbitDragOptions {
  readonly send: (yaw: number, pitch: number) => Promise<OrbitResult>;
  readonly schedule?: OrbitSchedule;
  readonly cancel?: OrbitCancel;
}

/** The real scheduler pair, or a macrotask fallback where rAF is absent. */
function framePair(): { schedule: OrbitSchedule; cancel: OrbitCancel } {
  if (typeof requestAnimationFrame === 'function' && typeof cancelAnimationFrame === 'function') {
    return {
      schedule: callback => requestAnimationFrame(callback),
      cancel: handle => cancelAnimationFrame(handle),
    };
  }
  // A WebView without rAF (or a test double) still gets coalescing, just on a
  // timer-based frame instead of a painted one.
  return {
    schedule: callback => setTimeout(callback, 16),
    cancel: handle => clearTimeout(handle),
  };
}

/**
 * Coalesces a drag's deltas into one summed `cad_orbit` per animation frame.
 *
 * Rules, all deliberate:
 *
 * - deltas accumulate; a scheduled frame sends their **sum**, then clears them;
 * - at most one request is in flight — a push during a request waits for its
 *   answer instead of queueing a second request (backpressure);
 * - `flush()` sends now (the frame callback uses it, and `pointerup` can drain
 *   the last movement before `end()`);
 * - a refused answer stops this gesture: nothing more is sent until `end()`, so a
 *   refusal cannot flood the banner once per frame;
 * - zero and non-finite deltas are dropped rather than sent;
 * - `end()` cancels a scheduled frame and never sends again.
 *
 * `send` reports the transport outcome; the class never claims a camera moved.
 */
export class OrbitDrag {
  #send: (yaw: number, pitch: number) => Promise<OrbitResult>;
  #schedule: OrbitSchedule;
  #cancel: OrbitCancel;
  #yaw = 0;
  #pitch = 0;
  #frame: number | null = null;
  #inFlight = false;
  #stopped = false;
  #ended = false;

  constructor({ send, schedule, cancel }: OrbitDragOptions) {
    const pair = framePair();
    this.#send = send;
    this.#schedule = schedule ?? pair.schedule;
    this.#cancel = cancel ?? pair.cancel;
  }

  /** The summed radians waiting for the next frame. */
  get pending(): readonly [number, number] {
    return [this.#yaw, this.#pitch];
  }

  /** Whether a refusal stopped this gesture. */
  get stopped(): boolean {
    return this.#stopped;
  }

  /** Accumulate one pointer delta (radians); ignored once stopped/ended. */
  push(yaw: number, pitch: number): void {
    if (this.#ended || this.#stopped) return;
    if (!Number.isFinite(yaw) || !Number.isFinite(pitch)) return;
    if (yaw === 0 && pitch === 0) return;
    this.#yaw += yaw;
    this.#pitch += pitch;
    this.#scheduleFrame();
  }

  /** Send the accumulated sum now, if nothing is already in flight. */
  flush(): void {
    if (this.#frame !== null) {
      // A manual flush supersedes the scheduled frame (cancelling an id that has
      // already fired is a no-op).
      this.#cancel(this.#frame);
      this.#frame = null;
    }
    if (this.#ended || this.#stopped) {
      this.#yaw = 0;
      this.#pitch = 0;
      return;
    }
    if (this.#inFlight) return;
    const yaw = this.#yaw;
    const pitch = this.#pitch;
    this.#yaw = 0;
    this.#pitch = 0;
    if (yaw === 0 && pitch === 0) return;
    this.#inFlight = true;
    const settle = (result: OrbitResult | null): void => {
      this.#inFlight = false;
      if (this.#ended) return;
      if (!result || result.kind !== 'accepted') {
        this.#stopped = true;
        this.#yaw = 0;
        this.#pitch = 0;
        return;
      }
      this.#scheduleFrame();
    };
    let pending: Promise<OrbitResult>;
    try {
      pending = this.#send(yaw, pitch);
    } catch {
      settle(null);
      return;
    }
    void pending.then(result => settle(result), () => settle(null));
  }

  /** End the gesture: cancel the frame and never send again. */
  end(): void {
    this.#ended = true;
    if (this.#frame !== null) {
      this.#cancel(this.#frame);
      this.#frame = null;
    }
    this.#yaw = 0;
    this.#pitch = 0;
  }

  /** Schedule one frame when there is work and nothing else pending. */
  #scheduleFrame(): void {
    if (this.#ended || this.#stopped || this.#inFlight) return;
    if (this.#frame !== null) return;
    if (this.#yaw === 0 && this.#pitch === 0) return;
    this.#frame = this.#schedule(() => { this.flush(); });
  }
}
