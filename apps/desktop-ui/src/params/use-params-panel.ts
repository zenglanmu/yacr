// The one React binding for the Parameters Manager panel's open flag.
import { useSyncExternalStore } from 'react';
import { paramsPanelSnapshot, subscribeParamsPanel } from './panel.ts';

/** Whether the Parameters Manager panel is open. */
export function useParamsPanelOpen(): boolean {
  return useSyncExternalStore(subscribeParamsPanel, paramsPanelSnapshot, paramsPanelSnapshot);
}
