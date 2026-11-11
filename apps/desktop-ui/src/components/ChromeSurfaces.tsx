// The one renderer for the chrome-local surfaces.
//
// `App` mounts this after the shell regions; the surfaces store decides what is
// open. Keeping a single mount point means a typed word, a ribbon entry and a
// panel's own control all reach the same component (and the same focus rules)
// instead of each growing a second overlay.
import React from 'react';
import { useShell } from '../context.ts';
import type { CadStatus } from '../cad/cad-status.ts';
import { useChromeSurfaceState } from '../chrome/use-chrome-store.ts';
import { useInsertBlock } from '../classic/use-insert-block.ts';
import {
  closeClassicPanel,
  closeClassicXref,
  closeQuickCalc,
  closeQuickView,
  closeWorkspaceSelector,
  closeWorkspaceSettings,
} from '../chrome/surfaces.ts';
import { QuickViewPanel } from './QuickViewPanel.tsx';
import { ClassicXrefPanel } from './ClassicXrefPanel.tsx';
import { ClassicInsertPanel } from './ClassicInsertPanel.tsx';
import { ClassicGroupPanel } from './ClassicGroupPanel.tsx';
import { ClassicImagePanel } from './ClassicImagePanel.tsx';
import { QuickCalcDialog } from './QuickCalcDialog.tsx';
import { WorkspaceSelector, WorkspaceSettingsDialog } from './WorkspaceSurfaces.tsx';

export function ChromeSurfaces() {
  const state = useChromeSurfaceState();
  const shell = useShell();
  const insertBlock = useInsertBlock({
    submitCommand: shell.runCommand,
    // The context's wire records are intentionally loosely typed (`ShellRecord`);
    // the hook narrows only the two fields this flow reads.
    submitToolText: text => shell.submitToolText(text) as Promise<{ readonly kind: string }>,
    status: (shell.status ?? null) as CadStatus | null,
    reportError: shell.recordHistoryError,
  });
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
      {state.classicXref ? <ClassicXrefPanel onClose={closeClassicXref} /> : null}
      {state.classicPanel === 'insert' ? (
        <ClassicInsertPanel onClose={closeClassicPanel} onInsert={insertBlock} />
      ) : null}
      {state.classicPanel === 'group' ? (
        <ClassicGroupPanel onClose={closeClassicPanel} />
      ) : null}
      {state.classicPanel === 'image' ? (
        <ClassicImagePanel onClose={closeClassicPanel} />
      ) : null}
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
