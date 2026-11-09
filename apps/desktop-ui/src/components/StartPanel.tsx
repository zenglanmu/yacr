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
        <h3 className="start-panel__section">{t('start.recent')}</h3>
        <div className="start-panel__empty">
          <Icon name="drawing" size={20} />
          <p>{t('start.recent_empty')}</p>
          <p className="start-panel__note">{t('start.recent_unavailable')}</p>
        </div>
      </div>
    </section>
  );
}
