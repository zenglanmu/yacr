// The typed push event source that replaced the `cad_status` poll (P3).
//
// The host pushes **one JSON object per event** on `cad_event` (Tauri) or through
// `cad_subscribe(cb)` (wasm). `revision` increments by one per event, so a gap is
// evidence that something was lost: the source asks for one fresh snapshot and
// keeps going. The only timer in this module is the **recovery** backoff while a
// transport is broken - there is no steady-state tick, and closing the
// subscription cancels everything.
//
// A transport that cannot be parsed is a transport failure, never a silent skip:
// a malformed `status` would otherwise leave the chrome rendering stale numbers
// forever. The one deliberate exception is an event type this build does not
// model yet: it is ignored (and its revision still advances the cursor), so a
// newer host can add a frame without breaking an older chrome.
//
// Both transports are normalised here; the chrome never sees a Tauri event or a
// wasm export. `host/invoke.ts` is the only module that imports
// `@tauri-apps/api`, and it hands this module a plain `listenJson` callback.

import { parseCadStatus } from './cad-status.ts';
import type { CadPopup, CadStatus, Invoke } from './cad-status.ts';
import { withTransportCapabilities } from '../host/transport-capabilities.ts';

/** The first retry delay of a broken transport (`500 ms * 2^n`). */
export const CAD_EVENT_BACKOFF_BASE_MS = 500;

/** The retry-delay ceiling; a broken transport never backs off beyond this. */
export const CAD_EVENT_BACKOFF_CAP_MS = 4000;

/** The Tauri event name carrying one frame (`host/invoke.ts` listens on it). */
export const CAD_EVENT_NAME = 'cad_event';

/**
 * One normalised host event.
 *
 * `revision` is the host's own cursor, not a local counter: it is what the gap
 * check compares against.
 */
export type CadEventFrame =
  | { readonly revision: number; readonly type: 'status-changed'; readonly status: CadStatus }
  | { readonly revision: number; readonly type: 'command-admitted'; readonly command: string }
  | { readonly revision: number; readonly type: 'command-rejected'; readonly command: string; readonly code: string }
  | { readonly revision: number; readonly type: 'selection-changed'; readonly count: number; readonly empty: boolean }
  | { readonly revision: number; readonly type: 'tool-step'; readonly tool: string; readonly step: number }
  | { readonly revision: number; readonly type: 'document-opened'; readonly label: string }
  | { readonly revision: number; readonly type: 'document-closed' }
  | { readonly revision: number; readonly type: 'error-raised'; readonly error: string }
  | { readonly revision: number; readonly type: 'popup-changed'; readonly popup: CadPopup | null }
  | { readonly revision: number; readonly type: 'vendor-pending'; readonly token: number; readonly generation: number; readonly entities: number }
  | { readonly revision: number; readonly type: 'vendor-settled'; readonly token: number; readonly applied: number; readonly diagnostics: readonly string[] }
  | { readonly revision: number; readonly type: 'events-dropped'; readonly count: number };

/**
 * One live subscription.
 *
 * `revision` is the last host revision the source accepted (also advanced by an
 * unmodelled event type). `requestStatus()` resolves the host's current snapshot
 * **once** - the gap recovery path - and `close()` cancels the transport and any
 * pending retry timer.
 */
export interface CadSubscription {
  readonly revision: number;
  requestStatus(): Promise<void>;
  close(): void;
}

/** A transport-neutral event source (Tauri or wasm, chosen at the invoke seam). */
export interface CadEventSource {
  /**
   * Start listening and deliver the initial snapshot through `onEvent`.
   *
   * The returned subscription is live immediately; the first snapshot arrives
   * asynchronously. `onError` receives a transport failure (the source then
   * resubscribes on its own with backoff until `close()`).
   */
  subscribe(onEvent: (frame: CadEventFrame) => void, onError: (error: string) => void): CadSubscription;
}

/** The clock a recovery backoff schedules on (global, or a test double). */
export interface CadEventClock {
  setTimeout(callback: () => void, delay: number): ReturnType<typeof setTimeout>;
  clearTimeout(handle?: ReturnType<typeof setTimeout>): void;
}

/** The Tauri listener seam: one JSON string per event; resolves to its unlisten. */
export type CadEventListener = (handler: (json: string) => void) => Promise<() => void>;

/** The one module export pair the wasm source calls (all optional for older artifacts). */
export interface WasmEventModule {
  cad_subscribe?(callback: (json: string) => void): unknown;
  cad_unsubscribe?(): unknown;
  shell_status?(): string;
}

