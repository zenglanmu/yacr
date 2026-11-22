// The open-time format provenance notice (ADR-0041 D5).
//
// A document opened from a format whose specification is not published says so
// once, when it opens: the reader is an independent reconstruction, exporting
// back may not be accepted by the originating application, and some information
// is not modelled here. That is news about the format, not a failure of the
// open - so this is the `UpdateNotice` pattern, never the modal host notice:
//
//   * an `aside` in a pointer-transparent layer docked above the status bar
//     (`Workspace` mounts it inside the workspace, so the native CAD surface
//     rect never changes);
//   * `role="status"` + `aria-live="polite"`: announced, never interrupting;
//   * no backdrop, no focus trap, no global key handler, and the card itself is
//     not focusable - Escape still belongs to the CAD session, and the only
//     focusable nodes are the two real buttons;
//   * the copy is entirely `cad/opened-source.ts`; this file only lays it out.
//
// Two answers, two scopes. "Got it" hides the card for this open only and writes
// nothing (`onDismiss`, an in-memory key in the host). "Don't show again" is the
// durable answer: it writes the `formatNoticeEnabled` preference, waits for the
// host's own re-read to show it, and only then silences the card - which is why
// the write lives here, beside the busy and failure states it produces.
import React, { useId, useState } from 'react';
import { Icon } from '../icons.tsx';
import { Button } from './primitives.tsx';
import { useI18n } from '../context.ts';
import { openNoticeView } from '../cad/opened-source.ts';
import type { CadOpenedSource } from '../cad/opened-source.ts';
import { disableFormatNotice } from '../cad/format-notice-setting.ts';
import { invoke } from '../host/invoke.ts';

export interface FormatProvenanceNoticeProps {
  /** The validated `cad_status.openedSource` block for the current document. */
  readonly source: CadOpenedSource;
  /** Hide the notice for this open only; nothing is written. */
  readonly onDismiss: () => void;
  /**
   * The host confirmed `formatNoticeEnabled: false`: this open is silenced too.
   * Called only after the read-back, never on the queue acknowledgement.
   */
  readonly onDisabled: () => void;
}

export function FormatProvenanceNotice({ source, onDismiss, onDisabled }: FormatProvenanceNoticeProps) {
  const { t } = useI18n();
  const titleId = useId();
  const [pending, setPending] = useState(false);
  const [failed, setFailed] = useState(false);
  const view = openNoticeView(source, t);
  const dismissLabel = t('open.notice.dismiss');
  const disableLabel = t('open.notice.dont_show_again');

  const turnOff = async () => {
    // One press at a time: the write plus its read-back takes a few ticks, and a
    // second press would queue a duplicate setting write.
    if (pending) return;
    setPending(true);
    setFailed(false);
    const result = await disableFormatNotice(invoke);
    setPending(false);
    if (result.kind === 'ok') {
      onDisabled();
      return;
    }
    // Nothing was confirmed, so nothing pretends it was: the card stays up and
    // says so. On a read-only carrier this is the stable outcome, not a glitch.
    setFailed(true);
  };

  return (
    <aside
      className="format-notice"
      role="status"
      aria-live="polite"
      aria-labelledby={titleId}
    >
      <span className="format-notice__mark" aria-hidden="true">
        <Icon name="info" size={18} />
      </span>
      <div className="format-notice__content">
        <p id={titleId} className="format-notice__title">{view.title}</p>
        {view.lines.map(line => (
          <p
            key={line.key}
            className={line.key === 'ymodel'
              ? 'format-notice__body is-recommend'
              : 'format-notice__body'}
          >
            {line.text}
          </p>
        ))}
      </div>
      <div className="format-notice__actions">
        {/* A real button, not a clickable card: the card stays unfocusable and
            carries no dismiss handler of its own. "Got it" is the primary
            answer (the filled chip); "Don't show again" is the quieter text
            button, because it changes a preference rather than closing a card. */}
        <Button
          variant="chip"
          size="sm"
          aria-label={dismissLabel}
          disabled={pending}
          onClick={onDismiss}
        >
          {dismissLabel}
        </Button>
        <Button
          variant="ghost"
          size="sm"
          aria-label={disableLabel}
          aria-busy={pending ? true : undefined}
          disabled={pending}
          onClick={() => { void turnOff(); }}
        >
          {disableLabel}
        </Button>
      </div>
      {failed ? (
        // An `alert`, not a toast: it belongs to the button that failed, and it
        // must not steal focus from the CAD session.
        <p className="format-notice__alert" role="alert">
          {t('open.notice.dont_show_again_failed')}
        </p>
      ) : null}
    </aside>
  );
}
