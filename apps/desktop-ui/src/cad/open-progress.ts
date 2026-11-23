// The open pipeline (`openProgress`/`openSummary`, ADR 0030 D9).
//
// Kept out of `cad-status.ts` so that file stays under the size gate. It owns
// only the shared wire shape and the per-stage view; `cad-status.ts` re-exports
// the types for the rest of the chrome.
import type { Translator } from '../i18n.ts';

/** Live import progress; present only while an import is genuinely running. */
export interface CadImportProgress {
  /** Stable machine key of the latest phase, or `null` before the first tick. */
  readonly phase: string | null;
  /** Real entities normalised/inserted so far. */
  readonly entitiesDone: number;
  /** Total entities expected, or `null` when the format does not expose it. */
  readonly entitiesTotal: number | null;
  /** Source bytes measured so far, only when actually known. */
  readonly bytes: number | null;
  /** Whether a cancel request can still be issued for this import. */
  readonly cancellable: boolean;
}

/** One live open-pipeline stage (`import` → `vendor` → `representations`). */
export interface CadOpenProgress {
  /** `import` | `vendor` | `representations` (open-ended stable string). */
  readonly stage: string;
  /** Stage-local phase (`records`/`decode`/`apply`/`rebuild`/import phase). */
  readonly phase: string | null;
  readonly done: number;
  readonly total: number | null;
  /** `entities` | `vendorRecords`. */
  readonly unit: string;
  readonly cancellable: boolean;
}

/** The terminal summary of the last open (`openSummary`). */
export interface CadOpenSummary {
  readonly label: string;
  readonly entities: number;
  readonly vendor: {
    readonly status: string;
    readonly applied: number;
    readonly diagnostics: readonly string[];
  };
}

/** A localised view of one live open stage. */
export interface OpenProgressView {
  readonly phase: string;
  readonly count: string;
  readonly detail: string;
  readonly ratio: number | null;
  readonly determinate: boolean;
  readonly cancellable: boolean;
}

/** The chrome's progress-view shape (import and open stages share it). */
export type ImportProgressView = OpenProgressView;

function nonNegativeInteger(value: unknown): boolean {
  return typeof value === 'number' && Number.isInteger(value) && value >= 0;
}

export function parseOpenProgress(value: unknown): CadOpenProgress | null {
  if (value === null || value === undefined) return null;
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw Error('CAD_STATUS_INVALID');
  }
  const progress = value as Record<string, unknown>;
  const count = (field: string): boolean => nonNegativeInteger(progress[field]);
  if (typeof progress.stage !== 'string' || progress.stage.length === 0
    || !(progress.phase === null || typeof progress.phase === 'string')
    || !count('done')
    || !(progress.total === null || count('total'))
    || typeof progress.unit !== 'string' || progress.unit.length === 0
    || typeof progress.cancellable !== 'boolean') {
    throw Error('CAD_STATUS_INVALID');
  }
  return value as CadOpenProgress;
}

export function parseOpenSummary(value: unknown): CadOpenSummary | null {
  if (value === null || value === undefined) return null;
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw Error('CAD_STATUS_INVALID');
  }
  const summary = value as Record<string, unknown>;
  const vendor = summary.vendor;
  if (typeof summary.label !== 'string' || !nonNegativeInteger(summary.entities)
    || !vendor || typeof vendor !== 'object' || Array.isArray(vendor)) {
    throw Error('CAD_STATUS_INVALID');
  }
  const block = vendor as Record<string, unknown>;
  if (typeof block.status !== 'string' || !nonNegativeInteger(block.applied)
    || !Array.isArray(block.diagnostics)
    || !block.diagnostics.every(item => typeof item === 'string')) {
    throw Error('CAD_STATUS_INVALID');
  }
  return value as CadOpenSummary;
}

/**
 * The progress affordance for the open pipeline.
 *
 * A stage with a real `total` is determinate; one without is indeterminate. The
 * count unit is never mixed (`vendorRecords` is not called "entities"), and no
 * stage is extrapolated to 100% before it completes.
 */
export function openProgressView(progress: CadOpenProgress, t: Translator): OpenProgressView {
  const determinate = progress.total !== null;
  const ratio = determinate
    ? (progress.total! > 0 ? Math.min(1, progress.done / progress.total!) : 1)
    : null;
  const count = progress.unit === 'vendorRecords'
    ? (determinate
      ? t('open.progress.vendorRecords', { done: progress.done, total: progress.total })
      : t('open.progress.vendorRecordsIndeterminate', { done: progress.done }))
    : determinate
      ? t('import.progress.count', { done: progress.done, total: progress.total })
      : t('import.progress.indeterminate', { done: progress.done });
  return {
    phase: openStageLabel(t, progress),
    count,
    detail: count,
    ratio,
    determinate,
    cancellable: progress.cancellable,
  };
}

/** Stage label for the open pipeline; the phase is appended when known. */
function openStageLabel(t: Translator, progress: CadOpenProgress): string {
  const stage = t(`open.stage.${progress.stage}`);
  if (progress.phase === null || progress.phase.length === 0) return stage;
  return `${stage} · ${t(`open.phase.${progress.phase}`)}`;
}
