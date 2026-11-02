// The status stream is pushed once per rendered frame (≈60 Hz). Most of that
// payload is slow-moving session state, but two fields really do change every
// frame: `frameCount` and `cursorWorld`. Feeding the whole snapshot into React
// state on every frame re-renders the entire shell 60×/s on the same WebKit
// thread that dispatches `pointermove`, which starves the input lane and makes a
// native cursor/crosshair arrive in irregular bursts.
//
// This module is the split: `frameCount` goes straight to the FPS sampler and
// `cursorWorld` goes to this tiny external store. Only the two leaf components
// that display them re-render; `sameStatusIgnoringVolatile` lets the event hook
// publish a semantic status immediately and skip the per-frame churn.
import { formatWorldCursor } from './coords.ts';
import type { WorldCursorReadout } from './coords.ts';
import type { CadStatus } from './cad-status.ts';

/** The per-frame slice consumed by leaf components, not by the shell tree. */
export interface VolatileStatus {
  /** The formatted status-bar coordinate readout. */
  readonly cursor: WorldCursorReadout;
  /** The rolling frame-rate reading (`null` while unknown/stale). */
  readonly fps: number | null;
}

/** A minimal external store (`useSyncExternalStore` contract). */
export interface VolatileStatusStore {
  getSnapshot(): VolatileStatus;
  /** The cursor slice; stable while only the FPS changes (a selector). */
  getCursor(): WorldCursorReadout;
  /** The FPS slice; stable while only the cursor changes (a selector). */
  getFrameRate(): number | null;
  subscribe(listener: () => void): () => void;
  /** Publish a new cursor; notifies only when the rendered text changes. */
  setCursor(cursorWorld: CadStatus['cursorWorld']): void;
  /** Publish an FPS reading; notifies only when the number changes. */
  setFrameRate(fps: number | null): void;
}

/** The one store the shell and its leaf readers share. */
export function createVolatileStatusStore(): VolatileStatusStore {
  let snapshot: VolatileStatus = {
    cursor: formatWorldCursor(null),
    fps: null,
  };
  const listeners = new Set<() => void>();
  const notify = (): void => {
    for (const listener of listeners) listener();
  };
  return {
    getSnapshot: () => snapshot,
    // Selectors: `useSyncExternalStore` re-reads these and bails when the
    // selected value is unchanged, so a cursor-only frame does not re-render the
    // diagnostics row and an FPS-only frame does not re-render the status bar.
    getCursor: () => snapshot.cursor,
    getFrameRate: () => snapshot.fps,
    subscribe(listener: () => void): () => void {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    setCursor(cursorWorld: CadStatus['cursorWorld']): void {
      const cursor = formatWorldCursor({ cursorWorld });
      // A coordinate that formats to the same text is not a visible change.
      if (cursor.text === snapshot.cursor.text && cursor.hintKey === snapshot.cursor.hintKey) return;
      snapshot = { ...snapshot, cursor };
      notify();
    },
    setFrameRate(fps: number | null): void {
      if (Object.is(fps, snapshot.fps)) return;
      snapshot = { ...snapshot, fps };
      notify();
    },
  };
}

/** The single instance the running shell uses (one shell per window). */
export const volatileStatusStore = createVolatileStatusStore();

/** Value equality for the plain JSON values a `cad_status` payload contains. */
function sameJsonValue(a: unknown, b: unknown): boolean {
  if (Object.is(a, b)) return true;
  if (a === null || b === null || typeof a !== typeof b) return false;
  if (Array.isArray(a) || Array.isArray(b)) {
    if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) return false;
    for (let index = 0; index < a.length; index += 1) {
      if (!sameJsonValue(a[index], b[index])) return false;
    }
    return true;
  }
  if (typeof a === 'object') {
    const left = a as Record<string, unknown>;
    const right = b as Record<string, unknown>;
    const leftKeys = Object.keys(left);
    if (leftKeys.length !== Object.keys(right).length) return false;
    for (const key of leftKeys) {
      if (!(key in right) || !sameJsonValue(left[key], right[key])) return false;
    }
    return true;
  }
  return false;
}

/**
 * Whether two status snapshots are identical apart from the two per-frame
 * volatile fields. `frameCount` and `cursorWorld` are published separately (the
 * FPS sampler and the coordinate readout), so they must not re-render the whole
 * shell. Every other field is semantic: selection, tool, layers, command log,
 * layout/space, errors, capabilities — a change there is applied immediately.
 */
export function sameStatusIgnoringVolatile(
  previous: CadStatus | null,
  next: CadStatus,
): boolean {
  if (previous === null) return false;
  if (previous === next) return true;
  const previousKeys = Object.keys(previous);
  const nextKeys = Object.keys(next);
  if (previousKeys.length !== nextKeys.length) return false;
  for (const key of previousKeys) {
    if (key === 'frameCount' || key === 'cursorWorld') continue;
    if (!(key in next)) return false;
    if (!sameJsonValue((previous as unknown as Record<string, unknown>)[key], (next as unknown as Record<string, unknown>)[key])) {
      return false;
    }
  }
  return true;
}
