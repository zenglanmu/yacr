// The one pre-save gate of the SAVE funnel, extracted from `App.tsx`
// (file-size ceiling) so the shell keeps only one call site and one render slot.
//
// SAVE itself goes straight to the host, which always writes the native
// `.ymodel` archive (ADR-0041). One question can still stand between an admitted
// SAVE and `cad_command`:
//
//   **The optional embed question** (`commands/save-embed.ts`). Asked only
//   when the host's `cad_ymodel_resource_preview` lists at least one resource;
//   a refusal, an empty list or a non-SAVE line all keep today's immediate
//   dispatch. The answer is applied with `cad_set_embed_resources` and only a
//   successful apply hands the SAVE back to the funnel.
//
// Other output formats do not pass through here at all: `EXPORT` opens the
// host's export picker, whose chosen extension decides the writer, so the chrome
// asks nothing before it.
//
// The flow is deliberately small pieces:
//
//   * `beforeDispatch` decides whether the question is due. It returns `true`
//     when it opened the dialog; the caller then dispatches nothing.
//   * `dialog` is the one modal slot; `isPending` is the stable keyboard guard
//     the shell reads (its Escape must cancel this dialog, not close a panel).
//   * Answering re-enters the funnel through the `redispatch` closure the caller
//     supplied. The embed answer is consumed by the next `beforeDispatch` entry
//     through the one-shot `embedAnswered` ref, so a double click can only
//     dispatch once.
import React, { useCallback, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { invoke } from '../host/invoke.ts';
import { admitCommandText } from '../commands/commands.ts';
import {
  embedNeedsDialog,
  isSaveFamilyCommand,
  readEmbedPreview,
  resolveSaveEmbed,
} from '../commands/save-embed.ts';
import type { EmbedPreview } from '../commands/save-embed.ts';
import { SaveEmbedDialog } from './SaveEmbedDialog.tsx';
import type { CadStatus } from '../cad/cad-status.ts';

interface PendingEmbed {
  readonly preview: EmbedPreview;
  readonly redispatch: () => void;
}

/** The shell-facing controller: the embed gate, one dialog slot, one key guard. */
export interface SaveGateController {
  /**
   * Ask the due pre-save question when `text` is an admitted SAVE-family line.
   *
   * `true` means a dialog opened and nothing was dispatched; `false` means the
   * caller keeps its normal path.
   */
  readonly beforeDispatch: (
    text: string,
    status: CadStatus | null,
    redispatch: () => void,
  ) => Promise<boolean>;
  /** The modal, or `null` when no question is open. */
  readonly dialog: ReactNode;
  /** Whether the question (or the apply it is waiting on) owns Escape/Tab. */
  readonly isPending: () => boolean;
}

export function useSaveGate(): SaveGateController {
  const [active, setActive] = useState<EmbedPreview | null>(null);
  const [embedBusy, setEmbedBusy] = useState(false);
  const [embedError, setEmbedError] = useState<string | null>(null);
  const pendingEmbed = useRef<PendingEmbed | null>(null);
  // The embed answer already applied, waiting for the funnel entry it re-sent.
  const embedAnswered = useRef(false);
  const busy = useRef(false);

  const beforeDispatch = useCallback(async (
    text: string,
    status: CadStatus | null,
    redispatch: () => void,
  ): Promise<boolean> => {
    // Read and clear first: the flag only ever covers the single re-entry the
    // confirm path performed, and a different line must not inherit it.
    const answered = embedAnswered.current;
    embedAnswered.current = false;
    if (!isSaveFamilyCommand(text)) return false;
    if (answered) return false;
    // The question is only due for a line the funnel would really dispatch: a
    // refused SAVE keeps its own reason instead of asking what to embed first.
    if (admitCommandText(text, { status, pending: false }).kind !== 'admitted') return false;
    // `readEmbedPreview` answers `null` for a carrier that cannot preview (and
    // for a payload it cannot read), which keeps today's immediate dispatch with
    // no dialog and no error banner.
    const preview = await readEmbedPreview(invoke);
    if (preview === null || !embedNeedsDialog(preview)) return false;
    setEmbedError(null);
    pendingEmbed.current = { preview, redispatch };
    setActive(preview);
    return true;
  }, []);

  const isPending = useCallback(
    () => pendingEmbed.current !== null || busy.current,
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

  const dialog = active === null ? null : (
    <SaveEmbedDialog
      preview={active}
      busy={embedBusy}
      error={embedError}
      onConfirm={confirmEmbed}
      onCancel={cancelEmbed}
    />
  );

  return { beforeDispatch, dialog, isPending };
}
