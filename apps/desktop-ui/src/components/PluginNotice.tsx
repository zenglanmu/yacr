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
// on the button, and a composing IME keeps its own keys.
import React, { useEffect, useId, useRef } from 'react';
import { createPortal } from 'react-dom';
import { Icon } from '../icons.tsx';
import { Button } from './primitives.tsx';
import { useI18n } from '../context.ts';
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

const FOCUSABLE = [
  'button:not([disabled])',
  '[href]',
  'input:not([disabled])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  '[tabindex]:not([tabindex="-1"])',
].join(', ');

export function PluginNotice({ message, onDismiss }: PluginNoticeProps) {
  const { t } = useI18n();
  const titleId = useId();
  const messageId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  const dismissRef = useRef<HTMLButtonElement>(null);

  // The notice owns the keyboard while it is open: focus starts on the only
  // action, Tab cannot leave the card, and Escape dismisses exactly once. The
  // shell's global handler is expected to stand down for the host's modal gate;
  // `preventDefault` keeps the key from also reaching a focused field.
  useEffect(() => {
    dismissRef.current?.focus();
  }, []);

  useEffect(() => {
    const node = dialogRef.current;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.isComposing || event.keyCode === 229) return;
      if (event.key === 'Escape') {
        if (event.defaultPrevented) return;
        event.preventDefault();
        onDismiss();
        return;
      }
      if (event.key !== 'Tab' || !node) return;
      const focusable = Array.from(node.querySelectorAll<HTMLElement>(FOCUSABLE))
        .filter(element => element.getClientRects().length > 0);
      if (focusable.length === 0) {
        event.preventDefault();
        node.focus();
        return;
      }
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      const active = document.activeElement;
      const inside = active !== null && node.contains(active);
      if (event.shiftKey && (!inside || active === first)) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && (!inside || active === last)) {
        event.preventDefault();
        first.focus();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [onDismiss]);

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
