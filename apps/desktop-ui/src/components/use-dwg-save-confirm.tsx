// The pre-save gates of the one SAVE funnel, extracted from `App.tsx`
// (file-size ceiling) so the shell keeps only one call site and one render slot.
//
// Two questions can stand between an admitted SAVE and `cad_command`:
//
//   1. **The experimental-DWG caution.** On a `.dwg` source the host writes a NEW
//      `<stem>-export.dwg` instead of overwriting the opened drawing, so the user
//      is told before the picker exists. `beforeDispatch` awaits the host
//      vocabulary and asks the same admission the funnel asks, so a refused SAVE
//      still reports its own reason instead of opening a question.
//   2. **The optional embed question** (`commands/save-embed.ts`). Asked only
//      when the host's `cad_ymodel_resource_preview` lists at least one resource;
//      a refusal, an empty list or a non-SAVE line all keep today's immediate
//      dispatch. The answer is applied with `cad_set_embed_resources` and only a
//      successful apply hands the SAVE back to the funnel.
//
// The flow is deliberately small pieces:
//
//   * `beforeDispatch` decides which question (if either) is due. It returns
//     `true` when it opened a dialog; the caller then dispatches nothing.
//   * `dialog` is the one modal slot; `isPending` is the stable keyboard guard
//     the shell reads (its Escape must cancel this dialog, not close a panel).
//   * Answering re-enters the funnel through the `redispatch` closure the caller
//     supplied. The DWG answer travels as the caller's own `skipDwg` flag; the
//     embed answer is consumed by the next `beforeDispatch` entry through the
//     one-shot `embedAnswered` ref, so a double click can only dispatch once.
import React, { useCallback, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { invoke } from '../host/invoke.ts';
import { admitCommandText } from '../commands/commands.ts';
import {
  dwgSaveConfirmationRequest,
  isDwgSaveCandidate,
  resolveDwgSaveConfirmation,
} from '../commands/save-confirm.ts';
import type { DwgSaveConfirmAction, DwgSaveConfirmation } from '../commands/save-confirm.ts';
import {
  embedNeedsDialog,
  isSaveFamilyCommand,
  readEmbedPreview,
  resolveSaveEmbed,
} from '../commands/save-embed.ts';
import type { EmbedPreview } from '../commands/save-embed.ts';
import { DwgSaveConfirmDialog } from './DwgSaveConfirmDialog.tsx';
import { SaveEmbedDialog } from './SaveEmbedDialog.tsx';
import type { CadStatus } from '../cad/cad-status.ts';

/** The in-flight host vocabulary promise admission awaits. */
export interface DwgSaveConfirmVocabulary {
  readonly current: Promise<unknown> | null;
}

/** The one dialog slot: the DWG caution, or the embed question. */
type ActiveSaveDialog =
  | DwgSaveConfirmation
  | { readonly kind: 'embed'; readonly preview: EmbedPreview };

/** The shell-facing controller: the gate chain, one dialog slot, one key guard. */
export interface SaveGuardController {
  /**
   * Ask the due pre-save question when `text` is an admitted SAVE-family line.
   *
   * `skipDwg` is the caller's own "the DWG caution was answered" flag (it re-sends
   * the same line through this hook). `true` means a dialog opened and nothing was
   * dispatched; `false` means the caller keeps its normal path.
   */
  readonly beforeDispatch: (
    text: string,
    status: CadStatus | null,
    documentLabel: string | null,
    vocabulary: DwgSaveConfirmVocabulary,
    skipDwg: boolean,
    redispatch: () => void,
  ) => Promise<boolean>;
  /** The modal, or `null` when no question is open. */
  readonly dialog: ReactNode;
  /** Whether a question (or the apply it is waiting on) owns Escape/Tab. */
  readonly isPending: () => boolean;
}

interface PendingDwgSave {
  readonly confirmation: DwgSaveConfirmation;
  readonly redispatch: () => void;
}

interface PendingEmbed {
  readonly preview: EmbedPreview;
  readonly redispatch: () => void;
}

export function useDwgSaveConfirm(): SaveGuardController {
  const [active, setActive] = useState<ActiveSaveDialog | null>(null);
  const [embedBusy, setEmbedBusy] = useState(false);
  const [embedError, setEmbedError] = useState<string | null>(null);
  const pending = useRef<PendingDwgSave | null>(null);
  const pendingEmbed = useRef<PendingEmbed | null>(null);
  // The embed answer already applied, waiting for the funnel entry it re-sent.
  const embedAnswered = useRef(false);
  const busy = useRef(false);

  const beforeDispatch = useCallback(async (
    text: string,
    status: CadStatus | null,
    documentLabel: string | null,
    vocabulary: DwgSaveConfirmVocabulary,
    skipDwg: boolean,
    redispatch: () => void,
  ): Promise<boolean> => {
    // Read and clear first: the flag only ever covers the single re-entry the
    // confirm path performed, and a different line must not inherit it.
    const answered = embedAnswered.current;
    embedAnswered.current = false;
    if (!isSaveFamilyCommand(text)) return false;
    if (answered) return false;
    if (!skipDwg && isDwgSaveCandidate(text, documentLabel)) {
      // A cold carrier may still be loading its vocabulary; the same wait the
      // command funnel performs applies here, so the first SAVE is not refused.
      try { await vocabulary.current; } catch { /* reported by the loader */ }
      const admission = admitCommandText(text, { status, pending: false });
      const confirmation = admission.kind === 'admitted'
        ? dwgSaveConfirmationRequest(admission.command, documentLabel)
        : null;
      if (confirmation !== null) {
        pending.current = { confirmation, redispatch };
        setActive(confirmation);
        return true;
      }
    }
    // The question is only due for a line the funnel would really dispatch: a
    // refused SAVE keeps its own reason instead of asking what to embed first.
    if (admitCommandText(text, { status, pending: false }).kind !== 'admitted') return false;
    // The optional embed question. `readEmbedPreview` answers `null` for a
    // carrier that cannot preview (and for a payload it cannot read), which keeps
    // today's immediate dispatch with no dialog and no error banner.
    const preview = await readEmbedPreview(invoke);
    if (preview === null || !embedNeedsDialog(preview)) return false;
    setEmbedError(null);
    pendingEmbed.current = { preview, redispatch };
    setActive({ kind: 'embed', preview });
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
  const isPending = useCallback(
    () => pending.current !== null || pendingEmbed.current !== null || busy.current,
    [],
  );

  /**
   * Apply the embed choice, then hand SAVE back to the funnel.
   *
   * A refusal keeps this dialog open with the host's own sentence and dispatches
   * nothing: saving anyway would silently ignore the answer the user just gave.
   */
  const confirmEmbed = useCallback(async (embed: boolean) => {
    const queued = pendingEmbed.current;
    if (queued === null || busy.current) return;
    busy.current = true;
    setEmbedBusy(true);
    setEmbedError(null);
    const outcome = await resolveSaveEmbed('confirm', embed, {
      invoke,
      dispatchSave: () => {
        embedAnswered.current = true;
        queued.redispatch();
      },
    });
    busy.current = false;
    setEmbedBusy(false);
    if (outcome.kind === 'failed') {
      setEmbedError(outcome.error);
      return;
    }
    pendingEmbed.current = null;
    setActive(null);
  }, []);

  /** Cancel aborts the whole save: nothing is applied and no command is sent. */
  const cancelEmbed = useCallback(() => {
    if (busy.current) return;
    pendingEmbed.current = null;
    setEmbedError(null);
    setActive(null);
  }, []);

  const dialog = active === null ? null : 'kind' in active ? (
    <SaveEmbedDialog
      preview={active.preview}
      busy={embedBusy}
      error={embedError}
      onConfirm={confirmEmbed}
      onCancel={cancelEmbed}
    />
  ) : (
    <DwgSaveConfirmDialog
      exportName={active.exportName}
      onProceed={proceed}
      onCancel={cancel}
    />
  );

  return { beforeDispatch, dialog, isPending };
}
