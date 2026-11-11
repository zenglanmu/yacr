// The classic external-references panel (`CLASSICXREF`).
//
// Every row is the host's `cad_status.references` entry projected by
// `referenceRows`: name, stored/resolved path, closed status, stable reason code
// and the overlay/depth metadata. Nothing is resolved, retried or located here.
//
// The panel is non-modal: no backdrop and no focus trap. It takes the keyboard on
// open, focuses a row on demand, closes on Escape and restores the previous
// focus. `attach`, `detach` and `reload` are shown as **disabled** actions with
// their stable refusal codes: no host command exists for them, so the panel must
// not pretend one ran.
import React, { useRef, useState } from 'react';
import { Icon } from '../icons.tsx';
import { IconButton } from './primitives.tsx';
import { useI18n, useShell } from '../context.ts';
import { referenceRows } from '../references/references.ts';
import type { ReferenceRow } from '../references/references.ts';
import { useSurfacePanelFocus } from './surface-focus.ts';

interface ClassicXrefPanelProps {
  readonly onClose: () => void;
}

/** One reference row with a real focus action and no invented file action. */
function ClassicRow({
  row,
  focused,
  onFocus,
}: {
  readonly row: ReferenceRow;
  readonly focused: boolean;
  readonly onFocus: () => void;
}) {
  const { t } = useI18n();
  const rowRef = useRef<HTMLLIElement | null>(null);
  return (
    <li
      ref={rowRef}
      tabIndex={-1}
      className={[`references__row`, `is-${row.tone}`, focused ? 'is-focused' : ''].filter(Boolean).join(' ')}
      data-status={row.status}
      aria-current={focused ? 'true' : undefined}
    >
      <div className="references__head">
        <span className="references__name" title={row.name}>{row.name}</span>
        <span className={`references__badge is-${row.tone}`}>{t(row.statusKey)}</span>
        <button
          type="button"
          className="quick-view__badge is-muted"
          onClick={() => { onFocus(); rowRef.current?.focus({ preventScroll: true }); }}
        >
          {t('classicxref.focus')}
        </button>
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
      {row.hintKey === null ? null : <p className="references__hint">{t(row.hintKey)}</p>}
      <p className="references__meta">
        <span>{t(row.overlay ? 'references.attachment.overlay' : 'references.attachment.attached')}</span>
        <span>{t('references.depth', { depth: row.depth })}</span>
        <span>{focused ? t('classicxref.focused') : ''}</span>
      </p>
    </li>
  );
}

export function ClassicXrefPanel({ onClose }: ClassicXrefPanelProps) {
  const { t } = useI18n();
  const shell = useShell();
  const { panelRef } = useSurfacePanelFocus(onClose);
  const [focusedKey, setFocusedKey] = useState<string | null>(null);
  const rows = referenceRows(shell.status);
  return (
    <aside
      ref={panelRef}
      tabIndex={-1}
      className="quick-view classic-xref"
      role="dialog"
      aria-label={t('classicxref.title')}
    >
      <header className="quick-view__head">
        <span className="quick-view__title">
          <Icon name="reference" size={14} />
          {t('classicxref.title')}
        </span>
        <IconButton icon="close" size="xs" label={t('resources.close')} onClick={onClose} />
      </header>
      <p className="quick-view__subtitle">{t('classicxref.subtitle')}</p>
      {rows.length === 0 ? (
        <p className="quick-view__empty">{t('resources.references.none')}</p>
      ) : (
        <div className="quick-view__grid">
          <ul className="references__list">
            {rows.map((row, index) => {
              const key = `${row.name}:${row.path}:${index}`;
              return (
                <ClassicRow
                  key={key}
                  row={row}
                  focused={focusedKey === key}
                  onFocus={() => setFocusedKey(key)}
                />
              );
            })}
          </ul>
        </div>
      )}
      <footer className="quick-view__foot">
        <p className="references__hint">{t('classicxref.refusal.note')}</p>
        <div className="references__meta">
          <button type="button" disabled title={t('classicxref.refusal.attach')}>{t('classicxref.action.attach')}</button>
          <button type="button" disabled title={t('classicxref.refusal.detach')}>{t('classicxref.action.detach')}</button>
          <button type="button" disabled title={t('classicxref.refusal.reload')}>{t('classicxref.action.reload')}</button>
          <code className="references__reason">classicxref.attach_unsupported</code>
          <code className="references__reason">classicxref.detach_unsupported</code>
          <code className="references__reason">classicxref.reload_unsupported</code>
        </div>
      </footer>
    </aside>
  );
}
