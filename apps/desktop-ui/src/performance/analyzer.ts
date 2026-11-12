// The Performance Analyzer's rows: real `cad_status` readings, formatted once.
//
// `PERFANALYZER` opens a non-modal float that shows the host's own
// `cad_status.performance` block (CPU, process RSS, GPU memory, VRAM) and the
// drawing/session counts the shell already holds. Nothing is sampled here and
// nothing is invented: every row is a field the host sent, and a field that is
// `null` (the host cannot measure it on this carrier) renders the catalog's own
// N/A word — never `0`, which would read as a measurement.
//
// The projection is framework-free so the "null is N/A, zero is a reading" rule
// is testable without a DOM, exactly like `format.ts` beside it.
import type { CadPerformance, CadStatus } from '../cad/cad-status.ts';
import { blocksOf, layoutTabsOf, referencesOf } from '../cad/cad-status.ts';
import { selectionCount } from '../cad/selection-wire.ts';
import type { Translator } from '../i18n.ts';
import { formatBytes, formatPercent } from './format.ts';

/** The catalog key the panel renders for a value the host did not measure. */
export const PERFORMANCE_NA_KEY = 'performance.na';

/** One analyzer row: its stable id, label key and already-formatted reading. */
export interface AnalyzerRow {
  readonly id: string;
  readonly labelKey: string;
  /** The real reading, or the catalog's N/A word when the host sent `null`. */
  readonly text: string;
  /** Whether this value is the N/A word rather than a measurement. */
  readonly unavailable: boolean;
}

/** A real measurement: finite and non-negative, the wire's own metric rule. */
function isMeasurement(value: number | null | undefined): value is number {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0;
}

function measured(
  id: string,
  labelKey: string,
  value: number | null | undefined,
  format: (reading: number) => string,
  na: string,
): AnalyzerRow {
  if (!isMeasurement(value)) return { id, labelKey, text: na, unavailable: true };
  return { id, labelKey, text: format(value), unavailable: false };
}

function counted(
  id: string,
  labelKey: string,
  value: number | null | undefined,
  na: string,
): AnalyzerRow {
  if (!isMeasurement(value)) return { id, labelKey, text: na, unavailable: true };
  return { id, labelKey, text: String(value), unavailable: false };
}

/**
 * The performance rows, in the panel's order.
 *
 * The label keys are the Diagnostics palette's own where the reading is the same
 * (`diagnostics.perf.*`), so one measurement has one wording in the chrome. The
 * GPU figures are the **renderer's** accounting, not device-wide usage, and
 * `vramTotalBytes` is the advertised device-local VRAM proxy — the rows keep
 * those names instead of claiming more than the host measured.
 */
export function performanceMetricRows(
  performance: CadPerformance | null | undefined,
  t: Translator,
): readonly AnalyzerRow[] {
  const na = t(PERFORMANCE_NA_KEY);
  return [
    measured('cpu', 'diagnostics.perf.cpu', performance?.cpuPercent, formatPercent, na),
    measured('rss', 'diagnostics.perf.memory', performance?.processRssBytes, formatBytes, na),
    measured('gpu', 'diagnostics.perf.gpuUsed', performance?.gpuMemoryUsedBytes, formatBytes, na),
    measured('gpuPeak', 'diagnostics.perf.gpuPeak', performance?.gpuMemoryPeakBytes, formatBytes, na),
    measured('gpuReserved', 'performance.metric.gpuReserved', performance?.gpuMemoryReservedBytes, formatBytes, na),
    measured('vram', 'diagnostics.perf.vramTotal', performance?.vramTotalBytes, formatBytes, na),
  ];
}

/**
 * The drawing/session counts the shell already holds.
 *
 * `entityCount` is `null` on the start page (and while the host cannot count),
 * so it is N/A there rather than a fabricated `0`; the block/layout/reference
 * counts are real list lengths (a real empty drawing is an honest `0`). The
 * frame and import counters are the host's own session counters.
 */
export function documentCountRows(
  status: Partial<CadStatus> | null | undefined,
  t: Translator,
): readonly AnalyzerRow[] {
  const na = t(PERFORMANCE_NA_KEY);
  return [
    counted('entities', 'performance.count.entities', status?.entityCount, na),
    counted('selection', 'performance.count.selection', selectionCount(status?.selection), na),
    counted('blocks', 'performance.count.blocks', blocksOf(status).length, na),
    counted('layouts', 'performance.count.layouts', layoutTabsOf(status).length, na),
    counted('references', 'performance.count.references', referencesOf(status).length, na),
    counted('frames', 'performance.session.frames', status?.frameCount, na),
    counted('imports', 'performance.session.imports', status?.importCount, na),
  ];
}
