// A single ordered IPC lane. Continuous events may merge only at the tail,
// never across a discrete gesture boundary. Capacity reserves terminal events.
import { isReportableRect, metricsKey, pointerPosition, rectFromElement } from './metrics.mjs';

export function createInputLane(send, onError, capacity = 64) {
  const queue = [];
  let busy = false;
  let sequence = 0;
  let disposed = false;
  const continuous = new Set(['pointer_move', 'wheel']);
  const mergeable = new Set([...continuous, 'resize']);
  async function drain() {
    if (busy || disposed) return;
    busy = true;
    while (queue.length && !disposed) {
      const input = queue.shift();
      try { await send(input); } catch (error) { onError(String(error)); }
    }
    busy = false;
  }
  return {
    canStart: () => !disposed && queue.length < capacity - 16,
    push(input) {
      if (disposed) return false;
      const tail = queue.at(-1);
      if (mergeable.has(input.kind) && tail?.kind === input.kind) {
        const next = { ...input, sequence: ++sequence };
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
    dispose() { disposed = true; queue.length = 0; },
  };
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
export function shiftField(event) {
  return { shift: event.shiftKey === true };
}

export function bindProbeInput(element, lane, env = window) {
  let active = null;
  let move = null;
  let frame = null;
  let resizeFrame = null;
  let accepting = true;
  // `button` is the DOM `MouseEvent.button` (0 primary / 1 middle / 2 secondary).
  // It is only meaningful on a press/release, so it rides `pointer_down` and
  // `pointer_up`; a `pointermove` (and a wheel) reports no button change as `-1`,
  // and the wire refuses anything outside `0..=2` — including that `-1` — instead
  // of guessing. Those kinds therefore omit the field entirely (never `null`,
  // `0` or `-1`), leaving the host to treat the omission as the pressed/primary
  // button. Normalizing this DOM quirk here matches the wheel-delta
  // normalization below.
  const position = event => pointerPosition(element, event);
  const buttonField = event => (event.button >= 0 ? { button: event.button } : {});
  const flush = () => {
    if (frame !== null) env.cancelAnimationFrame(frame);
    frame = null;
    if (move) lane.push(move);
    move = null;
  };
  const release = () => {
    const id = active;
    active = null;
    if (id !== null && element.hasPointerCapture(id)) element.releasePointerCapture(id);
  };
  const cancel = () => {
    flush();
    if (active !== null) { lane.push({ kind: 'pointer_cancel' }); release(); }
  };
  const down = event => {
    if (!accepting || active !== null || !lane.canStart()) return;
    resize();
    flush();
    active = event.pointerId;
    element.setPointerCapture(active);
    lane.push({ kind: 'pointer_down', ...position(event), ...buttonField(event), ...shiftField(event) });
  };
  const moving = event => {
    if (!accepting || (active !== null && event.pointerId !== active)) return;
    resize();
    move = { kind: 'pointer_move', ...position(event) };
    if (frame === null) frame = env.requestAnimationFrame(() => { frame = null; flush(); });
  };
  const up = event => {
    if (event.pointerId !== active) return;
    resize();
    flush();
    lane.push({ kind: 'pointer_up', ...position(event), ...buttonField(event), ...shiftField(event) });
    release();
  };
  const cancelled = event => { if (event.pointerId === active) cancel(); };
  // Leaving the surface clears the host's hover highlight. It carries no
  // position, button or metrics: the host only needs to know the pointer is
  // gone. A press owns the gesture (and a captured pointer should not report a
  // leave mid-capture at all), so an active press is never disturbed here.
  // Any coalesced move is flushed first, exactly like `down`/`up`/`wheel`, so an
  // in-flight move can never arrive *after* the leave and re-tint hover from a
  // stale in-surface position.
  const leave = () => {
    if (active !== null) return;
    flush();
    lane.push({ kind: 'pointer_leave' });
  };
  const wheel = event => {
    event.preventDefault();
    if (!accepting) return;
    resize();
    flush();
    // Wheel deltas are normalized to CSS pixels, including line/page modes.
    const factor = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? env.innerHeight : 1;
    lane.push({ kind: 'wheel', ...position(event), dx: event.deltaX * factor, dy: event.deltaY * factor });
  };
  const blur = () => { cancel(); lane.push({ kind: 'blur' }); };
  let lastMetrics = '';
  const resize = () => {
    const data = { kind: 'resize', ...rectFromElement(element, env.devicePixelRatio || 1) };
    if (!isReportableRect(data)) return;
    const key = metricsKey(data);
    if (key !== lastMetrics) {
      flush();
      if (lane.push(data) !== false) lastMetrics = key;
    }
  };
  const scheduleResize = () => {
    if (resizeFrame === null) resizeFrame = env.requestAnimationFrame(() => {
      resizeFrame = null;
      resize();
    });
  };
  let media;
  const dprChanged = () => { scheduleResize(); watchDpr(); };
  function watchDpr() {
    media?.removeEventListener('change', dprChanged);
    media = env.matchMedia(`(resolution: ${env.devicePixelRatio || 1}dppx)`);
    media.addEventListener('change', dprChanged);
  }
  const observer = new env.ResizeObserver(scheduleResize);
  observer.observe(element);
  // Chrome regions can shift the CAD origin or shrink the partition without a
  // pointer event (window resize, ribbon expand/collapse, palette dock, command
  // history expand, DPR change). Observing every region keeps the host's surface
  // metrics in sync, which is what anchors wheel zoom at the cursor.
  const layoutRoot = element.closest?.('.shell') ?? element.parentElement;
  for (const region of layoutRoot?.querySelectorAll('.app-region') ?? []) observer.observe(region);
  const handlers = { pointerdown: down, pointermove: moving, pointerup: up, pointercancel: cancelled, lostpointercapture: cancelled, pointerleave: leave, wheel };
  for (const [type, handler] of Object.entries(handlers)) element.addEventListener(type, handler, { passive: false });
  env.addEventListener('blur', blur);
  env.addEventListener('resize', scheduleResize);
  watchDpr();
  resize();
  return {
    suspend(value) { accepting = !value; if (value) cancel(); },
    dispose() {
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
