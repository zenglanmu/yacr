// Properties palette: the inspector for the current selection.
//
// Docked (resizes the drawing area) rather than floating, because the host
// composites a native CAD surface in the central partition. It has exactly three
// honest states — empty, single object, multiple objects — and never invents a
// value: an unknown machine key is shown by its raw key, and an absent block
// reads as the explicit empty state.
//
// The property **rows** stay read-only (selection is a host operation, so there
// is no editing or "clear" affordance for them). The one editable control is the
// text-content editor of a single selected text entity, which the host exposes
// as `selection.text` — a round-trippable string that is deliberately not a
// row — and which edits through `cad_set_selected_text` (ADR 0020 addendum B).
import React from 'react';
import { Icon } from '../icons.jsx';
import { IconButton } from './primitives.jsx';
import { SelectionTextEditor } from './SelectionTextEditor.jsx';
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

        {/* The text-content editor is mounted in every selection state, so a
            draft survives a selection change instead of being unmounted with the
            rows. It renders nothing unless the host reports editable text or the
            user has a parked draft to show. */}
        <SelectionTextEditor />
      </div>
    </section>
  );
}
