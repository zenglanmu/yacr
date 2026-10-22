// Strict host opt-in. URL parameters, storage and truthy strings never enable it.
export function isDomOnTopProbe(env) {
  return env?.__YACR_PROBE_DOM_ON_TOP__ === true;
}

// These are DOM observations, not proof of physical input or GPU composition.
export function createProbeDiagnostics() {
  return {
    mounted: false,
    counters: { pointerDown: 0, pointerMove: 0, pointerUp: 0, pointerCancel: 0, wheel: 0, dragMove: 0, button: 0 },
    popup: false,
    lastIpcError: null,
    ipc: {
      submitted: 0, resolved: 0, accepted: 0, refused: 0, unrecognized: 0,
      lastKind: null, lastResize: null, lastAck: null, lastRefusalReason: null,
    },
  };
}

// A resolved transport is not an admission. Only an explicit boolean counts;
// malformed acknowledgements remain visible and never become empty success.
export function recordProbeAck(diagnostics, ack) {
  const ipc = diagnostics.ipc;
  ipc.resolved += 1;
  ipc.lastAck = ack ?? null;
  if (ack?.accepted === true) {
    ipc.accepted += 1;
  } else if (ack?.accepted === false) {
    ipc.refused += 1;
    ipc.lastRefusalReason = typeof ack.reason === 'string' && ack.reason.length > 0 ? ack.reason : null;
  } else {
    ipc.unrecognized += 1;
  }
}

export function stopControlEvent(event) {
  event.stopPropagation();
}

export function observeCadInput(element, diagnostics, changed) {
  const active = new Set();
  const count = key => { diagnostics.counters[key] += 1; changed(); };
  const handlers = {
    pointerdown(event) { active.add(event.pointerId); count('pointerDown'); },
    pointermove(event) {
      if (active.has(event.pointerId)) diagnostics.counters.dragMove += 1;
      count('pointerMove');
    },
    pointerup(event) { active.delete(event.pointerId); count('pointerUp'); },
    pointercancel(event) { active.delete(event.pointerId); count('pointerCancel'); },
    lostpointercapture(event) { active.delete(event.pointerId); },
    wheel() { count('wheel'); },
  };
  for (const [type, handler] of Object.entries(handlers)) element.addEventListener(type, handler);
  return () => {
    active.clear();
    for (const [type, handler] of Object.entries(handlers)) element.removeEventListener(type, handler);
  };
}
