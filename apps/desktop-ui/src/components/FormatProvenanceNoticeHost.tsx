// The open-time provenance notice's one mount point, beside the update banner
// and the other shell notices in `Workspace`.
//
// This component owns only the wiring:
//
//   * read the current `cad_status.openedSource` (absent for every public-spec
//     document, New/Close and a native `.ymodel`, so those show nothing);
//   * key the dismissal on the document label + connector and keep it in memory
//     only; a dismissal belongs to one open, so it is cleared when the document
//     goes away (`key === null`) and a different file asks again;
//   * defer while a host dialog, another shell notice, or the update banner is
//     showing, so two cards never stack in the same slot;
//
// The suppression rule is the update banner's (`UpdateNoticeHost.tsx`), because
// both cards occupy the same docked slot above the status bar.
import React, { useEffect, useState, useSyncExternalStore } from 'react';
import { useShell } from '../context.ts';
import { isVisible as isHostNoticeVisible, subscribe as subscribeHostNotice } from '../host/notice.ts';
import { usePluginNotice } from '../plugins/host.ts';
import {
  isOpenNoticeDismissed,
  openNoticeKey,
  openedSourceOf,
} from '../cad/opened-source.ts';
import { useUpdateStatus } from '../update/store.ts';
import { FormatProvenanceNotice } from './FormatProvenanceNotice.tsx';

export function FormatProvenanceNoticeHost() {
  const shell = useShell();
  const pluginNotice = usePluginNotice();
  const update = useUpdateStatus();
  const hostNotice = useSyncExternalStore(subscribeHostNotice, isHostNoticeVisible, isHostNoticeVisible);
  const source = openedSourceOf(shell.status);
  const key = source === null ? null : openNoticeKey(shell.documentLabel, source.connector);
  const [dismissedKey, setDismissedKey] = useState<string | null>(null);

  // An open with no reverse-engineered source owns no dismissal: clearing it
  // here keeps New/Close from inheriting silence, and makes a reopen of the same
  // file ask again instead of being suppressed by a key from a previous open.
  useEffect(() => {
    if (key === null) setDismissedKey(null);
  }, [key]);

  if (source === null || key === null) return null;
  // `cad_status.popup` non-null is the host's live dialog. The card is not
  // portalled and must not paint under (or into) the dialog layer.
  if (shell.status?.popup) return null;
  if (hostNotice || pluginNotice !== null) return null;
  // The update banner shares this slot and offers its own Later/Skip, so it
  // keeps the slot while it is up; this card comes back as soon as that banner
  // is dismissed (the store notifies, no polling guess).
  if (update.kind === 'available') return null;
  if (isOpenNoticeDismissed(dismissedKey, key)) return null;

  return (
    <div className="format-notice-layer">
      <FormatProvenanceNotice source={source} onDismiss={() => setDismissedKey(key)} />
    </div>
  );
}
