// A single ordered IPC lane. Continuous events may merge only at the tail,
// never across a discrete gesture boundary. Capacity reserves terminal events.
import { isReportableRect, metricsKey, pointerPosition, rectFromElement } from './metrics.ts';

// Upper bound (ms) on how long a coalesced `pointermove` may wait for the
// animation frame. `requestAnimationFrame` is throttled or suspended in an
// occluded/unfocused webview — exactly the last evidence runs — so relying on
// the frame alone can buffer a move forever (until a press/release/leave forces
// a flush). A short timer guarantees delivery within this bound while the rAF
// path stays the preferred flush.
const MOVE_FLUSH_FALLBACK_MS = 8;

/**
 * One queued input message.
 *
 * `kind` is the wire discriminator; every other field is that kind's payload
 * (`x`/`y`, `button`, `shift`, wheel `dx`/`dy`, resize metrics, …) and is
 * forwarded verbatim. `sequence` is assigned by the lane, never by a caller.
 */
export interface QueueInput {
  kind: string;
  sequence?: number;
  dx?: number;
  dy?: number;
  [field: string]: unknown;
}

/** The completed-send callback (`cad_command`/`probe_input` style IPC). */
export type SendInput = (input: QueueInput) => unknown;

/** Error sink for a failed send; receives the failure as text. */
export type ReportError = (reason: string) => void;

/** The ordered lane the pointer binding pushes into. */
export interface InputLane {
  /** Whether a new gesture may start (capacity reserved for terminal events). */
  canStart(): boolean;
  /** Enqueue one message; `false` when it was dropped. */
  push(input: QueueInput): boolean;
  /** Messages still queued or in flight. */
  pending(): number;
  /** Drop everything; later pushes are refused and in-flight failures go unreported. */
  dispose(): void;
}

export function createInputLane(send: SendInput, onError: ReportError, capacity = 64): InputLane {
  const queue: QueueInput[] = [];
  let busy = false;
  let sequence = 0;
  let disposed = false;
  const continuous = new Set(['pointer_move', 'wheel']);
  const mergeable = new Set([...continuous, 'resize']);
  async function drain(): Promise<void> {
    if (busy || disposed) return;
    busy = true;
    while (queue.length && !disposed) {
      const input = queue.shift();
      if (input === undefined) break;
      try {
        await send(input);
      } catch (error) {
        // A **disposed** lane never reports. `dispose()` is what a carrier
        // switch (or an unmount) does, and a send that was already in flight
        // rejects *after* it — e.g. a `resize` on a `dom-canvas` host, where
        // `probe_input` is not a command the runtime serves. Surfacing that as
        // a UI error banner reports a refusal nobody asked for any more. A live
        // lane keeps exactly today's behaviour.
        if (!disposed) onError(String(error));
      }
    }
    busy = false;
  }
  return {
    canStart: () => !disposed && queue.length < capacity - 16,
    push(input: QueueInput): boolean {
      if (disposed) return false;
      const tail = queue.at(-1);
      if (mergeable.has(input.kind) && tail?.kind === input.kind) {
        const next: QueueInput = { ...input, sequence: ++sequence };
        if (input.kind === 'wheel') {
          next.dx = (tail.dx ?? 0) + (input.dx ?? 0);
          next.dy = (tail.dy ?? 0) + (input.dy ?? 0);
        }
        queue[queue.length - 1] = next;
        return true;
      }
      const limit = continuous.has(input.kind) ? capacity - 8 : capacity;
      if (queue.length >= limit) {
        if (!continuous.has(input.kind)) onError('IPC_QUEUE_CAPACITY');
        return false;
      }
      queue.push({ ...input, sequence: ++sequence });
      void drain();
      return true;
    },
    pending: () => queue.length + Number(busy),
    dispose(): void { disposed = true; queue.length = 0; },
  };
}

/** The two suspension reasons the CAD pointer lane tracks. */
export type SuspendReason = 'textFocus' | 'popupOpen';

/** The two-reason suspension gate (see [`createSuspendGate`]). */
export interface SuspendGate {
  /** Set one reason; `false` when the argument was not a known reason or a real change. */
  set(reason: string, value: unknown): boolean;
  /** Re-assert the current state onto a freshly bound lane. */
  sync(): boolean;
  /** Whether the lane is suspended right now. */
  isSuspended(): boolean;
  /** A snapshot of both reasons. */
  reasons(): { readonly textFocus: boolean; readonly popupOpen: boolean };
}

/**
 * Two-reason suspension gate for the CAD pointer lane (N5).
 *
 * `textFocus` — the command line owns the keyboard;
 * `popupOpen`  — the host has a modal dialog open.
 *
 * The lane stays suspended while **either** reason holds, and each owner may
 * clear only its own. Before this gate the command line's unconditional
 * `onBlur` (any outside press blurs it) resumed the lane underneath an open
 * popup, so forwarded-but-refused events surfaced as error banners on the main
 * window while the user was looking at the dialog.
 *
 * `apply(suspended)` runs on every real state change, and [`sync`] re-asserts
 * the current state onto a freshly bound lane (a remount must not resume while
 * a reason still holds). `set` reports whether the reason actually changed, so
 * a status poll with the same popup state applies nothing.
 */
