// The experimental-DWG save confirmation's state, extracted from `App.tsx`
// (file-size ceiling) so the shell keeps only one call site and one render slot.
//
// The flow is deliberately three small pieces:
//
//   1. `beforeDispatch` runs **before** the command funnel sends anything. It
//      only looks at SAVE-family lines on a `.dwg` source; for those it awaits
//      the host vocabulary and asks the same admission the funnel asks, so a
//      refused SAVE still reports its own reason instead of opening a question.
//   2. `dialog` is the modal; `isPending` is the stable keyboard guard the shell
//      reads (its Escape must cancel this dialog, not close a panel).
//   3. The pending entry is cleared **before** `redispatch` runs, so a double
//      click (or a repeated activation) can only dispatch once. Cancel never
//      calls it.
import React, { useCallback, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { admitCommandText } from '../commands/commands.ts';
import {
  dwgSaveConfirmationRequest,
  isDwgSaveCandidate,
  resolveDwgSaveConfirmation,
} from '../commands/save-confirm.ts';
import type { DwgSaveConfirmAction, DwgSaveConfirmation } from '../commands/save-confirm.ts';
import { DwgSaveConfirmDialog } from './DwgSaveConfirmDialog.tsx';
import type { CadStatus } from '../cad/cad-status.ts';

/** The in-flight host vocabulary promise admission awaits. */
export interface DwgSaveConfirmVocabulary {
  readonly current: Promise<unknown> | null;
}

/** The shell-facing controller: one gate, one dialog slot, one keyboard guard. */
export interface DwgSaveConfirmController {
  /**
   * Ask for confirmation when `text` is an admitted SAVE-family line on a
   * `.dwg` source. `true` means the dialog opened and nothing was dispatched;
   * `false` means the caller keeps its normal path.
   */
  readonly beforeDispatch: (
    text: string,
    status: CadStatus | null,
    documentLabel: string | null,
    vocabulary: DwgSaveConfirmVocabulary,
    redispatch: () => void,
  ) => Promise<boolean>;
  /** The modal, or `null` when no confirmation is open. */
  readonly dialog: ReactNode;
  /** Whether the confirmation currently owns Escape/Tab. */
  readonly isPending: () => boolean;
}

interface PendingDwgSave {
  readonly confirmation: DwgSaveConfirmation;
  readonly redispatch: () => void;
}

export function useDwgSaveConfirm(): DwgSaveConfirmController {
  const [active, setActive] = useState<DwgSaveConfirmation | null>(null);
  const pending = useRef<PendingDwgSave | null>(null);

  const beforeDispatch = useCallback(async (
    text: string,
    status: CadStatus | null,
    documentLabel: string | null,
    vocabulary: DwgSaveConfirmVocabulary,
    redispatch: () => void,
  ): Promise<boolean> => {
    if (!isDwgSaveCandidate(text, documentLabel)) return false;
    // A cold carrier may still be loading its vocabulary; the same wait the
    // command funnel performs applies here, so the first SAVE is not refused.
    try { await vocabulary.current; } catch { /* reported by the loader */ }
    const admission = admitCommandText(text, { status, pending: false });
    const confirmation = admission.kind === 'admitted'
      ? dwgSaveConfirmationRequest(admission.command, documentLabel)
      : null;
    if (confirmation === null) return false;
    pending.current = { confirmation, redispatch };
    setActive(confirmation);
    return true;
  }, []);

  const resolve = useCallback((action: DwgSaveConfirmAction) => {
    const pendingSave = pending.current;
    // One shot: clear first, then decide. A second answer finds nothing.
    pending.current = null;
    const resolved = resolveDwgSaveConfirmation(pendingSave?.confirmation ?? null, action);
    setActive(null);
    if (pendingSave !== null && resolved.dispatch !== null) pendingSave.redispatch();
  }, []);
  const proceed = useCallback(() => resolve('proceed'), [resolve]);
  const cancel = useCallback(() => resolve('cancel'), [resolve]);
  const isPending = useCallback(() => pending.current !== null, []);

  const dialog = active === null ? null : (
    <DwgSaveConfirmDialog
      exportName={active.exportName}
      onProceed={proceed}
      onCancel={cancel}
    />
  );

  return { beforeDispatch, dialog, isPending };
}
