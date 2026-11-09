// The update banner's one mount point, beside the plugin and host notices in
// `Workspace`.
//
// This component owns only the wiring:
//
//   * start the one session-scoped check (the store is idempotent, so a
//     StrictMode double-effect or a second mount does nothing);
//   * read the status and render exactly one banner for `available`; every other
//     kind - including `unavailable` - is silent, because a check that could not
//     run must never pretend the app is current (nor invent a notice);
//   * defer while a host dialog is open or another shell notice is showing, so
//     two notices never stack.
//
// The check's own throttling, persistence and failure modelling live in
// `update/store.ts` and are not duplicated here.
import React, { useEffect, useSyncExternalStore } from 'react';
import { useShell } from '../context.ts';
import { isVisible as isHostNoticeVisible, subscribe as subscribeHostNotice } from '../host/notice.ts';
import { usePluginNotice } from '../plugins/host.ts';
import {
  dismissUpdate,
  openReleasePage,
  skipUpdateVersion,
  startUpdateCheck,
  useUpdateStatus,
} from '../update/store.ts';
import { UpdateNotice } from './UpdateNotice.tsx';

export function UpdateNoticeHost() {
  useEffect(() => {
    startUpdateCheck();
  }, []);

  const status = useUpdateStatus();
  const shell = useShell();
  const pluginNotice = usePluginNotice();
  const hostNotice = useSyncExternalStore(subscribeHostNotice, isHostNoticeVisible, isHostNoticeVisible);

  if (status.kind !== 'available') return null;
  // `cad_status.popup` non-null is the host's live dialog. The banner is not
  // portalled and must not paint under (or into) that dialog's layer.
  if (shell.status?.popup) return null;
  if (hostNotice || pluginNotice !== null) return null;

  return (
    <div className="update-notice-layer">
      <UpdateNotice
        current={status.current}
        latest={status.latest}
        onOpen={openReleasePage}
        onSkip={skipUpdateVersion}
        onLater={dismissUpdate}
      />
    </div>
  );
}
