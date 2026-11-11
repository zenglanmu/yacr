// The chrome-local surfaces the typed words open, as one external store.
//
// These are the non-CAD windows and panels that have no router command:
// Quick View Drawings, Quick View Layouts, QuickCalc, the workspace selector and
// the workspace settings dialog. Keeping them in one store means
// `components/ChromeSurfaces.tsx` is the only renderer, the shell never grows a
// dozen `useState` pairs, and `local-selectors.ts` stays a pure dispatch table.
//
// Opening a panel closes the other floating panel (two QV panels at once would
// cover the same drawing area), and opening a modal closes every floating panel
// behind it. Closing is idempotent: closing an already-closed surface is a
// success, exactly like `PROPERTIESCLOSE`/`LAYERCLOSE`.
//
// No CAD state is read or written here; `ChromeSurfaces` reads `cad_status` from
// the shell context for its own data.

/** The two Quick View panels. */
export type QuickViewKind = 'drawings' | 'layouts';

/** The three classic block/group/image panels. */
export type ClassicPanelKind = 'insert' | 'group' | 'image';

/** Which workspace dialog `WSSAVE`/`WSSETTINGS` opened. */
export type WorkspaceDialogMode = 'settings' | 'save';

/** The surfaces' state. */
export interface ChromeSurfaceState {
  readonly quickView: QuickViewKind | null;
  /** Bumped by every `UPDATETHUMBSNOW`, so an open panel recomputes its cards. */
  readonly quickViewRevision: number;
  readonly quickCalc: boolean;
  /** The classic external-references panel (`CLASSICXREF`); non-modal. */
  readonly classicXref: boolean;
  /** The open classic insert/group/image panel, or `null`; non-modal. */
  readonly classicPanel: ClassicPanelKind | null;
  readonly workspaceSelector: boolean;
  readonly workspaceSettings: WorkspaceDialogMode | null;
}

const CLOSED: ChromeSurfaceState = Object.freeze({
  quickView: null,
  quickViewRevision: 0,
  quickCalc: false,
  classicXref: false,
  classicPanel: null,
  workspaceSelector: false,
  workspaceSettings: null,
});

let state: ChromeSurfaceState = CLOSED;
const LISTENERS = new Set<() => void>();

function publish(next: ChromeSurfaceState): void {
  state = Object.freeze(next);
  for (const listener of [...LISTENERS]) listener();
}

/** The current surface state (referentially stable until something changes). */
export function chromeSurfaceSnapshot(): ChromeSurfaceState {
  return state;
}

/** Subscribe to surface changes. */
export function subscribeChromeSurfaces(listener: () => void): () => void {
  LISTENERS.add(listener);
  return () => { LISTENERS.delete(listener); };
}

/** Open one Quick View panel, closing the other floating chrome. */
export function openQuickView(kind: QuickViewKind): void {
  publish({ ...CLOSED, quickView: kind, quickViewRevision: state.quickViewRevision });
}

/**
 * Recompute the Quick View preview cards now (`UPDATETHUMBSNOW`).
 *
 * The previews are vector diagrams drawn from the current session, so "refresh"
 * is a revision bump, not a file write: an open panel re-renders its cards and
 * shows a transient "updated" line. With no panel open, the drawings panel opens
 * so the refresh is visible instead of silent.
 */
export function refreshQuickView(): void {
  publish({
    ...state,
    quickView: state.quickView ?? 'drawings',
    quickViewRevision: state.quickViewRevision + 1,
    workspaceSelector: false,
  });
}

/** Close one Quick View panel; already closed (or the other panel) is a success. */
export function closeQuickView(kind?: QuickViewKind): void {
  if (state.quickView === null) return;
  if (kind !== undefined && state.quickView !== kind) return;
  publish({ ...state, quickView: null });
}

/** Open the QuickCalc dialog (a modal). */
export function openQuickCalc(): void {
  publish({ ...CLOSED, quickCalc: true, quickViewRevision: state.quickViewRevision });
}

/** Close QuickCalc; already closed is a success. */
export function closeQuickCalc(): void {
  if (!state.quickCalc) return;
  publish({ ...state, quickCalc: false });
}

/** Open the classic external-references panel; a repeat open re-focuses it. */
export function openClassicXref(): void {
  publish({ ...CLOSED, classicXref: true, quickViewRevision: state.quickViewRevision });
}

/** Close the classic xref panel; already closed is a success. */
export function closeClassicXref(): void {
  if (!state.classicXref) return;
  publish({ ...state, classicXref: false });
}

/** Open one classic panel; a repeat open re-focuses it and closes the others. */
export function openClassicPanel(panel: ClassicPanelKind): void {
  publish({ ...CLOSED, classicPanel: panel, quickViewRevision: state.quickViewRevision });
}

/** Close the open classic panel; already closed is a success. */
export function closeClassicPanel(): void {
  if (state.classicPanel === null) return;
  publish({ ...state, classicPanel: null });
}

/** Open the workspace selector (a floating panel). */
export function openWorkspaceSelector(): void {
  publish({ ...CLOSED, workspaceSelector: true, quickViewRevision: state.quickViewRevision });
}

/** Close the workspace selector; already closed is a success. */
export function closeWorkspaceSelector(): void {
  if (!state.workspaceSelector) return;
  publish({ ...state, workspaceSelector: false });
}

/** Open the workspace settings dialog in list mode or in save-current mode. */
export function openWorkspaceSettings(mode: WorkspaceDialogMode = 'settings'): void {
  publish({ ...CLOSED, workspaceSettings: mode, quickViewRevision: state.quickViewRevision });
}

/** Close the workspace settings dialog; already closed is a success. */
export function closeWorkspaceSettings(): void {
  if (state.workspaceSettings === null) return;
  publish({ ...state, workspaceSettings: null });
}

/** Reset the store. Contract tests call this between cases. */
export function resetChromeSurfaces(): void {
  publish(CLOSED);
}