/** The current snapshot reader both transports implement. */
type StatusReader = () => Promise<CadStatus>;

/**
 * How one transport presents the host's status.
 *
 * A native host answers for itself and overlays nothing; the page carrier adds
 * the capability only it serves (`host/transport-capabilities.ts`). Applied to
 * the initial frame, every live frame and the recovery read alike, so a pushed
 * frame can never undo the overlay.
 */
type StatusOverlay = (status: CadStatus) => CadStatus;

/** The native (or any honest) transport: the status is the host's own answer. */
const NO_STATUS_OVERLAY: StatusOverlay = status => status;

/** A non-negative safe integer, or `false` (revision/count validation). */
function isRevision(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
}

/** A non-empty string field, or a loud invalid-event failure. */
function requiredText(record: Record<string, unknown>, field: string): string {
  const value = record[field];
  if (typeof value !== 'string' || value.length === 0) throw new Error('CAD_EVENT_INVALID');
  return value;
}

/** A plain JSON object, or `null` (arrays are not records). */
function plainRecord(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

/** One parsed envelope: the revision is known even for an unmodelled type. */
export interface ParsedCadEvent {
  readonly revision: number;
  /** `null` for a type this build does not model (ignored, revision still advances). */
  readonly frame: CadEventFrame | null;
}

/**
 * Parse one event object.
 *
 * Throws `CAD_EVENT_INVALID` (or `CAD_STATUS_INVALID`) for a malformed event;
 * returns `frame: null` for an unrecognised `type` so a newer host can add frames
 * without breaking this build.
 */
export function parseCadEvent(value: unknown): ParsedCadEvent {
  const record = plainRecord(value);
  if (record === null) throw new Error('CAD_EVENT_INVALID');
  const revision = record.revision;
  if (!isRevision(revision)) throw new Error('CAD_EVENT_INVALID');
  const type = record.type;
  if (typeof type !== 'string') throw new Error('CAD_EVENT_INVALID');
  switch (type) {
    case 'status-changed':
      return { revision, frame: { revision, type, status: parseCadStatus(record.status) } };
    case 'command-admitted':
      return { revision, frame: { revision, type, command: requiredText(record, 'command') } };
    case 'command-rejected':
      return { revision, frame: { revision, type, command: requiredText(record, 'command'), code: requiredText(record, 'code') } };
    case 'selection-changed':
      if (!isRevision(record.count) || typeof record.empty !== 'boolean') throw new Error('CAD_EVENT_INVALID');
      return { revision, frame: { revision, type, count: record.count, empty: record.empty } };
    case 'tool-step':
      if (!isRevision(record.step)) throw new Error('CAD_EVENT_INVALID');
      return { revision, frame: { revision, type, tool: requiredText(record, 'tool'), step: record.step } };
    case 'document-opened':
      if (typeof record.label !== 'string') throw new Error('CAD_EVENT_INVALID');
      return { revision, frame: { revision, type, label: record.label } };
    case 'document-closed':
      return { revision, frame: { revision, type } };
    case 'error-raised':
      if (typeof record.error !== 'string') throw new Error('CAD_EVENT_INVALID');
      return { revision, frame: { revision, type, error: record.error } };
    case 'popup-changed': {
      const popup = record.popup;
      if (popup !== null && plainRecord(popup) === null) throw new Error('CAD_EVENT_INVALID');
      return { revision, frame: { revision, type, popup: popup as CadPopup | null } };
    }
    case 'vendor-pending':
      if (!isRevision(record.token) || !isRevision(record.generation) || !isRevision(record.entities)) {
        throw new Error('CAD_EVENT_INVALID');
      }
      return {
        revision,
        frame: {
          revision,
          type,
          token: record.token,
          generation: record.generation,
          entities: record.entities,
        },
      };
    case 'vendor-settled': {
      const diagnostics = record.diagnostics;
      if (
        !isRevision(record.token) ||
        !isRevision(record.applied) ||
        !Array.isArray(diagnostics) ||
        diagnostics.length > 16 ||
        diagnostics.some(item => typeof item !== 'string')
      ) {
        throw new Error('CAD_EVENT_INVALID');
      }
      return {
        revision,
        frame: {
          revision,
          type,
          token: record.token,
          applied: record.applied,
          diagnostics: diagnostics as string[],
        },
      };
    }
    case 'events-dropped':
      if (!isRevision(record.count)) throw new Error('CAD_EVENT_INVALID');
      return { revision, frame: { revision, type, count: record.count } };
    default:
      return { revision, frame: null };
  }
}

/** One frame from JSON text or an already-parsed object. */
function readEvent(raw: unknown): ParsedCadEvent {
  if (typeof raw === 'string') {
    let parsed: unknown;
    try {
      parsed = JSON.parse(raw);
    } catch (error) {
      throw new Error(`CAD_EVENT_INVALID: ${String(error)}`, { cause: error });
    }
    return parseCadEvent(parsed);
  }
  return parseCadEvent(raw);
}

/** The frame carried by one event, or `null` for an unmodelled type. */
export function parseCadEventFrame(value: unknown): CadEventFrame | null {
  return readEvent(value).frame;
}

/**
 * Whether this event means the model changed (and a page carrier needs frames).
 *
 * Deliberately **false** for `status-changed`: status arrives on every rendered
 * frame, so pumping from it would keep the frame loop asking forever. Status is
 * the sampler's source, not a pump trigger. `events-dropped` is a transport hint
 * (the gap check already asks for a snapshot), not a model change.
 */
export function cadEventPumpsFrames(type: CadEventFrame['type']): boolean {
  switch (type) {
    case 'status-changed':
    case 'events-dropped':
    // The vendor pass does not change the model yet: geometry insertion is a
    // later milestone, so these frames must not pump the page carrier.
    case 'vendor-pending':
    case 'vendor-settled':
      return false;
    default:
      return true;
  }
}

/**
 * The recovery delay after `failures` consecutive transport failures.
 *
 * The historical polling formula (`500 * 2^n`, capped at 4 s) is kept on purpose:
 * first retry 1 s, then 2 s, then 4 s. This is only scheduled **while broken**;
 * a successful (re)subscribe resets the count and cancels the timer.
 */
export function cadEventBackoff(failures: number): number {
  return Math.min(CAD_EVENT_BACKOFF_CAP_MS, CAD_EVENT_BACKOFF_BASE_MS * 2 ** Math.min(failures, 3));
}

/**
 * The shared session machinery: one active transport, gap recovery, and a
 * backoff loop that only exists while the transport is broken.
 */
function createEventSource(
  open: (report: (raw: unknown, initial: boolean) => void) => Promise<() => void>,
  readStatus: StatusReader,
  clock: CadEventClock,
  overlayStatus: StatusOverlay = NO_STATUS_OVERLAY,
): CadEventSource {
  return {
    subscribe(onEvent, onError) {
      let closed = false;
      let broken = false;
      let closer: (() => void) | null = null;
      let timer: ReturnType<typeof setTimeout> | undefined;
      let failures = 0;
      let lastRevision = 0;
      let resyncing = false;
      let starting = false;

      const clearTimer = (): void => {
        if (timer !== undefined) {
          clock.clearTimeout(timer);
          timer = undefined;
        }
      };

      const teardown = (): void => {
        const active = closer;
        closer = null;
        if (active !== null) {
          try { active(); } catch { /* the transport is already gone */ }
        }
      };

      /** One frame as the chrome sees it (a status frame carries the overlay). */
      const lift = (frame: CadEventFrame): CadEventFrame =>
        frame.type === 'status-changed' ? { ...frame, status: overlayStatus(frame.status) } : frame;

      const refresh = async (): Promise<void> => {
        if (closed || broken) return;
        try {
          const status = overlayStatus(await readStatus());
          if (!closed && !broken) onEvent({ revision: lastRevision, type: 'status-changed', status });
        } catch (error) {
          fail(error);
        }
      };

      const fail = (error: unknown): void => {
        if (closed || broken) return;
        broken = true;
        teardown();
        failures += 1;
        onError(String(error));
        clearTimer();
        timer = clock.setTimeout(() => {
          timer = undefined;
          void start();
        }, cadEventBackoff(failures));
      };

      const report = (raw: unknown, initial: boolean): void => {
        if (closed) return;
        let parsed: ParsedCadEvent;
        try {
          parsed = readEvent(raw);
        } catch (error) {
          // A frame we cannot read is a transport failure, never a silent skip.
          fail(error);
          return;
        }
        if (initial) {
          // A live event may arrive while the initial snapshot is still in
          // flight (the Tauri transport attaches the listener before its one
          // `cad_status` call). Seed from the highest cursor seen: the snapshot
          // is delivered at that cursor and never resets it to a synthetic 0,
          // which would look like a gap and force an extra `cad_status` pull.
          lastRevision = Math.max(lastRevision, parsed.revision);
          if (parsed.frame !== null) onEvent({ ...lift(parsed.frame), revision: lastRevision });
          return;
        }
        if (parsed.revision !== lastRevision + 1 && !resyncing) {
          // One recovery request per gap; a burst of buffered events must not
          // fan out into a burst of `cad_status` calls.
          resyncing = true;
          void refresh().finally(() => { resyncing = false; });
        }
        // An unmodelled type still owns its revision, so the next known event
        // stays contiguous instead of looking like a second gap.
        lastRevision = parsed.revision;
        if (parsed.frame !== null) onEvent(lift(parsed.frame));
      };

      const start = async (): Promise<void> => {
        if (closed || starting) return;
        starting = true;
        broken = false;
        try {
          const active = await open(report);
          // A failure delivered while `open` was still running already scheduled
          // the retry; keeping this closer would silently cancel it.
          if (closed || broken) {
            active();
            return;
          }
          closer = active;
          failures = 0;
          clearTimer();
        } catch (error) {
          fail(error);
        } finally {
          starting = false;
        }
      };

      void start();

      return {
        get revision(): number {
          return lastRevision;
        },
        requestStatus: refresh,
        close(): void {
          closed = true;
          clearTimer();
          teardown();
        },
      };
    },
  };
}

/**
 * The native transport: Tauri `cad_event` pushes plus **one** `cad_status` call
 * for the initial snapshot. There is no timer after that snapshot.
 */
export function createTauriEventSource(
  invoke: Invoke,
  listenJson: CadEventListener,
  deps: { readonly clock?: CadEventClock } = {},
): CadEventSource {
  return createEventSource(async report => {
    let stopped = false;
    const unlisten = await listenJson(raw => {
      if (!stopped) report(raw, false);
    });
    try {
      const status = parseCadStatus(await invoke('cad_status'));
      if (!stopped) report({ revision: 0, type: 'status-changed', status }, true);
    } catch (error) {
      stopped = true;
      try { unlisten(); } catch { /* already unlistened */ }
      throw error;
    }
    return () => {
      stopped = true;
      try { unlisten(); } catch { /* already unlistened */ }
    };
  }, async () => parseCadStatus(await invoke('cad_status')), deps.clock ?? globalThis);
}

/**
 * The wasm transport: `cad_subscribe(cb)` returns the initial `status-changed`
 * frame and forwards every later frame to `cb`; `cad_unsubscribe()` stops it.
 *
 * Every status it produces - the initial frame, each live frame and the
 * `shell_status` recovery read - goes through
 * [`withTransportCapabilities`], because the page owns the DOM file picker the
 * session honestly says it cannot serve (`canOpen: false`). Nothing else is
 * lifted. The native transport above overlays nothing: a native host reports its
 * own real `canOpen`.
 */
export function createWasmEventSource(
  module: WasmEventModule | (() => Promise<WasmEventModule>),
  deps: { readonly clock?: CadEventClock } = {},
): CadEventSource {
  const load: () => Promise<WasmEventModule> = typeof module === 'function'
    ? module
    : async () => module;
  let current: WasmEventModule | null = null;
  return createEventSource(async report => {
    const loaded = await load();
    current = loaded;
    if (typeof loaded.cad_subscribe !== 'function') {
      throw new Error('cad-events: the canvas module has no cad_subscribe export');
    }
    let stopped = false;
    const initial = loaded.cad_subscribe(raw => {
      if (!stopped) report(raw, false);
    });
    if (typeof initial !== 'string') {
      stopped = true;
      try { loaded.cad_unsubscribe?.(); } catch { /* already unsubscribed */ }
      throw new Error('cad-events: cad_subscribe returned no initial status frame');
    }
    report(initial, true);
    return () => {
      stopped = true;
      try { loaded.cad_unsubscribe?.(); } catch { /* already unsubscribed */ }
    };
  }, async () => {
    const loaded = current ?? await load();
    if (typeof loaded.shell_status !== 'function') {
      throw new Error('cad-events: the canvas module has no shell_status export');
    }
    const raw: unknown = loaded.shell_status();
    // The wasm export answers the session's own status JSON string (the adapter
    // parses exactly the same string); an object is accepted for test doubles.
    return parseCadStatus(typeof raw === 'string' ? JSON.parse(raw) : raw);
  }, deps.clock ?? globalThis, withTransportCapabilities);
}
