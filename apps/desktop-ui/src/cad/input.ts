// A single ordered IPC lane. Continuous events may merge only at the tail,
// never across a discrete gesture boundary. Capacity reserves terminal events.
import { isReportableRect, metricsKey, pointerPosition, rectFromElement } from './metrics.ts';
import type { PartitionRect, PointerPositionEvent } from './metrics.ts';
import { GesturePinch, wheelPinchScale } from './dom-pinch.ts';
import type { WebKitGestureEvent } from './dom-pinch.ts';

// Upper bound (ms) on how long a coalesced `pointermove` may wait for the
// animation frame. `requestAnimationFrame` is throttled or suspended in an
// occluded/unfocused webview — exactly the last evidence runs — so relying on
// the frame alone can buffer a move forever (until a press/release/leave forces
// a flush). A short timer guarantees delivery within this bound while the rAF
// path stays the preferred flush.
const MOVE_FLUSH_FALLBACK_MS = 8;
// Upper bound (ms) on how long a resize report may wait for the animation frame.
// The same rAF throttling applies; a resize report that never runs leaves the
// host's native CAD surface at its previous rect, which a transparent
// `native-below` window exposes as the desktop showing through the canvas.
const RESIZE_FLUSH_FALLBACK_MS = 120;
// Delay before the one reconciliation re-report of a changed partition rect.
//
// A native host applies the composited surface geometry only for an **accepted**
// report, so one lost report (a throttled frame, a refused IPC, a rejected rect)
// would leave the surface at the old rect forever — and a transparent window
// then shows the desktop through the uncovered canvas. Every real geometry change
// schedules exactly one delayed re-send of the then-current rect, so the steady
// state costs nothing.
const RESIZE_RECONCILE_DELAY_MS = 1500;
// How long after `gestureend` a `ctrl`+wheel is still read as part of the same
// WebKit pinch. WebKit 15+ (macOS Safari/WKWebView) reports one trackpad pinch
// as **both** the `gesturestart/change/end` sequence and `ctrl`+wheel; the
// gesture is authoritative, and this tail covers a wheel that arrives just
// after `gestureend` (the two signals are not ordered). A wheel in that window
// still cancels the page zoom; it just does not zoom CAD a second time.
const GESTURE_WHEEL_GUARD_MS = 500;

/**
 * A monotonic timestamp for the gesture/wheel dedupe window.
 *
 * `performance.now()` is the browser's clock; the fallbacks keep the binding
 * usable in a fake harness (and any non-browser test environment) without
 * changing behaviour, since only elapsed milliseconds matter.
 */
