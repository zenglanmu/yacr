// Diagnostics palette: the live performance read-out in a floating window
// (non-modal, so the drawing stays live underneath). The frame rate comes from
// the per-frame store; the process/GPU figures come from `cad_status.performance`.
// Every value is either a real host reading or an em dash — never `0`, never a
// number the host did not measure.
import React, { useSyncExternalStore } from 'react';
import { Icon } from '../icons.tsx';
import { Button, IconButton } from './primitives.tsx';
import { useI18n, useShell } from '../context.ts';
import { volatileStatusStore } from '../cad/status-cadence.ts';
import type { CadPerformance } from '../cad/cad-status.ts';
import { formatBytes, formatMemoryUsage, formatPercent } from '../performance/format.ts';

const DASH = '\u2014';

/** A usable ratio (`numerator / denominator`), or `null` when either side is not real. */
function ratio(
  numerator: number | null | undefined,
  denominator: number | null | undefined,
): number | null {
  if (typeof numerator !== 'number' || !Number.isFinite(numerator) || numerator < 0) return null;
  if (typeof denominator !== 'number' || !Number.isFinite(denominator) || denominator <= 0) return null;
  return numerator / denominator;
}

/** The system's memory in use: total minus what the host says is available. */
function memoryInUse(
  total: number | null | undefined,
  available: number | null | undefined,
): number | null {
  if (typeof total !== 'number' || typeof available !== 'number') return null;
  return total - available;
}

/** A thin usage meter under a value; the text beside it stays the source of truth. */
function Meter({ value }: { readonly value: number | null }) {
  if (value === null) return null;
  const percent = Math.min(100, Math.max(0, value * 100));
  return (
    <span
      className={['diagnostics__meter', percent >= 85 ? 'is-high' : ''].filter(Boolean).join(' ')}
      aria-hidden="true"
    >
      <span style={{ width: `${percent.toFixed(1)}%` }} />
    </span>
  );
}

function Row({
  label,
  children,
  meter = null,
}: {
  readonly label: React.ReactNode;
  readonly children: React.ReactNode;
  readonly meter?: number | null;
}) {
  return (
    <>
      <dt>{label}</dt>
      <dd>
        <span className="diagnostics__value">{children}</span>
        <Meter value={meter} />
      </dd>
    </>
  );
}

export function DiagnosticsPalette() {
  const { t } = useI18n();
  const shell = useShell();
  const status = shell.status;
  const performance = status?.performance as CadPerformance | null | undefined;
  // The frame rate is a per-frame field: reading it from the leaf store keeps
  // the 60 Hz sampler out of the shell's React state (see
  // `cad/status-cadence.ts`). The label still comes from the one catalog key.
  const fps = useSyncExternalStore(
    volatileStatusStore.subscribe,
    volatileStatusStore.getFrameRate,
    volatileStatusStore.getFrameRate,
  );
  const fpsText = fps === null || shell.displayedError
    ? t('render.fps.unavailable')
    : fps < 10 ? fps.toFixed(1) : String(Math.round(fps));
  const systemUsed = memoryInUse(performance?.systemTotalBytes, performance?.systemAvailableBytes);
  return (
    <section className="palette diagnostics" aria-label={t('diagnostics.title')}>
      <header className="palette__head">
        <h2><Icon name="diagnostics" size={15} />{t('diagnostics.title')}</h2>
        <IconButton icon="close" size="xs" label={t('diagnostics.close')} onClick={() => shell.setDiagnosticsOpen(false)} />
      </header>
      <div className="palette__body">
        {!status ? <p className="palette__empty">{t('diagnostics.empty')}</p> : null}
        <h3 className="palette__section">{t('diagnostics.section.performance')}</h3>
        <dl className="kv">
          <Row label={t('render.fps')}>{fpsText}</Row>
          <Row label={t('diagnostics.perf.backend')}>{status?.backend ?? DASH}</Row>
          <Row label={t('diagnostics.perf.device')}>{status?.adapter ?? DASH}</Row>
          <Row label={t('diagnostics.perf.cpu')} meter={ratio(performance?.cpuPercent, 100)}>
            {formatPercent(performance?.cpuPercent)}
          </Row>
          <Row label={t('diagnostics.perf.memory')}>{formatBytes(performance?.processRssBytes)}</Row>
          <Row
            label={t('diagnostics.perf.systemMemory')}
            meter={ratio(systemUsed, performance?.systemTotalBytes)}
          >
            {formatMemoryUsage(systemUsed, performance?.systemTotalBytes)}
          </Row>
          <Row label={t('diagnostics.perf.gpuUsed')}>
            {formatBytes(performance?.gpuMemoryUsedBytes)}
          </Row>
          <Row label={t('diagnostics.perf.gpuPeak')}>{formatBytes(performance?.gpuMemoryPeakBytes)}</Row>
          <Row label={t('diagnostics.perf.vramTotal')}>{formatBytes(performance?.vramTotalBytes)}</Row>
        </dl>
        {!performance ? <p className="diagnostics__note">{t('diagnostics.perf.unavailable')}</p> : null}
        {shell.displayedError ? <pre className="palette__error" role="alert">{shell.displayedError}</pre> : null}
        <Button
          variant="plain"
          size="xs"
          icon="close"
          className="diagnostics__close"
          onClick={() => shell.setDiagnosticsOpen(false)}
        >
          {t('diagnostics.close')}
        </Button>
      </div>
    </section>
  );
}
