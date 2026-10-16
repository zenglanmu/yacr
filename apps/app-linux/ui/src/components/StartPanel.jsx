// Start state: shown in a docked palette when no drawing is open. It offers the
// one real action (Open) and is explicit about what this build does not do.
import React from 'react';
import { Icon } from '../icons.jsx';
import { Button } from './primitives.jsx';
import { useI18n, useShell } from '../context.mjs';

export function StartPanel() {
  const { t } = useI18n();
  const shell = useShell();
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
            disabled={!shell.canCommand('OPEN')}
            title={shell.canCommand('OPEN') ? t('file.open') : t('command.unsupported', { command: t('file.open') })}
            onClick={() => shell.runCommand('OPEN')}
          >
            {t('file.open')}
          </Button>
          <Button
            variant="ghost"
            icon="new"
            iconSize={18}
            size="lg"
            disabled
            title={t('start.new_unavailable')}
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
