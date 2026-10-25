// Start state: shown in a docked palette when no drawing is open. It offers the
// two real creation actions — open an existing drawing and start a blank one —
// each enabled only by the shared admission rules (host vocabulary plus the
// `cad_status` capability booleans `canOpen`/`canNew`), and is explicit about
// what this build does not do.
import React from 'react';
import { Icon } from '../icons.tsx';
import { Button } from './primitives.tsx';
import { unavailableReasonKey } from '../commands.ts';
import { useI18n, useShell } from '../context.ts';

export function StartPanel() {
  const { t } = useI18n();
  const shell = useShell();
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
        <p className="start-panel__hint">{t('start.hint')}</p>
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
            title={canNew ? t('qat.new') : refusal('NEW', 'qat.new')}
            onClick={() => shell.runCommand('NEW')}
          >
            {t('qat.new')}
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
