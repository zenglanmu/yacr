// Properties palette: the read-only inspector for the current selection.
//
// Docked (resizes the drawing area) rather than floating, because the host
// composites a native CAD surface in the central partition. It has exactly three
// honest states — empty, single object, multiple objects — and never invents a
// value: an unknown machine key is shown by its raw key, and an absent block
// reads as the explicit empty state. Selection is a read-only host operation, so
// there is no editing or "clear" affordance here.
import React from 'react';
import { Icon } from '../icons.jsx';
import { IconButton } from './primitives.jsx';
import { useI18n, useShell } from '../context.mjs';
import { selectionView } from '../cad.mjs';

export function PropertiesPalette() {
  const { t } = useI18n();
  const shell = useShell();
  const view = selectionView(shell.status?.selection ?? null, t);
  return (
    <section className="palette properties" aria-label={t('properties.panel')}>
      <header className="palette__head">
        <h2><Icon name="properties" size={15} />{t('properties.panel')}</h2>
        <IconButton
          icon="close"
          size="xs"
          label={t('properties.close')}
          onClick={() => shell.setPropertiesOpen(false)}
        />
      </header>
      <div className="palette__body">
        {view.kind === 'empty' ? <p className="palette__empty">{t('properties.empty')}</p> : null}

        {view.kind === 'single' ? (
          <dl className="kv properties__rows">
            {view.rows.map(row => (
              <React.Fragment key={row.key}>
                <dt title={row.key}>{row.label}</dt>
                <dd>{row.value}</dd>
              </React.Fragment>
            ))}
          </dl>
        ) : null}

        {view.kind === 'multi' ? (
          <div className="properties__multi">
            <p className="properties__count">{view.countLabel}</p>
            {view.mixedLabel ? <p className="properties__mixed">{view.mixedLabel}</p> : null}
          </div>
        ) : null}
      </div>
    </section>
  );
}
