// Mount the error-report dialog in its own React root.
//
// The dialog must survive an `App` render crash, so it does not live inside the
// shell tree and does not read `ShellContext`/`I18nContext`. It gets a translator
// built straight from the shared catalogs and the initial locale instead.
import React from 'react';
import { createRoot } from 'react-dom/client';
import { CATALOGS } from '../catalogs.ts';
import { createTranslator, initialLocale } from '../i18n.ts';
import { ErrorReportHost } from '../components/ErrorReportHost.tsx';

export const ERROR_REPORT_ROOT_ID = 'error-report-root';

let mounted = false;

/**
 * Append the host container and render [`ErrorReportHost`] into it. Idempotent:
 * a second call is a no-op, so a hot reload cannot mount two roots on one node.
 */
export function mountErrorReportHost(): void {
  if (mounted || typeof document === 'undefined' || !document.body) return;
  mounted = true;
  let container = document.getElementById(ERROR_REPORT_ROOT_ID);
  if (!container) {
    container = document.createElement('div');
    container.id = ERROR_REPORT_ROOT_ID;
    document.body.appendChild(container);
  }
  const t = createTranslator(CATALOGS, initialLocale());
  createRoot(container).render(<ErrorReportHost t={t} />);
}
