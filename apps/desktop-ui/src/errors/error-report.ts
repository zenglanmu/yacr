// The global error report store.
//
// One bounded history feeds the error dialog and the status bar's "open details"
// trigger. It has no React and no DOM: the host component subscribes through
// `useSyncExternalStore`, and the capture module feeds it from window events /
// the wasm panic hook. Keeping the store pure is what lets the node suite pin
// dedupe, the bounded history and the clipboard text without a browser.
//
// Newest is last. The visible report is an index into that array, so navigating
// older/later never copies or rewrites a report, and dismissing one removes only
// the report the reader was looking at.
import type { Translator } from '../i18n.ts';

export type ErrorKind = 'uncaught' | 'rejection' | 'render' | 'panic' | 'host';

export interface ErrorReport {
  readonly id: number;
  readonly kind: ErrorKind;
  readonly message: string;
  readonly location?: string;
  readonly stack?: string;
  /**
   * Extra raw detail. For `kind: 'host'` it carries the host's original
   * diagnostic string when the displayed message is a shortened label.
   */
  readonly detail?: string;
  /** ISO timestamp of capture. */
  readonly at: string;
}

/** What a caller hands to [`reportError`]; the store owns the id and time. */
export interface ErrorReportInput {
  readonly kind: ErrorKind;
  readonly message: string;
  readonly location?: string;
  readonly stack?: string;
  readonly detail?: string;
}

/** The stable snapshot `useSyncExternalStore` reads. */
export interface ErrorReportState {
  readonly reports: readonly ErrorReport[];
  /** Index of the report the dialog shows, or -1 when the history is empty. */
  readonly visibleIndex: number;
  readonly visible: ErrorReport | null;
}

/** The history cap: old reports fall off the front once this is exceeded. */
export const ERROR_REPORT_LIMIT = 20;

/**
 * Catalog keys by kind. Explicit literals (not `` `error.kind.${kind}` ``) keep
 * the key set greppable by `scripts/check-i18n.py` and by review.
 */
const KIND_LABEL_KEYS: Readonly<Record<ErrorKind, string>> = Object.freeze({
  uncaught: 'error.kind.uncaught',
  rejection: 'error.kind.rejection',
  render: 'error.kind.render',
  panic: 'error.kind.panic',
  host: 'error.kind.host',
});

/** The catalog key naming one error kind. */
export function errorKindLabelKey(kind: ErrorKind): string {
  return KIND_LABEL_KEYS[kind] ?? KIND_LABEL_KEYS.host;
}

const EMPTY_REPORTS: readonly ErrorReport[] = Object.freeze([]);
const EMPTY_STATE: ErrorReportState = Object.freeze({
  reports: EMPTY_REPORTS,
  visibleIndex: -1,
  visible: null,
});

let nextId = 1;
let state: ErrorReportState = EMPTY_STATE;
const listeners = new Set<() => void>();

function notify(): void {
  for (const listener of [...listeners]) {
    try {
      listener();
    } catch {
      // One broken subscriber must never stop the others (or the capture path).
    }
  }
}

function commit(reports: readonly ErrorReport[], visibleIndex: number): void {
  const frozen = Object.freeze(reports.slice()) as readonly ErrorReport[];
  state = Object.freeze({
    reports: frozen,
    visibleIndex,
    visible: visibleIndex >= 0 ? frozen[visibleIndex] ?? null : null,
  });
  notify();
}

function optionalText(value: string | undefined): string | undefined {
  return typeof value === 'string' && value.trim() !== '' ? value : undefined;
}

/** Whether `input` repeats the last stored report (the dedupe rule). */
function repeatsLast(input: ErrorReportInput, message: string, location: string | undefined, stack: string | undefined): ErrorReport | null {
  const last = state.reports[state.reports.length - 1];
  if (!last) return null;
  if (last.kind !== input.kind || last.message !== message) return null;
  if (last.location !== location || last.stack !== stack) return null;
  return last;
}

/**
 * Add a report. An identical consecutive report (same kind, message, location
 * and stack) is not stacked again: the existing entry is returned and no
 * listener runs, so a repeating window event cannot flood the dialog.
 *
 * A fresh report becomes the visible one (newest is the default view). The
 * history is capped at [`ERROR_REPORT_LIMIT`], dropping the oldest entries.
 */
