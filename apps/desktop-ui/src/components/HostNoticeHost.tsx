// The host notice's one mount point, beside the plugin notice in `Workspace`.
//
// The store (`host/notice.ts`) owns the latch and the visibility flag; this
// component only subscribes, translates the catalog keys and renders the shared
// presentational `HostNotice`. It re-renders on open/dismiss only — a status
// snapshot that keeps the token present changes nothing here.
import React, { useSyncExternalStore } from 'react';
import { useI18n } from '../context.ts';
import { dismiss, isVisible, subscribe } from '../host/notice.ts';
import { HostNotice } from './HostNotice.tsx';

export function HostNoticeHost() {
  const visible = useSyncExternalStore(subscribe, isVisible, isVisible);
  const { t } = useI18n();
  if (!visible) return null;
  return (
    <HostNotice
      title={t('gpu.pulled.insufficient_vram.title')}
      message={t('gpu.pulled.insufficient_vram.body')}
      dismissLabel={t('gpu.pulled.insufficient_vram.dismiss')}
      onDismiss={dismiss}
    />
  );
}
