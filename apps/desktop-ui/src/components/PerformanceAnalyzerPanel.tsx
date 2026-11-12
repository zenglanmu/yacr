// The Performance Analyzer float (`PERFANALYZER`).
//
// Every reading is a real field of the host's `cad_status.performance` block plus
// the drawing/session counts the shell already holds. The projection lives in
// `performance/analyzer.ts`, which is where the "null reads N/A, zero is a
// reading" rule is enforced — this component only lays the rows out.
//
// There is deliberately no sampling loop and no refresh button: the panel shows
// the status the poll already delivers, so opening it never changes what the
// host measures. A carrier that publishes no performance block says so instead
// of painting zeros.
//
// The panel is non-modal: no backdrop and no focus trap. It takes the keyboard
// on open, closes on Escape and restores the previous focus.
import React, { Fragment } from 'react';
import { Icon } from '../icons.tsx';
import { IconButton } from './primitives.tsx';
import { useI18n, useShell } from '../context.ts';
import type { CadPerformance, CadStatus } from '../cad/cad-status.ts';
import { documentCountRows, performanceMetricRows } from '../performance/analyzer.ts';
import type { AnalyzerRow } from '../performance/analyzer.ts';
import { useSurfacePanelFocus } from './surface-focus.ts';

interface PerformanceAnalyzerPanelProps {
  readonly onClose: () => void;
}

/** One key/value section: a heading and the rows the projection returned. */
function Section({ title, rows }: { readonly title: string; readonly rows: readonly AnalyzerRow[] }) {
  const { t } = useI18n();
  return (
    <>
      <h2 className="quick-view__section">{title}</h2>
      <dl className="kv">
        {rows.map(row => (
          <Fragment key={row.id}>
            <dt>{t(row.labelKey)}</dt>
            <dd className={row.unavailable ? 'is-muted' : undefined}>{row.text}</dd>
          </Fragment>
        ))}
      </dl>
    </>
  );
}

export function PerformanceAnalyzerPanel({ onClose }: PerformanceAnalyzerPanelProps) {
  const { t } = useI18n();
  const shell = useShell();
  const { panelRef } = useSurfacePanelFocus(onClose);
  // The context's wire records are loosely typed (`ShellRecord`); this panel
  // narrows only the two blocks it reads, exactly like `ChromeSurfaces`.
  const status = (shell.status ?? null) as CadStatus | null;
  const performance = (status?.performance ?? null) as CadPerformance | null;
  const metrics = performanceMetricRows(performance, t);
  const counts = documentCountRows(status, t);
  return (
    <aside
      ref={panelRef}
      tabIndex={-1}
      className="quick-view performance-analyzer"
      role="dialog"
      aria-label={t('performance.title')}
    >
      <header className="quick-view__head">
        <span className="quick-view__title">
          <Icon name="diagnostics" size={14} />
          {t('performance.title')}
        </span>
        <IconButton icon="close" size="xs" label={t('performance.close')} onClick={onClose} />
      </header>
      <p className="quick-view__subtitle">{t('performance.subtitle')}</p>
      {status === null ? (
        <p className="quick-view__empty">{t('performance.empty')}</p>
      ) : (
        <div className="quick-view__body">
          <Section title={t('performance.section.performance')} rows={metrics} />
          {performance === null ? <p className="quick-view__note">{t('performance.unavailable')}</p> : null}
          <Section title={t('performance.section.counts')} rows={counts} />
        </div>
      )}
      <footer className="quick-view__foot">{t('performance.hint')}</footer>
    </aside>
  );
}
