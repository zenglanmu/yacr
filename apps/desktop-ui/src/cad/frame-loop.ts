// The `dom-canvas` frame loop: the **host** drives the session's frames.
//
// Nothing inside the wasm session starts a timer (`apps/app-web/src/page.rs`):
// `frame()` is one explicit step - tick, publish a prepared scene revision,
// render, blit - meant to be called from the page's `requestAnimationFrame` loop.
// That is the whole loop, and it lives here.
//
// # Why it is a pump and not a permanently running loop
//
// The renderer is **dirty-driven**: `render` reports `presented: false` when
// nothing changed since the last frame, so an unconditional 60 Hz loop would burn
// a frame budget (and, on a software rasteriser, most of a core) to present
// identical pixels. Instead the loop runs while it makes progress and stops when
// the session has caught up:
//
//   * [`FrameLoop.pump`] starts (or continues) a chain of animation frames. It is
//     called when something *happened* - a forwarded input, an admitted command, a
//     changed status - which is exactly when the pixels may differ;
//   * each step calls `frame()` and keeps the chain alive while the session
//     presented a frame. [`FRAME_IDLE_STEPS`] consecutive steps that present
//     nothing end the chain: the scene is committed and the camera did not move;
//   * [`FRAME_MAX_STEPS`] bounds one pump, so a session that reports a presented
//     frame forever can never turn this into an unbounded spin. The next `pump`
//     resumes; nothing is latched.
//
// A step that throws (the session died, or is not live yet) ends the chain and is
// remembered in `lastError`; it is **not** reported through the transport's
// failure path, because a failed frame step is already the session's own
// `cad_status.error` - the chrome must not hear the same failure twice.
import { parseCanvasStatus } from './canvas-host.ts';

/** The number of consecutive no-present steps that end a pump. */
export const FRAME_IDLE_STEPS = 2;

/** The hard bound on one pump (a spin guard, not a rendering budget). */
export const FRAME_MAX_STEPS = 600;

/** The window surface the loop needs (structural, like `ProbeEnv`). */
export interface FrameLoopEnv {
  requestAnimationFrame(callback: FrameRequestCallback): number;
  cancelAnimationFrame(handle: number): void;
}

/** The session export the loop calls (one synchronous step). */
export interface FrameStepModule {
  frame(): unknown;
}

/** What a caller may inject. */
export interface FrameLoopOptions {
  /** The loaded session module (its `frame` export is the whole step). */
  readonly module: FrameStepModule;
  /** The window the frames are scheduled on; defaults to the global one. */
  readonly env?: FrameLoopEnv;
  /** Called with the parsed frame line of every step (diagnostics and tests). */
  readonly onFrame?: (line: Record<string, unknown> | null) => void;
  /** Consecutive no-present steps that end a pump; defaults to [`FRAME_IDLE_STEPS`]. */
  readonly idleSteps?: number;
  /** Hard bound on one pump; defaults to [`FRAME_MAX_STEPS`]. */
  readonly maxSteps?: number;
}

/** The loop `App.tsx` drives. */
export interface FrameLoop {
  /**
   * Ask for frames until the session has caught up.
   *
   * Returns whether a frame is scheduled afterwards. Calling it while a chain is
   * already running simply continues that chain - the idle counter is reset, so a
   * kick always gets at least one fresh frame.
   */
  pump(): boolean;
  /** Whether a frame is scheduled right now. */
  readonly running: boolean;
  /** The last step's failure text, or `null` (diagnostics only). */
  readonly lastError: string | null;
  /** Resolves once the chain has stopped (immediately when it already has). */
  whenIdle(): Promise<void>;
  dispose(): void;
}

/**
 * A frame pump over one session module.
 *
 * `frame()` is the session's **synchronous** export: it returns one JSON frame
 * line (`{ok, presented, frames, waited, ticks, canvasFrames}`) or throws. A
 * thenable is not awaited here on purpose - awaiting inside a frame callback would
 * let two steps overlap, and the session allows exactly one presenter.
 */
export function createFrameLoop({
  module,
  env = globalThis,
  onFrame = () => {},
  idleSteps = FRAME_IDLE_STEPS,
  maxSteps = FRAME_MAX_STEPS,
}: FrameLoopOptions): FrameLoop {
  if (!module || typeof module.frame !== 'function') {
    throw new TypeError('createFrameLoop: module.frame is required');
  }
  if (!Number.isSafeInteger(idleSteps) || idleSteps < 1) {
    throw new TypeError('createFrameLoop: idleSteps must be a positive integer');
  }
  if (!Number.isSafeInteger(maxSteps) || maxSteps < idleSteps) {
    throw new TypeError('createFrameLoop: maxSteps must be an integer >= idleSteps');
  }
  let handle: number | null = null;
  let disposed = false;
  let idle = 0;
  let steps = 0;
  let lastError: string | null = null;
  let wake: (() => void) | null = null;

  const settled = (): void => {
    const resolve = wake;
    wake = null;
    resolve?.();
  };

  const stop = (): void => {
    handle = null;
    settled();
  };

  /** One step. Returns whether another frame is wanted. */
  const step = (): boolean => {
    let raw: unknown;
    try {
      raw = module.frame();
    } catch (error) {
      lastError = String(error);
      return false;
    }
    const line = parseCanvasStatus(raw);
    onFrame(line);
    steps += 1;
    if (line !== null && line.presented === true) {
      idle = 0;
      return steps < maxSteps;
    }
    idle += 1;
    return idle < idleSteps && steps < maxSteps;
  };

  const schedule = (): void => {
    handle = env.requestAnimationFrame(() => {
      if (disposed) {
        stop();
        return;
      }
      if (step()) {
        schedule();
        return;
      }
      stop();
    });
  };

  return {
    pump(): boolean {
      if (disposed) return false;
      // A kick always buys a fresh frame: the scene may be dirty even though the
      // previous chain already counted its idle steps. The **step budget** is only
      // reset by a chain that starts from scratch: a session that reports a
      // presented frame forever would otherwise have its spin guard cleared by the
      // next kick (the poll notices every frame), and the loop would never stop.
      idle = 0;
      lastError = null;
      if (handle !== null) return true;
      steps = 0;
      schedule();
      return handle !== null;
    },
    get running(): boolean {
      return handle !== null;
    },
    get lastError(): string | null {
      return lastError;
    },
    whenIdle(): Promise<void> {
      if (handle === null) return Promise.resolve();
      return new Promise<void>(resolve => { wake = resolve; });
    },
    dispose(): void {
      disposed = true;
      if (handle !== null) env.cancelAnimationFrame(handle);
      handle = null;
      settled();
    },
  };
}
