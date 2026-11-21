// The one React binding for the BIM panel's open flag (`panel.ts`).
import { useSyncExternalStore } from 'react';
import { bimPanelSnapshot, subscribeBimPanel } from './panel.ts';

/** Whether the BIM browse panel is open. */
export function useBimPanelOpen(): boolean {
  return useSyncExternalStore(subscribeBimPanel, bimPanelSnapshot, bimPanelSnapshot);
}
