// Model / layout tabs, left-aligned at the lower edge of the drawing area.
//
// The list is the host's own `cad_status.layouts` plus the always-present model
// space (`layout/layout-tabs.ts` is the single projection). A layout this build
// cannot draw stays visible but disabled, and its title says so with the
// catalog's `unsupported` marker plus the host's stable reason code — never an
// invented sentence. A click on a supported tab is one `cad_command`
// (`SPACE #<id>`, or `SPACE MODEL` back to model space) through the shared
// command funnel; it is queue admission, not a claimed switch, so the active
// state still comes from `activeSpace`.
import React from 'react';
import { Icon } from '../icons.tsx';
import { useI18n, useShell } from '../context.ts';
import { layoutTabsView, layoutUnsupportedTitle } from '../layout/layout-tabs.ts';
import type { LayoutTabView } from '../layout/layout-tabs.ts';
import type { Translator } from '../i18n.ts';

function tabLabel(t: Translator, tab: LayoutTabView): string {
  if (tab.id === null) return t('layout.tab.model');
  const name = tab.name?.trim() ?? '';
  return name.length > 0 ? name : t('layout.tab.unnamed');
}

export function LayoutTabs() {
  const { t } = useI18n();
  const shell = useShell();
  const tabs = layoutTabsView(shell.status);
  return (
    <div className="region region--layouttabs layouttabs app-region" aria-label={t('layout.panel')}>
      <div className="layouttabs__list" role="tablist" aria-label={t('layout.panel')}>
        {tabs.map(tab => {
          const label = tabLabel(t, tab);
          const command = tab.command;
          const unavailable = command !== null && !shell.canCommand(command);
          const disabled = !tab.supported || unavailable;
          const title = !tab.supported
            ? layoutUnsupportedTitle(t, tab)
            : unavailable
              ? t('command.unsupported', { command: 'SPACE' })
              : undefined;
          return (
            <button
              key={tab.id === null ? 'model' : `layout-${tab.id}`}
              type="button"
              role="tab"
              aria-selected={tab.active}
              disabled={disabled}
              title={title}
              className={[
                'layout-tab',
                tab.active ? 'is-active' : '',
                tab.supported ? '' : 'is-unsupported',
                disabled ? 'is-disabled' : '',
              ].filter(Boolean).join(' ')}
              onClick={command === null ? undefined : () => shell.runCommand(command)}
            >
              {tab.id === null ? null : <Icon name="layers" size={13} />}
              <span className="layout-tab__label">{label}</span>
              {tab.supported || tab.reasonCode === null ? null : (
                <span className="layout-tab__reason">{tab.reasonCode}</span>
              )}
            </button>
          );
        })}
      </div>
    </div>
  );
}
