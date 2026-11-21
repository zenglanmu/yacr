// Whether the BIM browse panel is open.
//
// The panel is a palette window on the desktop band and a bottom sheet on the
// compact band (the same `BimPanel`, as with the references and fonts panels).
// Its open flag therefore lives outside `App`'s palette state: the ribbon entry
// (`runAction('bim')`) calls [`openBimPanel`] and `Workspace` subscribes, so one
// module owns the flag and no second copy can disagree.
//
// Module-level (not component state) for the same reason the palette geometry
// is: crossing the 1024 px breakpoint remounts the panel container but must not
// close a panel the user opened. Opening an already-open panel is a no-op (not
// a toggle), so a repeat ribbon press does not close it by accident.
const LISTENERS = new Set<() => void>();
let open = false;

/** The current open flag (stable until [`openBimPanel`]/[`closeBimPanel`]). */
export function bimPanelSnapshot(): boolean {
  return open;
}

/** Subscribe to open/close changes. */
export function subscribeBimPanel(listener: () => void): () => void {
  LISTENERS.add(listener);
  return () => { LISTENERS.delete(listener); };
}

function publish(next: boolean): void {
  if (open === next) return;
  open = next;
  for (const listener of [...LISTENERS]) listener();
}

/** Open the panel (a no-op when it is already open). */
export function openBimPanel(): void {
  publish(true);
}

/** Close the panel; already closed is a success (no notification). */
export function closeBimPanel(): void {
  publish(false);
}

/** Flip the panel's open state (the panel's own close control uses the setter). */
export function toggleBimPanel(): void {
  publish(!open);
}

/** Reset to closed (tests and a fresh document's chrome). */
export function resetBimPanel(): void {
  publish(false);
}
