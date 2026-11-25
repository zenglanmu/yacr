// React binding for the domain/plugin UI registry.
//
// The registry is a mutable table with a revision counter; this hook reads the
// revision through `useSyncExternalStore` and memoises the merged layout on
// `(domain, revision)`. A slot therefore re-renders exactly when a domain layout
// or a plugin patch changes, never on every poll.
import { useMemo, useSyncExternalStore } from 'react';
import type { DomainId } from '../domains.ts';
import { domainUiRevision, subscribeDomainUi, uiLayoutFor } from './registry.ts';
import type { DomainUiLayout } from './slots.ts';

export function useDomainUi(domain: DomainId): DomainUiLayout {
  const revision = useSyncExternalStore(subscribeDomainUi, domainUiRevision, domainUiRevision);
  return useMemo(() => uiLayoutFor(domain), [domain, revision]);
}
