// Host notice: one dismissible explanation the shell itself raises (today the
// GPU pulled-copy fallback).
//
// It is the same dialog family as `PluginNotice`: the shared `.popup-overlay`
// backdrop, the same header/body/footer metrics and the same `notice-focus.ts`
// keyboard trap, so it reads as one more shell dialog rather than a system
// prompt. The mark is accented instead of danger red because the message is a
// degraded-performance warning, not a failure the user must fix.
//
// The host translates the strings and passes them in; this file owns no copy.
import React, { useId, useRef } from 'react';
import { createPortal } from 'react-dom';
import { Icon } from '../icons.tsx';
import { Button } from './primitives.tsx';
import { useNoticeFocus } from './notice-focus.ts';
import {
  DIALOG_BODY,
  DIALOG_FOOT,
  DIALOG_HEAD,
  DIALOG_TITLE,
} from '../popup/popup-layout.ts';

export interface HostNoticeProps {
  /** The notice's translated heading. */
  readonly title: string;
  /** The translated explanation; shown verbatim. */
  readonly message: string;
  /** The translated label of the one action, used for its visible text and aria-label. */
  readonly dismissLabel: string;
  /** Close the notice. Escape and the action both call this. */
  readonly onDismiss: () => void;
}

export function HostNotice({ title, message, dismissLabel, onDismiss }: HostNoticeProps) {
  const titleId = useId();
  const messageId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  const dismissRef = useRef<HTMLButtonElement>(null);
  useNoticeFocus({ dialog: dialogRef, dismiss: dismissRef }, onDismiss);

  const content = (
    <div className="popup-overlay host-notice-overlay">
      <div
        ref={dialogRef}
        className="popup-dialog host-notice"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={messageId}
        tabIndex={-1}
      >
        <header className="palette__head host-notice__head" style={DIALOG_HEAD}>
          <h2 id={titleId} className="host-notice__title" style={DIALOG_TITLE}>
            <Icon name="alert" size={15} />
            {title}
          </h2>
        </header>
        <div className="host-notice__body" style={DIALOG_BODY}>
          <div className="host-notice__row">
            <span className="host-notice__mark" aria-hidden="true">
              <Icon name="alert" size={18} />
            </span>
            <p id={messageId} className="host-notice__message">{message}</p>
          </div>
        </div>
        <footer className="host-notice__footer" style={DIALOG_FOOT}>
          <Button
            ref={dismissRef}
            variant="solid"
            onClick={onDismiss}
            aria-label={dismissLabel}
          >
            {dismissLabel}
          </Button>
        </footer>
      </div>
    </div>
  );

  return typeof document === 'undefined' ? content : createPortal(content, document.body);
}
