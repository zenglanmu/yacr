// Title / Quick Access Toolbar row: the application mark, the QAT, the brand
// label and the language switch.
import React from 'react';
import { Button, IconButton } from './primitives.jsx';
import { Icon } from '../icons.jsx';
import { unavailableReasonKey } from '../commands.mjs';
import { useI18n, useShell } from '../context.mjs';

export function TitleBar() {
  const { t, locale, setLocale, locales } = useI18n();
  const shell = useShell();
  const brand = t('app.title').split(' — ')[0];

  // `NEW`/`SAVE` carry real router actions now, exactly like `OPEN`: their
  // availability is the shared admission (`shell.canCommand` = host vocabulary +
  // `cad_status.canNew`/`canSave`), never a second local check here. A refusal is
  // worded with the same reason key the command line uses.
  const qat = [
    { id: 'NEW', icon: 'new', labelKey: 'qat.new', action: () => shell.runCommand('NEW') },
    { id: 'OPEN', icon: 'open', labelKey: 'file.open', action: () => shell.runCommand('OPEN') },
    { id: 'SAVE', icon: 'save', labelKey: 'qat.save', action: () => shell.runCommand('SAVE') },
    { id: 'UNDO', icon: 'undo', labelKey: 'toolbar.undo' },
    { id: 'REDO', icon: 'redo', labelKey: 'toolbar.redo' },
  ];

  const refusal = item => t(
    unavailableReasonKey(item.id, { status: shell.status, pending: shell.pending }),
    { command: t(item.labelKey) },
  );

  return (
    <div className="region region--title titlebar app-region" role="banner">
      <button
        type="button"
        className={['app-mark', shell.ribbonTab === 'file' ? 'is-active' : ''].filter(Boolean).join(' ')}
        aria-label={t('app.menu')}
        title={t('app.menu')}
        aria-pressed={shell.ribbonTab === 'file'}
        onClick={() => shell.setRibbonTab(shell.ribbonTab === 'file' ? 'home' : 'file')}
      >
        <Icon name="app" size={20} />
      </button>

      <div className="qat" role="toolbar" aria-label={t('qat.label')}>
        {qat.map(item => {
          const enabled = Boolean(item.action) && shell.canCommand(item.id);
          return (
            <IconButton
              key={item.id}
              icon={item.icon}
              label={t(item.labelKey)}
              size="sm"
              disabled={!enabled}
              title={enabled ? t(item.labelKey) : `${t(item.labelKey)} — ${refusal(item)}`}
              onClick={item.action}
            />
          );
        })}
        <span className="qat__rule" aria-hidden="true" />
        <IconButton icon="chevronDown" label={t('qat.customize')} size="sm" disabled />
      </div>

      <span className="titlebar__brand">{brand}</span>

      <div className="titlebar__spacer" />

      <label className="field-inline">
        <Icon name="language" size={15} />
        <span className="field-inline__label">{t('host.language_label')}</span>
        <select
          className="select"
          aria-label={t('host.language_label')}
          value={locale}
          onChange={event => setLocale(event.target.value)}
        >
          {locales.map(code => <option key={code} value={code}>{code}</option>)}
        </select>
      </label>
    </div>
  );
}
