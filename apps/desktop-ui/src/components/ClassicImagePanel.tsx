// The classic raster-image panel (`CLASSICIMAGE`).
//
// Every row is one real `SemanticGeometry::Image` entity the drawing stores,
// projected by `imageRows`: entity id, space, logical file key, pixel size,
// stored visibility and the clip summary. Nothing is opened, attached, clipped
// or adjusted here; the panel is a read-only listing with focus and close.
//
// The panel is non-modal: no backdrop and no focus trap. It takes the keyboard
// on open, closes on Escape and restores the previous focus.
import React, { useRef, useState } from 'react';
import { Icon } from '../icons.tsx';
import { IconButton } from './primitives.tsx';
import { useI18n, useShell } from '../context.ts';
import { imageRows } from '../classic/classic-rows.ts';
import type { CadImageStatus } from '../cad/cad-status.ts';
import { useSurfacePanelFocus } from './surface-focus.ts';

interface ClassicImagePanelProps {
  readonly onClose: () => void;
}

function spaceLabel(t: (key: string, args?: Record<string, unknown>) => string, row: CadImageStatus): string {
  switch (row.space) {
    case 'paper': return t('classicimage.space.paper', { id: row.spaceId ?? '?' });
    case 'block': return t('classicimage.space.block', { id: row.spaceId ?? '?' });
    default: return t('classicimage.space.model');
  }
}

function ImageRow({
  row,
  focused,
  onFocus,
}: {
  readonly row: CadImageStatus;
  readonly focused: boolean;
  readonly onFocus: () => void;
}) {
  const { t } = useI18n();
  const rowRef = useRef<HTMLLIElement | null>(null);
  return (
    <li
      ref={rowRef}
      tabIndex={-1}
      className={['references__row', 'is-muted', focused ? 'is-focused' : ''].filter(Boolean).join(' ')}
      aria-current={focused ? 'true' : undefined}
    >
      <div className="references__head">
        <span className="references__name" title={row.id}>{t('classicimage.entry', { id: row.id })}</span>
        <span className="references__badge is-ok">{spaceLabel(t, row)}</span>
        <button
          type="button"
          className="quick-view__badge is-muted"
          onClick={() => { onFocus(); rowRef.current?.focus({ preventScroll: true }); }}
        >
          {t('classicimage.focus')}
        </button>
      </div>
      <div className="references__field">
        <span className="references__label">{t('classicimage.file')}</span>
        <code className="references__path" title={row.file ?? undefined}>
          {row.file ?? t('classicimage.fileNone')}
        </code>
      </div>
      <p className="references__meta">
        <span>{t('classicimage.pixels', { width: row.pixels[0], height: row.pixels[1] })}</span>
        <span>{row.visible ? t('classicimage.visible') : t('classicimage.hidden')}</span>
        <span>
          {row.clip === null
            ? t('classicimage.clipNone')
            : t('classicimage.clip', {
              count: row.clip.vertexCount,
              side: row.clip.inside ? t('classicimage.clipInside') : t('classicimage.clipOutside'),
            })}
        </span>
        <span>{focused ? t('classicimage.focused') : ''}</span>
      </p>
    </li>
  );
}

export function ClassicImagePanel({ onClose }: ClassicImagePanelProps) {
  const { t } = useI18n();
  const shell = useShell();
  const { panelRef } = useSurfacePanelFocus(onClose);
  const [focusedKey, setFocusedKey] = useState<string | null>(null);
  const rows = imageRows(shell.status);
  return (
    <aside
      ref={panelRef}
      tabIndex={-1}
      className="quick-view classic-image"
      role="dialog"
      aria-label={t('classicimage.title')}
    >
      <header className="quick-view__head">
        <span className="quick-view__title">
          <Icon name="reference" size={14} />
          {t('classicimage.title')}
        </span>
        <IconButton icon="close" size="xs" label={t('resources.close')} onClick={onClose} />
      </header>
      <p className="quick-view__subtitle">{t('classicimage.subtitle')}</p>
      {rows.length === 0 ? (
        <p className="quick-view__empty">{t('classicimage.empty')}</p>
      ) : (
        <div className="quick-view__grid">
          <ul className="references__list">
            {rows.map((row, index) => {
              const key = `${row.id}:${index}`;
              return (
                <ImageRow
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
    </aside>
  );
}
