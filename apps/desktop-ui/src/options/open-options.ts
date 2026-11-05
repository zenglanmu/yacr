// One entry point for every Options open.
//
// The dialog is host-owned (`cad_open_popup`), but File > Plugins needs one page
// selected. The page is a chrome-local handoff: it is written before the request
// and consumed once by the dialog. A plain open writes `null`, so a failed
// deep-link cannot leave a stale page for the next ordinary open.
import { requestOptionsTab } from './options.ts';

/** Open Options, optionally on one page. */
export function openOptionsDialog(
  requestOpenPopup: (kind: 'options') => void,
  tab: string | null = null,
): void {
  requestOptionsTab(tab);
  requestOpenPopup('options');
}
