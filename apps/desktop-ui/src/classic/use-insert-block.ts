// Arm the real point-then-name INSERT flow with the block chosen in the classic
// panel.
//
// The router takes a command line only, so the chosen name cannot travel as an
// INSERT argument. It waits for the host's own `draw.insertblock` +
// `awaiting_text` state and then uses the same `cad_set_tool_text` channel the
// command line uses. The pending row is dropped when the flow is cancelled or
// another tool owns the pointer.
import { useCallback, useEffect, useRef, useState } from 'react';
import { closeClassicPanel, closeToolPalettes } from '../chrome/surfaces.ts';
import type { CadStatus } from '../cad/cad-status.ts';

interface ToolTextResult {
  readonly kind: string;
  readonly error?: unknown;
}

export interface InsertBlockHost {
  readonly submitCommand: (command: string) => void;
  readonly submitToolText: (text: string) => Promise<ToolTextResult>;
  readonly status: CadStatus | null;
  readonly reportError: (message: string) => void;
}

/** Returns the panel's Insert action: close the panel and arm INSERT. */
export function useInsertBlock({
  submitCommand,
  submitToolText,
  status,
  reportError,
}: InsertBlockHost): (name: string) => void {
  const pending = useRef<{ name: string; armed: boolean; ticks: number } | null>(null);
  const [revision, setRevision] = useState(0);

  const insertBlockByName = useCallback((name: string) => {
    // Both block surfaces arm this one flow; only one of them can be open (the
    // surfaces store closes the others on open), so closing both is a no-op for
    // whichever panel is not there.
    closeClassicPanel();
    closeToolPalettes();
    pending.current = { name, armed: false, ticks: 0 };
    setRevision(tick => tick + 1);
    submitCommand('INSERT');
  }, [submitCommand]);

  useEffect(() => {
    const current = pending.current;
    if (!current) return;
    const tool = status?.tool;
    if (tool?.key === 'draw.insertblock') {
      current.armed = true;
      if (tool.awaiting_text !== true) return;
      pending.current = null;
      void submitToolText(current.name).then(result => {
        if ('error' in result) reportError(String(result.error));
      });
      return;
    }
    // Before the tool appears, give the queued INSERT a couple of status ticks;
    // after it was seen, any other tool means the flow ended (cancel refused it).
    if (!current.armed && !tool) {
      current.ticks += 1;
      if (current.ticks <= 2) return;
    }
    pending.current = null;
  }, [status, revision, submitToolText, reportError]);

  return insertBlockByName;
}