function eventNow(): number {
  return typeof performance !== 'undefined' ? performance.now() : Date.now();
}

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
  scale?: number;
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
  const continuous = new Set(['pointer_move', 'wheel', 'pinch']);
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
        } else if (input.kind === 'pinch') {
          // Two pinch samples describe one cumulative gesture: positions are
          // latest-wins (like a move), incremental deltas add up, and the
          // incremental zoom multipliers compose by multiplication.
          next.dx = (tail.dx ?? 0) + (input.dx ?? 0);
          next.dy = (tail.dy ?? 0) + (input.dy ?? 0);
          next.scale = (tail.scale ?? 1) * (input.scale ?? 1);
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

/** The pointer device a wire input reports (the DOM `pointerType`). */
export type InputDevice = 'mouse' | 'touch' | 'pen';

/** One pressed contact tracked by [`bindProbeInput`]. */
interface Contact {
  x: number;
  y: number;
  device?: InputDevice;
}

/** The pinch baseline: midpoint and finger distance in CSS px. */
interface PinchState {
  x: number;
  y: number;
  distance: number;
}

/**
 * The `device` field of a wire pointer input.
 *
 * The DOM `pointerType` is read per event. Only the three devices the wire
 * models travel; anything else (an empty string on a synthetic event, an
 * unknown future type) omits the field entirely rather than guessing, and the
 * host then reads the absence as `mouse` — exactly the old wire.
 */
export function deviceField(event: { readonly pointerType?: string } | null | undefined): { device?: InputDevice } {
  const type = event?.pointerType;
  return type === 'mouse' || type === 'touch' || type === 'pen' ? { device: type } : {};
}

/**
 * Whether two contacts form a pinch.
 *
 * Touch always wins (`mouse` + `touch` is a hybrid laptop, not a second mouse
 * press), a pen pair pins, and two mouse contacts stay the old "second press is
 * ignored" case.
 */
function pinchPair(first: InputDevice | undefined, second: InputDevice | undefined): boolean {
  if (first === 'touch' || second === 'touch') return true;
  return first === 'pen' && second === 'pen';
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
  // Every pressed contact, keyed by `pointerId`. A single entry is exactly the
  // old single `active` pointer; a second touch/pen contact turns the map into a
  // pinch. `mouse` contacts never pair with `mouse` (the old "second press is
  // ignored" behaviour).
  const contacts = new Map<number, Contact>();
  let pinching = false;
  // After a pinch ends, its remaining fingers are held until every one lifts:
  // a late move/up must not become a stray pan, box select or tap.
  let ignoring = false;
  let pinchFrom: PinchState | null = null;
  let move: QueueInput | null = null;
  let frame: number | null = null;
  let timer: number | null = null;
  let resizeFrame: number | null = null;
  let resizeFallback: number | null = null;
  let reconcileTimer: number | null = null;
  let accepting = true;
  // WebKit 15+ (macOS) sends a trackpad pinch as **both** the `gesture*`
  // sequence and `ctrl`+wheel. These two track that overlap so one pinch stays
  // one CAD zoom: `lastGestureAt` starts at `-Infinity`, so a ctrl+wheel before
  // any gesture is never mistaken for a pinch echo.
  let gestureActive = false;
  let lastGestureAt = Number.NEGATIVE_INFINITY;
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
  // `MouseEvent`s). The WebKit gesture event carries `clientX/clientY` too.
  const position = (event: PointerPositionEvent): { x: number; y: number } => pointerPosition(element, event);
  // One layout read for a move: the point and the rect that sizes it come from
  // the same `getBoundingClientRect()`. A pointermove must not force layout twice
  // (50–120 events/s), so continuous moves use this instead of `position` plus
  // `resize`.
  const positionAndRect = (event: MouseEvent): { at: { x: number; y: number }; rect: PartitionRect } => {
    const rect = rectFromElement(element, env.devicePixelRatio || 1);
    return { at: { x: event.clientX - rect.x, y: event.clientY - rect.y }, rect };
  };
  const buttonField = (event: PointerEvent): { button?: number } => (event.button >= 0 ? { button: event.button } : {});
  const flush = (): void => {
    if (frame !== null) env.cancelAnimationFrame(frame);
    frame = null;
    if (timer !== null) env.clearTimeout(timer);
    timer = null;
    if (move) lane.push(move);
    move = null;
  };
  const bufferMove = (input: QueueInput): void => {
    move = input;
    // Preferred flush: one coalescing frame for the whole move burst. The timer
    // is scheduled exactly once per dirty buffer (both handles are cleared by
    // `flush`), so at most one pending flush exists; whichever fires first wins
    // and cancels the other, giving exactly-once, latest-wins delivery even
    // when `requestAnimationFrame` never ticks.
    if (frame === null) frame = env.requestAnimationFrame(() => { frame = null; flush(); });
    if (timer === null) timer = env.setTimeout(flush, MOVE_FLUSH_FALLBACK_MS);
  };
  const release = (id: number): void => {
    if (element.hasPointerCapture(id)) element.releasePointerCapture(id);
  };
  // The two oldest contacts own the pinch; a third contact is tracked (so its
  // release still counts) without becoming part of the midpoint/distance.
  const pair = (): [Contact, Contact] | null => {
    const iterator = contacts.values();
    const first = iterator.next();
    const second = iterator.next();
    if (first.done || second.done) return null;
    return [first.value, second.value];
  };
  const pinchState = (): PinchState | null => {
    const both = pair();
    if (both === null) return null;
    const [first, second] = both;
    return {
      x: (first.x + second.x) / 2,
      y: (first.y + second.y) / 2,
      distance: Math.hypot(second.x - first.x, second.y - first.y),
    };
  };
  const beginPinch = (): void => {
    pinching = true;
    ignoring = false;
    pinchFrom = pinchState();
  };
  // Ends a pinch without touching the remaining contacts: they stay in the map
  // (and keep their capture) until each lifts, but every event they send is
  // swallowed while `ignoring` is set.
  const endPinch = (): void => {
    pinching = false;
    pinchFrom = null;
    flush();
    lane.push({ kind: 'pointer_cancel' });
    ignoring = contacts.size > 0;
  };
  const cancel = (device?: { readonly pointerType?: string }): void => {
    flush();
    // A pinch is still a gesture the host owns until it sees its cancel; a
    // post-pinch ignored contact is not, so a later blur must not cancel twice.
    if (pinching || (!ignoring && contacts.size > 0)) {
      lane.push({ kind: 'pointer_cancel', ...deviceField(device) });
    }
    const ids = [...contacts.keys()];
    contacts.clear();
    pinching = false;
    ignoring = false;
    pinchFrom = null;
    for (const id of ids) release(id);
  };
  const down = (event: Event): void => {
    const pointer = event as PointerEvent;
    if (!accepting || ignoring) return;
    const at = position(pointer);
    const device = deviceField(pointer).device;
    if (pinching) {
      // A third finger neither starts a gesture nor joins the pair, but its
      // release is tracked so the post-pinch ignore covers it too.
      contacts.set(pointer.pointerId, { ...at, device });
      return;
    }
    if (contacts.size === 0) {
      if (!lane.canStart()) return;
      resize();
      flush();
      contacts.set(pointer.pointerId, { ...at, device });
      element.setPointerCapture(pointer.pointerId);
      lane.push({ kind: 'pointer_down', ...at, ...buttonField(pointer), ...shiftField(pointer), ...deviceField(pointer) });
      return;
    }
    const first = contacts.values().next();
    if (first.done || !pinchPair(first.value.device, device)) return;
    // The single-contact gesture/tool ends exactly once, then the two contacts
    // become a pinch. No `pointer_down` is sent for the second finger.
    resize();
    flush();
    contacts.set(pointer.pointerId, { ...at, device });
    element.setPointerCapture(pointer.pointerId);
    lane.push({ kind: 'pointer_cancel' });
    beginPinch();
  };
  const moving = (event: Event): void => {
    const pointer = event as PointerEvent;
    if (!accepting) return;
    const contact = contacts.get(pointer.pointerId);
    const { at, rect } = positionAndRect(pointer);
    if (pinching) {
      if (contact === undefined) return;
      contact.x = at.x;
      contact.y = at.y;
      reportRect(rect);
      const next = pinchState();
      const previous = pinchFrom;
      if (next === null) return;
      pinchFrom = next;
      if (previous === null) return;
      lane.push({
        kind: 'pinch',
        x: next.x,
        y: next.y,
        dx: next.x - previous.x,
        dy: next.y - previous.y,
        // A degenerate baseline (two fingers on the same pixel) has no ratio;
        // report the honest "no zoom" instead of a division by zero.
        scale: previous.distance === 0 ? 1 : next.distance / previous.distance,
      });
      return;
    }
    if (ignoring) return;
    if (contact === undefined) {
      // No contact means hover; while a press owns the gesture another pointer's
      // move is not hover and must not tint or drag.
      if (contacts.size > 0) return;
      reportRect(rect);
      bufferMove({ kind: 'pointer_move', ...at, ...deviceField(pointer) });
      return;
    }
    if (contacts.size !== 1) return;
    reportRect(rect);
    bufferMove({ kind: 'pointer_move', ...at, ...deviceField(pointer) });
  };
  const up = (event: Event): void => {
    const pointer = event as PointerEvent;
    const id = pointer.pointerId;
    if (ignoring) {
      if (contacts.delete(id)) release(id);
      if (contacts.size === 0) ignoring = false;
      return;
    }
    if (pinching) {
      if (!contacts.has(id)) return;
      contacts.delete(id);
      release(id);
      if (contacts.size < 2) endPinch();
      else pinchFrom = pinchState();
      return;
    }
    if (!contacts.has(id)) return;
    resize();
    flush();
    lane.push({ kind: 'pointer_up', ...position(pointer), ...buttonField(pointer), ...shiftField(pointer), ...deviceField(pointer) });
    contacts.delete(id);
    release(id);
  };
  const cancelled = (event: Event): void => {
    const pointer = event as PointerEvent;
    const id = pointer.pointerId;
    if (ignoring) {
      if (contacts.delete(id)) release(id);
      if (contacts.size === 0) ignoring = false;
      return;
    }
    if (pinching) {
      if (!contacts.has(id)) return;
      contacts.delete(id);
      release(id);
      if (contacts.size < 2) endPinch();
      else pinchFrom = pinchState();
      return;
    }
    if (contacts.has(id)) cancel(pointer);
  };
  // Leaving the surface clears the host's hover highlight. It carries no
  // position, button or metrics: the host only needs to know the pointer is
  // gone. A press owns the gesture (and a captured pointer should not report a
  // leave mid-capture at all), so an active press is never disturbed here.
  // Any coalesced move is flushed first, exactly like `down`/`up`/`wheel`, so an
  // in-flight move can never arrive *after* the leave and re-tint hover from a
  // stale in-surface position.
  const leave = (): void => {
    if (contacts.size > 0) return;
    flush();
    lane.push({ kind: 'pointer_leave' });
  };
  const wheel = (event: Event): void => {
    const wheelEvent = event as WheelEvent;
    // Unconditional: `ctrl`+wheel is WebView2/Chromium's trackpad-pinch signal,
    // and letting either shape through zooms the page instead of the drawing.
    wheelEvent.preventDefault();
    if (wheelEvent.ctrlKey) {
      // WebKit 15+ reports the same pinch as both a gesture and `ctrl`+wheel;
      // the gesture is authoritative. The page zoom above is still cancelled,
      // this sample is just not a second CAD zoom. (WebView2 only sends the
      // wheel form, and no gesture ever ran, so its path is untouched.)
      if (gestureActive || eventNow() - lastGestureAt < GESTURE_WHEEL_GUARD_MS) return;
    }
    if (!accepting) return;
    resize();
    flush();
    // Wheel deltas are normalized to CSS pixels, including line/page modes.
    const factor = wheelEvent.deltaMode === 1 ? 16 : wheelEvent.deltaMode === 2 ? env.innerHeight : 1;
    const dy = wheelEvent.deltaY * factor;
    if (wheelEvent.ctrlKey) {
      // A ctrl-modified wheel is a trackpad pinch on WebView2/Chromium, not a
      // wheel notch, so it travels as one pinch sample: no midpoint pan and a
      // purely incremental zoom multiplier. Anchored at the pointer, which the
      // host applies about the pinch midpoint (`zoom_continuously`). WebView2
      // quantizes the delta to 120-multiples, so the magnitude is coarse but
      // the sign (and therefore the zoom direction) is exact.
      lane.push({ kind: 'pinch', ...position(wheelEvent), dx: 0, dy: 0, scale: wheelPinchScale(dy) });
      return;
    }
    lane.push({
      kind: 'wheel',
      ...position(wheelEvent),
      dx: wheelEvent.deltaX * factor,
      dy,
    });
  };
  // The WebKit gesture sequence (WKWebView/Safari) is the other DOM shape of a
  // trackpad pinch. `defaults.ts` also cancels the page zoom; these listeners
  // are what turn the gesture into a CAD zoom on a `native-below` carrier.
  const gesturePinch = new GesturePinch();
  const gestureStart = (event: Event): void => {
    event.preventDefault();
    gestureActive = true;
    gesturePinch.start((event as unknown as WebKitGestureEvent).scale);
  };
  const gestureChange = (event: Event): void => {
    event.preventDefault();
    // Every sample keeps the wheel guard closed, not only the first: WebKit may
    // interleave the two signals for the whole pinch.
    gestureActive = true;
    lastGestureAt = eventNow();
    const gestureEvent = event as unknown as WebKitGestureEvent;
    // The baseline advances even while the lane is suspended, so resuming does
    // not apply a cumulative scale that was already consumed.
    const ratio = gesturePinch.change(gestureEvent.scale);
    if (ratio === null || !accepting) return;
    resize();
    flush();
    lane.push({ kind: 'pinch', ...position(gestureEvent), dx: 0, dy: 0, scale: ratio });
  };
  const gestureEnd = (event: Event): void => {
    event.preventDefault();
    gestureActive = false;
    // Opened here, not closed: the guard stays closed for the tail window so a
    // wheel belonging to this pinch cannot add a second zoom after it ended.
    lastGestureAt = eventNow();
    gesturePinch.end();
  };
  const blur = (): void => { cancel(); lane.push({ kind: 'blur' }); };
  let lastMetrics = '';
  /**
   * Publish a rect the caller already measured (one layout read per move).
   *
   * `force` re-sends an unchanged rect with a fresh lane sequence: the host's
   * layout tracker rejects an equal sequence, so this is what lets the delayed
   * reconciliation heal a report the host never applied.
   */
  const reportRect = (rect: PartitionRect, force = false): void => {
    const data = { kind: 'resize', ...rect };
    if (!isReportableRect(data)) return;
    const key = metricsKey(data);
    if (force || key !== lastMetrics) {
      flush();
      if (lane.push(data) !== false) {
        // The startup report is the host's presenter gate (no rect, no CAD at
        // all), not a geometry *change*; the delayed retry covers changes, so a
        // startup failure stays a visible no-CAD startup rather than a stale
        // surface.
        const initial = lastMetrics === '';
        lastMetrics = key;
        if (!force && !initial) scheduleReconcile();
      }
    }
  };
  const resize = (force = false): void => {
    reportRect(rectFromElement(element, env.devicePixelRatio || 1), force);
  };
  /**
   * Arm the one delayed reconciliation re-report of the current rect.
   *
   * Debounced: an interactive resize re-arms it on every changed rect, so it
   * fires once, `RESIZE_RECONCILE_DELAY_MS` after the layout settles.
   */
  const scheduleReconcile = (): void => {
    if (reconcileTimer !== null) env.clearTimeout(reconcileTimer);
    reconcileTimer = env.setTimeout(() => {
      reconcileTimer = null;
      resize(true);
    }, RESIZE_RECONCILE_DELAY_MS);
  };
  const clearResizeFallback = (): void => {
    if (resizeFallback !== null) {
      env.clearTimeout(resizeFallback);
      resizeFallback = null;
    }
  };
  /**
   * Report the partition on the next animation frame, with a timer fallback.
   *
   * `requestAnimationFrame` is throttled or suspended in an occluded/unfocused
   * webview (the same reason the pointer-move path has a timer). A resize report
   * that never runs would leave the native surface at the previous rect; the
   * timer guarantees delivery within [`RESIZE_FLUSH_FALLBACK_MS`]. Both paths
   * report at most once: whichever runs first clears the other.
   */
  const scheduleResize = (): void => {
    if (resizeFrame === null) {
      resizeFrame = env.requestAnimationFrame(() => {
        clearResizeFallback();
        resizeFrame = null;
        resize();
      });
    }
    if (resizeFallback === null) {
      resizeFallback = env.setTimeout(() => {
        resizeFallback = null;
        if (resizeFrame === null) return;
        env.cancelAnimationFrame(resizeFrame);
        resizeFrame = null;
        resize();
      }, RESIZE_FLUSH_FALLBACK_MS);
    }
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
  // pointer event (window resize, ribbon expand/collapse, start column open/close,
  // command history expand, DPR change). Observing every region keeps the host's
  // surface metrics in sync, which is what anchors wheel zoom at the cursor.
  // Floating palettes are deliberately *not* in this list: they are absolute
  // overlays and never move the partition rect.
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
    gesturestart: gestureStart,
    gesturechange: gestureChange,
    gestureend: gestureEnd,
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
      clearResizeFallback();
      if (reconcileTimer !== null) env.clearTimeout(reconcileTimer);
      observer.disconnect();
      media?.removeEventListener('change', dprChanged);
      env.removeEventListener('blur', blur);
      env.removeEventListener('resize', scheduleResize);
      for (const [type, handler] of Object.entries(handlers)) element.removeEventListener(type, handler);
    },
  };
}
