// References panel: the external references the host reports in
// `cad_status.references`. It is a floating palette window on the desktop and a
// bottom sheet on the compact layout (same component, same host rows).
//
// Every row is the host's: name, stored path, resolved path (only when it really
// resolved), state, stable reason code and the overlay/depth metadata. There is
// no reload or locate action, because no host command exists for either; the
// only instruction a row adds is the carrier's own limitation (a browser cannot
// read local files) or the plain "not found" fact for a missing path.
import React from 'react';
import { Icon } from '../icons.tsx';
import { IconButton } from './primitives.tsx';
import { useI18n, useShell } from '../context.ts';
import { referenceRows } from '../references/references.ts';
import type { ReferenceRow } from '../references/references.ts';

/** The panel's `variant`: the desktop window, or the modal compact sheet. */
export type ReferencesPanelVariant = 'window' | 'floating';

export interface ReferencesPanelProps {
  readonly variant?: ReferencesPanelVariant;
}

function ReferenceCard({ row }: { readonly row: ReferenceRow }) {
  const { t } = useI18n();
  return (
    <li className={`references__row is-${row.tone}`} data-status={row.status}>
      <div className="references__head">
        <span className="references__name" title={row.name}>{row.name}</span>
        <span className={`references__badge is-${row.tone}`}>{t(row.statusKey)}</span>
      </div>
      <div className="references__field">
        <span className="references__label">{t('references.label.path')}</span>
        <code className="references__path" title={row.path}>{row.path}</code>
      </div>
      {row.resolvedPath === null ? null : (
        <div className="references__field">
          <span className="references__label">{t('references.label.resolved')}</span>
          <code className="references__path" title={row.resolvedPath}>{row.resolvedPath}</code>
        </div>
      )}
      {row.reasonCode === null ? null : (
        <div className="references__field">
          <span className="references__label">{t('references.label.reason')}</span>
          <code className="references__reason">{row.reasonCode}</code>
        </div>
      )}
      {row.hintKey === null ? null : (
        <p className="references__hint">{t(row.hintKey)}</p>
      )}
      <p className="references__meta">
        <span>{t(row.overlay ? 'references.attachment.overlay' : 'references.attachment.attached')}</span>
        <span>{t('references.depth', { depth: row.depth })}</span>
      </p>
    </li>
  );
}

export function ReferencesPanel({ variant = 'window' }: ReferencesPanelProps) {
  const { t } = useI18n();
  const shell = useShell();
  const rows = referenceRows(shell.status);
  const close = () => shell.setReferencesOpen(false);
  const floating = variant === 'floating';
  return (
    <section
      className={['palette', 'references', floating ? 'references--floating' : ''].filter(Boolean).join(' ')}
      aria-label={t('resources.references')}
      role={floating ? 'dialog' : undefined}
      aria-modal={floating ? true : undefined}
    >
      <header className="palette__head">
        <h2><Icon name="reference" size={15} />{t('resources.references')}</h2>
        <IconButton icon="close" size="xs" label={t('resources.close')} onClick={close} />
      </header>
      <div className="palette__body references__body">
        {rows.length === 0 ? <p className="palette__empty">{t('resources.references.none')}</p> : (
          <ul className="references__list">
            {rows.map((row, index) => (
              <ReferenceCard key={`${row.name}:${row.path}:${index}`} row={row} />
            ))}
          </ul>
        )}
      </div>
    </section>
  );
}
