// The one renderer for the chrome-local surfaces.
//
// `App` mounts this after the shell regions; the surfaces store decides what is
// open. Keeping a single mount point means a typed word, a ribbon entry and a
// panel's own control all reach the same component (and the same focus rules)
// instead of each growing a second overlay.
import React from 'react';
import { useChromeSurfaceState } from '../chrome/use-chrome-store.ts';
import {
  closeQuickCalc,
  closeQuickView,
  closeWorkspaceSelector,
  closeWorkspaceSettings,
} from '../chrome/surfaces.ts';
import { QuickViewPanel } from './QuickViewPanel.tsx';
import { QuickCalcDialog } from './QuickCalcDialog.tsx';
import { WorkspaceSelector, WorkspaceSettingsDialog } from './WorkspaceSurfaces.tsx';

export function ChromeSurfaces() {
  const state = useChromeSurfaceState();
  return (
    <>
      {state.quickView !== null ? (
        <QuickViewPanel
          kind={state.quickView}
          revision={state.quickViewRevision}
          onClose={closeQuickView}
        />
      ) : null}
      {state.quickCalc ? <QuickCalcDialog onClose={closeQuickCalc} /> : null}
      {state.workspaceSelector ? <WorkspaceSelector onClose={closeWorkspaceSelector} /> : null}
      {state.workspaceSettings !== null ? (
        <WorkspaceSettingsDialog
          mode={state.workspaceSettings}
          onClose={closeWorkspaceSettings}
        />
      ) : null}
    </>
  );
}
