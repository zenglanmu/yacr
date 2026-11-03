// Font Manager: a read-only report of the host's font registry
// (`cad_fonts`). Docked on the desktop and a floating sheet on the compact
// layout (same component, same rows).
//
// The host owns the registry. This panel never registers, removes or reloads a
// face: there is no command for those, so there is no control for them either.
// Five states are deliberately distinct:
//
//   * loading/first read — the host has not answered yet;
//   * `cad_waiting` — the host has not run its first owner tick (still loading);
//   * unavailable — `{available:false}`: the **named** reason is shown, never the
//     empty state, because "unavailable" is not "there are no fonts";
//   * empty — the host really answered with no faces;
//   * rows — grouped loaded / failed / unresolved, exactly as the host sorted them.
import React, { useEffect, useRef, useSyncExternalStore } from 'react';
import { invoke } from '../host/invoke.ts';
import { Icon } from '../icons.tsx';
import { IconButton } from './primitives.tsx';
import { useI18n, useShell } from '../context.ts';
import { createFontsStore, fontSummary, projectFontRows } from '../fonts/fonts.ts';
import type { FontRow, FontsStore } from '../fonts/fonts.ts';

/** The panel's `variant`: docked in the left column, or the compact sheet. */
export type FontsPanelVariant = 'docked' | 'floating';

export interface FontsPanelProps {
  readonly variant?: FontsPanelVariant;
}

/** One row: identity, status badge, kind/source chips, file and host detail. */
function FontCard({ row }: { readonly row: FontRow }) {
  const { t } = useI18n();
  return (
    <li className={`references__row is-${row.tone}`} data-status={row.status}>
      <div className="references__head">
        <span className="references__name" title={row.key}>{row.key}</span>
        <span className={`references__badge is-${row.tone}`}>{t(row.statusKey)}</span>
      </div>
      <div className="fonts__tags">
        <code className="fonts__kind">{row.kind}</code>
        <span className="fonts__source">{t(row.sourceKey)}</span>
      </div>
      {row.file === null ? null : (
        <div className="references__field">
          <code className="references__path" title={row.file}>{row.file}</code>
        </div>
      )}
      {row.detail === null ? null : (
        <p className="references__hint" title={row.detail}>{row.detail}</p>
      )}
    </li>
  );
}

function FontSection({ title, rows }: { readonly title: string; readonly rows: readonly FontRow[] }) {
  if (rows.length === 0) return null;
  return (
    <section className="fonts__section" aria-label={title}>
      <h3 className="palette__section">{title}</h3>
      <ul className="references__list">
        {rows.map(row => <FontCard key={row.key} row={row} />)}
      </ul>
    </section>
  );
}

export function FontsPanel({ variant = 'docked' }: FontsPanelProps) {
  const { t } = useI18n();
  const shell = useShell();
  const storeRef = useRef<FontsStore | null>(null);
  if (!storeRef.current) storeRef.current = createFontsStore(invoke);
  const store = storeRef.current;
  const state = useSyncExternalStore(store.subscribe, store.getState);
  const answer = state.answer;

  // One read per mount: opening the panel *is* the refresh. There is no reload
  // control, because the panel has nothing more to ask than the host's report.
  useEffect(() => { void store.load(); }, [store]);

  const groups = answer === null ? null : projectFontRows(answer.faces);
  const summary = fontSummary(groups);
  const firstRead = state.phase === 'idle' || state.phase === 'loading';
  const close = () => shell.setFontsOpen(false);
  const floating = variant === 'floating';

  return (
    <section
      className={['palette', 'fonts', floating ? 'fonts--floating' : ''].filter(Boolean).join(' ')}
      aria-label={t('fonts.title')}
      role={floating ? 'dialog' : undefined}
      aria-modal={floating ? true : undefined}
    >
      <header className="palette__head">
        <h2><Icon name="font" size={15} />{t('fonts.title')}</h2>
        <IconButton icon="close" size="xs" label={t('fonts.close')} onClick={close} />
      </header>
      <div className="palette__body fonts__body" aria-busy={firstRead || state.phase === 'waiting' ? true : undefined}>
        {firstRead || state.phase === 'waiting' ? (
          <p className="palette__empty">{t('fonts.loading')}</p>
        ) : null}

        {state.phase === 'error' ? (
          <p className="palette__error" role="alert">{t('fonts.read_failed', { reason: state.error?.reason ?? '' })}</p>
        ) : null}

        {/* An unavailable report owns its named reason. It must never fall through
            to the empty state: no registry read is not the same as no fonts. */}
        {state.phase === 'unavailable' && answer !== null ? (
          <p className="palette__empty fonts__unavailable" role="status">
            {t('fonts.unavailable', { reason: t(answer.reason ?? '') })}
          </p>
        ) : null}

        {state.phase === 'empty' && answer !== null ? (
          <p className="palette__empty">{t('fonts.empty')}</p>
        ) : null}

        {answer !== null && state.phase !== 'unavailable' ? (
          <div className="fonts__summary">
            <p className="fonts__stats">
              <span className="fonts__locale" aria-label={answer.locale}>{answer.locale}</span>
              <span>{t('fonts.catalog_count', { count: answer.catalogEntries })}</span>
              {answer.defaultFace === null ? null : (
                <span>{t('fonts.default_face')}: <code>{answer.defaultFace}</code></span>
              )}
            </p>
            <p className="fonts__counts">
              <span className="is-ok">{t('fonts.section.loaded', { count: summary.loaded })}</span>
              <span className="is-error">{t('fonts.section.failed', { count: summary.failed })}</span>
              <span className="is-muted">{t('fonts.section.unresolved', { count: summary.unresolved })}</span>
            </p>
            {answer.fontsDir === null ? null : (
              <p className="fonts__dir" title={t('options.help.fontsDir')}>
                <span>{t('options.field.fontsDir')}</span>
                <code>{answer.fontsDir}</code>
              </p>
            )}
            {answer.localeMissing === null ? null : (
              <p className="fonts__warning" role="status">
                <Icon name="alert" size={13} />
                <span>{t('fonts.locale_missing', { detail: answer.localeMissing })}</span>
              </p>
            )}
          </div>
        ) : null}

        {state.phase === 'ready' && groups !== null ? (
          <>
            <FontSection title={t('fonts.section.loaded', { count: summary.loaded })} rows={groups.loaded} />
            <FontSection title={t('fonts.section.failed', { count: summary.failed })} rows={groups.failed} />
            <FontSection title={t('fonts.section.unresolved', { count: summary.unresolved })} rows={groups.unresolved} />
          </>
        ) : null}
      </div>
    </section>
  );
}
