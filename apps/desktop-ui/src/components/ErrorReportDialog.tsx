// The global error dialog.
//
// Portal to `document.body` from the error-report root, not from the shell tree:
// the card must still exist when the shell itself failed to render. The body is
// the scroller and every message/stack run is real selectable text — the whole
// point is to let a user copy a Rust diagnostic out of the window without
// retyping it.
//
// Sizing lives in CSS (`style.css` for the desktop card, `mobile.css` for the
// phone bottom sheet). The inline metrics below are the same `popup-layout.ts`
// data every other dialog uses.
import React, { useCallback, useEffect, useId, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Icon } from '../icons.tsx';
import { Button, IconButton } from './primitives.tsx';
import { useErrorDialogFocus } from './error-dialog-focus.ts';
import {
  dismissErrorReport,
  errorKindLabelKey,
  formatErrorReport,
  showNewerErrorReport,
  showOlderErrorReport,
} from '../errors/error-report.ts';
import type { ErrorReport } from '../errors/error-report.ts';
import { writeClipboardText } from '../clipboard.ts';
import type { ClipboardEnvironment } from '../clipboard.ts';
import type { Translator } from '../i18n.ts';
import {
  DIALOG_BODY,
  DIALOG_CARD,
  DIALOG_FOOT,
  DIALOG_HEAD,
  DIALOG_TITLE,
} from '../popup/popup-layout.ts';

export interface ErrorReportDialogProps {
  readonly report: ErrorReport;
  /** 1-based position of the visible report (for the `{index} / {total}` line). */
  readonly index: number;
  readonly total: number;
  readonly t: Translator;
  /** Clipboard injection point for tests; the real browser globals are default. */
  readonly clipboardEnv?: ClipboardEnvironment;
}

type CopyState = 'copied' | 'failed' | null;

/** The raw stack/detail run shown in the monospace well. */
function stackTextOf(report: ErrorReport): string {
  const stack = typeof report.stack === 'string' && report.stack.trim() !== '' ? report.stack : '';
  const detail = typeof report.detail === 'string'
    && report.detail.trim() !== ''
    && report.detail !== report.message
    ? report.detail
    : '';
  return [stack, detail && detail !== stack ? detail : ''].filter(Boolean).join('\n\n');
}

export function ErrorReportDialog({ report, index, total, t, clipboardEnv }: ErrorReportDialogProps) {
  const titleId = useId();
  const messageId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const backdropPressed = useRef(false);
  const [copyState, setCopyState] = useState<CopyState>(null);
  useErrorDialogFocus({ dialog: dialogRef, initial: closeRef }, dismissErrorReport);

  // A new report (navigation or a fresh capture) starts with a clean copy state;
  // "Copied" must never sit under a stack that was not the one copied.
  useEffect(() => {
    setCopyState(null);
  }, [report.id]);

  const onCopy = useCallback(async () => {
    const copied = await writeClipboardText(formatErrorReport(report, t), clipboardEnv);
    setCopyState(copied ? 'copied' : 'failed');
  }, [report, t, clipboardEnv]);

  function onBackdropPointerDown(event: React.PointerEvent<HTMLDivElement>): void {
    // Remember whether the gesture *started* on the backdrop. A drag that begins
    // inside the card (selecting stack text) and ends outside must not close it.
    backdropPressed.current = event.target === event.currentTarget;
  }

  function onBackdropClick(event: React.MouseEvent<HTMLDivElement>): void {
    const startedOnBackdrop = backdropPressed.current;
    backdropPressed.current = false;
    if (startedOnBackdrop && event.target === event.currentTarget) dismissErrorReport();
  }

  const copyLabel = copyState === 'copied'
    ? t('error.dialog.copied')
    : copyState === 'failed'
      ? t('error.dialog.copy_failed')
      : t('error.dialog.copy');
  const stackText = stackTextOf(report);

  const content = (
    <div
      className="popup-overlay error-report-overlay"
      onPointerDown={onBackdropPointerDown}
      onClick={onBackdropClick}
    >
      <div
        ref={dialogRef}
        className="popup-dialog error-report"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={messageId}
        tabIndex={-1}
        style={DIALOG_CARD}
      >
        <header className="palette__head error-report__head" style={DIALOG_HEAD}>
          <h2 id={titleId} className="error-report__title" style={DIALOG_TITLE}>
            <Icon name="alert" size={15} />
            {t('error.dialog.title')}
          </h2>
          <IconButton
            ref={closeRef}
            icon="close"
            size="xs"
            label={t('error.dialog.close')}
            onClick={dismissErrorReport}
          />
        </header>

        <div className="error-report__body" style={DIALOG_BODY}>
          <div className="error-report__meta">
            <span className="error-report__kind">{t(errorKindLabelKey(report.kind))}</span>
            <span className="error-report__time">{t('error.label.time')} · {report.at}</span>
            {total > 1 ? (
              <span className="error-report__nav">
                <IconButton
                  icon="chevronLeft"
                  size="xs"
                  label={t('error.dialog.earlier')}
                  disabled={index <= 1}
                  onClick={showOlderErrorReport}
                />
                <span className="error-report__position">
                  {t('error.dialog.position', { index, total })}
                </span>
                <IconButton
                  icon="chevronRight"
                  size="xs"
                  label={t('error.dialog.later')}
                  disabled={index >= total}
                  onClick={showNewerErrorReport}
                />
              </span>
            ) : null}
          </div>

          <div className="error-report__field">
            <span className="error-report__field-label">{t('error.label.message')}</span>
            <p id={messageId} className="error-report__message">{report.message}</p>
          </div>

          {report.location ? (
            <div className="error-report__field">
              <span className="error-report__field-label">{t('error.label.location')}</span>
              <p className="error-report__location">{report.location}</p>
            </div>
          ) : null}

          {stackText ? (
            <div className="error-report__field">
              <span className="error-report__field-label">{t('error.label.stack')}</span>
              <pre className="error-report__pre">{stackText}</pre>
            </div>
          ) : null}
        </div>

        <footer className="error-report__foot" style={DIALOG_FOOT}>
          <p className="error-report__hint">{t('error.dialog.hint')}</p>
          <Button icon="copy" iconSize={14} onClick={onCopy}>{copyLabel}</Button>
        </footer>
      </div>
    </div>
  );

  return typeof document === 'undefined' ? content : createPortal(content, document.body);
}
