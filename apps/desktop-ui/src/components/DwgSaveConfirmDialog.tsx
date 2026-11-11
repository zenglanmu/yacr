// The experimental-DWG save confirmation.
//
// SAVE on a `.dwg` source never overwrites the opened drawing: the host offers a
// NEW `<stem>-export.dwg`. That is easy to miss in a save picker, so the shell
// asks here, before the picker exists, and names the file it is about to suggest.
//
// This is the same in-DOM dialog family as the host notices and the ADR 0021
// popups: the shared `.popup-overlay` backdrop, the shared header/body/footer
// metrics, one warning mark and one button row. It is portalled to `document.body`
// so it sits above the shell (and the native CAD partition). Escape and the
// Cancel button both dismiss without dispatching anything; focus starts on
// Cancel and cannot leave the card (the trap lives in `confirm-focus.ts`).
//
// All copy comes from the shared catalogs; this file owns no sentence.
import React, { useId, useRef } from 'react';
import { createPortal } from 'react-dom';
import { Icon } from '../icons.tsx';
import { Button } from './primitives.tsx';
import { useI18n } from '../context.ts';
import { useConfirmDialogFocus } from './confirm-focus.ts';
import {
  DIALOG_BODY,
  DIALOG_FOOT,
  DIALOG_HEAD,
  DIALOG_TITLE,
} from '../popup/popup-layout.ts';

export interface DwgSaveConfirmDialogProps {
  /** The host's default new-file name, shown inside the body copy. */
  readonly exportName: string;
  /** Dispatch the SAVE-family command exactly once. */
  readonly onProceed: () => void;
  /** Close the confirmation and dispatch nothing. */
  readonly onCancel: () => void;
}

export function DwgSaveConfirmDialog({ exportName, onProceed, onCancel }: DwgSaveConfirmDialogProps) {
  const { t } = useI18n();
  const titleId = useId();
  const bodyId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  useConfirmDialogFocus({ dialog: dialogRef, initial: cancelRef }, onCancel);

  const content = (
    <div className="popup-overlay dwg-save-confirm-overlay">
      <div
        ref={dialogRef}
        className="popup-dialog dwg-save-confirm"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={bodyId}
        tabIndex={-1}
      >
        <header className="palette__head dwg-save-confirm__head" style={DIALOG_HEAD}>
          <h2 id={titleId} className="dwg-save-confirm__title" style={DIALOG_TITLE}>
            <Icon name="alert" size={15} />
            {t('save.dwg_confirm.title')}
          </h2>
        </header>
        <div className="dwg-save-confirm__body" style={DIALOG_BODY}>
          <div className="dwg-save-confirm__row">
            <span className="dwg-save-confirm__mark" aria-hidden="true">
              <Icon name="save" size={18} />
            </span>
            <p id={bodyId} className="dwg-save-confirm__message">
              {t('save.dwg_confirm.body', { name: exportName })}
            </p>
          </div>
        </div>
        <footer className="dwg-save-confirm__footer" style={DIALOG_FOOT}>
          <Button ref={cancelRef} variant="ghost" onClick={onCancel}>
            {t('save.dwg_confirm.cancel')}
          </Button>
          <Button variant="solid" icon="save" iconSize={14} onClick={onProceed}>
            {t('save.dwg_confirm.proceed')}
          </Button>
        </footer>
      </div>
    </div>
  );

  return typeof document === 'undefined' ? content : createPortal(content, document.body);
}
