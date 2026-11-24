// Whether the Action Recorder panel is open (ADR-0056 D4).
//
// Same module-store pattern as the Parameters Manager and BIM panels: the ribbon
// action and the workspace subscribe to one flag, so no second copy can
// disagree. `ACTMANAGER` itself is a router command (it lists/plays/deletes on
// the host); this flag is the chrome-local panel surface only.
const LISTENERS = new Set<() => void>();
let open = false;

/** The current open flag (stable until [`openRecorderPanel`]/[`closeRecorderPanel`]). */
export function recorderPanelSnapshot(): boolean {
  return open;
}

/** Subscribe to open/close changes. */
export function subscribeRecorderPanel(listener: () => void): () => void {
  LISTENERS.add(listener);
  return () => { LISTENERS.delete(listener); };
}

function publish(next: boolean): void {
  if (open === next) return;
  open = next;
  for (const listener of [...LISTENERS]) listener();
}

/** Open the panel (a no-op when it is already open). */
export function openRecorderPanel(): void {
  publish(true);
}

/** Close the panel; already closed is a success (no notification). */
export function closeRecorderPanel(): void {
  publish(false);
}

/** Reset to closed (tests and a fresh document's chrome). */
export function resetRecorderPanel(): void {
  publish(false);
}