export function createSuspendGate(apply: (suspended: boolean) => void = () => {}): SuspendGate {
  const reasons: Record<SuspendReason, boolean> = { textFocus: false, popupOpen: false };
  const suspended = (): boolean => reasons.textFocus || reasons.popupOpen;
  const known = (reason: string): reason is SuspendReason => Object.prototype.hasOwnProperty.call(reasons, reason);
  return {
    set(reason: string, value: unknown): boolean {
      if (!known(reason)) return false;
      // Only a real boolean participates: a truthy/falsy non-boolean is a
      // caller bug, and "not true" must never be read as "release the lane".
      if (typeof value !== 'boolean') return false;
      if (reasons[reason] === value) return false;
      reasons[reason] = value;
      apply(suspended());
      return true;
    },
    /** Re-assert the current state onto a freshly bound lane. */
    sync(): boolean {
      const value = suspended();
      apply(value);
      return value;
    },
    isSuspended: suspended,
    reasons: () => ({ ...reasons }),
  };
}

/** The Shift modifier payload of a press/release. */
export interface ShiftField {
  readonly shift: boolean;
}

/**
 * The Shift modifier field of a pointer press/release.
 *
 * Only `shift` travels as a modifier this round (ADR 0014 B.4: the first UI
 * version sends `Replace` and Shift sends `Add`), and it matters only on the
 * two phases that decide the selection mode, so the existing button
 * normalization is left untouched. The browser value at the event is forwarded
 * verbatim; a missing/undefined `shiftKey` is **not held** (`false`), never a
 * guessed `true`.
 */
export function shiftField(event: { readonly shiftKey?: boolean } | null | undefined): ShiftField {
  return { shift: event?.shiftKey === true };
}

/**
 * The window surface [`bindProbeInput`] needs. Structural, so the real `window`
 * and the fake environments the tests pass both fit.
 */
export interface ProbeEnv {
  readonly devicePixelRatio: number;
  readonly innerHeight: number;
  readonly ResizeObserver: typeof ResizeObserver;
  requestAnimationFrame(callback: FrameRequestCallback): number;
  cancelAnimationFrame(handle: number): void;
  setTimeout(handler: () => void, timeout?: number): number;
  clearTimeout(handle: number): void;
  matchMedia(query: string): MediaQueryList;
  addEventListener(type: string, listener: EventListener): void;
  removeEventListener(type: string, listener: EventListener): void;
}

/** The live binding, as `App.jsx` holds it. */
export interface ProbeInputBinding {
  /** Stop (or resume) forwarding; suspending cancels any gesture in flight. */
  suspend(value: boolean): void;
  /** Detach every listener and observer. */
  dispose(): void;
}

