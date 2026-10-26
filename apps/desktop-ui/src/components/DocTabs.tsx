// Drawing-tab strip (just below the ribbon). Shows the Start state when no
// drawing is open and the open drawing otherwise. While a document open is
// running it also hosts the real import-progress affordance: a determinate bar
// when the importer exposed an entity total, an indeterminate one otherwise,
// with the live count/bytes and a cancel control only while cancellation is
// still possible.
import React from 'react';
import { Icon } from '../icons.tsx';
import { IconButton } from './primitives.tsx';
import { useI18n, useShell } from '../context.ts';
import { importProgressView } from '../cad/cad.ts';
import type { CadImportProgress } from '../cad/cad.ts';
import { ONE_DOCUMENT_KEY } from '../limits.ts';

export function DocTabs() {
  const { t } = useI18n();
  const shell = useShell();
  const label = shell.documentLabel;
  const progress = importProgressView(shell.importProgress as CadImportProgress | null, t);
  const percent = progress && progress.determinate ? Math.round((progress.ratio ?? 1) * 100) : null;
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
        {/* One drawing per session is a real limitation, not missing wiring: the
            `+` keeps the tab-strip convention but says so, instead of leaving a
            disabled button that explains nothing. Opening another file replaces
            this session's drawing (it never opens a second tab). */}
        <IconButton icon="plus" size="xs" label={t('doctabs.new_tab')} disabled title={t(ONE_DOCUMENT_KEY)} />
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
            aria-valuenow={progress.determinate && percent !== null ? percent : undefined}
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
