// Strict host opt-in. URL parameters, storage and truthy strings never enable it.
// DOM-on-top itself is the production arrangement; this only gates the fixture.

/** What the fixture only needs from an event: it must not reach the CAD lane. */
export interface PropagationEvent {
  stopPropagation(): void;
}

/** One input-observation counter of the fixture (`data-probe` readout). */
export type StackingCounters = {
  pointerDown: number;
  pointerMove: number;
  pointerUp: number;
  pointerCancel: number;
  wheel: number;
  dragMove: number;
  button: number;
};

/** The fixture's own record of the IPC it submitted and the answers it got. */
export type StackingIpc = {
  submitted: number;
  resolved: number;
  accepted: number;
  refused: number;
  unrecognized: number;
  lastKind: string | null;
  lastResize: Record<string, unknown> | null;
  lastAck: unknown;
  lastRefusalReason: string | null;
};

/** Everything the fixture keeps for the compositor capture readout. */
export interface StackingDiagnostics {
  mounted: boolean;
  counters: StackingCounters;
  popup: boolean;
  lastIpcError: string | null;
  ipc: StackingIpc;
}

declare global {
  interface Window {
    /** The host's init-script opt-in; only the literal `true` enables the fixture. */
    readonly __YACR_STACKING_REGRESSION__?: unknown;
    /** The fixture's live record, published for the evidence driver. */
    __yacr_stacking_regression?: StackingDiagnostics;
  }
}

export function isStackingRegression(env: Window | null | undefined): boolean {
  return env?.__YACR_STACKING_REGRESSION__ === true;
}

// These are DOM observations, not proof of physical input or GPU composition.
export function createStackingDiagnostics(): StackingDiagnostics {
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
export function recordProbeAck(diagnostics: StackingDiagnostics, ack: unknown): void {
  const ipc = diagnostics.ipc;
  ipc.resolved += 1;
  ipc.lastAck = ack ?? null;
  const record = ack !== null && typeof ack === 'object'
    ? ack as { readonly accepted?: unknown; readonly reason?: unknown }
    : null;
  if (record?.accepted === true) {
    ipc.accepted += 1;
  } else if (record?.accepted === false) {
    ipc.refused += 1;
    ipc.lastRefusalReason = typeof record.reason === 'string' && record.reason.length > 0 ? record.reason : null;
  } else {
    ipc.unrecognized += 1;
  }
}

export function stopControlEvent(event: PropagationEvent): void {
  event.stopPropagation();
}

export function observeCadInput(
  element: HTMLElement,
  diagnostics: StackingDiagnostics,
  changed: () => void,
): () => void {
  const active = new Set<number>();
  const count = (key: keyof StackingCounters): void => { diagnostics.counters[key] += 1; changed(); };
  const handlers: Readonly<Record<string, (event: PointerEvent) => void>> = {
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
  for (const [type, handler] of Object.entries(handlers)) element.addEventListener(type, handler as EventListener);
  return () => {
    active.clear();
    for (const [type, handler] of Object.entries(handlers)) element.removeEventListener(type, handler as EventListener);
  };
}
