// The ADR-0056 action-recorder catalog rows.
//
// These are real router selectors now: the session owns the recording buffer,
// the versioned macro file and playback. The panel is a chrome-local shell action
// (`MANAGE_RECORDER_PANEL`), so no row here opens a dialog.
import type { CommandCatalogEntry } from './catalog.ts';

/** The seven action-recorder rows, in AutoCAD's command order. */
export const ACTION_RECORDER_COMMAND_CATALOG: readonly CommandCatalogEntry[] = Object.freeze([
  { id: 'ACTRECORD', labelKey: 'draw.kind.actrecord', icon: 'settings', group: 'modify' },
  { id: 'ACTSTOP', labelKey: 'draw.kind.actstop', icon: 'settings', group: 'modify' },
  { id: '-ACTSTOP', labelKey: 'draw.kind.dashactstop', icon: 'settings', group: 'modify' },
  { id: 'ACTUSERINPUT', labelKey: 'draw.kind.actuserinput', icon: 'settings', group: 'modify' },
  { id: 'ACTUSERMESSAGE', labelKey: 'draw.kind.actusermessage', icon: 'settings', group: 'modify' },
  { id: '-ACTUSERMESSAGE', labelKey: 'draw.kind.dashactusermessage', icon: 'settings', group: 'modify' },
  { id: 'ACTMANAGER', labelKey: 'draw.kind.actmanager', icon: 'palette', group: 'modify' },
]);
