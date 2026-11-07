// Vendor notice: the modal a plugin's `command` provider raises when it refuses
// an action it does not support (ADR 0030 D5). The chrome funnel shows it and
// the host-owned modal gate suspends the CAD pointer lane for as long as this
// overlay is mounted.
//
// It reuses the popup layer's visual language (ADR 0021): the same
// `.popup-overlay` backdrop and `.popup-dialog` card, the same header/body/
// footer metrics, so it reads as one more shell dialog and not as a system
// prompt. It is narrower than the three document dialogs because it carries one
// explanation and one button.
//
// The host translates the message and passes it in; this file never invents a
// refusal. Escape and the single button both dismiss, focus starts (and stays)
// on the button, and a composing IME keeps its own keys (the trap itself is the
// shared `notice-focus.ts`).
import React, { useId, useRef } from 'react';
import { createPortal } from 'react-dom';
import { Icon } from '../icons.tsx';
import { Button } from './primitives.tsx';
import { useI18n } from '../context.ts';
import { useNoticeFocus } from './notice-focus.ts';
import {
  DIALOG_BODY,
  DIALOG_FOOT,
  DIALOG_HEAD,
  DIALOG_TITLE,
} from '../popup/popup-layout.ts';

export interface PluginNoticeProps {
  /** The host's translated refusal sentence; shown verbatim. */
  readonly message: string;
  /** Dismiss the notice and release the host's modal gate. */
  readonly onDismiss: () => void;
}

export function PluginNotice({ message, onDismiss }: PluginNoticeProps) {
  const { t } = useI18n();
  const titleId = useId();
  const messageId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  const dismissRef = useRef<HTMLButtonElement>(null);
  useNoticeFocus({ dialog: dialogRef, dismiss: dismissRef }, onDismiss);

  const content = (
    <div className="popup-overlay plugin-notice-overlay">
      <div
        ref={dialogRef}
        className="popup-dialog plugin-notice"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={messageId}
        tabIndex={-1}
      >
        <header className="palette__head plugin-notice__head" style={DIALOG_HEAD}>
          <h2 id={titleId} className="plugin-notice__title" style={DIALOG_TITLE}>
            <Icon name="alert" size={15} />
            {t('vendor.editing.title')}
          </h2>
        </header>
        <div className="plugin-notice__body" style={DIALOG_BODY}>
          <div className="plugin-notice__row">
            <span className="plugin-notice__mark" aria-hidden="true">
              <Icon name="alert" size={18} />
            </span>
            <p id={messageId} className="plugin-notice__message">{message}</p>
          </div>
        </div>
        <footer className="plugin-notice__footer" style={DIALOG_FOOT}>
          <Button
            ref={dismissRef}
            variant="solid"
            onClick={onDismiss}
            aria-label={t('vendor.editing.dismiss')}
          >
            {t('vendor.editing.dismiss')}
          </Button>
        </footer>
      </div>
    </div>
  );

  return typeof document === 'undefined' ? content : createPortal(content, document.body);
}