export function bindProbeInput(element: HTMLElement, lane: InputLane, env: ProbeEnv = window): ProbeInputBinding {
  let active: number | null = null;
  let move: QueueInput | null = null;
  let frame: number | null = null;
  let timer: number | null = null;
  let resizeFrame: number | null = null;
  let accepting = true;
  // `button` is the DOM `MouseEvent.button` (0 primary / 1 middle / 2 secondary).
  // It is only meaningful on a press/release, so it rides `pointer_down` and
  // `pointer_up`; a `pointermove` (and a wheel) reports no button change as `-1`,
  // and the wire refuses anything outside `0..=2` — including that `-1` — instead
  // of guessing. Those kinds therefore omit the field entirely (never `null`,
  // `0` or `-1`), leaving the host to treat the omission as the pressed/primary
  // button. Normalizing this DOM quirk here matches the wheel-delta
  // normalization below.
  // Both the pointer handlers and the wheel handler carry a client position, so
  // the shared type is their common base (`PointerEvent`/`WheelEvent` are both
  // `MouseEvent`s).
  const position = (event: MouseEvent): { x: number; y: number } => pointerPosition(element, event);
  const buttonField = (event: PointerEvent): { button?: number } => (event.button >= 0 ? { button: event.button } : {});
  const flush = (): void => {
    if (frame !== null) env.cancelAnimationFrame(frame);
    frame = null;
    if (timer !== null) env.clearTimeout(timer);
    timer = null;
    if (move) lane.push(move);
    move = null;
  };
  const release = (): void => {
    const id = active;
    active = null;
    if (id !== null && element.hasPointerCapture(id)) element.releasePointerCapture(id);
  };
  const cancel = (): void => {
    flush();
    if (active !== null) { lane.push({ kind: 'pointer_cancel' }); release(); }
  };
  const down = (event: Event): void => {
    const pointer = event as PointerEvent;
    if (!accepting || active !== null || !lane.canStart()) return;
    resize();
    flush();
    active = pointer.pointerId;
    element.setPointerCapture(active);
    lane.push({ kind: 'pointer_down', ...position(pointer), ...buttonField(pointer), ...shiftField(pointer) });
  };
  const moving = (event: Event): void => {
    const pointer = event as PointerEvent;
    if (!accepting || (active !== null && pointer.pointerId !== active)) return;
    resize();
    move = { kind: 'pointer_move', ...position(pointer) };
    // Preferred flush: one coalescing frame for the whole move burst. The timer
    // is scheduled exactly once per dirty buffer (both handles are cleared by
    // `flush`), so at most one pending flush exists; whichever fires first wins
    // and cancels the other, giving exactly-once, latest-wins delivery even
    // when `requestAnimationFrame` never ticks.
    if (frame === null) frame = env.requestAnimationFrame(() => { frame = null; flush(); });
    if (timer === null) timer = env.setTimeout(flush, MOVE_FLUSH_FALLBACK_MS);
  };
  const up = (event: Event): void => {
    const pointer = event as PointerEvent;
    if (pointer.pointerId !== active) return;
    resize();
    flush();
    lane.push({ kind: 'pointer_up', ...position(pointer), ...buttonField(pointer), ...shiftField(pointer) });
    release();
  };
  const cancelled = (event: Event): void => { if ((event as PointerEvent).pointerId === active) cancel(); };
  // Leaving the surface clears the host's hover highlight. It carries no
  // position, button or metrics: the host only needs to know the pointer is
  // gone. A press owns the gesture (and a captured pointer should not report a
  // leave mid-capture at all), so an active press is never disturbed here.
  // Any coalesced move is flushed first, exactly like `down`/`up`/`wheel`, so an
  // in-flight move can never arrive *after* the leave and re-tint hover from a
  // stale in-surface position.
  const leave = (): void => {
    if (active !== null) return;
    flush();
    lane.push({ kind: 'pointer_leave' });
  };
  const wheel = (event: Event): void => {
    const wheelEvent = event as WheelEvent;
    wheelEvent.preventDefault();
    if (!accepting) return;
    resize();
    flush();
    // Wheel deltas are normalized to CSS pixels, including line/page modes.
    const factor = wheelEvent.deltaMode === 1 ? 16 : wheelEvent.deltaMode === 2 ? env.innerHeight : 1;
    lane.push({
      kind: 'wheel',
      ...position(wheelEvent),
      dx: wheelEvent.deltaX * factor,
      dy: wheelEvent.deltaY * factor,
    });
  };
  const blur = (): void => { cancel(); lane.push({ kind: 'blur' }); };
  let lastMetrics = '';
  const resize = (): void => {
    const data = { kind: 'resize', ...rectFromElement(element, env.devicePixelRatio || 1) };
    if (!isReportableRect(data)) return;
    const key = metricsKey(data);
    if (key !== lastMetrics) {
      flush();
      if (lane.push(data) !== false) lastMetrics = key;
    }
  };
  const scheduleResize = (): void => {
    if (resizeFrame === null) resizeFrame = env.requestAnimationFrame(() => {
      resizeFrame = null;
      resize();
    });
  };
  let media: MediaQueryList | null = null;
  const dprChanged = (): void => { scheduleResize(); watchDpr(); };
  function watchDpr(): void {
    media?.removeEventListener('change', dprChanged);
    const query = env.matchMedia(`(resolution: ${env.devicePixelRatio || 1}dppx)`);
    media = query;
    query.addEventListener('change', dprChanged);
  }
  const observer = new env.ResizeObserver(scheduleResize);
  observer.observe(element);
  // Chrome regions can shift the CAD origin or shrink the partition without a
  // pointer event (window resize, ribbon expand/collapse, palette dock, command
  // history expand, DPR change). Observing every region keeps the host's surface
  // metrics in sync, which is what anchors wheel zoom at the cursor.
  const layoutRoot = element.closest?.('.shell') ?? element.parentElement;
  for (const region of layoutRoot?.querySelectorAll('.app-region') ?? []) observer.observe(region);
  // Every handler is registered for one DOM event type; the concrete event class
  // (`PointerEvent`, `WheelEvent`) is the one that slot delivers, and the cast is
  // the only place that mapping is stated.
  const handlers: Record<string, (event: Event) => void> = {
    pointerdown: down,
    pointermove: moving,
    pointerup: up,
    pointercancel: cancelled,
    lostpointercapture: cancelled,
    pointerleave: leave,
    wheel,
  };
  for (const [type, handler] of Object.entries(handlers)) element.addEventListener(type, handler, { passive: false });
  env.addEventListener('blur', blur);
  env.addEventListener('resize', scheduleResize);
  watchDpr();
  resize();
  return {
    suspend(value: boolean): void { accepting = !value; if (value) cancel(); },
    dispose(): void {
      cancel();
      if (resizeFrame !== null) env.cancelAnimationFrame(resizeFrame);
      observer.disconnect();
      media?.removeEventListener('change', dprChanged);
      env.removeEventListener('blur', blur);
      env.removeEventListener('resize', scheduleResize);
      for (const [type, handler] of Object.entries(handlers)) element.removeEventListener(type, handler);
    },
  };
}
