// The plugin diagnostic notice (ADR 0030 §4).
//
// A plugin veto or a rejected multi-selection panel is not a command-line
// refusal: it is a modal explanation. The host stores only a catalog key plus
// parameters; this component translates it and renders the shared presentational
// `PluginNotice`. An unknown key falls back to the stable catalog sentence that
// still names the original key instead of showing a raw identifier.
import React from 'react';
import { useI18n } from '../context.ts';
import { dismissPluginNotice, usePluginNotice } from '../plugins/host.ts';
import { PluginNotice } from './PluginNotice.tsx';

export function PluginNoticeHost() {
  const notice = usePluginNotice();
  const { t } = useI18n();
  if (!notice) return null;
  const message = t(notice.key, notice.params);
  const text = message === notice.key
    ? t('properties.vendor.unknown_key', { key: notice.key })
    : message;
  return <PluginNotice message={text} onDismiss={dismissPluginNotice} />;
}
