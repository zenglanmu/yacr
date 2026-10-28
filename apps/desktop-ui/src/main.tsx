// Entry point: mount the desktop shell. Behaviour lives in `App.tsx`, visual
// regions under `components/`.
import React from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App.tsx';
import './style.css';
// Compact-layout additions. Loaded after `style.css` so its guarded overrides win
// on equal specificity; every shared-surface rule is scoped to
// `[data-layout="mobile"]`, so the desktop chrome is untouched.
import './mobile.css';
import { StackingRegression } from './stacking-regression/Probe.tsx';
import { isStackingRegression } from './stacking-regression/state.ts';

createRoot(document.getElementById('root') as HTMLElement).render(
  isStackingRegression(window) ? <StackingRegression /> : <App />,
);
