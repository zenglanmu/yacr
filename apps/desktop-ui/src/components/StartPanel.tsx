// Start state: shown in the left column when no drawing is open. It is the
// product's welcome screen, not a generic empty panel: the mark and product line
// lead, a short intro follows, and the two real creation actions are the
// only bright elements. Each action is enabled only by the shared admission
// rules (host vocabulary plus the `cad_status` capability booleans
// `canOpen`/`canNew`).
import React from 'react';
import { Icon } from '../icons.tsx';
import { Button } from './primitives.tsx';
import { unavailableReasonKey } from '../commands/commands.ts';
import { useI18n, useShell } from '../context.ts';
import { useRecentFiles } from '../files/recent.ts';
import type { RecentFileEntry } from '../files/recent.ts';
import '../files/recent.css';

export function StartPanel() {
  const { t } = useI18n();
  const shell = useShell();
  // `app.title` is `yacr — <tagline>`; the brand is the part before the
  // separator, exactly as the title bar derives it, so both stay one string.
  const brand = t('app.title').split(' \u2014 ')[0];
  // One refusal sentence for both actions: the shared admission reason key, so a
  // blocked New reads exactly like the command line's answer (the busy sentence
  // while a load runs, "not supported yet" when the host cannot create a
  // drawing). Nothing is decided locally here.
  const refusal = (id: string, labelKey: string) => t(
    unavailableReasonKey(id, { status: shell.status, pending: shell.pending }),
    { command: t(labelKey) },
  );
  const canOpen = shell.canCommand('OPEN');
  const canNew = shell.canCommand('NEW');
  // Recent drawings: the host lists them; this panel only renders the answer.
  // A row is disabled while a load runs or while OPEN is not admitted, and its
  // title is the same refusal sentence the Open button uses.
  const recent = useRecentFiles();
  const recentDisabled = shell.loading || !canOpen;
  const openRefusal = refusal('OPEN', 'file.open');
  const openRecent = (entry: RecentFileEntry) => {
    if (!recentDisabled) recent.open(entry);
  };
  return (
    <section className="palette start-panel" aria-label={t('start.title')}>
      <header className="palette__head">
        <h2>{t('start.title')}</h2>
      </header>
      <div className="start-panel__body">
        <div className="start-panel__hero">
          <span className="start-panel__mark" aria-hidden="true"><Icon name="app" size={24} /></span>
          <div className="start-panel__identity">
            <p className="start-panel__brand">{brand}</p>
            <p className="start-panel__tagline">{t('start.product')}</p>
          </div>
        </div>
        <p className="start-panel__intro">{t('start.intro')}</p>
        <div className="start-panel__actions">
          <Button
            variant="solid"
            icon="open"
            iconSize={18}
            size="lg"
            disabled={!canOpen}
            title={canOpen ? t('file.open') : refusal('OPEN', 'file.open')}
            onClick={() => shell.runCommand('OPEN')}
          >
            {t('file.open')}
          </Button>
          <Button
            variant="ghost"
            icon="new"
            iconSize={18}
            size="lg"
            disabled={!canNew}
            title={canNew ? t('start.new_drawing') : refusal('NEW', 'start.new_drawing')}
            onClick={() => shell.runCommand('NEW')}
          >
            {t('start.new_drawing')}
          </Button>
        </div>
        <section className="recent-files" aria-labelledby="start-recent-heading">
          <h3 id="start-recent-heading" className="start-panel__section">{t('start.recent')}</h3>
          {recent.openError ? (
            <p className="recent-files__error" role="alert">
              {t('start.recent_failed', { name: recent.openError.name, reason: recent.openError.reason })}
            </p>
          ) : null}
          {recent.state.kind === 'loading' ? (
            <div className="recent-files__state recent-files__state--loading" role="status" aria-busy="true">
              <Icon name="spinner" size={16} className="spin" />
              <span className="sr-only">{t('start.recent')}</span>
            </div>
          ) : recent.state.kind === 'unsupported' ? (
            <div className="recent-files__state">
              <Icon name="info" size={16} />
              <p>{t('start.recent_unavailable')}</p>
            </div>
          ) : recent.state.kind === 'failed' ? (
            <div className="recent-files__state recent-files__state--error" role="alert">
              <Icon name="alert" size={16} />
              <p>{t('start.recent_failed', { name: t('start.recent'), reason: recent.state.reason })}</p>
            </div>
          ) : recent.state.entries.length === 0 ? (
            <div className="recent-files__state">
              <Icon name="drawing" size={20} />
              <p>{t('start.recent_empty')}</p>
            </div>
          ) : (
            <ul className="recent-files__list">
              {recent.state.entries.map((entry, index) => {
                const openLabel = t('start.recent_open', { name: entry.displayName });
                return (
                  <li key={`${index}:${entry.opaque}`}>
                    <button
                      type="button"
                      className="recent-files__row"
                      disabled={recentDisabled}
                      aria-label={openLabel}
                      title={recentDisabled ? openRefusal : openLabel}
                      onClick={() => openRecent(entry)}
                    >
                      <span className="recent-files__icon" aria-hidden="true">
                        <Icon name="drawing" size={16} />
                      </span>
                      <span className="recent-files__text">
                        <span className="recent-files__name">{entry.displayName}</span>
                        {entry.location ? (
                          <span className="recent-files__path" title={entry.location}>{entry.location}</span>
                        ) : null}
                      </span>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </section>
      </div>
    </section>
  );
}
