// The open-time provenance notice's one mount point, beside the update banner
// and the other shell notices in `Workspace`.
//
// This component owns only the wiring:
//
//   * read the current `cad_status.openedSource` (absent for every public-spec
//     document, New/Close and a native `.ymodel`, so those show nothing);
//   * read the user's `formatNoticeEnabled` preference and, when it is `false`,
//     render nothing at all - that check comes first, before every other reason
//     to step aside;
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
import { formatNoticeVisible, readFormatNoticePreference } from '../cad/format-notice-setting.ts';
import type { FormatNoticePreference } from '../cad/format-notice-setting.ts';
import { invoke } from '../host/invoke.ts';
import { useUpdateStatus } from '../update/store.ts';
import { FormatProvenanceNotice } from './FormatProvenanceNotice.tsx';

export function FormatProvenanceNoticeHost() {
  const shell = useShell();
  const pluginNotice = usePluginNotice();
  const update = useUpdateStatus();
  const hostNotice = useSyncExternalStore(subscribeHostNotice, isHostNoticeVisible, isHostNoticeVisible);
  const source = openedSourceOf(shell.status);
  const key = source === null ? null : openNoticeKey(shell.documentLabel, source.connector);
  const popupOpen = Boolean(shell.status?.popup);
  const [dismissedKey, setDismissedKey] = useState<string | null>(null);
  // `null` until the host has answered; the card waits for that answer instead of
  // flashing and then vanishing for a user who turned the notice off.
  const [preference, setPreference] = useState<FormatNoticePreference | null>(null);

  // An open with no reverse-engineered source owns no dismissal: clearing it
  // here keeps New/Close from inheriting silence, and makes a reopen of the same
  // file ask again instead of being suppressed by a key from a previous open.
  useEffect(() => {
    if (key === null) setDismissedKey(null);
  }, [key]);

  // Re-read per open, and again when a dialog closes: Options is where the same
  // preference is edited, so a card must not come back from a dialog that just
  // turned it off. The previous answer stays on screen while the new one is in
  // flight (no blink for the common case, which never changes).
  useEffect(() => {
    if (key === null) return;
    let alive = true;
    void readFormatNoticePreference(invoke).then(next => {
      if (alive) setPreference(next);
    });
    return () => { alive = false; };
  }, [key, popupOpen]);

  if (source === null || key === null) return null;
  // The user's own setting comes before every "step aside" rule: with the notice
  // off there is nothing to defer for, and a later dialog or banner must not make
  // it reappear. `disabled` renders nothing, and so does "no answer yet".
  if (!formatNoticeVisible(preference)) return null;
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
      <FormatProvenanceNotice
        source={source}
        onDismiss={() => setDismissedKey(key)}
        onDisabled={() => setPreference('disabled')}
      />
    </div>
  );
}
