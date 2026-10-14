// A single ordered IPC lane. Continuous events may merge only at the tail,
// never across a discrete gesture boundary. Capacity reserves terminal events.
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

export function bindProbeInput(element, lane, env = window) {
  let active = null;
  let move = null;
  let frame = null;
  let resizeFrame = null;
  let accepting = true;
  const position = event => {
    const rect = element.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  };
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
    lane.push({ kind: 'pointer_down', ...position(event) });
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
    lane.push({ kind: 'pointer_up', ...position(event) });
    release();
  };
  const cancelled = event => { if (event.pointerId === active) cancel(); };
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
  let lastSize = '';
  const resize = () => {
    const rect = element.getBoundingClientRect();
    const data = { kind: 'resize', x: rect.left, y: rect.top, width: rect.width, height: rect.height, dpr: env.devicePixelRatio || 1 };
    const key = JSON.stringify(data);
    if (key !== lastSize) {
      flush();
      if (lane.push(data) !== false) lastSize = key;
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
  // Layout contributors can shift the CAD origin without changing its size.
  for (const chrome of element.parentElement?.querySelectorAll('.ribbon, .command-bar') ?? []) observer.observe(chrome);
  const handlers = { pointerdown: down, pointermove: moving, pointerup: up, pointercancel: cancelled, lostpointercapture: cancelled, wheel };
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
