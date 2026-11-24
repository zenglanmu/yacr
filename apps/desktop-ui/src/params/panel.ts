// Whether the Parameters Manager panel is open.
//
// The panel is a palette window on the desktop band and a bottom sheet on the
// compact band (the same component), exactly like the BIM and Font panels. Its
// open flag lives outside `App`'s palette state: the ribbon entry
// (`runAction('parameters')`) and the typed `PARAMETERS` word call
// [`openParamsPanel`]/[`closeParamsPanel`], and `Workspace` subscribes — one
// module owns the flag, so no second copy can disagree.
//
// Module-level (not component state) for the same reason the palette geometry
// is: crossing the 1024 px breakpoint remounts the panel container but must not
// close a panel the user opened. `PARAMETERS` opens (or re-focuses) and
// `PARAMETERSCLOSE` closes; closing an already-closed panel is a success.
const LISTENERS = new Set<() => void>();
let open = false;

/** The current open flag (stable until [`openParamsPanel`]/[`closeParamsPanel`]). */
export function paramsPanelSnapshot(): boolean {
  return open;
}

/** Subscribe to open/close changes. */
export function subscribeParamsPanel(listener: () => void): () => void {
  LISTENERS.add(listener);
  return () => { LISTENERS.delete(listener); };
}

function publish(next: boolean): void {
  if (open === next) return;
  open = next;
  for (const listener of [...LISTENERS]) listener();
}

/** Open the panel (a no-op when it is already open). */
export function openParamsPanel(): void {
  publish(true);
}

/** Close the panel; already closed is a success (no notification). */
export function closeParamsPanel(): void {
  publish(false);
}

/** Flip the panel's open state (the panel's own close control uses the setter). */
export function toggleParamsPanel(): void {
  publish(!open);
}

/** Reset to closed (tests and a fresh document's chrome). */
export function resetParamsPanel(): void {
  publish(false);
}
