// The one React binding for the Action Recorder panel's open flag.
import { useSyncExternalStore } from 'react';
import { recorderPanelSnapshot, subscribeRecorderPanel } from './panel.ts';

/** Whether the Action Recorder panel is open. */
export function useRecorderPanelOpen(): boolean {
  return useSyncExternalStore(subscribeRecorderPanel, recorderPanelSnapshot, recorderPanelSnapshot);
}
