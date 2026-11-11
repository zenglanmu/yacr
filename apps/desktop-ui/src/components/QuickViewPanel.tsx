// Quick View, as one floating panel for both modes.
//
// Drawings mode lists the single open drawing as a card with a **vector**
// preview (a drawn wireframe, not a stored thumbnail: this build keeps no
// bitmap). Layouts mode lists every layout the host reports, using the same
// `SPACE #<id>` / `MODEL` tokens the strip dispatches.
//
// The panel is non-modal: no backdrop, no focus trap. It takes the keyboard on
// open (its first card), closes on Escape and restores focus; choosing the
// current drawing hands the keyboard to the CAD partition instead.
//
// `UPDATETHUMBSNOW` bumps the store's revision. The panel re-renders its cards
// and shows a transient "updated" line; nothing is written to a file.
import React, { useEffect, useState } from 'react';
import { Icon } from '../icons.tsx';
import { IconButton } from './primitives.tsx';
import { useI18n, useShell } from '../context.ts';
import { quickViewDrawings, quickViewLayouts } from '../chrome/quick-view.ts';
import { refreshQuickView } from '../chrome/surfaces.ts';
import type { QuickViewKind } from '../chrome/surfaces.ts';
import { useSurfacePanelFocus } from './surface-focus.ts';

/** How long the "updated" line stays after a refresh (browser only). */
const UPDATED_MS = 1800;

const DRAWING_PREVIEW = (
  <svg viewBox="0 0 120 72" role="presentation" focusable="false">
    <rect x="10" y="9" width="100" height="54" rx="2" opacity=".22" />
    <path d="M10 63 42 31l16 16 12-14 50 30" opacity=".55" />
    <circle cx="82" cy="26" r="9" opacity=".65" />
    <path d="M24 18h18M24 24h10" opacity=".45" />
  </svg>
);

const LAYOUT_PREVIEW = (
  <svg viewBox="0 0 120 72" role="presentation" focusable="false">
    <rect x="10" y="9" width="100" height="54" rx="2" opacity=".22" />
    <rect x="18" y="17" width="52" height="38" rx="1" opacity=".45" />
    <rect x="76" y="17" width="26" height="17" rx="1" opacity=".45" />
    <rect x="76" y="38" width="26" height="17" rx="1" opacity=".45" />
  </svg>
);

interface QuickViewPanelProps {
  readonly kind: QuickViewKind;
  /** The store's refresh counter (`UPDATETHUMBSNOW`). */
  readonly revision: number;
  readonly onClose: () => void;
}

export function QuickViewPanel({ kind, revision, onClose }: QuickViewPanelProps) {
  const { t } = useI18n();
  const shell = useShell();
  const { panelRef, closeToSurface } = useSurfacePanelFocus(onClose);
  const [updated, setUpdated] = useState(false);

  // A refresh happened while the panel was open: show the transient line. The
  // first mount does not claim a refresh (revision starts at 0).
  useEffect(() => {
    if (revision === 0) return undefined;
    setUpdated(true);
    const timer = window.setTimeout(() => setUpdated(false), UPDATED_MS);
    return () => window.clearTimeout(timer);
  }, [revision]);

  const drawings = kind === 'drawings' ? quickViewDrawings(shell.documentLabel, shell.documentVisible) : [];
  const layouts = kind === 'layouts' ? quickViewLayouts(shell.status) : [];
  const title = kind === 'drawings' ? t('quickview.title.drawings') : t('quickview.title.layouts');
  const subtitle = kind === 'drawings' ? t('quickview.subtitle.drawings') : t('quickview.subtitle.layouts');
  const empty = (kind === 'drawings' && drawings.length === 0) || (kind === 'layouts' && layouts.length === 0);

  return (
    <aside
      ref={panelRef}
      tabIndex={-1}
      className={['quick-view', updated ? 'is-updated' : ''].filter(Boolean).join(' ')}
      role="dialog"
      aria-label={title}
    >
      <header className="quick-view__head">
        <span className="quick-view__title">
          <Icon name={kind === 'drawings' ? 'drawing' : 'layers'} size={14} />
          {title}
        </span>
        {updated ? <span className="quick-view__updated" role="status">{t('quickview.updated')}</span> : null}
        <IconButton
          icon="refresh"
          size="xs"
          label={t('quickview.refresh')}
          title={t('quickview.refresh')}
          onClick={refreshQuickView}
        />
        <IconButton icon="close" size="xs" label={t('quickview.close')} onClick={onClose} />
      </header>
      <p className="quick-view__subtitle">{subtitle}</p>

      {empty ? (
        <p className="quick-view__empty">
          {kind === 'drawings' ? t('quickview.empty.drawings') : t('quickview.empty.layouts')}
        </p>
      ) : (
        <div className="quick-view__grid" role="list">
          {drawings.map(card => (
            <button
              key={card.id}
              type="button"
              role="listitem"
              className="quick-view__card is-active"
              onClick={closeToSurface}
            >
              <span className="quick-view__preview" aria-hidden="true">{DRAWING_PREVIEW}</span>
              <span className="quick-view__name">{card.label ?? t('quickview.unnamed')}</span>
              <span className="quick-view__badge">{t('quickview.current')}</span>
            </button>
          ))}
          {layouts.map(card => {
            const label = card.nameKey !== null
              ? t(card.nameKey)
              : (card.name?.trim() ?? '') || t('layout.tab.unnamed');
            const meta = card.active
              ? t('quickview.current')
              : card.viewportCount !== null
                ? t('quickview.layout.viewports', { count: card.viewportCount })
                : null;
            return (
              <button
                key={card.key}
                type="button"
                role="listitem"
                className={['quick-view__card', card.active ? 'is-active' : '', card.supported ? '' : 'is-unsupported'].filter(Boolean).join(' ')}
                disabled={!card.supported}
                title={card.supported ? label : t('quickview.layout.unsupported')}
                onClick={() => {
                  if (card.command !== null) shell.runCommand(card.command);
                  onClose();
                }}
              >
                <span className="quick-view__preview is-layout" aria-hidden="true">{LAYOUT_PREVIEW}</span>
                <span className="quick-view__name">{label}</span>
                {card.supported ? (
                  <span className="quick-view__meta">{meta}</span>
                ) : (
                  <span className="quick-view__badge is-muted">{t('quickview.layout.unsupported')}</span>
                )}
              </button>
            );
          })}
        </div>
      )}

      <footer className="quick-view__foot">{t('quickview.hint')}</footer>
    </aside>
  );
}
