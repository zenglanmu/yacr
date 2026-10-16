// Drawing-tab strip (just below the ribbon). Shows the Start state when no
// drawing is open and the open drawing otherwise. While a document open is
// running it also hosts the real import-progress affordance: a determinate bar
// when the importer exposed an entity total, an indeterminate one otherwise,
// with the live count/bytes and a cancel control only while cancellation is
// still possible.
import React from 'react';
import { Icon } from '../icons.jsx';
import { IconButton } from './primitives.jsx';
import { useI18n, useShell } from '../context.mjs';
import { importProgressView } from '../cad.mjs';

export function DocTabs() {
  const { t } = useI18n();
  const shell = useShell();
  const label = shell.documentLabel;
  const progress = importProgressView(shell.importProgress, t);
  const percent = progress && progress.determinate ? Math.round(progress.ratio * 100) : null;
  return (
    <div className="region region--doctabs doctabs app-region" aria-label={t('doctabs.label')}>
      <div className="doctabs__list" role="tablist" aria-label={t('doctabs.label')}>
        <button
          type="button"
          role="tab"
          aria-selected={!label}
          className={['doc-tab', label ? 'is-muted' : 'is-active'].filter(Boolean).join(' ')}
          aria-disabled={label ? true : undefined}
          title={label ? t('doctabs.start_unavailable') : t('start.title')}
        >
          <Icon name="start" size={14} />
          <span>{t('doctabs.start')}</span>
        </button>
        {label ? (
          <button type="button" role="tab" aria-selected className="doc-tab is-active">
            <Icon name="drawing" size={14} />
            <span className="doc-tab__label">{label}</span>
          </button>
        ) : null}
        <IconButton icon="plus" size="xs" label={t('doctabs.new_tab')} disabled />
      </div>
      {progress ? (
        <div className="doctabs__progress">
          <Icon name="spinner" size={13} className="spin" />
          <span className="import-progress__phase">{progress.phase}</span>
          <span
            className={['import-progress__bar', progress.determinate ? '' : 'is-indeterminate'].filter(Boolean).join(' ')}
            role="progressbar"
            aria-label={progress.phase}
            aria-valuemin={progress.determinate ? 0 : undefined}
            aria-valuemax={progress.determinate ? 100 : undefined}
            aria-valuenow={progress.determinate ? percent : undefined}
          >
            <span
              className="import-progress__fill"
              style={progress.determinate ? { width: `${percent}%` } : undefined}
            />
          </span>
          <span className="import-progress__detail">{progress.detail}</span>
          {progress.cancellable ? (
            <IconButton
              icon="close"
              size="xs"
              label={t('import.cancel')}
              onClick={() => shell.cancelImport()}
            />
          ) : null}
        </div>
      ) : shell.loading ? (
        <span className="doctabs__progress" role="status">
          <Icon name="spinner" size={13} className="spin" />
          {t('import.panel')}
        </span>
      ) : null}
    </div>
  );
}
