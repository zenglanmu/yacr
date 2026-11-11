// Entry point: mount the desktop shell. Behaviour lives in `App.tsx`, visual
// regions under `components/`.
import React from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App.tsx';
import './style.css';
// Compact-layout additions. Loaded after `style.css` so its guarded overrides win
// on equal specificity; every shared-surface rule is scoped to
// `:root[data-layout="mobile"]`, so the desktop chrome is untouched.
import './mobile.css';
import { StackingRegression } from './stacking-regression/Probe.tsx';
import { isStackingRegression } from './stacking-regression/state.ts';
import { installGlobalErrorCapture } from './errors/install-global-capture.ts';
import { mountErrorReportHost } from './errors/mount-error-report-host.tsx';
import { reportRenderError } from './errors/root-error-reporting.ts';
import { applyTheme, initialTheme } from './theme/theme.ts';

// Capture before anything renders, so an import-time failure (including the wasm
// session's panic log) is already visible. The error dialog gets its own root:
// it must outlive an App render crash, and it must not read the contexts that
// crash would have taken down with it.
installGlobalErrorCapture();
mountErrorReportHost();

// First paint: the cached, host-confirmed theme is already on `<html>` before the
// first React render, so a light-theme restart never flashes dark chrome. The
// host's first `cad_status` remains the authority and replaces this value.
applyTheme(initialTheme());

createRoot(document.getElementById('root') as HTMLElement, {
  // An uncaught render error takes the tree down, so it opens the global dialog.
  onUncaughtError: (error, info) => reportRenderError(error, info),
  // A caught error already has its local fallback (the popup body boundary shows
  // `dialog.popup.body_failed`). Reporting it globally too would stack a modal on
  // top of the dialog that just explained itself; keep the console line only.
  onCaughtError: (error, info) => console.error(error, info?.componentStack ?? ''),
}).render(
  isStackingRegression(window) ? <StackingRegression /> : <App />,
);
