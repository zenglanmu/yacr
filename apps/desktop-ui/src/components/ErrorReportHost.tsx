// The always-mounted listener for the global error store.
//
// Rendered from its own React root (`errors/mount-error-report-host.tsx`) so it
// survives an `App` render failure. It takes a translator as a prop instead of
// reading `I18nContext`, precisely because that context may not exist when the
// shell crashed.
import React, { useSyncExternalStore } from 'react';
import { getErrorReportState, subscribe } from '../errors/error-report.ts';
import type { Translator } from '../i18n.ts';
import { ErrorReportDialog } from './ErrorReportDialog.tsx';

export interface ErrorReportHostProps {
  readonly t: Translator;
}

export function ErrorReportHost({ t }: ErrorReportHostProps) {
  const state = useSyncExternalStore(subscribe, getErrorReportState, getErrorReportState);
  const report = state.visible;
  if (!report) return null;
  return (
    <ErrorReportDialog
      report={report}
      index={state.visibleIndex + 1}
      total={state.reports.length}
      t={t}
    />
  );
}
