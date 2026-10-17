// Workspace: the central CAD partition plus the DOM chrome docked at its edges.
//
// The native CAD surface is composited by the host at exactly the rect this
// element reports, so `.native-input` holds **no** DOM children and nothing
// floats across it. Palettes dock to the left (shrinking the partition) and the
// navigation rail docks to the right.
import React from 'react';
import { NavRail } from './NavRail.jsx';
import { StartPanel } from './StartPanel.jsx';
import { PropertiesPalette } from './PropertiesPalette.jsx';
import { DiagnosticsPalette } from './DiagnosticsPalette.jsx';
import { useI18n, useShell } from '../context.mjs';

export function Workspace() {
  const { t } = useI18n();
  const shell = useShell();
  const showStart = !shell.documentOpen;
  // The inspector is only meaningful with a drawing open; on the start screen the
  // start panel keeps the whole dock. Off by default so the startup partition
  // geometry is unchanged until the user opens it.
  const showProperties = shell.propertiesOpen && !showStart;
  const leftOpen = showStart || showProperties || shell.diagnosticsOpen;
  return (
    <div className="region region--workspace workspace app-region">
      <div className={['workspace__left', leftOpen ? 'is-open' : ''].filter(Boolean).join(' ')}>
        {showStart ? <StartPanel /> : null}
        {showProperties ? <PropertiesPalette /> : null}
        {shell.diagnosticsOpen ? <DiagnosticsPalette /> : null}
      </div>
      <div className="workspace__canvas">
        {/* The native CAD partition. Keep this node childless. */}
        <div ref={shell.surfaceRef} className="native-input" aria-label={t('a11y.canvas_label')} />
      </div>
      <NavRail />
    </div>
  );
}
