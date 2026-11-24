// The chrome's domain state: the document's domain (host-reported) plus the two
// session-only pieces the chrome owns.
//
// 1. **Preview** — with no document, the switcher may show another profile. It
//    is not written anywhere, does not change the NEW default, and is ignored the
//    moment a document reports a domain (ADR-0053 D2).
// 2. **Switcher visibility** — a popover flag, so the `DOMAIN` command and the
//    dock button open the same surface.
//
// A small external store (`subscribe` + an immutable snapshot) so the dock, the
// surface card and the command path read one source through
// `useSyncExternalStore`. Every function is a pure state transition over frozen
// values, which is what the Node contract tests exercise without a DOM.

import { domainOf } from './domains.ts';
import type { DomainId } from './domains.ts';

/** The observable value. */
export interface DomainSnapshot {
  /** The profile shown while no document is open, or `null`. */
  readonly preview: DomainId | null;
  /** Whether the switcher popover is open. */
  readonly switcherOpen: boolean;
  /** Bumped when a preview is explicitly chosen, so a repeated choice re-applies. */
  readonly revision: number;
}

const CLOSED: DomainSnapshot = Object.freeze({ preview: null, switcherOpen: false, revision: 0 });

let snapshot: DomainSnapshot = CLOSED;
const LISTENERS = new Set<() => void>();

function publish(next: Partial<DomainSnapshot>): void {
  snapshot = Object.freeze({ ...snapshot, ...next });
  for (const listener of [...LISTENERS]) listener();
}

/** The current store value (referentially stable until something changes). */
export function domainStoreSnapshot(): DomainSnapshot {
  return snapshot;
}

/** Subscribe to store changes; returns the unsubscribe function. */
export function subscribeDomainStore(listener: () => void): () => void {
  LISTENERS.add(listener);
  return () => { LISTENERS.delete(listener); };
}

/**
 * The domain the chrome renders.
 *
 * With an open document the host's token is the only authority. Without one, the
 * session preview (when chosen) stands in for it — a UI state, never a document
 * write.
 */
export function currentDomain(
  status: { readonly applicationDomain?: unknown; readonly documentLabel?: unknown } | null | undefined,
): DomainId {
  if (status?.documentLabel) return domainOf(status);
  return snapshot.preview ?? domainOf(status);
}

/** Choose a preview profile (no document only; the caller decides when). */
export function previewDomain(id: DomainId): void {
  publish({ preview: id, revision: snapshot.revision + 1 });
}

/** Drop the preview (a document took over, or the user returned to drafting). */
export function clearPreviewDomain(): void {
  if (snapshot.preview === null) return;
  publish({ preview: null });
}

/** Open or close the switcher popover. */
export function setSwitcherOpen(open: boolean): void {
  if (snapshot.switcherOpen === open) return;
  publish({ switcherOpen: open });
}

/** Flip the switcher popover (`DOMAIN` with no argument). */
export function toggleSwitcher(): void {
  publish({ switcherOpen: !snapshot.switcherOpen });
}

/** Reset for tests. */
export function resetDomainStore(): void {
  snapshot = CLOSED;
  for (const listener of [...LISTENERS]) listener();
}
