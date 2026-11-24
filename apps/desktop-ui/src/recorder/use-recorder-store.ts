// The one React binding for the Action Recorder store.
//
// The store itself is plain data + IPC; this hook only owns the store instance,
// the initial read and the subscription. `invoke` is the host adapter's single
// import point.
import { useEffect, useMemo, useSyncExternalStore } from 'react';
import { invoke } from '../host/invoke.ts';
import { createRecorderStore } from './recorder-store.ts';
import type { RecorderState, RecorderStore } from './recorder-store.ts';

/** The panel's store plus its current published state. */
export interface RecorderBinding {
  readonly store: RecorderStore;
  readonly state: RecorderState;
}

/** Build (once) and subscribe to the panel store. */
export function useRecorderStore(): RecorderBinding {
  const store = useMemo(() => createRecorderStore(invoke), []);
  const state = useSyncExternalStore(store.subscribe, store.getState, store.getState);
  useEffect(() => { void store.load(); }, [store]);
  return { store, state };
}