export function reportError(input: ErrorReportInput): ErrorReport {
  const message = typeof input.message === 'string' ? input.message : String(input.message ?? '');
  const location = optionalText(input.location);
  const stack = optionalText(input.stack);
  const duplicate = repeatsLast(input, message, location, stack);
  if (duplicate) return duplicate;

  let detail = optionalText(input.detail);
  // A host sentence is the raw diagnostic; keep it available even when the bar
  // shows a shorter translated label later.
  if (input.kind === 'host' && detail === undefined) detail = message;

  const report: ErrorReport = Object.freeze({
    id: nextId,
    kind: input.kind,
    message,
    location,
    stack,
    detail,
    at: new Date().toISOString(),
  });
  nextId += 1;

  const reports = [...state.reports, report];
  if (reports.length > ERROR_REPORT_LIMIT) {
    reports.splice(0, reports.length - ERROR_REPORT_LIMIT);
  }
  commit(reports, reports.length - 1);
  return report;
}

/** Every stored report, oldest first. */
export function getReports(): readonly ErrorReport[] {
  return state.reports;
}

/** The report the dialog shows right now, or `null`. */
export function getVisibleReport(): ErrorReport | null {
  return state.visible;
}

/**
 * Whether the global error dialog is on screen.
 *
 * The dialog lives in its own React root, outside the shell tree, so the shell's
 * key handler cannot see it through `popupOpen`. It reads this live store value
 * at event time instead: while the dialog is up it owns Escape and every other
 * key, so a shortcut can never act on a drawing the user cannot see.
 */
export function isErrorReportVisible(): boolean {
  return state.visible !== null;
}

/**
 * The whole snapshot in one stable object. `useSyncExternalStore` needs a value
 * that only changes when the store does, which is what [`commit`] guarantees.
 */
export function getErrorReportState(): ErrorReportState {
  return state;
}

/** Subscribe to any store change (report, dismiss, navigation). */
export function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/**
 * Dismiss the visible report. It is removed from the history, not merely hidden:
 * the same failure arriving again must be able to open the dialog. The next
 * newer report (or, at the newest end, the previous one) becomes visible.
 */
export function dismissErrorReport(): void {
  if (state.visibleIndex < 0) return;
  const reports = state.reports.slice();
  const index = state.visibleIndex;
  reports.splice(index, 1);
  if (reports.length === 0) {
    commit(EMPTY_REPORTS, -1);
    return;
  }
  commit(reports, Math.min(index, reports.length - 1));
}

/** Show the next older report, if any. */
export function showOlderErrorReport(): void {
  if (state.visibleIndex <= 0) return;
  commit(state.reports, state.visibleIndex - 1);
}

/** Show the next newer report, if any. */
export function showNewerErrorReport(): void {
  if (state.visibleIndex < 0 || state.visibleIndex >= state.reports.length - 1) return;
  commit(state.reports, state.visibleIndex + 1);
}

/** Clear the store between tests (also resets the id sequence). */
export function resetErrorReportsForTests(): void {
  nextId = 1;
  state = EMPTY_STATE;
  notify();
}

/**
 * The plain-text block copied by the dialog. Pure: `t` is passed in, so the
 * node suite can pin every label without a catalog or a DOM.
 *
 * `detail` is only printed when it differs from the message: a host report whose
 * raw string is also the displayed sentence must not be copied twice.
 */
export function formatErrorReport(report: ErrorReport, t: Translator): string {
  const lines: string[] = [
    t(errorKindLabelKey(report.kind)),
    `${t('error.label.time')}: ${report.at}`,
    `${t('error.label.message')}: ${report.message}`,
  ];
  if (report.location) lines.push(`${t('error.label.location')}: ${report.location}`);
  if (report.stack) {
    lines.push(`${t('error.label.stack')}:`);
    lines.push(report.stack);
  }
  const detail = report.detail && report.detail !== report.message ? report.detail : undefined;
  if (detail) {
    if (!report.stack) lines.push(`${t('error.label.stack')}:`);
    lines.push(detail);
  }
  return lines.join('\n');
}
